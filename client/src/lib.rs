//! TERESA terminal client library.
//!
//! The binary in `main.rs` wires a terminal and a socket to what lives here;
//! everything with behaviour worth testing is in the library, and the UI
//! state machine ([`app`]) runs entirely without either.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    // Millisecond-scale numerics in animations and layout.
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    // Effects queues document themselves; per-fn error docs would be noise.
    clippy::missing_errors_doc
)]

pub mod app;
pub mod config;
pub mod intro;
pub mod net;
pub mod session;
pub mod ui;

/// The client crate version, shown by `/version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
