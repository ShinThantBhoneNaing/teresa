//! The server's single error type.
//!
//! Everything fallible below the API layer returns [`ServerError`]. The API
//! layer then converts it exactly once — into an HTTP status for REST callers,
//! or a [`ServerFrame::Error`] for socket callers. Centralising that conversion
//! is what keeps an internal database message from ever reaching a client:
//! [`ServerError::client_message`] decides what is safe to say, and nothing
//! else formats an error for output.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use teresa_proto::{ErrorCode, ProtoError, ServerFrame};

/// Anything that can go wrong handling a request.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// No credentials, or the wrong ones.
    #[error("invalid username or password")]
    InvalidCredentials,

    /// The session token is unknown, expired or revoked.
    #[error("session expired")]
    SessionExpired,

    /// Authenticated, but not permitted.
    #[error("{0}")]
    Forbidden(&'static str),

    /// The named entity does not exist.
    #[error("{0} not found")]
    NotFound(&'static str),

    /// The name is taken.
    #[error("{0} already exists")]
    Conflict(String),

    /// Input failed validation.
    #[error(transparent)]
    Invalid(#[from] ProtoError),

    /// Input was structurally fine but semantically wrong.
    #[error("{0}")]
    BadRequest(String),

    /// A rate limit was exceeded.
    #[error("rate limit exceeded; retry in {retry_after_secs}s")]
    RateLimited {
        /// How long the caller should wait before trying again.
        retry_after_secs: u64,
    },

    /// The client could not keep up with its own outbound queue.
    #[error("connection is too far behind")]
    Overloaded,

    /// The database failed.
    #[error("database error")]
    Database(#[from] sqlx::Error),

    /// Password hashing or verification failed for a reason other than a mismatch.
    #[error("password hashing failed")]
    PasswordHash,

    /// Anything else the server did wrong.
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl ServerError {
    /// The protocol code this error maps to.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidCredentials => ErrorCode::Unauthorized,
            Self::SessionExpired => ErrorCode::SessionExpired,
            Self::Forbidden(_) => ErrorCode::Forbidden,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::Conflict(_) => ErrorCode::Conflict,
            Self::Invalid(_) | Self::BadRequest(_) => ErrorCode::Invalid,
            Self::RateLimited { .. } => ErrorCode::RateLimited,
            Self::Overloaded => ErrorCode::Overloaded,
            Self::Database(_) | Self::PasswordHash | Self::Internal(_) => ErrorCode::Internal,
        }
    }

    /// The HTTP status this error maps to.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::InvalidCredentials | Self::SessionExpired => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Invalid(_) | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Overloaded => StatusCode::SERVICE_UNAVAILABLE,
            Self::Database(_) | Self::PasswordHash | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// The text that is safe to send to a client.
    ///
    /// Internal failures collapse to a fixed string. A database error can carry
    /// table names, constraint names and occasionally row contents; a client is
    /// entitled to know that something broke, not to know what.
    #[must_use]
    pub fn client_message(&self) -> String {
        match self {
            Self::Database(_) | Self::PasswordHash | Self::Internal(_) => {
                "internal server error".to_owned()
            }
            other => other.to_string(),
        }
    }

    /// Whether this error should be logged at `error` rather than `debug`.
    ///
    /// A user typing a wrong password is not an incident. A failing database is.
    #[must_use]
    pub const fn is_server_fault(&self) -> bool {
        matches!(
            self,
            Self::Database(_) | Self::PasswordHash | Self::Internal(_)
        )
    }

    /// Renders this error as a protocol frame.
    #[must_use]
    pub fn to_frame(&self) -> ServerFrame {
        ServerFrame::error(self.code(), self.client_message())
    }
}

/// The JSON body of a failed REST call.
#[derive(Debug, Serialize)]
struct ErrorBody {
    /// Machine-readable reason.
    code: ErrorCode,
    /// Human-readable detail, already filtered by [`ServerError::client_message`].
    message: String,
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        // Logged here rather than at each call site, so no error path can
        // forget to record itself.
        if self.is_server_fault() {
            tracing::error!(error = ?self, "request failed");
        } else {
            tracing::debug!(error = %self, "request refused");
        }

        let body = axum::Json(ErrorBody {
            code: self.code(),
            message: self.client_message(),
        });

        let mut response = (self.status(), body).into_response();

        // Give a throttled client something better to act on than a guess.
        if let Self::RateLimited { retry_after_secs } = self {
            if let Ok(value) = retry_after_secs.to_string().parse() {
                response.headers_mut().insert("retry-after", value);
            }
        }

        response
    }
}

/// The server's result alias.
pub type Result<T, E = ServerError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_failures_never_leak_their_detail() {
        let err = ServerError::Internal(anyhow::anyhow!("connection string: user:hunter2@db"));
        assert_eq!(err.client_message(), "internal server error");
        assert!(!err.client_message().contains("hunter2"));
    }

    #[test]
    fn database_errors_never_leak_their_detail() {
        let err = ServerError::Database(sqlx::Error::RowNotFound);
        assert_eq!(err.client_message(), "internal server error");
    }

    #[test]
    fn client_mistakes_keep_their_explanation() {
        let err = ServerError::Conflict("room #backend".into());
        assert_eq!(err.client_message(), "room #backend already exists");
    }

    #[test]
    fn wrong_password_is_not_an_incident() {
        assert!(!ServerError::InvalidCredentials.is_server_fault());
        assert!(!ServerError::NotFound("room").is_server_fault());
        assert!(ServerError::Database(sqlx::Error::RowNotFound).is_server_fault());
    }

    #[test]
    fn statuses_match_their_protocol_codes() {
        assert_eq!(
            ServerError::InvalidCredentials.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ServerError::InvalidCredentials.code(),
            ErrorCode::Unauthorized
        );
        assert_eq!(
            ServerError::RateLimited {
                retry_after_secs: 30
            }
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            ServerError::RateLimited {
                retry_after_secs: 30
            }
            .code(),
            ErrorCode::RateLimited
        );
    }

    #[test]
    fn session_expiry_tells_the_client_to_reauthenticate() {
        assert!(ServerError::SessionExpired.code().requires_reauth());
        assert!(!ServerError::Forbidden("not a member")
            .code()
            .requires_reauth());
    }

    #[test]
    fn validation_errors_convert_from_the_protocol_crate() {
        let err: ServerError = ProtoError::Empty { field: "message" }.into();
        assert_eq!(err.code(), ErrorCode::Invalid);
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn frames_carry_the_filtered_message() {
        let frame = ServerError::Database(sqlx::Error::RowNotFound).to_frame();
        match frame {
            ServerFrame::Error { code, message } => {
                assert_eq!(code, ErrorCode::Internal);
                assert_eq!(message, "internal server error");
            }
            other => panic!("expected an error frame, got {other:?}"),
        }
    }
}
