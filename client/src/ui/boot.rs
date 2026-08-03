//! The boot splash and the connection veil — the cinematic moments.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, ConnState};

use super::{anim, centered};

/// The big logo, 49 columns wide.
const LOGO: [&str; 6] = [
    "████████╗███████╗██████╗ ███████╗███████╗ █████╗ ",
    "╚══██╔══╝██╔════╝██╔══██╗██╔════╝██╔════╝██╔══██╗",
    "   ██║   █████╗  ██████╔╝█████╗  ███████╗███████║",
    "   ██║   ██╔══╝  ██╔══██╗██╔══╝  ╚════██║██╔══██║",
    "   ██║   ███████╗██║  ██║███████╗███████║██║  ██║",
    "   ╚═╝   ╚══════╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝",
];

/// The staged boot narration. Pure theatre — the work it claims is either
/// instant or genuinely happening (the socket dials behind the splash).
const BOOT_LINES: [&str; 6] = [
    "[ ok ] terminal seized",
    "[ ok ] entropy pool warmed",
    "[ ok ] keyring unlocked",
    "[ ok ] frame codec loaded",
    "[ ok ] handshake armed",
    "[ .. ] tracing the node…",
];

/// The full-screen splash shown for the first [`anim::BOOT_DURATION`].
pub fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let elapsed = app.started.elapsed();

    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Logo, or a compact mark when the terminal is narrow.
    if area.width >= 52 && area.height >= 18 {
        for row in LOGO {
            lines.push(Line::from(Span::styled(
                row,
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "✦ T E R E S A ✦",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )));
    }

    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        format!("v{} — no system is safe", crate::VERSION),
        Style::default().fg(theme.muted),
    )));
    lines.push(Line::default());

    // Staged narration.
    let visible = anim::boot_lines_visible(elapsed, BOOT_LINES.len());
    for (index, text) in BOOT_LINES.iter().take(visible).enumerate() {
        let style = if index + 1 == visible && visible < BOOT_LINES.len() {
            Style::default().fg(theme.secondary)
        } else {
            Style::default().fg(theme.muted)
        };
        lines.push(Line::from(Span::styled(*text, style)));
    }
    for _ in visible..BOOT_LINES.len() {
        lines.push(Line::default());
    }

    lines.push(Line::default());
    let bar_width = 40.min(area.width.saturating_sub(6));
    lines.push(Line::from(Span::styled(
        anim::progress_bar(bar_width, anim::boot_progress(elapsed)),
        Style::default().fg(theme.primary),
    )));

    let height = lines.len() as u16;
    let target = centered(area, area.width.min(56), height);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        target,
    );
}

/// The overlay shown over the body while the socket is down: spinner, cause,
/// countdown. Sits on top of the (dimmed by absence) chat rather than hiding
/// the whole app, so scrollback stays readable during a blip.
pub fn connection_veil(frame: &mut Frame<'_>, app: &App, body: Rect) {
    let theme = &app.theme;
    let elapsed = app.started.elapsed();

    let (headline, detail, color) = match &app.conn {
        ConnState::Connecting { attempt } => (
            format!("{} ESTABLISHING UPLINK", anim::spinner(elapsed)),
            if *attempt > 1 {
                format!("attempt #{attempt}")
            } else {
                "negotiating…".to_owned()
            },
            theme.warning,
        ),
        ConnState::Offline { reason, retry_at } => {
            let secs = retry_at
                .saturating_duration_since(std::time::Instant::now())
                .as_secs();
            (
                format!("{} LINK LOST", anim::spinner(elapsed)),
                format!("{reason} — retrying in {}s (Ctrl+R now)", secs + 1),
                theme.danger,
            )
        }
        ConnState::AuthFailed { message } => (
            "✖ SESSION TERMINATED".to_owned(),
            message.clone(),
            theme.danger,
        ),
        ConnState::Online { .. } => return,
    };

    let width = (detail.len() as u16 + 6).clamp(30, body.width.saturating_sub(2));
    let rect = centered(body, width, 5);

    let text = Text::from(vec![
        Line::from(Span::styled(
            headline,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(detail, Style::default().fg(theme.fg))),
    ]);

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text).alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(color))
                .style(Style::default().bg(theme.surface)),
        ),
        rect,
    );
}
