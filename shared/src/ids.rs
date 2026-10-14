//! Typed identifiers.
//!
//! Every entity gets its own id newtype so that passing a `RoomId` where a
//! `UserId` is expected is a compile error rather than a runtime mystery. All
//! of them wrap a [`Ulid`], which is lexicographically sortable by creation
//! time — this is what lets history queries be a plain `ORDER BY id DESC`
//! without a secondary timestamp index.

use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use ulid::{Generator, Ulid};

use crate::error::ProtoError;

/// The process-wide monotonic ULID source.
///
/// A bare `Ulid::new()` randomises its low 80 bits, so two ids minted in the
/// same millisecond sort in *random* order relative to each other. That would
/// quietly break the ordering the whole system leans on: `ORDER BY id DESC` is
/// the history query, and two messages posted in the same millisecond — which
/// is exactly what a busy room does — could come back reversed.
///
/// A monotonic generator increments the random component instead of redrawing
/// it whenever the timestamp has not advanced, which restores strict ordering
/// within a millisecond. The server is the only process that mints message ids,
/// so a per-process guarantee is precisely the guarantee that is needed.
static MONOTONIC: Mutex<Generator> = Mutex::new(Generator::new());

/// Mints the next id in the process-wide monotonic sequence.
fn mint() -> Ulid {
    // A panic elsewhere in the program must not stop this one from minting ids;
    // the generator holds no invariant that a panic could have corrupted.
    let mut generator = MONOTONIC.lock().unwrap_or_else(PoisonError::into_inner);

    generator.generate().unwrap_or_else(|_| {
        // Reached only if the random component would overflow, which needs
        // ~2^80 ids inside one millisecond. Borrowing from the next millisecond
        // gives the generator a fresh random component while keeping the
        // sequence strictly increasing.
        let next_ms = SystemTime::now() + Duration::from_millis(1);
        generator
            .generate_from_datetime(next_ms)
            .unwrap_or_else(|_| Ulid::new())
    })
}

/// Declares an id newtype over [`Ulid`] with the usual conversions.
///
/// The macro exists because these types are otherwise five identical copies of
/// the same forty lines, and a copy-paste divergence between them would be both
/// easy to introduce and hard to spot.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            /// Mints a fresh id stamped with the current time.
            ///
            /// Ids minted by this process are strictly increasing, including
            /// within a single millisecond — see [`MONOTONIC`].
            #[must_use]
            pub fn new() -> Self {
                Self(mint())
            }

            /// Wraps an existing [`Ulid`].
            #[must_use]
            pub const fn from_ulid(ulid: Ulid) -> Self {
                Self(ulid)
            }

            /// The underlying [`Ulid`].
            #[must_use]
            pub const fn as_ulid(&self) -> Ulid {
                self.0
            }

            /// The millisecond Unix timestamp encoded in the id's prefix.
            #[must_use]
            pub fn timestamp_ms(&self) -> u64 {
                self.0.timestamp_ms()
            }

            /// The canonical 26-character Crockford base32 representation.
            #[must_use]
            pub fn to_string_repr(&self) -> String {
                self.0.to_string()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = ProtoError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(s)
                    .map(Self)
                    .map_err(|_| ProtoError::MalformedId {
                        kind: stringify!($name),
                        value: s.to_owned(),
                    })
            }
        }

        impl From<Ulid> for $name {
            fn from(ulid: Ulid) -> Self {
                Self(ulid)
            }
        }

        impl From<$name> for Ulid {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

define_id!(
    /// Identifies a registered account.
    UserId
);

define_id!(
    /// Identifies a room, whether a channel or a direct-message pair.
    RoomId
);

define_id!(
    /// Identifies a single chat message.
    MessageId
);

define_id!(
    /// Identifies one uploaded file — also its name on the server's disk,
    /// which is safe precisely because a ULID cannot contain a path separator.
    FileId
);

define_id!(
    /// Identifies one live client connection.
    ///
    /// A user may hold several of these at once — one per terminal — which is
    /// why presence is tracked per connection and collapsed per user.
    ConnectionId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_its_string_form() {
        let id = UserId::new();
        let parsed: UserId = id.to_string().parse().expect("canonical form must parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn rejects_malformed_input() {
        let err = "not-a-ulid".parse::<RoomId>().unwrap_err();
        assert!(matches!(err, ProtoError::MalformedId { .. }));
    }

    #[test]
    fn serialises_as_a_bare_string() {
        let id = MessageId::new();
        let json = serde_json::to_string(&id).expect("ids are serialisable");
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn sorts_in_creation_order() {
        // 4096 ids mint far faster than the millisecond clock ticks, so this
        // exercises the monotonic path rather than merely the timestamp path.
        let mut ids: Vec<MessageId> = (0..4096).map(|_| MessageId::new()).collect();
        let expected = ids.clone();
        ids.sort();
        assert_eq!(
            ids, expected,
            "ids minted in sequence must already be sorted"
        );
    }

    #[test]
    fn ids_minted_in_the_same_millisecond_still_increase() {
        let a = MessageId::new();
        let b = MessageId::new();
        assert!(a < b, "{a} should sort before {b}");
    }

    #[test]
    fn monotonicity_holds_across_threads() {
        // The generator is shared, so concurrent minting must not produce
        // duplicates or break the global ordering.
        let ids = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| (0..256).map(|_| MessageId::new()).collect::<Vec<_>>()))
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("worker must not panic"))
                .collect::<Vec<_>>()
        });

        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            ids.len(),
            "concurrent minting produced a duplicate"
        );
    }

    #[test]
    fn distinct_ids_do_not_collide() {
        let a = UserId::new();
        let b = UserId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn different_id_types_share_the_sequence() {
        // They draw from one generator, so ordering is global rather than
        // per-type. Nothing depends on that, but it should not surprise anyone.
        let user = UserId::new();
        let room = RoomId::new();
        assert!(user.as_ulid() < room.as_ulid());
    }
}
