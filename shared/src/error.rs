//! Errors that both sides of the wire understand.
//!
//! [`ProtoError`] covers malformed data caught locally. [`ErrorCode`] is the
//! machine-readable half of [`crate::frame::ServerFrame::Error`]: the client
//! matches on the code to decide how to react (re-authenticate, back off,
//! surface a message) and shows the human string as-is.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A value that failed validation or parsing before it ever reached the network.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ProtoError {
    /// An id string was not a valid ULID.
    #[error("`{value}` is not a valid {kind}")]
    MalformedId {
        /// The id newtype that rejected the value.
        kind: &'static str,
        /// The offending input.
        value: String,
    },

    /// A field was outside its permitted length.
    #[error("{field} must be between {min} and {max} characters, got {actual}")]
    Length {
        /// Which field failed.
        field: &'static str,
        /// Inclusive lower bound.
        min: usize,
        /// Inclusive upper bound.
        max: usize,
        /// The length that was supplied.
        actual: usize,
    },

    /// A field contained characters that are not permitted for its kind.
    #[error("{field} contains disallowed characters: {reason}")]
    Charset {
        /// Which field failed.
        field: &'static str,
        /// Why it was rejected, phrased for a human.
        reason: &'static str,
    },

    /// A field that must not be blank was empty or all whitespace.
    #[error("{field} must not be empty")]
    Empty {
        /// Which field failed.
        field: &'static str,
    },

    /// A frame could not be decoded from its JSON representation.
    #[error("malformed frame: {0}")]
    Decode(String),
}

/// The machine-readable reason a request was refused.
///
/// Codes are stable: renaming a variant is a breaking protocol change, so the
/// serde representation is pinned explicitly rather than derived from the
/// variant name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Credentials did not match, or no session was presented.
    Unauthorized,
    /// Authenticated, but not permitted to do this.
    Forbidden,
    /// The session token expired or was revoked; the client must log in again.
    SessionExpired,
    /// The named user, room, or message does not exist.
    NotFound,
    /// The name is already taken.
    Conflict,
    /// The request was well-formed JSON but semantically invalid.
    Invalid,
    /// The client exceeded a rate limit and should back off.
    RateLimited,
    /// The client sent frames faster than the server could deliver them.
    Overloaded,
    /// Something failed server-side; the client did nothing wrong.
    Internal,
}

impl ErrorCode {
    /// Whether the client should discard its session and re-authenticate.
    #[must_use]
    pub const fn requires_reauth(self) -> bool {
        matches!(self, Self::Unauthorized | Self::SessionExpired)
    }

    /// Whether retrying the same request later could plausibly succeed.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Overloaded | Self::Internal)
    }

    /// A short lowercase slug, used for log lines and the status bar.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::SessionExpired => "session_expired",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Invalid => "invalid",
            Self::RateLimited => "rate_limited",
            Self::Overloaded => "overloaded",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reauth_codes_are_exactly_the_session_ones() {
        assert!(ErrorCode::Unauthorized.requires_reauth());
        assert!(ErrorCode::SessionExpired.requires_reauth());
        assert!(!ErrorCode::Forbidden.requires_reauth());
        assert!(!ErrorCode::NotFound.requires_reauth());
    }

    #[test]
    fn retryable_codes_exclude_client_mistakes() {
        assert!(ErrorCode::RateLimited.is_retryable());
        assert!(ErrorCode::Internal.is_retryable());
        assert!(!ErrorCode::Invalid.is_retryable());
        assert!(!ErrorCode::Conflict.is_retryable());
    }

    #[test]
    fn wire_names_are_pinned() {
        // These strings are protocol surface. If this test fails, an older
        // client can no longer understand the server.
        let json = serde_json::to_string(&ErrorCode::SessionExpired).unwrap();
        assert_eq!(json, "\"session_expired\"");
        assert_eq!(
            serde_json::to_string(&ErrorCode::RateLimited).unwrap(),
            "\"rate_limited\""
        );
    }

    #[test]
    fn display_matches_the_wire_name() {
        for code in [
            ErrorCode::Unauthorized,
            ErrorCode::Forbidden,
            ErrorCode::SessionExpired,
            ErrorCode::NotFound,
            ErrorCode::Conflict,
            ErrorCode::Invalid,
            ErrorCode::RateLimited,
            ErrorCode::Overloaded,
            ErrorCode::Internal,
        ] {
            let wire = serde_json::to_string(&code).unwrap();
            assert_eq!(format!("\"{code}\""), wire);
        }
    }
}
