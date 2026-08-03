//! The one event stream the application consumes.
//!
//! Terminal input, network traffic and the animation clock all funnel into
//! [`Event`]; `update` is the only consumer. This single-inbox shape is what
//! keeps the UI logic testable — a test is just a sequence of events.

use ratatui::crossterm::event::{KeyEvent, MouseEvent};

use crate::net::ws::NetEvent;

/// Anything that can happen to the application.
#[derive(Debug)]
pub enum Event {
    /// A key was pressed.
    Key(KeyEvent),
    /// Text arrived via bracketed paste.
    Paste(String),
    /// Mouse activity (wheel scrolling, clicks).
    Mouse(MouseEvent),
    /// The terminal was resized.
    Resize,
    /// The ~30 Hz heartbeat: animations, typing expiry, toast expiry.
    Tick,
    /// The network layer reported something.
    Net(NetEvent),
}
