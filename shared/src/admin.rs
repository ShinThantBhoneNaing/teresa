//! Server maintenance, for admins: how much the node is storing, and the
//! levers that shrink it.
//!
//! The host is one small office box; history and shared files are the only
//! things that grow without bound, so they are the only things an admin
//! needs to be able to trim from a chat window.

use serde::{Deserialize, Serialize};

/// One maintenance request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MaintenanceOp {
    /// Report storage use; changes nothing.
    Stats,
    /// Delete shared files older than this many days (0 = all of them),
    /// bytes and records both. Their cards stay in history but can no longer
    /// be fetched.
    PurgeFiles {
        /// Age threshold in days.
        older_than_days: u32,
    },
    /// Delete messages older than this many days (0 = all history).
    PurgeMessages {
        /// Age threshold in days.
        older_than_days: u32,
    },
    /// Give freed database pages back to the disk.
    Compact,
}

/// What the node is holding right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    /// Registered accounts.
    pub users: u64,
    /// Stored messages, every room.
    pub messages: u64,
    /// Shared files on record.
    pub files: u64,
    /// Bytes those files occupy on disk.
    pub file_bytes: u64,
    /// Bytes the database occupies on disk, write-ahead log included.
    pub db_bytes: u64,
}

/// The answer to a [`MaintenanceOp`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceReport {
    /// The request this answers.
    pub op: MaintenanceOp,
    /// How many files or messages went (0 for stats and compaction).
    pub removed: u64,
    /// Disk space given back, in bytes.
    pub freed_bytes: u64,
    /// Storage after the operation.
    pub stats: StorageStats,
}
