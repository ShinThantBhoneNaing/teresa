-- Shared files. The bytes live on disk under the server's files directory,
-- named by id; this table is what makes them findable and authorisable.

CREATE TABLE files (
    id         TEXT    PRIMARY KEY,
    -- Deleting a room deletes its files' rows; housekeeping then removes the
    -- orphaned bytes from disk.
    room_id    TEXT    NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    -- Historical record, like messages.user_id: not a foreign key.
    user_id    TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    size       INTEGER NOT NULL,
    sha256     TEXT    NOT NULL,
    kind       TEXT    NOT NULL DEFAULT 'file',      -- file | folder | zip
    entries    INTEGER,
    created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX files_by_room ON files(room_id, id);
