//! Persistence ports.
//!
//! Services depend on these traits, never on SQLite directly. The SQLite
//! adapters live in [`sqlite`]; a Postgres backend later is a second adapter
//! module and one construction-site change, with no service touched.
//!
//! Row types here are storage-shaped, not wire-shaped: [`UserRow`] carries the
//! password hash, which [`teresa_proto::User`] must never. The conversion in
//! one direction only — row → wire — is the mechanism that keeps secrets out
//! of frames.

pub mod sqlite;

use teresa_proto::{
    MemberRole, Message, MessageId, RoomId, RoomKind, Timestamp, UserId, UserRole, UserStatus,
    Visibility,
};

use crate::error::Result;

/// A user as stored, including material that must never reach the wire.
#[derive(Debug, Clone)]
pub struct UserRow {
    /// Stable id.
    pub id: UserId,
    /// Unique lowercase handle.
    pub username: String,
    /// Display name.
    pub display_name: String,
    /// Argon2id PHC hash.
    pub password_hash: String,
    /// Last chosen presence. Reported presence also factors in live connections.
    pub status: UserStatus,
    /// Free status text.
    pub status_text: Option<String>,
    /// Server-wide authority.
    pub role: UserRole,
    /// Account creation time.
    pub created_at: Timestamp,
    /// Last connection or disconnection time.
    pub last_seen_at: Timestamp,
}

impl UserRow {
    /// The wire-safe projection of this row.
    ///
    /// `live` tells the projection whether the user currently has connections;
    /// a stored status of `coding` with no live connection must present as
    /// `offline`, or the member list would show ghosts.
    #[must_use]
    pub fn to_user(&self, live: bool) -> teresa_proto::User {
        teresa_proto::User {
            id: self.id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
            status: if live {
                self.status
            } else {
                UserStatus::Offline
            },
            status_text: self.status_text.clone(),
            role: self.role,
            created_at: self.created_at,
        }
    }
}

/// A session as stored.
#[derive(Debug, Clone)]
pub struct SessionRow {
    /// SHA-256 of the bearer token.
    pub token_hash: String,
    /// The account it authenticates.
    pub user_id: UserId,
    /// When it was minted.
    pub created_at: Timestamp,
    /// When it was last presented.
    pub last_used_at: Timestamp,
    /// When it stops working.
    pub expires_at: Timestamp,
}

/// A room as stored.
#[derive(Debug, Clone)]
pub struct RoomRow {
    /// Stable id.
    pub id: RoomId,
    /// Unique name.
    pub name: String,
    /// Topic.
    pub topic: Option<String>,
    /// Channel or DM.
    pub kind: RoomKind,
    /// Visibility.
    pub visibility: Visibility,
    /// Creator, if any.
    pub owner_id: Option<UserId>,
    /// Creation time.
    pub created_at: Timestamp,
}

impl RoomRow {
    /// The wire-safe projection, with the member count supplied by the caller.
    #[must_use]
    pub fn to_room(&self, member_count: u32) -> teresa_proto::Room {
        teresa_proto::Room {
            id: self.id,
            name: self.name.clone(),
            topic: self.topic.clone(),
            kind: self.kind,
            visibility: self.visibility,
            owner_id: self.owner_id,
            created_at: self.created_at,
            member_count,
        }
    }
}

/// One user's membership of one room.
#[derive(Debug, Clone)]
pub struct MembershipRow {
    /// The room.
    pub room_id: RoomId,
    /// The member.
    pub user_id: UserId,
    /// Authority within the room.
    pub role: MemberRole,
    /// When they joined.
    pub joined_at: Timestamp,
    /// Newest message id they have read; empty string for none.
    pub last_read_id: String,
}

/// A membership joined with its room and unread count — the shape the
/// `Ready` frame needs, fetched in one query instead of N.
#[derive(Debug, Clone)]
pub struct JoinedRoomRow {
    /// The room.
    pub room: RoomRow,
    /// This user's role in it.
    pub role: MemberRole,
    /// Messages newer than their read marker.
    pub unread: u32,
    /// Members in the room.
    pub member_count: u32,
}

/// User persistence.
#[allow(async_fn_in_trait)] // internal trait; no Send bound gymnastics needed
pub trait UserRepo {
    /// Inserts a new user.
    async fn insert_user(&self, user: &UserRow) -> Result<()>;
    /// Fetches by id.
    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRow>>;
    /// Fetches by exact (already lowercased) username.
    async fn user_by_username(&self, username: &str) -> Result<Option<UserRow>>;
    /// Fetches several users at once, in no particular order.
    async fn users_by_ids(&self, ids: &[UserId]) -> Result<Vec<UserRow>>;
    /// Updates presence.
    async fn set_status(&self, id: UserId, status: UserStatus, text: Option<&str>) -> Result<()>;
    /// Updates display name.
    async fn set_display_name(&self, id: UserId, display_name: &str) -> Result<()>;
    /// Stamps last-seen.
    async fn touch_last_seen(&self, id: UserId, at: Timestamp) -> Result<()>;
    /// Total registered accounts.
    async fn count_users(&self) -> Result<u64>;
}

/// Session persistence.
#[allow(async_fn_in_trait)]
pub trait SessionRepo {
    /// Inserts a new session.
    async fn insert_session(&self, session: &SessionRow) -> Result<()>;
    /// Fetches a live (unexpired) session by token hash, bumping its sliding
    /// expiry as a side effect.
    async fn take_live(
        &self,
        token_hash: &str,
        now: Timestamp,
        extend_to: Timestamp,
    ) -> Result<Option<SessionRow>>;
    /// Deletes one session — logout.
    async fn delete_session(&self, token_hash: &str) -> Result<()>;
    /// Deletes all of a user's sessions — logout everywhere / account action.
    async fn delete_for_user(&self, user_id: UserId) -> Result<u64>;
    /// Purges expired sessions, returning how many went.
    async fn purge_expired(&self, now: Timestamp) -> Result<u64>;
}

/// Room and membership persistence.
#[allow(async_fn_in_trait)]
pub trait RoomRepo {
    /// Inserts a room and its first member atomically.
    ///
    /// `first_member` is `None` only for seed rooms created at boot.
    async fn insert_room(&self, room: &RoomRow, first_member: Option<&MembershipRow>)
        -> Result<()>;
    /// Fetches by id.
    async fn room_by_id(&self, id: RoomId) -> Result<Option<RoomRow>>;
    /// Fetches by exact name.
    async fn room_by_name(&self, name: &str) -> Result<Option<RoomRow>>;
    /// Deletes a room and, via cascade, its members, invites and messages.
    async fn delete_room(&self, id: RoomId) -> Result<()>;
    /// All rooms a user could list: public and private channels, plus anything
    /// they are a member of or invited to. Invite-only rooms they have no tie
    /// to are absent, which is what makes them invisible.
    async fn visible_to(&self, user: UserId) -> Result<Vec<(RoomRow, u32)>>;
    /// The rooms a user is a member of, with unread counts, for `Ready`.
    async fn joined_by(&self, user: UserId) -> Result<Vec<JoinedRoomRow>>;

    /// Adds a member; idempotent if they already belong.
    async fn add_member(&self, membership: &MembershipRow) -> Result<()>;
    /// Removes a member. Returns whether they were a member.
    async fn remove_member(&self, room: RoomId, user: UserId) -> Result<bool>;
    /// One membership, if it exists.
    async fn membership(&self, room: RoomId, user: UserId) -> Result<Option<MembershipRow>>;
    /// All memberships of a room.
    async fn members_of(&self, room: RoomId) -> Result<Vec<MembershipRow>>;
    /// Member ids only — the fan-out path needs nothing more.
    async fn member_ids(&self, room: RoomId) -> Result<Vec<UserId>>;
    /// Moves a member's read marker forward (never backward).
    async fn mark_read(&self, room: RoomId, user: UserId, up_to: MessageId) -> Result<()>;

    /// Records an invite; idempotent.
    async fn add_invite(&self, room: RoomId, user: UserId, invited_by: UserId) -> Result<()>;
    /// Whether an invite exists.
    async fn has_invite(&self, room: RoomId, user: UserId) -> Result<bool>;
    /// Consumes an invite once the user joins.
    async fn remove_invite(&self, room: RoomId, user: UserId) -> Result<()>;
}

/// Message persistence.
#[allow(async_fn_in_trait)]
pub trait MessageRepo {
    /// Inserts a message.
    async fn insert_message(&self, message: &Message) -> Result<()>;
    /// Up to `limit` messages strictly older than `before` (or the newest, if
    /// `None`), returned oldest-first ready for display.
    async fn page(
        &self,
        room: RoomId,
        before: Option<MessageId>,
        limit: u16,
    ) -> Result<Vec<Message>>;
    /// Id of the newest message in a room, if any.
    async fn newest_id(&self, room: RoomId) -> Result<Option<MessageId>>;
    /// Total stored messages.
    async fn count_messages(&self) -> Result<u64>;
}
