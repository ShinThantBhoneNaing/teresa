//! HTTP surface: router assembly and middleware.

pub mod auth;
pub mod health;
pub mod ws;

use std::time::Duration;

use axum::routing::{get, post};
use axum::Router;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::state::SharedState;

/// Builds the complete router over the shared state.
///
/// Everything lives under `/api/v1/`; the version segment is the promise that
/// a breaking change gets a new prefix instead of new semantics on an old one.
pub fn router(state: SharedState) -> Router {
    // The WebSocket route must not sit behind the request timeout — a healthy
    // chat connection is *supposed* to be old.
    let realtime = Router::new().route("/api/v1/ws", get(ws::upgrade));

    let rest = Router::new()
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/healthz", get(health::healthz))
        .route("/api/v1/metrics", get(health::metrics))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ));

    realtime
        .merge(rest)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
