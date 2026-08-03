//! TERESA server library.
//!
//! The binary in `main.rs` is a thin shell over this crate; everything is a
//! library so integration tests can assemble a real server in-process.
//!
//! Layering, outermost first:
//!
//! - [`api`] — HTTP router, WebSocket upgrade, the connection actor
//! - [`hub`] — in-memory fan-out between live connections
//! - [`services`] — the application: auth, rooms, messages, presence
//! - [`repo`] — persistence ports and their SQLite adapters
//! - [`security`] — hashing, tokens, rate limiting
//!
//! Dependencies point strictly downward; nothing below [`services`] knows a
//! network exists.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    // Row-count conversions from SQLite's i64 are checked with .max(0).
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    // Services return long Results; the alias in `error` documents them.
    clippy::missing_errors_doc,
    // The hub returns delivery counts that callers legitimately ignore.
    clippy::must_use_candidate
)]

pub mod api;
pub mod config;
pub mod error;
pub mod hub;
pub mod repo;
pub mod security;
pub mod services;
pub mod state;

/// The server crate version, reported in `ServerInfo`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use config::ServerConfig;
pub use error::{Result, ServerError};
