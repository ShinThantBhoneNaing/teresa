//! The composition root's product: everything a request handler may touch.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use teresa_proto::ConnectionId;

use crate::config::ServerConfig;
use crate::repo::sqlite::SqliteStore;
use crate::security::RateLimiter;
use crate::services::Core;

/// The application as instantiated in production: the service core over
/// SQLite. Handlers name this alias, never the generic, so swapping the
/// storage backend later touches exactly this line.
pub type App = Core<SqliteStore>;

/// Shared state handed to every handler.
#[derive(Debug)]
pub struct AppState {
    /// The application core.
    pub core: App,
    /// Login and registration attempts, per client IP.
    pub auth_limiter: RateLimiter<IpAddr>,
    /// Message postings, per live connection.
    pub message_limiter: RateLimiter<ConnectionId>,
    /// When the process started, for the metrics endpoint.
    pub started_at: Instant,
}

/// How handlers actually receive the state.
pub type SharedState = Arc<AppState>;

impl AppState {
    /// Assembles the state from an already-bootstrapped core.
    #[must_use]
    pub fn new(core: App) -> SharedState {
        let limits = &core.config().limits;
        let auth_limiter = RateLimiter::per_window(limits.auth_per_minute, Duration::from_secs(60));
        let message_limiter =
            RateLimiter::per_window(limits.messages_per_10s, Duration::from_secs(10));

        Arc::new(Self {
            core,
            auth_limiter,
            message_limiter,
            started_at: Instant::now(),
        })
    }

    /// One housekeeping sweep: expired sessions, idle channels, stale buckets.
    pub async fn housekeeping(&self) {
        if let Err(error) = self.core.housekeeping().await {
            tracing::warn!(%error, "housekeeping sweep failed");
        }
        self.auth_limiter.prune();
        self.message_limiter.prune();
    }

    /// The live configuration.
    #[must_use]
    pub fn config(&self) -> &ServerConfig {
        self.core.config()
    }
}
