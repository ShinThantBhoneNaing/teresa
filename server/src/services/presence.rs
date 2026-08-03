//! Connection lifecycle, status, profile.

use teresa_proto::{validate, ConnectionId, ServerFrame, Timestamp, User, UserId, UserStatus};

use crate::error::{Result, ServerError};
use crate::hub::ConnectionSender;
use crate::repo::UserRow;

use super::{Core, Store};

impl<S: Store> Core<S> {
    /// Registers a live connection and assembles its `Ready` frame.
    ///
    /// If this is the account's first connection, the rest of the server hears
    /// a presence change. The connection cap is enforced here — registering
    /// and then immediately unregistering on overflow keeps the count exact
    /// under concurrent connects.
    pub async fn connect(
        &self,
        user: &UserRow,
        conn: ConnectionId,
        sender: ConnectionSender,
    ) -> Result<ServerFrame> {
        let count = self.hub().register(user.id, conn, sender);
        if count > self.config().limits.connections_per_user {
            self.hub().unregister(user.id, conn);
            return Err(ServerError::Forbidden(
                "too many simultaneous connections for this account",
            ));
        }

        if count == 1 {
            self.store()
                .touch_last_seen(user.id, Timestamp::now())
                .await?;
            self.hub().publish_global(ServerFrame::Presence {
                user: user.id,
                status: user.status,
                text: user.status_text.clone(),
            });
        }

        let mut rooms = Vec::new();
        for row in self.store().joined_by(user.id).await? {
            rooms.push(self.project_joined_room(row, user.id).await?);
        }

        Ok(ServerFrame::Ready {
            user: user.to_user(true),
            rooms,
            server: self.server_info(),
        })
    }

    /// Removes a connection; announces offline if it was the last.
    pub async fn disconnect(&self, user: UserId, conn: ConnectionId) -> Result<()> {
        if self.hub().unregister(user, conn) {
            self.store().touch_last_seen(user, Timestamp::now()).await?;
            self.hub().publish_global(ServerFrame::Presence {
                user,
                status: UserStatus::Offline,
                text: None,
            });
        }
        Ok(())
    }

    /// Changes an account's presence and broadcasts it.
    pub async fn set_status(
        &self,
        actor: &UserRow,
        status: UserStatus,
        text: Option<&str>,
    ) -> Result<()> {
        if status == UserStatus::Offline {
            return Err(ServerError::BadRequest(
                "offline is not a status you can set; disconnect instead".to_owned(),
            ));
        }
        let text = match text {
            Some(raw) => validate::status_text(raw)?,
            None => None,
        };

        self.store()
            .set_status(actor.id, status, text.as_deref())
            .await?;
        self.hub().publish_global(ServerFrame::Presence {
            user: actor.id,
            status,
            text,
        });
        Ok(())
    }

    /// Changes a display name and broadcasts the new profile.
    ///
    /// Returns the updated wire profile so the caller can refresh its cached
    /// copy of the actor.
    pub async fn set_display_name(&self, actor: &UserRow, display_name: &str) -> Result<User> {
        let display_name = validate::display_name(display_name)?;
        self.store()
            .set_display_name(actor.id, display_name.as_str())
            .await?;

        let mut updated = actor.clone();
        updated.display_name = display_name;

        let user = updated.to_user(self.hub().is_live(updated.id));
        self.hub()
            .publish_global(ServerFrame::ProfileUpdated { user: user.clone() });
        Ok(user)
    }

    /// Relays a typing indicator to a room's subscribers.
    ///
    /// Membership is checked — otherwise anyone could haunt rooms they cannot
    /// even see with phantom "is typing" lines.
    pub async fn typing(
        &self,
        actor: &UserRow,
        room: teresa_proto::RoomId,
        active: bool,
    ) -> Result<()> {
        if self.store().membership(room, actor.id).await?.is_none() {
            return Err(ServerError::Forbidden("not a member of that room"));
        }
        self.hub().publish_room(
            room,
            ServerFrame::Typing {
                room,
                user: actor.id,
                display_name: actor.display_name.clone(),
                active,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::super::testkit;
    use super::*;
    use crate::repo::{RoomRepo, UserRepo};

    /// Connects a registered user on a fresh connection id, returning the
    /// pieces a connection actor would hold.
    async fn connect_user(
        core: &Core<crate::repo::sqlite::SqliteStore>,
        token: &str,
    ) -> (
        UserRow,
        ConnectionId,
        ServerFrame,
        mpsc::Receiver<crate::hub::SharedFrame>,
    ) {
        let row = core.authenticate(token).await.unwrap();
        let conn = ConnectionId::new();
        let (tx, rx) = mpsc::channel(64);
        let ready = core.connect(&row, conn, tx).await.unwrap();
        (row, conn, ready, rx)
    }

    #[tokio::test]
    async fn ready_carries_the_profile_and_room_list() {
        let core = testkit::core().await;
        let (token, _) = testkit::register(&core, "neo").await;
        let (_, _, ready, _rx) = connect_user(&core, &token).await;

        let ServerFrame::Ready {
            user,
            rooms,
            server,
        } = ready
        else {
            panic!("expected Ready, got {ready:?}");
        };
        assert_eq!(user.username, "neo");
        assert_eq!(user.status, UserStatus::Online);
        assert_eq!(rooms.len(), 1, "registration joined #general");
        assert_eq!(rooms[0].room.name, "general");
        assert_eq!(server.protocol, teresa_proto::PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn the_first_connection_announces_presence_and_the_last_announces_offline() {
        let core = testkit::core().await;
        let (token, user) = testkit::register(&core, "neo").await;
        let mut global = core.hub().subscribe_global();

        let (row, first_conn, _, _rx1) = connect_user(&core, &token).await;

        // First connect → Presence online.
        let frame = global.recv().await.unwrap();
        assert!(matches!(
            frame.as_ref(),
            ServerFrame::Presence { user: u, status: UserStatus::Online, .. } if *u == user.id
        ));

        // Second terminal: no new announcement.
        let (_, second_conn, _, _rx2) = connect_user(&core, &token).await;
        assert!(global.try_recv().is_err(), "second connect must be silent");

        // Closing the first terminal: still online elsewhere, still silent.
        core.disconnect(row.id, first_conn).await.unwrap();
        assert!(global.try_recv().is_err());

        // Closing the last: offline.
        core.disconnect(row.id, second_conn).await.unwrap();
        let frame = global.recv().await.unwrap();
        assert!(matches!(
            frame.as_ref(),
            ServerFrame::Presence {
                status: UserStatus::Offline,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn the_connection_cap_is_enforced() {
        let core = testkit::core().await;
        let (token, _) = testkit::register(&core, "neo").await;
        let row = core.authenticate(&token).await.unwrap();

        let cap = core.config().limits.connections_per_user;
        let mut receivers = Vec::new();
        for _ in 0..cap {
            let (tx, rx) = mpsc::channel(8);
            core.connect(&row, ConnectionId::new(), tx).await.unwrap();
            receivers.push(rx);
        }

        let (tx, _rx) = mpsc::channel(8);
        let err = core
            .connect(&row, ConnectionId::new(), tx)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Forbidden(_)));
        // The refused connection must not have leaked into the count.
        assert_eq!(core.hub().connection_count(), cap);
    }

    #[tokio::test]
    async fn status_changes_broadcast_and_validate() {
        let core = testkit::core().await;
        let (token, user) = testkit::register(&core, "neo").await;
        let row = core.authenticate(&token).await.unwrap();
        let mut global = core.hub().subscribe_global();

        core.set_status(&row, UserStatus::Coding, Some("  in the construct  "))
            .await
            .unwrap();

        let frame = global.recv().await.unwrap();
        match frame.as_ref() {
            ServerFrame::Presence {
                user: u,
                status,
                text,
            } => {
                assert_eq!(*u, user.id);
                assert_eq!(*status, UserStatus::Coding);
                assert_eq!(text.as_deref(), Some("in the construct"));
            }
            other => panic!("expected Presence, got {other:?}"),
        }

        // And it persisted.
        let stored = core.store().user_by_id(user.id).await.unwrap().unwrap();
        assert_eq!(stored.status, UserStatus::Coding);

        // Offline cannot be chosen.
        let err = core
            .set_status(&row, UserStatus::Offline, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::BadRequest(_)));
    }

    #[tokio::test]
    async fn nick_changes_broadcast_the_new_profile() {
        let core = testkit::core().await;
        let (token, _) = testkit::register(&core, "neo").await;
        let row = core.authenticate(&token).await.unwrap();
        let mut global = core.hub().subscribe_global();

        let updated = core.set_display_name(&row, "The One").await.unwrap();
        assert_eq!(updated.display_name, "The One");
        assert_eq!(updated.username, "neo", "the handle never changes");

        let frame = global.recv().await.unwrap();
        assert!(matches!(
            frame.as_ref(),
            ServerFrame::ProfileUpdated { user } if user.display_name == "The One"
        ));
    }

    #[tokio::test]
    async fn typing_requires_membership() {
        let core = testkit::core().await;
        let (token, _) = testkit::register(&core, "neo").await;
        let row = core.authenticate(&token).await.unwrap();

        let general = core.store().room_by_name("general").await.unwrap().unwrap();
        let random = core.store().room_by_name("random").await.unwrap().unwrap();

        let mut room_rx = core.hub().subscribe_room(general.id);
        core.typing(&row, general.id, true).await.unwrap();
        assert!(matches!(
            room_rx.recv().await.unwrap().as_ref(),
            ServerFrame::Typing { active: true, .. }
        ));

        // Registered users join #general only; #random is off limits.
        let err = core.typing(&row, random.id, true).await.unwrap_err();
        assert!(matches!(err, ServerError::Forbidden(_)));
    }
}
