//! Accounts, presence and permissions.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ProtoError;
use crate::ids::UserId;
use crate::stamp::Timestamp;

/// What a user is currently doing.
///
/// `Offline` is never *set* by a client — the server derives it from having no
/// live connections. Every other variant is a deliberate choice made with
/// `/status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    /// Connected and available.
    #[default]
    Online,
    /// No live connections.
    Offline,
    /// Connected but does not want to be interrupted.
    Busy,
    /// Heads-down writing code.
    Coding,
    /// Working through a pull request.
    Reviewing,
    /// Connected but not at the keyboard.
    Away,
}

impl UserStatus {
    /// Every status a user may set on themselves, in the order `/help` lists them.
    ///
    /// [`UserStatus::Offline`] is excluded: it is a server observation, not a
    /// choice.
    pub const SELECTABLE: [Self; 5] = [
        Self::Online,
        Self::Busy,
        Self::Coding,
        Self::Reviewing,
        Self::Away,
    ];

    /// The lowercase wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Busy => "busy",
            Self::Coding => "coding",
            Self::Reviewing => "reviewing",
            Self::Away => "away",
        }
    }

    /// A one-character indicator for the member list.
    ///
    /// These are all single-width glyphs from ranges that terminals have
    /// rendered reliably for decades. Emoji were tried and rejected: their
    /// width is inconsistent across terminals, which shifts the whole column.
    #[must_use]
    pub const fn indicator(self) -> char {
        match self {
            Self::Online => '●',
            Self::Offline => '○',
            Self::Busy => '◆',
            Self::Coding => '▶',
            Self::Reviewing => '◈',
            Self::Away => '◐',
        }
    }

    /// Whether the user is connected at all.
    #[must_use]
    pub const fn is_connected(self) -> bool {
        !matches!(self, Self::Offline)
    }

    /// Whether a notification should be surfaced rather than silently counted.
    ///
    /// `Busy` is the only connected status that suppresses noise; `Away` still
    /// rings, because the whole point of being away is to find out what you
    /// missed when you get back.
    #[must_use]
    pub const fn accepts_interruption(self) -> bool {
        matches!(
            self,
            Self::Online | Self::Coding | Self::Reviewing | Self::Away
        )
    }
}

impl fmt::Display for UserStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UserStatus {
    type Err = ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "online" | "active" | "here" => Ok(Self::Online),
            "offline" => Ok(Self::Offline),
            "busy" | "dnd" => Ok(Self::Busy),
            "coding" | "code" | "hacking" => Ok(Self::Coding),
            "reviewing" | "review" | "pr" => Ok(Self::Reviewing),
            "away" | "afk" => Ok(Self::Away),
            _ => Err(ProtoError::Charset {
                field: "status",
                reason: "expected one of: online, busy, coding, reviewing, away",
            }),
        }
    }
}

/// A user's authority across the whole server.
///
/// Distinct from [`crate::room::MemberRole`], which is authority within a single
/// room. An `Admin` may delete any room; a room `Owner` may delete only theirs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    /// An ordinary account.
    #[default]
    Member,
    /// May delete other people's rooms and messages.
    Moderator,
    /// Full control, including over other accounts.
    Admin,
}

impl UserRole {
    /// The lowercase wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
        }
    }

    /// Whether this role may act on rooms and messages it does not own.
    ///
    /// Relies on the `Ord` derive, so inserting a role between the existing
    /// ones in the enum declaration automatically places it correctly.
    #[must_use]
    pub fn can_moderate(self) -> bool {
        self >= Self::Moderator
    }
}

impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UserRole {
    type Err = ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "member" => Ok(Self::Member),
            "moderator" | "mod" => Ok(Self::Moderator),
            "admin" => Ok(Self::Admin),
            _ => Err(ProtoError::Charset {
                field: "role",
                reason: "expected one of: member, moderator, admin",
            }),
        }
    }
}

/// A user's public profile, as every other client sees it.
///
/// There is deliberately no password hash, email or session data here: this
/// struct is what gets broadcast to a room, so anything private must live in a
/// server-side type instead. Keeping the wire type separate from the storage
/// row makes leaking a secret an act of commission rather than omission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Stable identifier.
    pub id: UserId,
    /// Unique lowercase handle used for mentions and `/dm`.
    pub username: String,
    /// The name shown in the message list; may be changed with `/nick`.
    pub display_name: String,
    /// Current presence.
    pub status: UserStatus,
    /// Optional free text set with `/status`, e.g. "deep in a bug".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    /// Server-wide authority.
    pub role: UserRole,
    /// When the account was created.
    pub created_at: Timestamp,
}

impl User {
    /// The name to render in a message header, falling back to the handle if a
    /// display name was somehow stored blank.
    #[must_use]
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.username
        } else {
            &self.display_name
        }
    }

    /// The mention form of this user's handle, e.g. `@alice`.
    #[must_use]
    pub fn mention(&self) -> String {
        format!("@{}", self.username)
    }
}

/// The minimal author stamp embedded in every [`crate::message::Message`].
///
/// A message carries its author's name inline rather than just an id. Clients
/// therefore never have to resolve an id against a directory they may not have
/// loaded yet, and scrollback keeps showing the name that was in use when the
/// message was sent — which is what the conversation actually referred to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    /// The author's stable identifier.
    pub id: UserId,
    /// The author's handle at the time of sending.
    pub username: String,
    /// The author's display name at the time of sending.
    pub display_name: String,
}

impl Author {
    /// The name to render, falling back to the handle.
    #[must_use]
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.username
        } else {
            &self.display_name
        }
    }
}

impl From<&User> for Author {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_user() -> User {
        User {
            id: UserId::new(),
            username: "neo".into(),
            display_name: "Neo".into(),
            status: UserStatus::Coding,
            status_text: Some("following the white rabbit".into()),
            role: UserRole::Member,
            created_at: Timestamp::now(),
        }
    }

    #[test]
    fn status_parses_its_aliases() {
        assert_eq!("AFK".parse::<UserStatus>().unwrap(), UserStatus::Away);
        assert_eq!("dnd".parse::<UserStatus>().unwrap(), UserStatus::Busy);
        assert_eq!(
            " Hacking ".parse::<UserStatus>().unwrap(),
            UserStatus::Coding
        );
        assert!("napping".parse::<UserStatus>().is_err());
    }

    #[test]
    fn status_round_trips_through_its_wire_name() {
        for status in [
            UserStatus::Online,
            UserStatus::Offline,
            UserStatus::Busy,
            UserStatus::Coding,
            UserStatus::Reviewing,
            UserStatus::Away,
        ] {
            assert_eq!(status.as_str().parse::<UserStatus>().unwrap(), status);
        }
    }

    #[test]
    fn only_busy_suppresses_notifications_while_connected() {
        assert!(!UserStatus::Busy.accepts_interruption());
        assert!(UserStatus::Away.accepts_interruption());
        assert!(UserStatus::Online.accepts_interruption());
        assert!(!UserStatus::Offline.accepts_interruption());
    }

    #[test]
    fn selectable_statuses_exclude_offline() {
        assert!(!UserStatus::SELECTABLE.contains(&UserStatus::Offline));
        assert_eq!(UserStatus::SELECTABLE.len(), 5);
    }

    #[test]
    fn status_indicators_are_all_distinct() {
        let mut seen = Vec::new();
        for status in [
            UserStatus::Online,
            UserStatus::Offline,
            UserStatus::Busy,
            UserStatus::Coding,
            UserStatus::Reviewing,
            UserStatus::Away,
        ] {
            assert!(
                !seen.contains(&status.indicator()),
                "{status} reuses a glyph"
            );
            seen.push(status.indicator());
        }
    }

    #[test]
    fn roles_order_by_authority() {
        assert!(UserRole::Admin > UserRole::Moderator);
        assert!(UserRole::Moderator > UserRole::Member);
        assert!(UserRole::Admin.can_moderate());
        assert!(!UserRole::Member.can_moderate());
    }

    #[test]
    fn label_falls_back_to_the_handle() {
        let mut user = sample_user();
        user.display_name = "   ".into();
        assert_eq!(user.label(), "neo");
    }

    #[test]
    fn author_is_derived_from_a_user() {
        let user = sample_user();
        let author = Author::from(&user);
        assert_eq!(author.id, user.id);
        assert_eq!(author.username, "neo");
        assert_eq!(author.label(), "Neo");
    }

    #[test]
    fn absent_status_text_is_omitted_from_the_wire() {
        let mut user = sample_user();
        user.status_text = None;
        let json = serde_json::to_string(&user).unwrap();
        assert!(!json.contains("status_text"));
    }

    #[test]
    fn user_round_trips_through_json() {
        let user = sample_user();
        let json = serde_json::to_string(&user).unwrap();
        assert_eq!(serde_json::from_str::<User>(&json).unwrap(), user);
    }
}
