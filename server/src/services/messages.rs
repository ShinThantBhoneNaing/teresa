//! Posting and history.

use teresa_proto::{
    validate, Author, Message, MessageId, MessageKind, RoomId, ServerFrame, Timestamp,
    HISTORY_PAGE_DEFAULT, HISTORY_PAGE_MAX,
};

use crate::error::Result;
use crate::repo::UserRow;

use super::{Core, Store};

impl<S: Store> Core<S> {
    /// Accepts a message into a room: validates, persists, fans out.
    ///
    /// The returned message is what the sender's actor wraps in a `Posted`
    /// acknowledgement. The room broadcast reaches the sender's own
    /// subscription too; clients de-duplicate by message id, which costs one
    /// hash lookup and spares every other member a filtered channel.
    pub async fn post_message(
        &self,
        actor: &UserRow,
        room: RoomId,
        body: &str,
        kind: MessageKind,
    ) -> Result<Message> {
        debug_assert!(
            kind.is_notable(),
            "system lines are posted via system_line, not post_message"
        );
        self.require_member(room, actor.id).await?;
        let body = validate::message_body(body)?;

        let message = Message {
            id: MessageId::new(),
            room_id: room,
            author: Some(Author {
                id: actor.id,
                username: actor.username.clone(),
                display_name: actor.display_name.clone(),
            }),
            kind,
            body,
            created_at: Timestamp::now(),
        };

        // Persist before publishing: a message every client saw but a restart
        // forgets is corrosive in a way a rejected send is not.
        self.store().insert_message(&message).await?;
        self.hub().publish_room(
            room,
            ServerFrame::Message {
                message: message.clone(),
            },
        );
        Ok(message)
    }

    /// Pages backwards through a room's history.
    ///
    /// Returns the page oldest-first plus a completeness flag; a page shorter
    /// than requested means the top of the room has been reached.
    pub async fn history(
        &self,
        actor: &UserRow,
        room: RoomId,
        before: Option<MessageId>,
        limit: u16,
    ) -> Result<(Vec<Message>, bool)> {
        self.require_member(room, actor.id).await?;

        let limit = match limit {
            0 => HISTORY_PAGE_DEFAULT,
            n => n.min(HISTORY_PAGE_MAX),
        };
        let messages = self.store().page(room, before, limit).await?;
        let complete = messages.len() < usize::from(limit);
        Ok((messages, complete))
    }

    /// Persists and broadcasts a system narration line.
    ///
    /// Server-authored, so no membership check and no author stamp — but it
    /// still flows through the same storage and fan-out as everything else,
    /// which is what keeps history and live view identical.
    pub(crate) async fn system_line(&self, room: RoomId, text: String) -> Result<()> {
        let message = Message::system(room, text);
        self.store().insert_message(&message).await?;
        self.hub()
            .publish_room(room, ServerFrame::Message { message });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit;
    use super::*;
    use crate::error::ServerError;
    use crate::repo::sqlite::SqliteStore;
    use crate::repo::RoomRepo;

    async fn actor(core: &Core<SqliteStore>, name: &str) -> UserRow {
        let (token, _) = testkit::register(core, name).await;
        core.authenticate(&token).await.unwrap()
    }

    #[tokio::test]
    async fn a_posted_message_reaches_subscribers_and_the_database() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();

        let mut rx = core.hub().subscribe_room(general.id);
        let posted = core
            .post_message(&alice, general.id, "finished the API 🚀", MessageKind::Chat)
            .await
            .unwrap();

        // Fan-out delivered the same message the sender got back.
        let frame = rx.recv().await.unwrap();
        match frame.as_ref() {
            ServerFrame::Message { message } => {
                assert_eq!(message.id, posted.id);
                assert_eq!(message.body, "finished the API 🚀");
                assert_eq!(message.author.as_ref().unwrap().username, "alice");
            }
            other => panic!("expected Message, got {other:?}"),
        }

        // And it is in history for the next client.
        let (page, complete) = core.history(&alice, general.id, None, 10).await.unwrap();
        assert!(complete);
        assert_eq!(page.last().unwrap().id, posted.id);
    }

    #[tokio::test]
    async fn non_members_may_neither_post_nor_read() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let (room, _) = core
            .create_room(
                &alice,
                "sanctum",
                None,
                teresa_proto::Visibility::InviteOnly,
            )
            .await
            .unwrap();
        let mallory = actor(&core, "mallory").await;

        let post = core
            .post_message(&mallory, room.room.id, "let me in", MessageKind::Chat)
            .await
            .unwrap_err();
        assert!(matches!(post, ServerError::Forbidden(_)));

        let read = core
            .history(&mallory, room.room.id, None, 10)
            .await
            .unwrap_err();
        assert!(matches!(read, ServerError::Forbidden(_)));
    }

    #[tokio::test]
    async fn oversized_and_escape_laden_bodies_are_rejected() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();

        let oversized = "x".repeat(teresa_proto::validate::MESSAGE_MAX + 1);
        assert!(core
            .post_message(&alice, general.id, &oversized, MessageKind::Chat)
            .await
            .is_err());
        assert!(core
            .post_message(
                &alice,
                general.id,
                "look \x1b[2J clean screen",
                MessageKind::Chat
            )
            .await
            .is_err());
        assert!(core
            .post_message(&alice, general.id, "   ", MessageKind::Chat)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn history_pages_clamp_and_terminate() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();

        for i in 0..5 {
            core.post_message(&alice, general.id, &format!("m{i}"), MessageKind::Chat)
                .await
                .unwrap();
        }

        // limit 0 means "the default", not "nothing".
        let (page, complete) = core.history(&alice, general.id, None, 0).await.unwrap();
        assert_eq!(page.len(), 5);
        assert!(complete);

        // Walk two pages of two; the cursor moves strictly backwards.
        let (newest, _) = core.history(&alice, general.id, None, 2).await.unwrap();
        assert_eq!(
            newest.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
            ["m3", "m4"]
        );
        let (older, complete) = core
            .history(&alice, general.id, Some(newest[0].id), 2)
            .await
            .unwrap();
        assert_eq!(
            older.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
            ["m1", "m2"]
        );
        assert!(!complete);
    }

    #[tokio::test]
    async fn emotes_carry_their_kind() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();

        let posted = core
            .post_message(&alice, general.id, "fixes the build", MessageKind::Emote)
            .await
            .unwrap();
        assert_eq!(posted.kind, MessageKind::Emote);
    }
}
