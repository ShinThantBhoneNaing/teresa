//! The wire protocol.
//!
//! Two enums define every byte that crosses the socket after the HTTP
//! handshake. Adding a capability means adding a variant, at which point the
//! compiler enumerates every site that must learn to handle it — which is the
//! entire reason the protocol lives in a crate shared by both binaries rather
//! than being written twice.
//!
//! Frames are JSON, internally tagged on `t`:
//!
//! ```json
//! {"t":"send","room":"01J8...","body":"ship it","nonce":"a3f2"}
//! ```
//!
//! Field names are abbreviated where the meaning survives, because these
//! strings are repeated on every frame. Variant names are never abbreviated,
//! because they are the vocabulary of the protocol.

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, ProtoError};
use crate::ids::{MessageId, RoomId, UserId};
use crate::message::Message;
use crate::room::{MemberRole, Room, Visibility};
use crate::user::{User, UserStatus};

/// How many messages a single history page may contain.
pub const HISTORY_PAGE_MAX: u16 = 200;
/// The page size a client should ask for when it has no reason to want another.
pub const HISTORY_PAGE_DEFAULT: u16 = 50;

/// Anything the client can say to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientFrame {
    /// Post a message to a room.
    Send {
        /// Target room.
        room: RoomId,
        /// Raw body; markdown is not interpreted server-side.
        body: String,
        /// Client-chosen correlation id, echoed back in [`ServerFrame::Posted`].
        ///
        /// This is what makes a local echo safe: the client renders the message
        /// immediately in a pending state and reconciles when its own nonce
        /// returns, instead of waiting a round trip to show your own typing.
        nonce: String,
    },

    /// Post a `/me` action.
    Emote {
        /// Target room.
        room: RoomId,
        /// The action text, without the leading `/me`.
        body: String,
        /// Correlation id, as for [`ClientFrame::Send`].
        nonce: String,
    },

    /// Announce that the user has started or stopped typing.
    Typing {
        /// Room being typed in.
        room: RoomId,
        /// `true` on start, `false` on send or timeout.
        active: bool,
    },

    /// Join a channel by name, creating nothing.
    Join {
        /// Room name without its sigil.
        name: String,
    },

    /// Leave a room.
    Leave {
        /// Room to leave.
        room: RoomId,
    },

    /// Create a channel and join it.
    CreateRoom {
        /// Room name without its sigil.
        name: String,
        /// Optional one-line description.
        topic: Option<String>,
        /// Who may see and enter it.
        visibility: Visibility,
    },

    /// Delete a channel.
    DeleteRoom {
        /// Room to delete.
        room: RoomId,
    },

    /// Invite a user into a room.
    Invite {
        /// Room to invite into.
        room: RoomId,
        /// Handle of the user to invite.
        username: String,
    },

    /// Open — or reopen — the direct conversation with a user.
    ///
    /// Idempotent: the same pair always resolves to the same room.
    OpenDm {
        /// Handle of the other participant.
        username: String,
    },

    /// Page backwards through a room's history.
    History {
        /// Room to page.
        room: RoomId,
        /// Return messages strictly older than this id; `None` starts at the newest.
        before: Option<MessageId>,
        /// How many to return, clamped to [`HISTORY_PAGE_MAX`].
        limit: u16,
    },

    /// Ask for the member list of a room.
    Members {
        /// Room to enumerate.
        room: RoomId,
    },

    /// Ask for the rooms this user can see.
    ListRooms,

    /// Change presence.
    SetStatus {
        /// New presence.
        status: UserStatus,
        /// Optional free text, or `None` to clear it.
        text: Option<String>,
    },

    /// Change display name.
    SetDisplayName {
        /// The new display name.
        display_name: String,
    },

    /// Mark a room read up to its newest message.
    MarkRead {
        /// Room to mark.
        room: RoomId,
    },

    /// Round-trip probe. The server answers with [`ServerFrame::Pong`].
    Ping {
        /// Echoed back verbatim; the client uses it to measure latency.
        nonce: u64,
    },
}

/// Anything the server can say to the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerFrame {
    /// Sent once, immediately after the socket opens.
    ///
    /// Carries everything the UI needs for its first paint, so the client makes
    /// exactly one round trip between connecting and being usable.
    Ready {
        /// The authenticated user's own profile.
        user: User,
        /// Rooms the user is a member of.
        rooms: Vec<JoinedRoom>,
        /// Server identity and capabilities.
        server: ServerInfo,
    },

    /// A message was posted to a room the client is in.
    Message {
        /// The message itself.
        message: Message,
    },

    /// Acknowledges one of this client's own sends.
    ///
    /// Separate from [`ServerFrame::Message`] so the sender can reconcile its
    /// optimistic local echo without every other client paying for a `nonce`
    /// field it will ignore.
    Posted {
        /// The accepted message, now with a server id and timestamp.
        message: Message,
        /// The nonce from the originating [`ClientFrame::Send`].
        nonce: String,
    },

    /// A page of history, oldest first.
    History {
        /// The room paged.
        room: RoomId,
        /// The messages, in ascending id order.
        messages: Vec<Message>,
        /// `true` when the page reached the beginning of the room.
        complete: bool,
    },

    /// The rooms this user may see, in answer to [`ClientFrame::ListRooms`].
    RoomList {
        /// Visible rooms, whether joined or not.
        rooms: Vec<Room>,
    },

    /// A room the client can now see was created.
    RoomCreated {
        /// The new room.
        room: Room,
    },

    /// A room was deleted.
    RoomDeleted {
        /// The room that no longer exists.
        room: RoomId,
    },

    /// This client joined a room. Carries the member list to avoid a second round trip.
    Joined {
        /// The room, as the client should now list it.
        room: JoinedRoom,
        /// Its current members.
        members: Vec<User>,
    },

    /// This client left a room.
    Left {
        /// The room departed.
        room: RoomId,
    },

    /// Somebody else joined a room this client is in.
    MemberJoined {
        /// The room.
        room: RoomId,
        /// The user who joined.
        user: User,
    },

    /// Somebody else left a room this client is in.
    MemberLeft {
        /// The room.
        room: RoomId,
        /// The user who left.
        user: UserId,
    },

    /// A member list, in answer to [`ClientFrame::Members`].
    Members {
        /// The room enumerated.
        room: RoomId,
        /// Its members.
        members: Vec<User>,
    },

    /// A user's presence changed.
    Presence {
        /// The user.
        user: UserId,
        /// Their new presence.
        status: UserStatus,
        /// Their new status text, if any.
        text: Option<String>,
    },

    /// A user started or stopped typing.
    ///
    /// Carries the display name inline so the indicator can render even for a
    /// user the client has not yet seen in a member list.
    Typing {
        /// The room being typed in.
        room: RoomId,
        /// Who is typing.
        user: UserId,
        /// Their display name.
        display_name: String,
        /// `true` on start, `false` on stop.
        active: bool,
    },

    /// A user's profile changed — a `/nick`, typically.
    ProfileUpdated {
        /// The updated profile.
        user: User,
    },

    /// This client was invited into a room.
    Invited {
        /// The room invited to.
        room: Room,
        /// Handle of the user who sent the invite.
        by: String,
    },

    /// Out-of-band server narration not tied to a room, such as a shutdown warning.
    Notice {
        /// Text to surface to the user.
        text: String,
    },

    /// A request was refused.
    Error {
        /// Machine-readable reason.
        code: ErrorCode,
        /// Human-readable detail.
        message: String,
    },

    /// Answers [`ClientFrame::Ping`].
    Pong {
        /// The nonce from the ping.
        nonce: u64,
    },
}

/// A room plus this user's relationship to it.
///
/// [`Room`] is what a room *is*; this is what it is *to you* — your role, your
/// unread count. Keeping them separate means broadcasting a room to a hundred
/// clients does not require building a hundred variants of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinedRoom {
    /// The room itself.
    #[serde(flatten)]
    pub room: Room,
    /// This user's authority within it.
    pub role: MemberRole,
    /// Messages posted since this user last read it.
    pub unread: u32,
    /// For a DM, the other participant's handle, so the client can label the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
}

impl JoinedRoom {
    /// The label for the room list, resolving a DM to its peer's handle.
    #[must_use]
    pub fn label(&self) -> String {
        self.room.display_name(self.peer.as_deref())
    }
}

/// Server identity, sent in [`ServerFrame::Ready`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Operator-chosen server name, shown in the client's header.
    pub name: String,
    /// The server's crate version.
    pub version: String,
    /// Optional message of the day, posted as a system line on connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motd: Option<String>,
    /// The protocol revision the server speaks.
    pub protocol: u16,
}

/// The protocol revision this build implements.
///
/// Bumped only for changes an older peer cannot tolerate. Adding a frame
/// variant is backwards-compatible — an old client rejects the unknown tag as a
/// decode error on that one frame and carries on — so it does not bump this.
pub const PROTOCOL_VERSION: u16 = 1;

impl ClientFrame {
    /// Encodes the frame for transmission.
    ///
    /// # Errors
    /// Returns [`ProtoError::Decode`] only if a body contains something serde
    /// cannot represent, which for these types means never in practice.
    pub fn encode(&self) -> Result<String, ProtoError> {
        serde_json::to_string(self).map_err(|e| ProtoError::Decode(e.to_string()))
    }

    /// Decodes a frame received from a client.
    ///
    /// # Errors
    /// Returns [`ProtoError::Decode`] if the text is not a known frame.
    pub fn decode(raw: &str) -> Result<Self, ProtoError> {
        serde_json::from_str(raw).map_err(|e| ProtoError::Decode(e.to_string()))
    }

    /// Whether this frame may be sent faster than the per-connection message
    /// budget allows.
    ///
    /// Typing notifications and pings are frequent by nature and cost the
    /// server almost nothing; counting them against the flood limit would
    /// disconnect people for typing quickly.
    #[must_use]
    pub const fn is_cheap(&self) -> bool {
        matches!(
            self,
            Self::Typing { .. } | Self::Ping { .. } | Self::MarkRead { .. }
        )
    }
}

impl ServerFrame {
    /// Encodes the frame for transmission.
    ///
    /// # Errors
    /// Returns [`ProtoError::Decode`] if serialisation fails.
    pub fn encode(&self) -> Result<String, ProtoError> {
        serde_json::to_string(self).map_err(|e| ProtoError::Decode(e.to_string()))
    }

    /// Decodes a frame received from the server.
    ///
    /// # Errors
    /// Returns [`ProtoError::Decode`] if the text is not a known frame.
    pub fn decode(raw: &str) -> Result<Self, ProtoError> {
        serde_json::from_str(raw).map_err(|e| ProtoError::Decode(e.to_string()))
    }

    /// Builds an error frame.
    #[must_use]
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }

    /// The room this frame concerns, if any.
    ///
    /// The hub uses this to decide which subscribers a frame is relevant to,
    /// and the client uses it to route a frame to the right buffer.
    #[must_use]
    pub const fn room(&self) -> Option<RoomId> {
        match self {
            Self::Message { message } | Self::Posted { message, .. } => Some(message.room_id),
            Self::History { room, .. }
            | Self::RoomDeleted { room }
            | Self::Left { room }
            | Self::MemberJoined { room, .. }
            | Self::MemberLeft { room, .. }
            | Self::Members { room, .. }
            | Self::Typing { room, .. } => Some(*room),
            Self::Joined { room, .. } => Some(room.room.id),
            Self::RoomCreated { room } | Self::Invited { room, .. } => Some(room.id),
            Self::Ready { .. }
            | Self::RoomList { .. }
            | Self::Presence { .. }
            | Self::ProfileUpdated { .. }
            | Self::Notice { .. }
            | Self::Error { .. }
            | Self::Pong { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::UserId;
    use crate::message::MessageKind;
    use crate::room::{RoomKind, Visibility};
    use crate::stamp::Timestamp;
    use crate::user::{Author, UserRole};

    fn sample_user() -> User {
        User {
            id: UserId::new(),
            username: "neo".into(),
            display_name: "Neo".into(),
            status: UserStatus::Online,
            status_text: None,
            role: UserRole::Member,
            created_at: Timestamp::now(),
        }
    }

    fn sample_room() -> Room {
        Room {
            id: RoomId::new(),
            name: "backend".into(),
            topic: Some("api work".into()),
            kind: RoomKind::Channel,
            visibility: Visibility::Public,
            owner_id: Some(UserId::new()),
            created_at: Timestamp::now(),
            member_count: 3,
        }
    }

    fn sample_message() -> Message {
        Message {
            id: MessageId::new(),
            room_id: RoomId::new(),
            author: Some(Author {
                id: UserId::new(),
                username: "alice".into(),
                display_name: "Alice".into(),
            }),
            kind: MessageKind::Chat,
            body: "finished the API 🚀".into(),
            created_at: Timestamp::now(),
        }
    }

    fn sample_joined_room() -> JoinedRoom {
        JoinedRoom {
            room: sample_room(),
            role: MemberRole::Owner,
            unread: 4,
            peer: None,
        }
    }

    /// Every client frame, so the round-trip test cannot silently skip one.
    fn all_client_frames() -> Vec<ClientFrame> {
        vec![
            ClientFrame::Send {
                room: RoomId::new(),
                body: "ship it".into(),
                nonce: "a3f2".into(),
            },
            ClientFrame::Emote {
                room: RoomId::new(),
                body: "fixes the build".into(),
                nonce: "b1c4".into(),
            },
            ClientFrame::Typing {
                room: RoomId::new(),
                active: true,
            },
            ClientFrame::Join {
                name: "backend".into(),
            },
            ClientFrame::Leave {
                room: RoomId::new(),
            },
            ClientFrame::CreateRoom {
                name: "security".into(),
                topic: Some("blue team".into()),
                visibility: Visibility::InviteOnly,
            },
            ClientFrame::DeleteRoom {
                room: RoomId::new(),
            },
            ClientFrame::Invite {
                room: RoomId::new(),
                username: "alice".into(),
            },
            ClientFrame::OpenDm {
                username: "bob".into(),
            },
            ClientFrame::History {
                room: RoomId::new(),
                before: Some(MessageId::new()),
                limit: HISTORY_PAGE_DEFAULT,
            },
            ClientFrame::Members {
                room: RoomId::new(),
            },
            ClientFrame::ListRooms,
            ClientFrame::SetStatus {
                status: UserStatus::Coding,
                text: Some("deep in a bug".into()),
            },
            ClientFrame::SetDisplayName {
                display_name: "Neo".into(),
            },
            ClientFrame::MarkRead {
                room: RoomId::new(),
            },
            ClientFrame::Ping { nonce: 42 },
        ]
    }

    /// Every server frame, for the same reason.
    fn all_server_frames() -> Vec<ServerFrame> {
        vec![
            ServerFrame::Ready {
                user: sample_user(),
                rooms: vec![sample_joined_room()],
                server: ServerInfo {
                    name: "zion".into(),
                    version: "1.0.0".into(),
                    motd: Some("no system is safe".into()),
                    protocol: PROTOCOL_VERSION,
                },
            },
            ServerFrame::Message {
                message: sample_message(),
            },
            ServerFrame::Posted {
                message: sample_message(),
                nonce: "a3f2".into(),
            },
            ServerFrame::History {
                room: RoomId::new(),
                messages: vec![sample_message(), sample_message()],
                complete: false,
            },
            ServerFrame::RoomList {
                rooms: vec![sample_room()],
            },
            ServerFrame::RoomCreated {
                room: sample_room(),
            },
            ServerFrame::RoomDeleted {
                room: RoomId::new(),
            },
            ServerFrame::Joined {
                room: sample_joined_room(),
                members: vec![sample_user()],
            },
            ServerFrame::Left {
                room: RoomId::new(),
            },
            ServerFrame::MemberJoined {
                room: RoomId::new(),
                user: sample_user(),
            },
            ServerFrame::MemberLeft {
                room: RoomId::new(),
                user: UserId::new(),
            },
            ServerFrame::Members {
                room: RoomId::new(),
                members: vec![sample_user()],
            },
            ServerFrame::Presence {
                user: UserId::new(),
                status: UserStatus::Away,
                text: None,
            },
            ServerFrame::Typing {
                room: RoomId::new(),
                user: UserId::new(),
                display_name: "Alice".into(),
                active: true,
            },
            ServerFrame::ProfileUpdated {
                user: sample_user(),
            },
            ServerFrame::Invited {
                room: sample_room(),
                by: "alice".into(),
            },
            ServerFrame::Notice {
                text: "restarting in 5m".into(),
            },
            ServerFrame::error(ErrorCode::RateLimited, "slow down"),
            ServerFrame::Pong { nonce: 42 },
        ]
    }

    #[test]
    fn every_client_frame_round_trips() {
        for frame in all_client_frames() {
            let encoded = frame.encode().expect("frame encodes");
            let decoded = ClientFrame::decode(&encoded).expect("frame decodes");
            assert_eq!(decoded, frame, "round trip failed for {encoded}");
        }
    }

    #[test]
    fn every_server_frame_round_trips() {
        for frame in all_server_frames() {
            let encoded = frame.encode().expect("frame encodes");
            let decoded = ServerFrame::decode(&encoded).expect("frame decodes");
            assert_eq!(decoded, frame, "round trip failed for {encoded}");
        }
    }

    #[test]
    fn frames_carry_a_tag_field() {
        for frame in all_client_frames() {
            let json: serde_json::Value = serde_json::from_str(&frame.encode().unwrap()).unwrap();
            assert!(
                json.get("t").is_some_and(serde_json::Value::is_string),
                "missing tag: {json}"
            );
        }
        for frame in all_server_frames() {
            let json: serde_json::Value = serde_json::from_str(&frame.encode().unwrap()).unwrap();
            assert!(
                json.get("t").is_some_and(serde_json::Value::is_string),
                "missing tag: {json}"
            );
        }
    }

    #[test]
    fn tags_are_the_names_the_wire_documents() {
        let frame = ClientFrame::Send {
            room: RoomId::new(),
            body: "hi".into(),
            nonce: "n".into(),
        };
        assert!(frame.encode().unwrap().starts_with(r#"{"t":"send""#));

        let frame = ClientFrame::CreateRoom {
            name: "x".into(),
            topic: None,
            visibility: Visibility::Public,
        };
        assert!(frame.encode().unwrap().contains(r#""t":"create_room""#));
    }

    #[test]
    fn joined_room_flattens_the_room_it_wraps() {
        // The client reads `id` and `name` directly off the object rather than
        // through a nested `room` key; nesting here would be a protocol change.
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&sample_joined_room()).unwrap()).unwrap();
        assert!(
            json.get("id").is_some(),
            "room fields must be flattened: {json}"
        );
        assert!(json.get("name").is_some());
        assert!(json.get("role").is_some());
        assert!(json.get("room").is_none());
    }

    #[test]
    fn joined_room_round_trips_despite_the_flatten() {
        let joined = sample_joined_room();
        let json = serde_json::to_string(&joined).unwrap();
        assert_eq!(serde_json::from_str::<JoinedRoom>(&json).unwrap(), joined);
    }

    #[test]
    fn unknown_tags_are_rejected_rather_than_silently_dropped() {
        let err = ClientFrame::decode(r#"{"t":"self_destruct"}"#).unwrap_err();
        assert!(matches!(err, ProtoError::Decode(_)));
    }

    #[test]
    fn truncated_json_is_rejected() {
        assert!(ServerFrame::decode(r#"{"t":"pong","non"#).is_err());
    }

    #[test]
    fn room_routing_covers_every_room_bearing_frame() {
        assert!(ServerFrame::Message {
            message: sample_message()
        }
        .room()
        .is_some());
        assert!(ServerFrame::Joined {
            room: sample_joined_room(),
            members: vec![],
        }
        .room()
        .is_some());
        assert!(ServerFrame::Pong { nonce: 1 }.room().is_none());
        assert!(ServerFrame::Notice { text: "x".into() }.room().is_none());
    }

    #[test]
    fn joined_frame_routes_to_the_wrapped_rooms_id() {
        let joined = sample_joined_room();
        let expected = joined.room.id;
        let frame = ServerFrame::Joined {
            room: joined,
            members: vec![],
        };
        assert_eq!(frame.room(), Some(expected));
    }

    #[test]
    fn cheap_frames_bypass_the_flood_budget() {
        assert!(ClientFrame::Typing {
            room: RoomId::new(),
            active: true
        }
        .is_cheap());
        assert!(ClientFrame::Ping { nonce: 1 }.is_cheap());
        assert!(!ClientFrame::Send {
            room: RoomId::new(),
            body: "x".into(),
            nonce: "n".into(),
        }
        .is_cheap());
    }

    #[test]
    fn a_dm_joined_room_labels_itself_with_its_peer() {
        let joined = JoinedRoom {
            room: Room {
                kind: RoomKind::Dm,
                name: "dm-a-b".into(),
                ..sample_room()
            },
            role: MemberRole::Member,
            unread: 0,
            peer: Some("alice".into()),
        };
        assert_eq!(joined.label(), "@alice");
    }
}
