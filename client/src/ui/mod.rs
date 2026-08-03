//! Terminal rendering.
//!
//! [`render`] draws the whole frame from [`crate::app::App`] alone — no
//! channels, no clocks it does not receive, no globals. That is what lets the
//! snapshot tests drive it with a `TestBackend` and assert on the pixels.

pub mod anim;
pub mod boot;
pub mod highlight;
pub mod markdown;
pub mod overlays;
pub mod theme;
pub mod widgets;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use crate::app::{App, ConnState};

/// Draws one complete frame.
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();

    // The whole-screen boot splash owns the first moments.
    if app.started.elapsed() < anim::BOOT_DURATION {
        boot::render(frame, app, area);
        return;
    }

    // Background wash.
    frame.render_widget(
        ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(app.theme.bg)),
        area,
    );

    let [header_area, body_area, status_area, input_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(area);

    widgets::header(frame, app, header_area);

    // Side panels collapse on narrow terminals rather than crushing the chat.
    let wide = area.width >= 100;
    let medium = area.width >= 72;
    let constraints = if wide {
        vec![
            Constraint::Length(22),
            Constraint::Min(30),
            Constraint::Length(24),
        ]
    } else if medium {
        vec![Constraint::Length(18), Constraint::Min(30)]
    } else {
        vec![Constraint::Min(20)]
    };
    let columns = Layout::horizontal(constraints).split(body_area);

    match columns.len() {
        3 => {
            widgets::rooms(frame, app, columns[0]);
            widgets::messages(frame, app, columns[1]);
            widgets::members(frame, app, columns[2]);
        }
        2 => {
            widgets::rooms(frame, app, columns[0]);
            widgets::messages(frame, app, columns[1]);
        }
        _ => widgets::messages(frame, app, columns[0]),
    }

    widgets::status_bar(frame, app, status_area);
    widgets::input(frame, app, input_area);

    // Uplink veil: when not online, dim the world and say why.
    if !matches!(app.conn, ConnState::Online { .. }) {
        boot::connection_veil(frame, app, body_area);
    }

    widgets::toasts(frame, app, area);

    if app.overlay.is_some() {
        overlays::render(frame, app, area);
    }
}

/// Centres a `width`×`height` box inside `area`, clamped to fit.
#[must_use]
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
