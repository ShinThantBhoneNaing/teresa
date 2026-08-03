//! Registration, login and logout over REST.
//!
//! Authentication is HTTP rather than a socket frame so that it can be rate
//! limited per IP before any account exists, cached behind nothing, and
//! debugged with `curl`.

use axum::extract::{ConnectInfo, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use teresa_proto::User;

use crate::error::{Result, ServerError};
use crate::state::SharedState;

/// Body of a registration request.
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// Desired handle.
    pub username: String,
    /// Password, verbatim; hashed before it is ever stored.
    pub password: String,
    /// Name to display; defaults to the username if omitted.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Body of a login request.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Account handle.
    pub username: String,
    /// Password, verbatim.
    pub password: String,
}

/// A successful authentication, either flavour.
#[derive(Debug, Serialize)]
pub struct AuthResponse {
    /// The bearer token for the WebSocket handshake and future requests.
    ///
    /// This is the only time the server ever emits it; it stores just a hash.
    pub token: String,
    /// The authenticated profile.
    pub user: User,
}

/// `POST /api/v1/auth/register`
pub async fn register(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>)> {
    check_auth_rate(&state, addr)?;

    let display_name = body.display_name.unwrap_or_else(|| body.username.clone());
    let (token, user) = state
        .core
        .register(&body.username, &body.password, &display_name)
        .await?;
    Ok((StatusCode::CREATED, Json(AuthResponse { token, user })))
}

/// `POST /api/v1/auth/login`
pub async fn login(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<AuthResponse>> {
    check_auth_rate(&state, addr)?;

    let (token, user) = state.core.login(&body.username, &body.password).await?;
    Ok(Json(AuthResponse { token, user }))
}

/// `POST /api/v1/auth/logout` — revokes the presented session.
pub async fn logout(State(state): State<SharedState>, headers: HeaderMap) -> Result<StatusCode> {
    let token = bearer_token(&headers)?;
    state.core.logout(token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Applies the per-IP authentication budget.
fn check_auth_rate(state: &SharedState, addr: SocketAddr) -> Result<()> {
    let decision = state.auth_limiter.check(addr.ip());
    if decision.is_allowed() {
        Ok(())
    } else {
        Err(ServerError::RateLimited {
            retry_after_secs: decision.retry_after_secs(),
        })
    }
}

/// Pulls the bearer token out of an `Authorization` header.
pub fn bearer_token(headers: &HeaderMap) -> Result<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or(ServerError::SessionExpired)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value.parse().unwrap());
        headers
    }

    #[test]
    fn bearer_extraction_accepts_the_standard_form() {
        let headers = headers_with("Bearer abc123");
        assert_eq!(bearer_token(&headers).unwrap(), "abc123");
    }

    #[test]
    fn bearer_extraction_rejects_everything_else() {
        assert!(bearer_token(&HeaderMap::new()).is_err());
        assert!(bearer_token(&headers_with("Basic dXNlcjpwdw==")).is_err());
        assert!(bearer_token(&headers_with("Bearer ")).is_err());
        assert!(
            bearer_token(&headers_with("bearer abc")).is_err(),
            "scheme is case-sensitive"
        );
    }
}
