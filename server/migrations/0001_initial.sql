-- TERESA initial schema.
--
-- Ids are 26-character ULID strings. They sort lexicographically in creation
-- order, which is why every "newest first" query below orders by id rather
-- than by a timestamp column.

CREATE TABLE users (
    id            TEXT    PRIMARY KEY,
    -- Stored lowercase; validation folds case before it gets here.
    username      TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    -- Argon2id PHC string. Never a plaintext password, never a reversible form.
    password_hash TEXT    NOT NULL,
    status        TEXT    NOT NULL DEFAULT 'offline',
    status_text   TEXT,
    role          TEXT    NOT NULL DEFAULT 'member',
    created_at    INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE sessions (
    -- SHA-256 of the bearer token; the token itself is never stored.
    token_hash   TEXT    PRIMARY KEY,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    -- Sliding expiry: bumped on use, expired sessions are purged lazily.
    last_used_at INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_by_user ON sessions(user_id);
CREATE INDEX sessions_by_expiry ON sessions(expires_at);

CREATE TABLE rooms (
    id         TEXT    PRIMARY KEY,
    -- Channel names are unique server-wide; DM rooms use the canonical
    -- dm-<lo>-<hi> form, which makes pair uniqueness a side effect of this
    -- constraint rather than something application code must enforce.
    name       TEXT    NOT NULL UNIQUE,
    topic      TEXT,
    kind       TEXT    NOT NULL DEFAULT 'channel',   -- channel | dm
    visibility TEXT    NOT NULL DEFAULT 'public',    -- public | private | invite_only
    -- NULL for seed rooms and DMs: they belong to nobody.
    owner_id   TEXT    REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE room_members (
    room_id      TEXT    NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role         TEXT    NOT NULL DEFAULT 'member',  -- member | moderator | owner
    joined_at    INTEGER NOT NULL,
    -- Id of the newest message this member has read; '' means none.
    -- Unread count is then "messages with id greater than this", one range scan.
    last_read_id TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (room_id, user_id)
) STRICT;

-- The reverse direction of the primary key: "which rooms is this user in".
CREATE INDEX room_members_by_user ON room_members(user_id);

CREATE TABLE room_invites (
    room_id    TEXT    NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id    TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invited_by TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE INDEX room_invites_by_user ON room_invites(user_id);

CREATE TABLE messages (
    id         TEXT    PRIMARY KEY,
    room_id    TEXT    NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    -- The author stamp: id and names frozen at send time, NULL only for system
    -- lines. Deliberately NOT a foreign key — this is historical record, not a
    -- live reference, and history must survive account deletion intact. It also
    -- keeps the history query joinless.
    user_id    TEXT,
    author_username     TEXT,
    author_display_name TEXT,
    kind       TEXT    NOT NULL DEFAULT 'chat',      -- chat | emote | system
    body       TEXT    NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

-- The one query that must stay fast forever: a room's newest N messages.
-- (room_id, id) makes it an index-order scan with no sort step.
CREATE INDEX messages_by_room ON messages(room_id, id);
