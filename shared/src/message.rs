//! Chat messages.

use serde::{Deserialize, Serialize};

use crate::ids::{MessageId, RoomId};
use crate::stamp::Timestamp;
use crate::user::Author;

/// What kind of line this is.
///
/// System lines are carried as messages rather than as a separate frame so that
/// scrollback, history paging and rendering all have exactly one code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Something a user typed.
    #[default]
    Chat,
    /// A `/me` action, rendered in italics without a name header.
    Emote,
    /// Server narration: joins, leaves, topic changes, errors.
    System,
    /// A shared file, folder or archive. The body is a [`crate::FileCard`]
    /// as JSON; the bytes themselves travel over HTTP, never the socket.
    File,
}

impl MessageKind {
    /// Whether this kind should count toward a room's unread badge.
    ///
    /// System narration is deliberately excluded: a badge that lights up
    /// because someone joined trains people to ignore badges.
    #[must_use]
    pub const fn is_notable(self) -> bool {
        matches!(self, Self::Chat | Self::Emote | Self::File)
    }

    /// The lowercase wire name, also used as the storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Emote => "emote",
            Self::System => "system",
            Self::File => "file",
        }
    }
}

impl std::str::FromStr for MessageKind {
    type Err = crate::error::ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chat" => Ok(Self::Chat),
            "emote" => Ok(Self::Emote),
            "system" => Ok(Self::System),
            "file" => Ok(Self::File),
            _ => Err(crate::error::ProtoError::Charset {
                field: "message kind",
                reason: "expected one of: chat, emote, system, file",
            }),
        }
    }
}

/// A single message as stored and broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Stable identifier; sorts by creation time.
    pub id: MessageId,
    /// The room it belongs to.
    pub room_id: RoomId,
    /// Who sent it, stamped with their name at the time of sending.
    ///
    /// `None` for [`MessageKind::System`] lines, which the server authors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// Chat, emote or system narration.
    pub kind: MessageKind,
    /// The raw body. Markdown is rendered client-side, never pre-rendered here.
    pub body: String,
    /// When the server accepted it.
    pub created_at: Timestamp,
}

impl Message {
    /// Builds a system narration line for a room.
    #[must_use]
    pub fn system(room_id: RoomId, body: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            room_id,
            author: None,
            kind: MessageKind::System,
            body: body.into(),
            created_at: Timestamp::now(),
        }
    }

    /// The name to render in the message header.
    #[must_use]
    pub fn author_label(&self) -> &str {
        self.author.as_ref().map_or("system", Author::label)
    }

    /// Whether `username` is mentioned by name in the body.
    ///
    /// Matching is case-insensitive and requires the `@`. The character after
    /// the handle must not be a word character, so `@al` does not fire on
    /// `@alice` — without that check, anyone with a short handle would be
    /// notified by half the messages on the server.
    #[must_use]
    pub fn mentions(&self, username: &str) -> bool {
        if username.is_empty() {
            return false;
        }
        let haystack = self.body.to_lowercase();
        let needle = format!("@{}", username.to_lowercase());

        haystack.match_indices(&needle).any(|(index, matched)| {
            haystack[index + matched.len()..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_alphanumeric() && next != '_' && next != '-')
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::UserId;

    fn chat(body: &str) -> Message {
        Message {
            id: MessageId::new(),
            room_id: RoomId::new(),
            author: Some(Author {
                id: UserId::new(),
                username: "bob".into(),
                display_name: "Bob".into(),
            }),
            kind: MessageKind::Chat,
            body: body.into(),
            created_at: Timestamp::now(),
        }
    }

    #[test]
    fn mentions_match_case_insensitively() {
        assert!(chat("hey @Alice look").mentions("alice"));
        assert!(chat("hey @alice look").mentions("ALICE"));
    }

    #[test]
    fn mentions_require_the_sigil() {
        assert!(!chat("alice was here").mentions("alice"));
    }

    #[test]
    fn mentions_do_not_fire_on_a_longer_handle() {
        // The bug this guards: a user named `al` matching every `@alice`.
        assert!(!chat("ping @alice").mentions("al"));
        assert!(!chat("ping @alice-bot").mentions("alice"));
        assert!(!chat("ping @alice_2").mentions("alice"));
    }

    #[test]
    fn mentions_match_at_the_end_of_a_body() {
        assert!(chat("ping @alice").mentions("alice"));
    }

    #[test]
    fn mentions_match_before_punctuation() {
        assert!(chat("@alice, can you look?").mentions("alice"));
        assert!(chat("cc @alice.").mentions("alice"));
    }

    #[test]
    fn mentions_scan_past_a_false_start() {
        // The first occurrence is a prefix match that must be rejected without
        // abandoning the search for the real one later in the body.
        assert!(chat("@alicent and @alice").mentions("alice"));
    }

    #[test]
    fn an_empty_handle_never_matches() {
        assert!(!chat("@ hello").mentions(""));
    }

    #[test]
    fn system_lines_have_no_author_and_do_not_notify() {
        let msg = Message::system(RoomId::new(), "alice joined");
        assert!(msg.author.is_none());
        assert_eq!(msg.author_label(), "system");
        assert!(!msg.kind.is_notable());
    }

    #[test]
    fn system_lines_omit_the_author_field_on_the_wire() {
        let json = serde_json::to_string(&Message::system(RoomId::new(), "hi")).unwrap();
        assert!(!json.contains("author"));
    }

    #[test]
    fn message_kind_round_trips_through_its_storage_form() {
        for kind in [
            MessageKind::Chat,
            MessageKind::Emote,
            MessageKind::System,
            MessageKind::File,
        ] {
            assert_eq!(kind.as_str().parse::<MessageKind>().unwrap(), kind);
        }
        assert!("sticker".parse::<MessageKind>().is_err());
    }

    #[test]
    fn message_round_trips_through_json() {
        let msg = chat("ship it 🚀");
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), msg);
    }
}
