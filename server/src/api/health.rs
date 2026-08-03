//! Liveness and operational metrics.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::Result;
use crate::repo::{MessageRepo, UserRepo};
use crate::state::SharedState;

/// `GET /api/v1/healthz` — cheap liveness for load balancers and scripts.
#[derive(Debug, Serialize)]
pub struct Health {
    /// Always `"ok"` if the process can answer at all.
    pub status: &'static str,
    /// Crate version.
    pub version: &'static str,
}

/// The health endpoint.
pub async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: crate::VERSION,
    })
}

/// `GET /api/v1/metrics` — a JSON gauge snapshot.
///
/// Deliberately JSON rather than Prometheus text: the audience is an operator
/// with `curl` and `jq`, and a scraper can translate trivially if one ever
/// appears.
#[derive(Debug, Serialize)]
pub struct Metrics {
    /// Seconds since the process started.
    pub uptime_secs: u64,
    /// Live WebSocket connections.
    pub connections: usize,
    /// Distinct accounts currently connected.
    pub online_users: usize,
    /// Registered accounts.
    pub total_users: u64,
    /// Stored messages.
    pub total_messages: u64,
    /// Rate-limiter buckets currently tracked.
    pub tracked_rate_keys: usize,
}

/// The metrics endpoint.
pub async fn metrics(State(state): State<SharedState>) -> Result<Json<Metrics>> {
    Ok(Json(Metrics {
        uptime_secs: state.started_at.elapsed().as_secs(),
        connections: state.core.hub().connection_count(),
        online_users: state.core.hub().live_user_count(),
        total_users: state.core.store().count_users().await?,
        total_messages: state.core.store().count_messages().await?,
        tracked_rate_keys: state.auth_limiter.tracked_keys() + state.message_limiter.tracked_keys(),
    }))
}
