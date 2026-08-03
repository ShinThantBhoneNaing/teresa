//! Channels, direct messages and membership.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ProtoError;
use crate::ids::{RoomId, UserId};
use crate::stamp::Timestamp;
use crate::user::UserRole;

/// Whether a room is a named channel or a two-person conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomKind {
    /// A named, joinable channel such as `#backend`.
    #[default]
    Channel,
    /// A direct conversation between exactly two users.
    Dm,
}

impl RoomKind {
    /// The sigil the client prefixes to the room's name.
    #[must_use]
    pub const fn sigil(self) -> char {
        match self {
            Self::Channel => '#',
            Self::Dm => '@',
        }
    }

    /// The lowercase wire name, also used as the storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Dm => "dm",
        }
    }
}

impl FromStr for RoomKind {
    type Err = ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "channel" => Ok(Self::Channel),
            "dm" => Ok(Self::Dm),
            _ => Err(ProtoError::Charset {
                field: "room kind",
                reason: "expected one of: channel, dm",
            }),
        }
    }
}

/// Who may see and enter a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Listed to everyone; anyone may join.
    #[default]
    Public,
    /// Listed to everyone, but joining requires an invite.
    Private,
    /// Not listed at all; invisible until you are invited.
    InviteOnly,
}

impl Visibility {
    /// The lowercase wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::InviteOnly => "invite_only",
        }
    }

    /// Whether a user may join without being invited first.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Public)
    }

    /// Whether the room appears in `/rooms` for a user who is not a member.
    #[must_use]
    pub const fn is_discoverable(self) -> bool {
        matches!(self, Self::Public | Self::Private)
    }

    /// A one-character badge for the room list.
    #[must_use]
    pub const fn badge(self) -> char {
        match self {
            Self::Public => ' ',
            Self::Private => '⊘',
            Self::InviteOnly => '⊗',
        }
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Visibility {
    type Err = ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().replace('-', "_").as_str() {
            "public" | "open" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            "invite_only" | "invite" | "secret" => Ok(Self::InviteOnly),
            _ => Err(ProtoError::Charset {
                field: "visibility",
                reason: "expected one of: public, private, invite-only",
            }),
        }
    }
}

/// A member's authority inside one room.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    /// May read and post.
    #[default]
    Member,
    /// May also invite and remove members.
    Moderator,
    /// Created the room; may delete it.
    Owner,
}

impl MemberRole {
    /// The lowercase wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Moderator => "moderator",
            Self::Owner => "owner",
        }
    }

    /// Whether this role may invite others into the room.
    #[must_use]
    pub fn can_invite(self) -> bool {
        self >= Self::Moderator
    }

    /// Whether this role may delete the room.
    #[must_use]
    pub fn can_delete(self) -> bool {
        self == Self::Owner
    }
}

impl fmt::Display for MemberRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemberRole {
    type Err = ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "member" => Ok(Self::Member),
            "moderator" | "mod" => Ok(Self::Moderator),
            "owner" => Ok(Self::Owner),
            _ => Err(ProtoError::Charset {
                field: "member role",
                reason: "expected one of: member, moderator, owner",
            }),
        }
    }
}

/// A room as the client knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    /// Stable identifier.
    pub id: RoomId,
    /// Unique lowercase name, without the leading sigil.
    pub name: String,
    /// Optional one-line description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Channel or direct message.
    pub kind: RoomKind,
    /// Who may see and enter.
    pub visibility: Visibility,
    /// The creator. `None` for rooms seeded at first boot, which belong to nobody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<UserId>,
    /// When the room was created.
    pub created_at: Timestamp,
    /// How many users are members.
    pub member_count: u32,
}

impl Room {
    /// The name as it should be displayed, including its sigil.
    ///
    /// For a DM this is the *other* participant's handle rather than the
    /// internal `dm-…` name, which is an implementation detail no user should
    /// ever see. Callers supply that handle because only they know which side
    /// of the conversation they are on.
    #[must_use]
    pub fn display_name(&self, dm_peer: Option<&str>) -> String {
        match (self.kind, dm_peer) {
            (RoomKind::Dm, Some(peer)) => format!("@{peer}"),
            (RoomKind::Dm, None) => format!("@{}", self.name),
            (RoomKind::Channel, _) => format!("#{}", self.name),
        }
    }

    /// Whether a user with this server role may delete this room.
    ///
    /// Ownership is checked first so that a room's creator can always remove
    /// it, then moderator authority as an override for abandoned rooms.
    #[must_use]
    pub fn can_be_deleted_by(&self, user: UserId, role: UserRole) -> bool {
        // A DM has no owner and is never deleted — it is left, and reopening it
        // must return the same history.
        if self.kind == RoomKind::Dm {
            return false;
        }
        self.owner_id == Some(user) || role.can_moderate()
    }
}

/// The canonical name of the DM room between two users.
///
/// Sorting the two ids means `dm_room_name(a, b) == dm_room_name(b, a)`, so
/// "open a DM with alice" is an idempotent lookup-or-create on a unique index
/// rather than a two-way search that can race into two rooms for one pair.
#[must_use]
pub fn dm_room_name(a: UserId, b: UserId) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    format!("dm-{lo}-{hi}")
}

/// Recovers the two participants from a canonical DM room name.
///
/// Returns `None` if the name is not in `dm-<ulid>-<ulid>` form.
#[must_use]
pub fn dm_participants(name: &str) -> Option<(UserId, UserId)> {
    let rest = name.strip_prefix("dm-")?;
    // ULIDs are a fixed 26 characters and contain no hyphen, so splitting at
    // the delimiter is unambiguous.
    let (left, right) = rest.split_once('-')?;
    Some((left.parse().ok()?, right.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(name: &str, owner: Option<UserId>) -> Room {
        Room {
            id: RoomId::new(),
            name: name.into(),
            topic: None,
            kind: RoomKind::Channel,
            visibility: Visibility::Public,
            owner_id: owner,
            created_at: Timestamp::now(),
            member_count: 1,
        }
    }

    #[test]
    fn dm_names_are_order_independent() {
        let a = UserId::new();
        let b = UserId::new();
        assert_eq!(dm_room_name(a, b), dm_room_name(b, a));
    }

    #[test]
    fn dm_names_round_trip_to_their_participants() {
        let a = UserId::new();
        let b = UserId::new();
        let (lo, hi) = dm_participants(&dm_room_name(a, b)).unwrap();
        let mut expected = [a, b];
        expected.sort();
        assert_eq!((lo, hi), (expected[0], expected[1]));
    }

    #[test]
    fn dm_participants_rejects_channel_names() {
        assert!(dm_participants("backend").is_none());
        assert!(dm_participants("dm-nonsense").is_none());
    }

    #[test]
    fn dm_display_name_prefers_the_peer_handle() {
        let a = UserId::new();
        let b = UserId::new();
        let room = Room {
            kind: RoomKind::Dm,
            name: dm_room_name(a, b),
            visibility: Visibility::InviteOnly,
            ..channel("unused", None)
        };
        assert_eq!(room.display_name(Some("alice")), "@alice");
    }

    #[test]
    fn channel_display_name_carries_the_hash() {
        assert_eq!(channel("backend", None).display_name(None), "#backend");
    }

    #[test]
    fn owners_and_moderators_may_delete_a_channel() {
        let owner = UserId::new();
        let stranger = UserId::new();
        let room = channel("backend", Some(owner));

        assert!(room.can_be_deleted_by(owner, UserRole::Member));
        assert!(room.can_be_deleted_by(stranger, UserRole::Moderator));
        assert!(!room.can_be_deleted_by(stranger, UserRole::Member));
    }

    #[test]
    fn ownerless_seed_rooms_need_moderator_authority() {
        let room = channel("general", None);
        assert!(!room.can_be_deleted_by(UserId::new(), UserRole::Member));
        assert!(room.can_be_deleted_by(UserId::new(), UserRole::Admin));
    }

    #[test]
    fn dms_can_never_be_deleted() {
        let owner = UserId::new();
        let room = Room {
            kind: RoomKind::Dm,
            ..channel("dm-x", Some(owner))
        };
        assert!(!room.can_be_deleted_by(owner, UserRole::Admin));
    }

    #[test]
    fn invite_only_rooms_are_hidden_but_private_ones_are_listed() {
        assert!(!Visibility::InviteOnly.is_discoverable());
        assert!(Visibility::Private.is_discoverable());
        assert!(!Visibility::Private.is_open());
        assert!(Visibility::Public.is_open());
    }

    #[test]
    fn visibility_parses_its_aliases() {
        assert_eq!(
            "invite-only".parse::<Visibility>().unwrap(),
            Visibility::InviteOnly
        );
        assert_eq!(
            "SECRET".parse::<Visibility>().unwrap(),
            Visibility::InviteOnly
        );
        assert_eq!("open".parse::<Visibility>().unwrap(), Visibility::Public);
        assert!("nonsense".parse::<Visibility>().is_err());
    }

    #[test]
    fn member_roles_order_by_authority() {
        assert!(MemberRole::Owner > MemberRole::Moderator);
        assert!(MemberRole::Moderator.can_invite());
        assert!(!MemberRole::Member.can_invite());
        assert!(MemberRole::Owner.can_delete());
        assert!(!MemberRole::Moderator.can_delete());
    }

    #[test]
    fn room_kind_round_trips_through_its_storage_form() {
        for kind in [RoomKind::Channel, RoomKind::Dm] {
            assert_eq!(kind.as_str().parse::<RoomKind>().unwrap(), kind);
        }
        assert!("group".parse::<RoomKind>().is_err());
    }

    #[test]
    fn room_round_trips_through_json() {
        let room = channel("security", Some(UserId::new()));
        let json = serde_json::to_string(&room).unwrap();
        assert_eq!(serde_json::from_str::<Room>(&json).unwrap(), room);
    }
}
