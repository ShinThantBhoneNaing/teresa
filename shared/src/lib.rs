//! Shared vocabulary for the TERESA server and client.
//!
//! This crate is the contract between the two binaries. Neither depends on the
//! other; both depend on this. A change to a frame therefore breaks the build
//! on whichever side has not been updated, which is precisely when you want to
//! find out.
//!
//! # Layout
//!
//! - [`ids`] — typed ULID newtypes, so a `RoomId` can never be passed as a `UserId`
//! - [`stamp`] — millisecond wire timestamps
//! - [`user`] — accounts, presence, permissions
//! - [`room`] — channels, direct messages, membership
//! - [`message`] — chat lines
//! - [`frame`] — the protocol proper
//! - [`validate`] — input rules both sides enforce
//! - [`error`] — shared failure vocabulary
//!
//! # Example
//!
//! ```
//! use teresa_proto::{ClientFrame, RoomId};
//!
//! let frame = ClientFrame::Send {
//!     room: RoomId::new(),
//!     body: "no system is safe".into(),
//!     nonce: "7f3a".into(),
//! };
//!
//! let wire = frame.encode()?;
//! assert_eq!(ClientFrame::decode(&wire)?, frame);
//! # Ok::<(), teresa_proto::ProtoError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
// Frame enums are inherently wide; splitting them to satisfy a lint would hide
// the protocol rather than clarify it.
#![allow(clippy::module_name_repetitions)]

pub mod error;
pub mod frame;
pub mod ids;
pub mod message;
pub mod room;
pub mod stamp;
pub mod user;
pub mod validate;

pub use error::{ErrorCode, ProtoError};
pub use frame::{
    ClientFrame, JoinedRoom, ServerFrame, ServerInfo, HISTORY_PAGE_DEFAULT, HISTORY_PAGE_MAX,
    PROTOCOL_VERSION,
};
pub use ids::{ConnectionId, MessageId, RoomId, UserId};
pub use message::{Message, MessageKind};
pub use room::{dm_participants, dm_room_name, MemberRole, Room, RoomKind, Visibility};
pub use stamp::Timestamp;
pub use user::{Author, User, UserRole, UserStatus};

/// The version of this crate, and therefore of the protocol implementation.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
