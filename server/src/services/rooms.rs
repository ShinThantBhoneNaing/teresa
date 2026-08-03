//! Channels, membership, invites, direct messages.

use teresa_proto::{
    dm_participants, dm_room_name, validate, JoinedRoom, MemberRole, Room, RoomId, RoomKind,
    ServerFrame, Timestamp, User, UserId, Visibility,
};

use crate::error::{Result, ServerError};
use crate::repo::{JoinedRoomRow, MembershipRow, RoomRow, UserRow};

use super::{Core, Store};

impl<S: Store> Core<S> {
    /// Creates a channel with the actor as owner, and joins them to it.
    pub async fn create_room(
        &self,
        actor: &UserRow,
        name: &str,
        topic: Option<&str>,
        visibility: Visibility,
    ) -> Result<(JoinedRoom, Vec<User>)> {
        let name = validate::room_name(name)?;
        let topic = match topic {
            Some(raw) => validate::topic(raw)?,
            None => None,
        };

        let now = Timestamp::now();
        let row = RoomRow {
            id: RoomId::new(),
            name,
            topic,
            kind: RoomKind::Channel,
            visibility,
            owner_id: Some(actor.id),
            created_at: now,
        };
        let membership = MembershipRow {
            room_id: row.id,
            user_id: actor.id,
            role: MemberRole::Owner,
            joined_at: now,
            last_read_id: String::new(),
        };
        self.store().insert_room(&row, Some(&membership)).await?;

        self.system_line(row.id, format!("@{} created #{}", actor.username, row.name))
            .await?;

        // Everyone may learn of a discoverable room; an invite-only room's
        // existence is itself the secret.
        if visibility.is_discoverable() {
            self.hub().publish_global(ServerFrame::RoomCreated {
                room: row.to_room(1),
            });
        }

        tracing::info!(room = %row.name, owner = %actor.username, "room created");

        let joined = JoinedRoom {
            room: row.to_room(1),
            role: MemberRole::Owner,
            unread: 0,
            peer: None,
        };
        Ok((joined, vec![actor.to_user(true)]))
    }

    /// Joins a channel by name.
    ///
    /// Idempotent for an existing member: rejoining a room you are in simply
    /// returns its current state, which lets the client treat `/join` as
    /// "switch to", the way IRC users expect.
    pub async fn join_room(&self, actor: &UserRow, name: &str) -> Result<(JoinedRoom, Vec<User>)> {
        let name = validate::room_name(name)?;
        let room = self
            .store()
            .room_by_name(&name)
            .await?
            .ok_or(ServerError::NotFound("room"))?;

        if room.kind == RoomKind::Dm {
            return Err(ServerError::BadRequest(
                "that is a direct message; use /dm <user> instead".to_owned(),
            ));
        }

        let existing = self.store().membership(room.id, actor.id).await?;
        if existing.is_none() {
            if !self.store().has_invite(room.id, actor.id).await? {
                match room.visibility {
                    Visibility::Public => {}
                    // A private room is listed, so refusing it openly leaks nothing.
                    Visibility::Private => {
                        return Err(ServerError::Forbidden("this room is invite-only for you"));
                    }
                    // An invite-only room must be indistinguishable from a room
                    // that does not exist.
                    Visibility::InviteOnly => {
                        return Err(ServerError::NotFound("room"));
                    }
                }
            }

            self.store()
                .add_member(&MembershipRow {
                    room_id: room.id,
                    user_id: actor.id,
                    role: MemberRole::Member,
                    joined_at: Timestamp::now(),
                    last_read_id: String::new(),
                })
                .await?;
            self.store().remove_invite(room.id, actor.id).await?;

            // Order matters: the join narration lands before the member-list
            // update, so clients hear "alice joined" and then see her appear.
            self.system_line(room.id, format!("@{} joined", actor.username))
                .await?;
            self.hub().publish_room(
                room.id,
                ServerFrame::MemberJoined {
                    room: room.id,
                    user: actor.to_user(true),
                },
            );
        }

        let members = self.members_as_users(room.id).await?;
        let joined = JoinedRoom {
            room: room.to_room(members.len() as u32),
            role: existing.map_or(MemberRole::Member, |m| m.role),
            unread: 0,
            peer: None,
        };
        Ok((joined, members))
    }

    /// Leaves a room.
    pub async fn leave_room(&self, actor: &UserRow, room_id: RoomId) -> Result<()> {
        let room = self.load_room(room_id).await?;
        let was_member = self.store().remove_member(room_id, actor.id).await?;
        if !was_member {
            return Err(ServerError::BadRequest(
                "you are not in that room".to_owned(),
            ));
        }

        // DMs part silently: "bob left" in a two-person conversation is noise
        // with exactly one possible reading.
        if room.kind == RoomKind::Channel {
            self.system_line(room_id, format!("@{} left", actor.username))
                .await?;
            self.hub().publish_room(
                room_id,
                ServerFrame::MemberLeft {
                    room: room_id,
                    user: actor.id,
                },
            );
        }
        Ok(())
    }

    /// Deletes a channel, subject to [`Room::can_be_deleted_by`].
    pub async fn delete_room(&self, actor: &UserRow, room_id: RoomId) -> Result<()> {
        let room = self.load_room(room_id).await?;
        if !room.to_room(0).can_be_deleted_by(actor.id, actor.role) {
            return Err(ServerError::Forbidden(
                "only the room's owner or a moderator may delete it",
            ));
        }

        // Announce first — after the delete there is no channel to hear it on.
        let frame = ServerFrame::RoomDeleted { room: room_id };
        self.hub().publish_room(room_id, frame.clone());
        if room.visibility.is_discoverable() {
            self.hub().publish_global(frame);
        }

        self.store().delete_room(room_id).await?;
        self.hub().drop_room(room_id);
        tracing::info!(room = %room.name, by = %actor.username, "room deleted");
        Ok(())
    }

    /// Invites a user into a room.
    pub async fn invite(&self, actor: &UserRow, room_id: RoomId, username: &str) -> Result<()> {
        let membership = self.require_member(room_id, actor.id).await?;
        let room = self.load_room(room_id).await?;

        // In a public room any member may invite — it is a pointer, not a key.
        // Where the room is restricted, the invite *is* the key, so it takes
        // room moderator standing (or server moderator authority) to mint one.
        if room.visibility != Visibility::Public
            && !membership.role.can_invite()
            && !actor.role.can_moderate()
        {
            return Err(ServerError::Forbidden(
                "only the room's moderators may invite people here",
            ));
        }

        let username = validate::username(username)?;
        let target = self
            .store()
            .user_by_username(&username)
            .await?
            .ok_or(ServerError::NotFound("user"))?;

        if self.store().membership(room_id, target.id).await?.is_some() {
            return Err(ServerError::BadRequest(format!(
                "@{username} is already a member"
            )));
        }

        self.store()
            .add_invite(room_id, target.id, actor.id)
            .await?;

        let member_count = self.store().member_ids(room_id).await?.len() as u32;
        self.hub().send_user(
            target.id,
            ServerFrame::Invited {
                room: room.to_room(member_count),
                by: actor.username.clone(),
            },
        );
        tracing::debug!(room = %room.name, from = %actor.username, to = %username, "invite sent");
        Ok(())
    }

    /// Opens — or returns — the direct conversation between the actor and
    /// another user. One pair, one room, forever.
    pub async fn open_dm(
        &self,
        actor: &UserRow,
        username: &str,
    ) -> Result<(JoinedRoom, Vec<User>)> {
        let username = validate::username(username)?;
        let peer = self
            .store()
            .user_by_username(&username)
            .await?
            .ok_or(ServerError::NotFound("user"))?;
        if peer.id == actor.id {
            return Err(ServerError::BadRequest(
                "a direct message needs someone else on the other end".to_owned(),
            ));
        }

        let name = dm_room_name(actor.id, peer.id);
        let now = Timestamp::now();

        let room = if let Some(existing) = self.store().room_by_name(&name).await? {
            existing
        } else {
            let row = RoomRow {
                id: RoomId::new(),
                name: name.clone(),
                topic: None,
                kind: RoomKind::Dm,
                visibility: Visibility::InviteOnly,
                owner_id: None,
                created_at: now,
            };
            // A concurrent open from the other side may have won the race;
            // the unique name makes that a Conflict, and the room it made
            // is exactly the room we wanted.
            match self.store().insert_room(&row, None).await {
                Ok(()) => row,
                Err(ServerError::Conflict(_)) => self
                    .store()
                    .room_by_name(&name)
                    .await?
                    .ok_or(ServerError::NotFound("room"))?,
                Err(other) => return Err(other),
            }
        };

        // Both sides are members; re-adding is a no-op. The peer's membership
        // may be freshly minted here — if so, their terminals are told, so the
        // conversation appears for them without a refresh.
        let peer_was_member = self.store().membership(room.id, peer.id).await?.is_some();
        for user_id in [actor.id, peer.id] {
            self.store()
                .add_member(&MembershipRow {
                    room_id: room.id,
                    user_id,
                    role: MemberRole::Member,
                    joined_at: now,
                    last_read_id: String::new(),
                })
                .await?;
        }

        let members = self.members_as_users(room.id).await?;

        if !peer_was_member {
            self.hub().send_user(
                peer.id,
                ServerFrame::Joined {
                    room: JoinedRoom {
                        room: room.to_room(members.len() as u32),
                        role: MemberRole::Member,
                        unread: 0,
                        peer: Some(actor.username.clone()),
                    },
                    members: members.clone(),
                },
            );
        }

        let joined = JoinedRoom {
            room: room.to_room(members.len() as u32),
            role: MemberRole::Member,
            unread: 0,
            peer: Some(peer.username),
        };
        Ok((joined, members))
    }

    /// The channels this user may see.
    pub async fn list_rooms(&self, actor: UserId) -> Result<Vec<Room>> {
        Ok(self
            .store()
            .visible_to(actor)
            .await?
            .into_iter()
            .map(|(row, count)| row.to_room(count))
            .collect())
    }

    /// The member list of a room the actor belongs to.
    pub async fn members(&self, actor: UserId, room_id: RoomId) -> Result<Vec<User>> {
        self.require_member(room_id, actor).await?;
        self.members_as_users(room_id).await
    }

    /// Moves the actor's read marker to the room's newest message.
    pub async fn mark_read(&self, actor: UserId, room_id: RoomId) -> Result<()> {
        self.require_member(room_id, actor).await?;
        if let Some(newest) = self.store().newest_id(room_id).await? {
            self.store().mark_read(room_id, actor, newest).await?;
        }
        Ok(())
    }

    // --- shared plumbing ----------------------------------------------------

    /// A room by id, or [`ServerError::NotFound`].
    pub(crate) async fn load_room(&self, id: RoomId) -> Result<RoomRow> {
        self.store()
            .room_by_id(id)
            .await?
            .ok_or(ServerError::NotFound("room"))
    }

    /// The actor's membership of a room, or [`ServerError::Forbidden`].
    pub(crate) async fn require_member(&self, room: RoomId, user: UserId) -> Result<MembershipRow> {
        self.store()
            .membership(room, user)
            .await?
            .ok_or(ServerError::Forbidden("not a member of that room"))
    }

    /// A room's members as wire users with live presence, sorted by handle so
    /// every client renders the same list.
    pub(crate) async fn members_as_users(&self, room: RoomId) -> Result<Vec<User>> {
        let ids = self.store().member_ids(room).await?;
        let mut users: Vec<User> = self
            .store()
            .users_by_ids(&ids)
            .await?
            .iter()
            .map(|row| row.to_user(self.hub().is_live(row.id)))
            .collect();
        users.sort_by(|a, b| a.username.cmp(&b.username));
        Ok(users)
    }

    /// Projects a stored membership row into the wire shape, resolving the
    /// peer handle for direct messages.
    pub(crate) async fn project_joined_room(
        &self,
        row: JoinedRoomRow,
        viewer: UserId,
    ) -> Result<JoinedRoom> {
        let peer = if row.room.kind == RoomKind::Dm {
            match dm_participants(&row.room.name) {
                Some((a, b)) => {
                    let other = if a == viewer { b } else { a };
                    self.store()
                        .user_by_id(other)
                        .await?
                        .map(|user| user.username)
                }
                None => None,
            }
        } else {
            None
        };

        Ok(JoinedRoom {
            room: row.room.to_room(row.member_count),
            role: row.role,
            unread: row.unread,
            peer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit;
    use super::*;
    use crate::repo::RoomRepo;

    async fn actor(core: &Core<crate::repo::sqlite::SqliteStore>, name: &str) -> UserRow {
        let (token, _) = testkit::register(core, name).await;
        core.authenticate(&token).await.unwrap()
    }

    #[tokio::test]
    async fn create_join_leave_narrates_to_the_room() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let bob = actor(&core, "bob").await;

        let (created, members) = core
            .create_room(&alice, "#WarCouncil", Some("planning"), Visibility::Public)
            .await
            .unwrap();
        assert_eq!(
            created.room.name, "warcouncil",
            "the sigil strips and case folds"
        );
        assert_eq!(created.role, MemberRole::Owner);
        assert_eq!(members.len(), 1);

        // Bob subscribes (as his actor would) and joins.
        let mut room_rx = core.hub().subscribe_room(created.room.id);
        let (joined, members) = core.join_room(&bob, "warcouncil").await.unwrap();
        assert_eq!(joined.role, MemberRole::Member);
        assert_eq!(members.len(), 2);

        // The room heard the narration, then the member-list update.
        let first = room_rx.recv().await.unwrap();
        assert!(matches!(
            first.as_ref(),
            ServerFrame::Message { message } if message.body == "@bob joined"
        ));
        let second = room_rx.recv().await.unwrap();
        assert!(matches!(second.as_ref(), ServerFrame::MemberJoined { .. }));

        core.leave_room(&bob, created.room.id).await.unwrap();
        let third = room_rx.recv().await.unwrap();
        assert!(matches!(
            third.as_ref(),
            ServerFrame::Message { message } if message.body == "@bob left"
        ));
    }

    #[tokio::test]
    async fn rejoining_is_idempotent() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;

        // alice is already in #general from registration.
        let (joined, _) = core.join_room(&alice, "general").await.unwrap();
        assert_eq!(joined.room.name, "general");
        let again = core.store().joined_by(alice.id).await.unwrap();
        assert_eq!(again.len(), 1, "no duplicate membership");
    }

    #[tokio::test]
    async fn deletion_respects_the_permission_matrix() {
        let core = testkit::core().await;
        let admin = actor(&core, "admin").await; // first account: server admin
        let owner = actor(&core, "owner").await;
        let bystander = actor(&core, "bystander").await;

        let (room, _) = core
            .create_room(&owner, "doomed", None, Visibility::Public)
            .await
            .unwrap();

        let err = core
            .delete_room(&bystander, room.room.id)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Forbidden(_)));

        // The owner may delete their own room.
        core.delete_room(&owner, room.room.id).await.unwrap();

        // A server admin may delete anyone's.
        let (other, _) = core
            .create_room(&owner, "doomed2", None, Visibility::Public)
            .await
            .unwrap();
        core.delete_room(&admin, other.room.id).await.unwrap();

        assert!(core.store().room_by_name("doomed").await.unwrap().is_none());
        assert!(core
            .store()
            .room_by_name("doomed2")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn invite_only_rooms_are_invisible_until_invited() {
        let core = testkit::core().await;
        let host = actor(&core, "host").await;
        let guest = actor(&core, "guest").await;

        let (secret, _) = core
            .create_room(&host, "warroom", None, Visibility::InviteOnly)
            .await
            .unwrap();

        // Uninvited, the room reads as nonexistent — not merely locked.
        let err = core.join_room(&guest, "warroom").await.unwrap_err();
        assert!(matches!(err, ServerError::NotFound(_)));
        assert!(
            !core
                .list_rooms(guest.id)
                .await
                .unwrap()
                .iter()
                .any(|r| r.name == "warroom"),
            "invite-only rooms must not be listed to outsiders"
        );

        // The guest's terminal hears the invite arrive.
        let guest_conn = teresa_proto::ConnectionId::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        core.connect(&guest, guest_conn, tx).await.unwrap();

        core.invite(&host, secret.room.id, "guest").await.unwrap();
        let frame = rx.recv().await.unwrap();
        assert!(matches!(
            frame.as_ref(),
            ServerFrame::Invited { room, by } if room.name == "warroom" && by == "host"
        ));

        // Now the room is visible, joinable, and the invite is consumed.
        assert!(core
            .list_rooms(guest.id)
            .await
            .unwrap()
            .iter()
            .any(|r| r.name == "warroom"));
        core.join_room(&guest, "warroom").await.unwrap();
        assert!(!core
            .store()
            .has_invite(secret.room.id, guest.id)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn plain_members_cannot_mint_keys_to_restricted_rooms() {
        let core = testkit::core().await;
        let _admin = actor(&core, "admin").await;
        let host = actor(&core, "host").await;
        let member = actor(&core, "member").await;
        let outsider = actor(&core, "outsider").await;

        let (room, _) = core
            .create_room(&host, "vault", None, Visibility::InviteOnly)
            .await
            .unwrap();
        core.invite(&host, room.room.id, "member").await.unwrap();
        core.join_room(&member, "vault").await.unwrap();

        // A plain member of a restricted room cannot invite.
        let err = core
            .invite(&member, room.room.id, "outsider")
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Forbidden(_)));

        // A non-member cannot invite at all, even to a public room.
        let (public, _) = core
            .create_room(&host, "town", None, Visibility::Public)
            .await
            .unwrap();
        let err = core
            .invite(&outsider, public.room.id, "member")
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Forbidden(_)));
    }

    #[tokio::test]
    async fn inviting_an_existing_member_is_called_out() {
        let core = testkit::core().await;
        let host = actor(&core, "host").await;
        let (room, _) = core
            .create_room(&host, "club", None, Visibility::Public)
            .await
            .unwrap();
        let err = core.invite(&host, room.room.id, "host").await.unwrap_err();
        assert!(matches!(err, ServerError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_dm_is_one_room_no_matter_who_opens_it() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let bob = actor(&core, "bob").await;

        let (from_alice, _) = core.open_dm(&alice, "bob").await.unwrap();
        let (from_bob, _) = core.open_dm(&bob, "alice").await.unwrap();

        assert_eq!(from_alice.room.id, from_bob.room.id);
        assert_eq!(from_alice.peer.as_deref(), Some("bob"));
        assert_eq!(from_bob.peer.as_deref(), Some("alice"));
        assert_eq!(from_alice.label(), "@bob");
    }

    #[tokio::test]
    async fn opening_a_dm_tells_the_peers_live_terminals() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let bob = actor(&core, "bob").await;

        let (tx, mut bob_rx) = tokio::sync::mpsc::channel(8);
        core.connect(&bob, teresa_proto::ConnectionId::new(), tx)
            .await
            .unwrap();

        core.open_dm(&alice, "bob").await.unwrap();
        let frame = bob_rx.recv().await.unwrap();
        assert!(matches!(
            frame.as_ref(),
            ServerFrame::Joined { room, .. } if room.peer.as_deref() == Some("alice")
        ));

        // Reopening must not spam bob again.
        core.open_dm(&alice, "bob").await.unwrap();
        assert!(bob_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_dm_with_yourself_is_refused() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let err = core.open_dm(&alice, "alice").await.unwrap_err();
        assert!(matches!(err, ServerError::BadRequest(_)));
    }

    #[tokio::test]
    async fn ready_projects_dms_with_their_peer_handle() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let bob = actor(&core, "bob").await;
        core.open_dm(&alice, "bob").await.unwrap();

        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let ready = core
            .connect(&bob, teresa_proto::ConnectionId::new(), tx)
            .await
            .unwrap();
        let ServerFrame::Ready { rooms, .. } = ready else {
            panic!()
        };
        let dm = rooms
            .iter()
            .find(|r| r.room.kind == RoomKind::Dm)
            .expect("dm listed");
        assert_eq!(dm.peer.as_deref(), Some("alice"));
        assert_eq!(dm.label(), "@alice");
    }

    #[tokio::test]
    async fn mark_read_clears_the_unread_count() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let bob = actor(&core, "bob").await;

        let general = core.store().room_by_name("general").await.unwrap().unwrap();
        core.post_message(
            &alice,
            general.id,
            "morning",
            teresa_proto::MessageKind::Chat,
        )
        .await
        .unwrap();

        assert_eq!(core.store().joined_by(bob.id).await.unwrap()[0].unread, 1);
        core.mark_read(bob.id, general.id).await.unwrap();
        assert_eq!(core.store().joined_by(bob.id).await.unwrap()[0].unread, 0);
    }
}
