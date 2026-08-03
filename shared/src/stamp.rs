//! Wire timestamps.
//!
//! The protocol carries milliseconds since the Unix epoch as a bare integer
//! rather than an RFC 3339 string. It halves the byte cost of every message
//! frame, sidesteps every timezone ambiguity on the wire, and leaves
//! presentation entirely to the client — which is the only party that knows
//! what timezone the human is sitting in.

use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Milliseconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    /// The Unix epoch itself.
    pub const EPOCH: Self = Self(0);

    /// The current moment.
    #[must_use]
    pub fn now() -> Self {
        Self::from_datetime(OffsetDateTime::now_utc())
    }

    /// Wraps a raw millisecond count.
    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// The raw millisecond count.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Truncates a date-time to millisecond precision.
    ///
    /// Dates outside the roughly ±292-million-year range an `i64` of
    /// milliseconds can hold saturate to the nearest bound. Saturating rather
    /// than wrapping matters: a wrapped timestamp would sort a message from the
    /// far future to the top of the scrollback.
    #[must_use]
    pub fn from_datetime(dt: OffsetDateTime) -> Self {
        let millis = dt.unix_timestamp_nanos() / 1_000_000;
        Self(i64::try_from(millis).unwrap_or(if millis.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }))
    }

    /// Converts back to a UTC date-time.
    ///
    /// Returns [`OffsetDateTime::UNIX_EPOCH`] for values so far outside the
    /// representable range that they could only be corrupt — a garbled
    /// timestamp is not worth crashing a chat client over.
    #[must_use]
    pub fn to_datetime(self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.0) * 1_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    /// Milliseconds elapsed from `self` to `later`, saturating rather than
    /// wrapping if the two are absurdly far apart.
    #[must_use]
    pub const fn elapsed_to(self, later: Self) -> i64 {
        later.0.saturating_sub(self.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<OffsetDateTime> for Timestamp {
    fn from(dt: OffsetDateTime) -> Self {
        Self::from_datetime(dt)
    }
}

impl From<Timestamp> for OffsetDateTime {
    fn from(ts: Timestamp) -> Self {
        ts.to_datetime()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_datetime() {
        let ts = Timestamp::from_millis(1_750_000_000_123);
        assert_eq!(Timestamp::from_datetime(ts.to_datetime()), ts);
    }

    #[test]
    fn serialises_as_a_bare_integer() {
        let ts = Timestamp::from_millis(1_750_000_000_000);
        assert_eq!(serde_json::to_string(&ts).unwrap(), "1750000000000");
    }

    #[test]
    fn now_is_after_the_epoch() {
        assert!(Timestamp::now() > Timestamp::EPOCH);
    }

    #[test]
    fn handles_times_before_the_epoch() {
        let ts = Timestamp::from_millis(-86_400_000);
        assert_eq!(Timestamp::from_datetime(ts.to_datetime()), ts);
    }

    #[test]
    fn absurd_values_degrade_instead_of_panicking() {
        let ts = Timestamp::from_millis(i64::MAX);
        assert_eq!(ts.to_datetime(), OffsetDateTime::UNIX_EPOCH);
    }

    #[test]
    fn elapsed_saturates_at_the_extremes() {
        let far_past = Timestamp::from_millis(i64::MIN);
        let far_future = Timestamp::from_millis(i64::MAX);
        assert_eq!(far_past.elapsed_to(far_future), i64::MAX);
    }
}
