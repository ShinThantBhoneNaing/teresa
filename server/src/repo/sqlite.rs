//! The SQLite adapters.
//!
//! One [`SqliteStore`] implements every repository trait over a shared
//! connection pool. Queries use the runtime API rather than the compile-time
//! checked macros so the crate builds without a `DATABASE_URL`; the trade-off
//! is covered by the adapter tests below, which run every query against a real
//! in-memory database.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, SqlitePool};
use teresa_proto::{Author, Message, MessageId, RoomId, Timestamp, UserId};

use crate::error::{Result, ServerError};

use super::{
    JoinedRoomRow, MembershipRow, MessageRepo, RoomRepo, RoomRow, SessionRepo, SessionRow,
    UserRepo, UserRow,
};

/// The shared SQLite store. Cloning shares the pool.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Opens (creating if necessary) a database file and runs migrations.
    ///
    /// WAL journalling lets readers proceed while a write is in flight, which
    /// is what makes "the database is not the hot path" true under load.
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened or a migration fails.
    pub async fn open(path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            // WAL plus Normal loses at most the final transactions on OS
            // crash, never consistency. Full would fsync every message.
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;

        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    /// An in-memory store for tests.
    ///
    /// Capped at one connection: each in-memory SQLite connection is otherwise
    /// its own separate empty database.
    ///
    /// # Errors
    /// Returns an error if a migration fails.
    pub async fn memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(pool)
            .await
            .map_err(|e| ServerError::Internal(e.into()))
    }

    /// The underlying pool, for health checks.
    #[must_use]
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Maps a unique-constraint violation to [`ServerError::Conflict`], leaving
/// every other database error as what it is.
fn conflict_on_unique(err: sqlx::Error, what: impl FnOnce() -> String) -> ServerError {
    match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => ServerError::Conflict(what()),
        _ => ServerError::Database(err),
    }
}

/// Parses a stored enum string, treating failure as the corruption it is.
fn parse_stored<T: FromStr>(value: &str, what: &'static str) -> Result<T> {
    value.parse().map_err(|_| {
        ServerError::Internal(anyhow::anyhow!("corrupt {what} in database: {value:?}"))
    })
}

fn user_from_row(row: &SqliteRow) -> Result<UserRow> {
    Ok(UserRow {
        id: parse_stored(row.try_get("id")?, "user id")?,
        username: row.try_get("username")?,
        display_name: row.try_get("display_name")?,
        password_hash: row.try_get("password_hash")?,
        status: parse_stored(row.try_get("status")?, "user status")?,
        status_text: row.try_get("status_text")?,
        role: parse_stored(row.try_get("role")?, "user role")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        last_seen_at: Timestamp::from_millis(row.try_get("last_seen_at")?),
    })
}

fn session_from_row(row: &SqliteRow) -> Result<SessionRow> {
    Ok(SessionRow {
        token_hash: row.try_get("token_hash")?,
        user_id: parse_stored(row.try_get("user_id")?, "user id")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        last_used_at: Timestamp::from_millis(row.try_get("last_used_at")?),
        expires_at: Timestamp::from_millis(row.try_get("expires_at")?),
    })
}

fn room_from_row(row: &SqliteRow) -> Result<RoomRow> {
    let owner: Option<String> = row.try_get("owner_id")?;
    Ok(RoomRow {
        id: parse_stored(row.try_get("id")?, "room id")?,
        name: row.try_get("name")?,
        topic: row.try_get("topic")?,
        kind: parse_stored(row.try_get("kind")?, "room kind")?,
        visibility: parse_stored(row.try_get("visibility")?, "room visibility")?,
        owner_id: owner
            .as_deref()
            .map(|o| parse_stored(o, "owner id"))
            .transpose()?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}

fn membership_from_row(row: &SqliteRow) -> Result<MembershipRow> {
    Ok(MembershipRow {
        room_id: parse_stored(row.try_get("room_id")?, "room id")?,
        user_id: parse_stored(row.try_get("user_id")?, "user id")?,
        role: parse_stored(row.try_get("role")?, "member role")?,
        joined_at: Timestamp::from_millis(row.try_get("joined_at")?),
        last_read_id: row.try_get("last_read_id")?,
    })
}

fn message_from_row(row: &SqliteRow) -> Result<Message> {
    let user_id: Option<String> = row.try_get("user_id")?;
    let username: Option<String> = row.try_get("author_username")?;
    let display_name: Option<String> = row.try_get("author_display_name")?;

    // An author exists only when all three columns are present; a system line
    // has none of them. A row with some-but-not-all is corrupt, and surfaces
    // as an authorless line rather than a crash.
    let author = match (user_id, username, display_name) {
        (Some(id), Some(username), Some(display_name)) => Some(Author {
            id: parse_stored(&id, "author id")?,
            username,
            display_name,
        }),
        _ => None,
    };

    Ok(Message {
        id: parse_stored(row.try_get("id")?, "message id")?,
        room_id: parse_stored(row.try_get("room_id")?, "room id")?,
        author,
        kind: parse_stored(row.try_get("kind")?, "message kind")?,
        body: row.try_get("body")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}

impl UserRepo for SqliteStore {
    async fn insert_user(&self, user: &UserRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO users
               (id, username, display_name, password_hash, status, status_text, role, created_at, last_seen_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(user.status.as_str())
        .bind(&user.status_text)
        .bind(user.role.as_str())
        .bind(user.created_at.as_millis())
        .bind(user.last_seen_at.as_millis())
        .execute(&self.pool)
        .await
        .map_err(|e| conflict_on_unique(e, || format!("username {}", user.username)))?;
        Ok(())
    }

    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRow>> {
        sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .map(|row| user_from_row(&row))
            .transpose()
    }

    async fn user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        sqlx::query("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| user_from_row(&row))
            .transpose()
    }

    async fn users_by_ids(&self, ids: &[UserId]) -> Result<Vec<UserRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // SQLite has no array bind; the placeholder list is built to size.
        // Ids are bound, never spliced, so this stays injection-proof.
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!("SELECT * FROM users WHERE id IN ({placeholders})");

        let mut query = sqlx::query(&sql);
        for id in ids {
            query = query.bind(id.to_string());
        }

        query
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(user_from_row)
            .collect()
    }

    async fn set_status(
        &self,
        id: UserId,
        status: teresa_proto::UserStatus,
        text: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE users SET status = ?, status_text = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(text)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_display_name(&self, id: UserId, display_name: &str) -> Result<()> {
        sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
            .bind(display_name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn touch_last_seen(&self, id: UserId, at: Timestamp) -> Result<()> {
        sqlx::query("UPDATE users SET last_seen_at = ? WHERE id = ?")
            .bind(at.as_millis())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn count_users(&self) -> Result<u64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.max(0) as u64)
    }
}

impl SessionRepo for SqliteStore {
    async fn insert_session(&self, session: &SessionRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, last_used_at, expires_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&session.token_hash)
        .bind(session.user_id.to_string())
        .bind(session.created_at.as_millis())
        .bind(session.last_used_at.as_millis())
        .bind(session.expires_at.as_millis())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn take_live(
        &self,
        token_hash: &str,
        now: Timestamp,
        extend_to: Timestamp,
    ) -> Result<Option<SessionRow>> {
        // Lookup and sliding-expiry bump in one atomic statement: two racing
        // requests with the same token both see a live session or neither does.
        sqlx::query(
            "UPDATE sessions SET last_used_at = ?, expires_at = ?
             WHERE token_hash = ? AND expires_at > ?
             RETURNING token_hash, user_id, created_at, last_used_at, expires_at",
        )
        .bind(now.as_millis())
        .bind(extend_to.as_millis())
        .bind(token_hash)
        .bind(now.as_millis())
        .fetch_optional(&self.pool)
        .await?
        .map(|row| session_from_row(&row))
        .transpose()
    }

    async fn delete_session(&self, token_hash: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_for_user(&self, user_id: UserId) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn purge_expired(&self, now: Timestamp) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(now.as_millis())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

impl RoomRepo for SqliteStore {
    async fn insert_room(
        &self,
        room: &RoomRow,
        first_member: Option<&MembershipRow>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO rooms (id, name, topic, kind, visibility, owner_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(room.id.to_string())
        .bind(&room.name)
        .bind(&room.topic)
        .bind(room.kind.as_str())
        .bind(room.visibility.as_str())
        .bind(room.owner_id.map(|o| o.to_string()))
        .bind(room.created_at.as_millis())
        .execute(&mut *tx)
        .await
        .map_err(|e| conflict_on_unique(e, || format!("room #{}", room.name)))?;

        if let Some(member) = first_member {
            sqlx::query(
                "INSERT INTO room_members (room_id, user_id, role, joined_at, last_read_id)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(member.room_id.to_string())
            .bind(member.user_id.to_string())
            .bind(member.role.as_str())
            .bind(member.joined_at.as_millis())
            .bind(&member.last_read_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn room_by_id(&self, id: RoomId) -> Result<Option<RoomRow>> {
        sqlx::query("SELECT * FROM rooms WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .map(|row| room_from_row(&row))
            .transpose()
    }

    async fn room_by_name(&self, name: &str) -> Result<Option<RoomRow>> {
        sqlx::query("SELECT * FROM rooms WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| room_from_row(&row))
            .transpose()
    }

    async fn delete_room(&self, id: RoomId) -> Result<()> {
        // Members, invites and messages go with it, via ON DELETE CASCADE.
        sqlx::query("DELETE FROM rooms WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn visible_to(&self, user: UserId) -> Result<Vec<(RoomRow, u32)>> {
        // Channels only: DMs surface through joined_by, never through listing.
        // Invite-only rooms appear solely via the membership / invite arms,
        // which is what makes them invisible to everyone else.
        let rows = sqlx::query(
            "SELECT r.*,
                    (SELECT COUNT(*) FROM room_members rm WHERE rm.room_id = r.id) AS member_count
             FROM rooms r
             WHERE r.kind = 'channel'
               AND (r.visibility IN ('public', 'private')
                    OR EXISTS (SELECT 1 FROM room_members rm
                               WHERE rm.room_id = r.id AND rm.user_id = ?)
                    OR EXISTS (SELECT 1 FROM room_invites ri
                               WHERE ri.room_id = r.id AND ri.user_id = ?))
             ORDER BY r.name",
        )
        .bind(user.to_string())
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let count: i64 = row.try_get("member_count")?;
                Ok((room_from_row(row)?, count.max(0) as u32))
            })
            .collect()
    }

    async fn joined_by(&self, user: UserId) -> Result<Vec<JoinedRoomRow>> {
        // last_read_id of '' sorts before every ULID, so `id > last_read_id`
        // counts everything when nothing has been read. System lines are
        // excluded to match MessageKind::is_notable.
        let rows = sqlx::query(
            "SELECT r.*, m.role AS member_role,
                    (SELECT COUNT(*) FROM messages msg
                     WHERE msg.room_id = r.id
                       AND msg.id > m.last_read_id
                       AND msg.kind != 'system') AS unread,
                    (SELECT COUNT(*) FROM room_members rm WHERE rm.room_id = r.id) AS member_count
             FROM room_members m
             JOIN rooms r ON r.id = m.room_id
             WHERE m.user_id = ?
             ORDER BY r.name",
        )
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let unread: i64 = row.try_get("unread")?;
                let member_count: i64 = row.try_get("member_count")?;
                Ok(JoinedRoomRow {
                    room: room_from_row(row)?,
                    role: parse_stored(row.try_get("member_role")?, "member role")?,
                    unread: unread.max(0) as u32,
                    member_count: member_count.max(0) as u32,
                })
            })
            .collect()
    }

    async fn add_member(&self, membership: &MembershipRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO room_members (room_id, user_id, role, joined_at, last_read_id)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (room_id, user_id) DO NOTHING",
        )
        .bind(membership.room_id.to_string())
        .bind(membership.user_id.to_string())
        .bind(membership.role.as_str())
        .bind(membership.joined_at.as_millis())
        .bind(&membership.last_read_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_member(&self, room: RoomId, user: UserId) -> Result<bool> {
        let result = sqlx::query("DELETE FROM room_members WHERE room_id = ? AND user_id = ?")
            .bind(room.to_string())
            .bind(user.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn membership(&self, room: RoomId, user: UserId) -> Result<Option<MembershipRow>> {
        sqlx::query("SELECT * FROM room_members WHERE room_id = ? AND user_id = ?")
            .bind(room.to_string())
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await?
            .map(|row| membership_from_row(&row))
            .transpose()
    }

    async fn members_of(&self, room: RoomId) -> Result<Vec<MembershipRow>> {
        sqlx::query("SELECT * FROM room_members WHERE room_id = ? ORDER BY joined_at")
            .bind(room.to_string())
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(membership_from_row)
            .collect()
    }

    async fn member_ids(&self, room: RoomId) -> Result<Vec<UserId>> {
        let rows = sqlx::query("SELECT user_id FROM room_members WHERE room_id = ?")
            .bind(room.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| parse_stored(row.try_get("user_id")?, "user id"))
            .collect()
    }

    async fn mark_read(&self, room: RoomId, user: UserId, up_to: MessageId) -> Result<()> {
        // Forward-only: a stale MarkRead from a reconnecting client must not
        // resurrect unread counts.
        sqlx::query(
            "UPDATE room_members SET last_read_id = ?
             WHERE room_id = ? AND user_id = ? AND last_read_id < ?",
        )
        .bind(up_to.to_string())
        .bind(room.to_string())
        .bind(user.to_string())
        .bind(up_to.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn add_invite(&self, room: RoomId, user: UserId, invited_by: UserId) -> Result<()> {
        sqlx::query(
            "INSERT INTO room_invites (room_id, user_id, invited_by, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (room_id, user_id) DO NOTHING",
        )
        .bind(room.to_string())
        .bind(user.to_string())
        .bind(invited_by.to_string())
        .bind(Timestamp::now().as_millis())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn has_invite(&self, room: RoomId, user: UserId) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM room_invites WHERE room_id = ? AND user_id = ?",
        )
        .bind(room.to_string())
        .bind(user.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    async fn remove_invite(&self, room: RoomId, user: UserId) -> Result<()> {
        sqlx::query("DELETE FROM room_invites WHERE room_id = ? AND user_id = ?")
            .bind(room.to_string())
            .bind(user.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl MessageRepo for SqliteStore {
    async fn insert_message(&self, message: &Message) -> Result<()> {
        let author = message.author.as_ref();
        sqlx::query(
            "INSERT INTO messages
               (id, room_id, user_id, author_username, author_display_name, kind, body, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(message.id.to_string())
        .bind(message.room_id.to_string())
        .bind(author.map(|a| a.id.to_string()))
        .bind(author.map(|a| a.username.as_str()))
        .bind(author.map(|a| a.display_name.as_str()))
        .bind(message.kind.as_str())
        .bind(&message.body)
        .bind(message.created_at.as_millis())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn page(
        &self,
        room: RoomId,
        before: Option<MessageId>,
        limit: u16,
    ) -> Result<Vec<Message>> {
        // Newest-first under the cursor, then reversed for display order.
        // The (room_id, id) index serves this as a backward scan with no sort.
        let rows = match before {
            Some(before) => {
                sqlx::query(
                    "SELECT * FROM messages WHERE room_id = ? AND id < ?
                     ORDER BY id DESC LIMIT ?",
                )
                .bind(room.to_string())
                .bind(before.to_string())
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query("SELECT * FROM messages WHERE room_id = ? ORDER BY id DESC LIMIT ?")
                    .bind(room.to_string())
                    .bind(i64::from(limit))
                    .fetch_all(&self.pool)
                    .await?
            }
        };

        let mut messages = rows
            .iter()
            .map(message_from_row)
            .collect::<Result<Vec<_>>>()?;
        messages.reverse();
        Ok(messages)
    }

    async fn newest_id(&self, room: RoomId) -> Result<Option<MessageId>> {
        let id: Option<String> =
            sqlx::query_scalar("SELECT MAX(id) FROM messages WHERE room_id = ?")
                .bind(room.to_string())
                .fetch_one(&self.pool)
                .await?;
        id.as_deref()
            .map(|id| parse_stored(id, "message id"))
            .transpose()
    }

    async fn count_messages(&self) -> Result<u64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.max(0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use teresa_proto::{MemberRole, MessageKind, RoomKind, UserRole, UserStatus, Visibility};

    use super::*;

    fn user(name: &str) -> UserRow {
        UserRow {
            id: UserId::new(),
            username: name.to_owned(),
            display_name: name.to_owned(),
            password_hash: "$argon2id$test".to_owned(),
            status: UserStatus::Online,
            status_text: None,
            role: UserRole::Member,
            created_at: Timestamp::now(),
            last_seen_at: Timestamp::now(),
        }
    }

    fn room(name: &str, owner: Option<UserId>, visibility: Visibility) -> RoomRow {
        RoomRow {
            id: RoomId::new(),
            name: name.to_owned(),
            topic: None,
            kind: RoomKind::Channel,
            visibility,
            owner_id: owner,
            created_at: Timestamp::now(),
        }
    }

    fn membership(room: RoomId, user: UserId, role: MemberRole) -> MembershipRow {
        MembershipRow {
            room_id: room,
            user_id: user,
            role,
            joined_at: Timestamp::now(),
            last_read_id: String::new(),
        }
    }

    fn chat(room: RoomId, author: &UserRow, body: &str) -> Message {
        Message {
            id: MessageId::new(),
            room_id: room,
            author: Some(Author {
                id: author.id,
                username: author.username.clone(),
                display_name: author.display_name.clone(),
            }),
            kind: MessageKind::Chat,
            body: body.to_owned(),
            created_at: Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn users_round_trip_with_every_field() {
        let store = SqliteStore::memory().await.unwrap();
        let mut row = user("neo");
        row.status = UserStatus::Coding;
        row.status_text = Some("in the construct".into());
        row.role = UserRole::Admin;

        store.insert_user(&row).await.unwrap();
        let fetched = store
            .user_by_id(row.id)
            .await
            .unwrap()
            .expect("user exists");

        assert_eq!(fetched.username, "neo");
        assert_eq!(fetched.status, UserStatus::Coding);
        assert_eq!(fetched.status_text.as_deref(), Some("in the construct"));
        assert_eq!(fetched.role, UserRole::Admin);
        assert_eq!(fetched.password_hash, "$argon2id$test");

        let by_name = store
            .user_by_username("neo")
            .await
            .unwrap()
            .expect("found by name");
        assert_eq!(by_name.id, row.id);
    }

    #[tokio::test]
    async fn duplicate_usernames_conflict() {
        let store = SqliteStore::memory().await.unwrap();
        store.insert_user(&user("neo")).await.unwrap();
        let err = store.insert_user(&user("neo")).await.unwrap_err();
        assert!(matches!(err, ServerError::Conflict(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn by_ids_fetches_exactly_the_requested_users() {
        let store = SqliteStore::memory().await.unwrap();
        let a = user("alice");
        let b = user("bob");
        let c = user("carol");
        for u in [&a, &b, &c] {
            store.insert_user(u).await.unwrap();
        }

        let fetched = store.users_by_ids(&[a.id, c.id]).await.unwrap();
        let mut names: Vec<_> = fetched.iter().map(|u| u.username.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["alice", "carol"]);

        assert!(store.users_by_ids(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn sessions_expire_and_slide() {
        let store = SqliteStore::memory().await.unwrap();
        let owner = user("neo");
        store.insert_user(&owner).await.unwrap();

        let now = Timestamp::now();
        let plus = |secs: i64| Timestamp::from_millis(now.as_millis() + secs * 1000);

        store
            .insert_session(&SessionRow {
                token_hash: "hash-a".into(),
                user_id: owner.id,
                created_at: now,
                last_used_at: now,
                expires_at: plus(60),
            })
            .await
            .unwrap();

        // Live: found, and the expiry slides forward.
        let live = store
            .take_live("hash-a", plus(30), plus(90))
            .await
            .unwrap()
            .expect("session is live");
        assert_eq!(live.user_id, owner.id);
        assert_eq!(live.expires_at, plus(90));

        // At its (old) deadline it would have died; the slide saved it.
        assert!(store
            .take_live("hash-a", plus(70), plus(130))
            .await
            .unwrap()
            .is_some());

        // Past the slid deadline: gone.
        assert!(store
            .take_live("hash-a", plus(200), plus(260))
            .await
            .unwrap()
            .is_none());

        // Unknown token: gone.
        assert!(store
            .take_live("no-such", now, plus(60))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn purge_removes_only_expired_sessions() {
        let store = SqliteStore::memory().await.unwrap();
        let owner = user("neo");
        store.insert_user(&owner).await.unwrap();

        let now = Timestamp::now();
        let at = |secs: i64| Timestamp::from_millis(now.as_millis() + secs * 1000);
        for (hash, expires) in [("dead", at(-10)), ("alive", at(600))] {
            store
                .insert_session(&SessionRow {
                    token_hash: hash.into(),
                    user_id: owner.id,
                    created_at: at(-3600),
                    last_used_at: at(-3600),
                    expires_at: expires,
                })
                .await
                .unwrap();
        }

        assert_eq!(store.purge_expired(now).await.unwrap(), 1);
        assert!(store
            .take_live("alive", now, at(1200))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn logout_everywhere_deletes_all_of_a_users_sessions() {
        let store = SqliteStore::memory().await.unwrap();
        let owner = user("neo");
        let other = user("bob");
        store.insert_user(&owner).await.unwrap();
        store.insert_user(&other).await.unwrap();

        let now = Timestamp::now();
        let later = Timestamp::from_millis(now.as_millis() + 600_000);
        for (hash, uid) in [("s1", owner.id), ("s2", owner.id), ("s3", other.id)] {
            store
                .insert_session(&SessionRow {
                    token_hash: hash.into(),
                    user_id: uid,
                    created_at: now,
                    last_used_at: now,
                    expires_at: later,
                })
                .await
                .unwrap();
        }

        assert_eq!(store.delete_for_user(owner.id).await.unwrap(), 2);
        assert!(store.take_live("s3", now, later).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn room_insert_is_atomic_with_its_first_member() {
        let store = SqliteStore::memory().await.unwrap();
        let owner = user("neo");
        store.insert_user(&owner).await.unwrap();

        let r = room("backend", Some(owner.id), Visibility::Public);
        store
            .insert_room(&r, Some(&membership(r.id, owner.id, MemberRole::Owner)))
            .await
            .unwrap();

        let member = store
            .membership(r.id, owner.id)
            .await
            .unwrap()
            .expect("is member");
        assert_eq!(member.role, MemberRole::Owner);

        // A name collision must roll the membership back too.
        let dupe = room("backend", Some(owner.id), Visibility::Public);
        let err = store
            .insert_room(
                &dupe,
                Some(&membership(dupe.id, owner.id, MemberRole::Owner)),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::Conflict(_)));
        assert!(store.membership(dupe.id, owner.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn visibility_rules_hide_exactly_the_right_rooms() {
        let store = SqliteStore::memory().await.unwrap();
        let member = user("member");
        let invited = user("invited");
        let outsider = user("outsider");
        for u in [&member, &invited, &outsider] {
            store.insert_user(u).await.unwrap();
        }

        let public = room("public", None, Visibility::Public);
        let private = room("private", None, Visibility::Private);
        let secret = room("secret", Some(member.id), Visibility::InviteOnly);
        for r in [&public, &private, &secret] {
            store.insert_room(r, None).await.unwrap();
        }
        store
            .add_member(&membership(secret.id, member.id, MemberRole::Owner))
            .await
            .unwrap();
        store
            .add_invite(secret.id, invited.id, member.id)
            .await
            .unwrap();

        let names =
            |rooms: Vec<(RoomRow, u32)>| rooms.into_iter().map(|(r, _)| r.name).collect::<Vec<_>>();

        // The outsider sees public and private, never the invite-only room.
        assert_eq!(
            names(store.visible_to(outsider.id).await.unwrap()),
            ["private", "public"]
        );
        // Membership reveals it.
        assert_eq!(
            names(store.visible_to(member.id).await.unwrap()),
            ["private", "public", "secret"]
        );
        // So does a standing invite.
        assert_eq!(
            names(store.visible_to(invited.id).await.unwrap()),
            ["private", "public", "secret"]
        );
    }

    #[tokio::test]
    async fn dms_never_appear_in_the_room_listing() {
        let store = SqliteStore::memory().await.unwrap();
        let a = user("alice");
        let b = user("bob");
        store.insert_user(&a).await.unwrap();
        store.insert_user(&b).await.unwrap();

        let dm = RoomRow {
            kind: RoomKind::Dm,
            ..room(
                &teresa_proto::dm_room_name(a.id, b.id),
                None,
                Visibility::InviteOnly,
            )
        };
        store.insert_room(&dm, None).await.unwrap();
        store
            .add_member(&membership(dm.id, a.id, MemberRole::Member))
            .await
            .unwrap();

        assert!(store.visible_to(a.id).await.unwrap().is_empty());
        // But it does appear among the rooms they have joined.
        assert_eq!(store.joined_by(a.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unread_counts_follow_the_read_marker_and_skip_system_lines() {
        let store = SqliteStore::memory().await.unwrap();
        let reader = user("reader");
        let writer = user("writer");
        store.insert_user(&reader).await.unwrap();
        store.insert_user(&writer).await.unwrap();

        let r = room("busy", None, Visibility::Public);
        store.insert_room(&r, None).await.unwrap();
        store
            .add_member(&membership(r.id, reader.id, MemberRole::Member))
            .await
            .unwrap();

        let first = chat(r.id, &writer, "one");
        let second = chat(r.id, &writer, "two");
        store.insert_message(&first).await.unwrap();
        store.insert_message(&second).await.unwrap();
        store
            .insert_message(&Message::system(r.id, "writer joined"))
            .await
            .unwrap();

        // Nothing read: both chat lines count, the system line does not.
        assert_eq!(store.joined_by(reader.id).await.unwrap()[0].unread, 2);

        store.mark_read(r.id, reader.id, first.id).await.unwrap();
        assert_eq!(store.joined_by(reader.id).await.unwrap()[0].unread, 1);

        // A stale marker must not move the count backwards.
        store.mark_read(r.id, reader.id, second.id).await.unwrap();
        store.mark_read(r.id, reader.id, first.id).await.unwrap();
        assert_eq!(store.joined_by(reader.id).await.unwrap()[0].unread, 0);
    }

    #[tokio::test]
    async fn membership_is_idempotent_and_removal_reports_truthfully() {
        let store = SqliteStore::memory().await.unwrap();
        let u = user("neo");
        store.insert_user(&u).await.unwrap();
        let r = room("general", None, Visibility::Public);
        store.insert_room(&r, None).await.unwrap();

        store
            .add_member(&membership(r.id, u.id, MemberRole::Member))
            .await
            .unwrap();
        // Rejoining must not clobber the original role or joined_at.
        store
            .add_member(&membership(r.id, u.id, MemberRole::Owner))
            .await
            .unwrap();
        assert_eq!(
            store.membership(r.id, u.id).await.unwrap().unwrap().role,
            MemberRole::Member
        );

        assert!(store.remove_member(r.id, u.id).await.unwrap());
        assert!(!store.remove_member(r.id, u.id).await.unwrap());
    }

    #[tokio::test]
    async fn invites_are_recorded_consumed_and_idempotent() {
        let store = SqliteStore::memory().await.unwrap();
        let host = user("host");
        let guest = user("guest");
        store.insert_user(&host).await.unwrap();
        store.insert_user(&guest).await.unwrap();
        let r = room("secret", Some(host.id), Visibility::InviteOnly);
        store.insert_room(&r, None).await.unwrap();

        assert!(!store.has_invite(r.id, guest.id).await.unwrap());
        store.add_invite(r.id, guest.id, host.id).await.unwrap();
        store.add_invite(r.id, guest.id, host.id).await.unwrap(); // idempotent
        assert!(store.has_invite(r.id, guest.id).await.unwrap());

        store.remove_invite(r.id, guest.id).await.unwrap();
        assert!(!store.has_invite(r.id, guest.id).await.unwrap());
    }

    #[tokio::test]
    async fn message_pages_walk_backwards_in_display_order() {
        let store = SqliteStore::memory().await.unwrap();
        let author = user("writer");
        store.insert_user(&author).await.unwrap();
        let r = room("log", None, Visibility::Public);
        store.insert_room(&r, None).await.unwrap();

        let bodies: Vec<String> = (0..7).map(|i| format!("msg-{i}")).collect();
        let mut ids = Vec::new();
        for body in &bodies {
            let msg = chat(r.id, &author, body);
            ids.push(msg.id);
            store.insert_message(&msg).await.unwrap();
        }

        // Newest page, display order.
        let newest = store.page(r.id, None, 3).await.unwrap();
        let texts: Vec<_> = newest.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(texts, ["msg-4", "msg-5", "msg-6"]);

        // The next page continues from the cursor.
        let older = store.page(r.id, Some(newest[0].id), 3).await.unwrap();
        let texts: Vec<_> = older.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(texts, ["msg-1", "msg-2", "msg-3"]);

        // The final page is short, signalling the start of history.
        let oldest = store.page(r.id, Some(older[0].id), 3).await.unwrap();
        let texts: Vec<_> = oldest.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(texts, ["msg-0"]);

        assert_eq!(store.newest_id(r.id).await.unwrap(), ids.last().copied());
        assert_eq!(store.count_messages().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn deleting_a_room_cascades_to_everything_it_owns() {
        let store = SqliteStore::memory().await.unwrap();
        let u = user("neo");
        store.insert_user(&u).await.unwrap();
        let r = room("doomed", Some(u.id), Visibility::Public);
        store
            .insert_room(&r, Some(&membership(r.id, u.id, MemberRole::Owner)))
            .await
            .unwrap();
        store
            .insert_message(&chat(r.id, &u, "goodbye"))
            .await
            .unwrap();

        store.delete_room(r.id).await.unwrap();

        assert!(store.room_by_id(r.id).await.unwrap().is_none());
        assert!(store.membership(r.id, u.id).await.unwrap().is_none());
        assert_eq!(store.count_messages().await.unwrap(), 0);
        assert!(store.page(r.id, None, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn system_messages_survive_their_missing_author() {
        let store = SqliteStore::memory().await.unwrap();
        let r = room("sys", None, Visibility::Public);
        store.insert_room(&r, None).await.unwrap();

        store
            .insert_message(&Message::system(r.id, "server started"))
            .await
            .unwrap();
        let fetched = store.page(r.id, None, 10).await.unwrap();
        assert_eq!(fetched.len(), 1);
        assert!(fetched[0].author.is_none());
        assert_eq!(fetched[0].kind, MessageKind::System);
    }

    #[tokio::test]
    async fn deleting_a_user_keeps_their_messages_readable() {
        // No FK on messages.user_id: history must outlive accounts.
        let store = SqliteStore::memory().await.unwrap();
        let u = user("ghost");
        store.insert_user(&u).await.unwrap();
        let r = room("archive", None, Visibility::Public);
        store.insert_room(&r, None).await.unwrap();
        store
            .insert_message(&chat(r.id, &u, "remember me"))
            .await
            .unwrap();

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(u.id.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let fetched = store.page(r.id, None, 10).await.unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].body, "remember me");
        // The denormalised name survives even though the account is gone.
        assert_eq!(fetched[0].author.as_ref().unwrap().username, "ghost");
    }
}
