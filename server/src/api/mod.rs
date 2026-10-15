//! HTTP surface: router assembly and middleware.

pub mod auth;
pub mod files;
pub mod health;
pub mod releases;
pub mod ws;

use std::time::Duration;

use axum::extract::DefaultBodyLimit;
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

    // File bodies stream straight to disk, so neither the request timeout nor
    // the in-memory body cap applies; the service enforces its own size limit
    // while the bytes are still arriving.
    let transfer = Router::new()
        .route("/api/v1/rooms/{room}/files", post(files::upload))
        .route("/api/v1/files/{id}", get(files::download))
        .route(
            "/api/v1/client/download/{platform}",
            get(releases::download),
        )
        .layer(DefaultBodyLimit::disable());

    let rest = Router::new()
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/healthz", get(health::healthz))
        .route("/api/v1/metrics", get(health::metrics))
        .route("/api/v1/client/latest", get(releases::latest))
        // Outside /api/v1: these are typed by people, so they stay short.
        .route("/install.sh", get(releases::install_sh))
        .route("/install.ps1", get(releases::install_ps1))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ));

    realtime
        .merge(transfer)
        .merge(rest)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
