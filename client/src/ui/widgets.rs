//! The main-screen panels: header, rooms, messages, members, status bar,
//! input, toasts.

use std::time::Instant;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use teresa_proto::{MessageKind, RoomKind, UserStatus};
use time::UtcOffset;

use crate::app::{App, ConnState, RoomState, ToastKind};

use super::{anim, markdown, theme::Theme};

/// The status glyph's colour.
fn status_color(theme: &Theme, status: UserStatus) -> ratatui::style::Color {
    match status {
        UserStatus::Online => theme.success,
        UserStatus::Offline => theme.muted,
        UserStatus::Busy => theme.danger,
        UserStatus::Coding => theme.secondary,
        UserStatus::Reviewing => theme.tertiary,
        UserStatus::Away => theme.warning,
    }
}

/// Renders a message timestamp in the local clock.
fn clock(app: &App, ts: teresa_proto::Timestamp) -> String {
    let utc = ts.to_datetime();
    let local = UtcOffset::current_local_offset().map_or(utc, |offset| utc.to_offset(offset));
    if app.config.clock_24h {
        format!("{:02}:{:02}", local.hour(), local.minute())
    } else {
        let (hour, meridiem) = match local.hour() {
            0 => (12, "a"),
            h @ 1..=11 => (h, "a"),
            12 => (12, "p"),
            h => (h - 12, "p"),
        };
        format!("{hour:2}:{:02}{meridiem}", local.minute())
    }
}

// --- header ----------------------------------------------------------------

/// One line: mark · server │ room + topic │ identity.
pub fn header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let mut spans = vec![
        Span::styled(" ✦ ", Style::default().fg(theme.primary)),
        Span::styled(
            "TERESA",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ✦ ", Style::default().fg(theme.primary)),
    ];

    if let Some(server) = &app.server {
        spans.push(Span::styled(
            format!(" {} ", server.name),
            Style::default().fg(theme.muted),
        ));
    }

    if let Some(room) = app.active_room() {
        spans.push(Span::styled("│ ", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            room.label(),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ));
        if let Some(topic) = &room.info.room.topic {
            spans.push(Span::styled(
                format!(" — {topic}"),
                Style::default().fg(theme.muted),
            ));
        }
    }

    let left = Line::from(spans);

    let right = app.me.as_ref().map_or_else(Line::default, |me| {
        Line::from(vec![
            Span::styled(
                format!("{} ", me.status.indicator()),
                Style::default().fg(status_color(theme, me.status)),
            ),
            Span::styled(
                me.display_name.clone(),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" @{} ", me.username),
                Style::default().fg(theme.muted),
            ),
        ])
    });

    let base = Style::default().bg(theme.surface);
    frame.render_widget(Paragraph::new(left).style(base), area);
    frame.render_widget(
        Paragraph::new(right)
            .style(base)
            .alignment(Alignment::Right),
        area,
    );
}

// --- rooms panel -----------------------------------------------------------

/// The left panel: channels, then direct messages, with unread badges.
pub fn rooms(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let mut lines = Vec::new();

    let section = |label: &str| {
        Line::from(Span::styled(
            format!(" {label}"),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let entry = |state: &RoomState, active: bool| {
        let mut spans = vec![Span::styled(
            if active { " ▸ " } else { "   " },
            Style::default().fg(theme.primary),
        )];
        let name_style = if active {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else if state.info.unread > 0 {
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg)
        };
        spans.push(Span::styled(state.label(), name_style));
        if state.mentions > 0 {
            spans.push(Span::styled(
                format!(" @{}", state.mentions),
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if state.info.unread > 0 {
            spans.push(Span::styled(
                format!(" {}", state.info.unread),
                Style::default().fg(theme.warning),
            ));
        }
        let mut line = Line::from(spans);
        if active {
            line = line.style(Style::default().bg(theme.highlight_bg));
        }
        line
    };

    lines.push(section("CHANNELS"));
    for id in &app.room_order {
        if let Some(state) = app.rooms.get(id) {
            if state.info.room.kind == RoomKind::Channel {
                lines.push(entry(state, app.active == Some(*id)));
            }
        }
    }

    let has_dms = app
        .room_order
        .iter()
        .filter_map(|id| app.rooms.get(id))
        .any(|s| s.info.room.kind == RoomKind::Dm);
    if has_dms {
        lines.push(Line::default());
        lines.push(section("DIRECT"));
        for id in &app.room_order {
            if let Some(state) = app.rooms.get(id) {
                if state.info.room.kind == RoomKind::Dm {
                    lines.push(entry(state, app.active == Some(*id)));
                }
            }
        }
    }

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    frame.render_widget(Paragraph::new(lines).block(block), area);

    // Bottom hint, inside the panel.
    if area.height > 2 {
        let hint = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " ctrl+w browse ",
                Style::default().fg(theme.muted),
            ))),
            hint,
        );
    }
}

// --- messages --------------------------------------------------------------

/// The centre panel: scrollback with markdown, grouped by author.
#[allow(clippy::too_many_lines)] // one pass over one panel; splitting it would
                                 // scatter the layout math it exists to keep together
pub fn messages(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default().style(Style::default().bg(theme.bg));
    let inner_width = area.width.saturating_sub(2).max(10);

    let Some(room) = app.active_room() else {
        let empty = Paragraph::new(Text::from(vec![
            Line::default(),
            Line::from(Span::styled(
                "no channel tuned",
                Style::default().fg(theme.muted),
            )),
            Line::from(Span::styled(
                "Ctrl+W opens the switchboard",
                Style::default().fg(theme.muted),
            )),
        ]))
        .alignment(Alignment::Center)
        .block(block);
        frame.render_widget(empty, area);
        return;
    };

    let height = usize::from(area.height);
    let budget = height + usize::from(room.scroll) + 8;

    // Build lines newest-first until the viewport (plus scroll) is filled,
    // then reverse. Bounds the per-frame work by what is visible, not by
    // what is stored.
    let mut chunks: Vec<Vec<Line<'static>>> = Vec::new();
    let mut lines_gathered = 0usize;
    let mut previous_author: Option<(String, i64)> = None;

    // Pending sends render at the very tail, dim, with a spinner.
    let mut pending: Vec<Line<'static>> = room
        .pending
        .values()
        .map(|send| {
            Line::from(vec![
                Span::styled(
                    format!("{} ", anim::spinner(send.at.elapsed())),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(
                    send.body.clone(),
                    Style::default()
                        .fg(theme.muted)
                        .add_modifier(Modifier::ITALIC),
                ),
            ])
        })
        .collect();
    lines_gathered += pending.len();

    for message in room.messages.iter().rev() {
        if lines_gathered >= budget {
            break;
        }
        let mut chunk: Vec<Line<'static>> = Vec::new();

        match message.kind {
            MessageKind::System => {
                chunk.push(Line::from(vec![
                    Span::styled("── ", Style::default().fg(theme.border)),
                    Span::styled(
                        message.body.clone(),
                        Style::default()
                            .fg(theme.muted)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            MessageKind::Emote => {
                let name = message.author_label().to_owned();
                let color =
                    theme.nick_color(message.author.as_ref().map_or("system", |a| &a.username));
                let mut line = Line::from(vec![
                    Span::styled("· ", Style::default().fg(theme.tertiary)),
                    Span::styled(
                        name,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {}", message.body),
                        Style::default()
                            .fg(theme.tertiary)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]);
                // Being the target of an action reads like a mention.
                let mentioned = app
                    .me
                    .as_ref()
                    .is_some_and(|me| message.mentions(&me.username));
                if mentioned {
                    line = line.style(Style::default().bg(theme.highlight_bg));
                }
                chunk.push(line);
            }
            MessageKind::Chat => {
                let username = message
                    .author
                    .as_ref()
                    .map_or("system", |a| a.username.as_str());
                let is_me = app
                    .me
                    .as_ref()
                    .zip(message.author.as_ref())
                    .is_some_and(|(me, a)| me.id == a.id);
                let mentioned = app
                    .me
                    .as_ref()
                    .is_some_and(|me| message.mentions(&me.username));

                // Same author within three minutes: continuation, no header.
                let stamp_minute = message.created_at.as_millis() / 60_000;
                let grouped = previous_author.as_ref().is_some_and(|(who, minute)| {
                    who == username && (minute - stamp_minute).abs() <= 3
                });

                let mut body_lines = markdown::render(&message.body, theme);
                if mentioned {
                    body_lines = body_lines
                        .into_iter()
                        .map(|l| l.style(Style::default().bg(theme.highlight_bg)))
                        .collect();
                }

                if grouped {
                    chunk.extend(body_lines);
                } else {
                    let name_color = if is_me {
                        theme.primary
                    } else {
                        theme.nick_color(username)
                    };
                    let mut header = vec![
                        Span::styled(
                            format!("{} ", clock(app, message.created_at)),
                            Style::default().fg(theme.muted),
                        ),
                        Span::styled(
                            message.author_label().to_owned(),
                            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if let Some(author) = &message.author {
                        if author.display_name != author.username {
                            header.push(Span::styled(
                                format!(" @{}", author.username),
                                Style::default().fg(theme.muted),
                            ));
                        }
                    }
                    chunk.push(Line::default());
                    chunk.push(Line::from(header));
                    chunk.extend(body_lines);
                }
                previous_author = Some((username.to_owned(), stamp_minute));
            }
        }

        lines_gathered += chunk.len();
        chunks.push(chunk);
    }

    let mut lines: Vec<Line<'static>> = chunks.into_iter().rev().flatten().collect();
    lines.append(&mut pending);

    // Typing indicator rides the last line of the panel.
    if !room.typing.is_empty() && app.config.typing_indicators {
        let names: Vec<&str> = room
            .typing
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect();
        let verb = if names.len() == 1 { "is" } else { "are" };
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} typing{}",
                names.join(", "),
                verb,
                anim::typing_dots(app.started.elapsed())
            ),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .block(block);

    // Scroll from the bottom: pin the tail unless the user has scrolled up.
    let total = paragraph.line_count(inner_width) as u16;
    let offset = total
        .saturating_sub(area.height)
        .saturating_sub(room.scroll);
    frame.render_widget(paragraph.scroll((offset, 0)), area);

    // "More below" marker while scrolled up.
    if room.scroll > 0 {
        let marker = Rect {
            x: area.x + area.width.saturating_sub(14),
            y: area.y + area.height.saturating_sub(1),
            width: 13.min(area.width),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                " ▼ more below ",
                Style::default().fg(theme.bg).bg(theme.warning),
            )),
            marker,
        );
    }
}

// --- members ---------------------------------------------------------------

/// The right panel: who is here, live first.
pub fn members(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let mut lines = Vec::new();

    if let Some(room) = app.active_room() {
        let (online, offline): (Vec<_>, Vec<_>) =
            room.members.iter().partition(|m| m.status.is_connected());

        lines.push(Line::from(Span::styled(
            format!(" ONLINE — {}", online.len()),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        )));
        for member in &online {
            let mut spans = vec![
                Span::styled(
                    format!(" {} ", member.status.indicator()),
                    Style::default().fg(status_color(theme, member.status)),
                ),
                Span::styled(
                    member.display_name.clone(),
                    Style::default().fg(theme.nick_color(&member.username)),
                ),
            ];
            // A landed action (/shoot and friends) blinks its glyph on the
            // target's name until it expires.
            if let Some(fx) = room.fx.iter().find(|fx| fx.user == member.id) {
                if fx.at.elapsed().as_millis() / 250 % 2 == 0 {
                    spans.push(Span::styled(
                        format!(" {}", fx.glyph),
                        Style::default()
                            .fg(theme.danger)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }
            if let Some(text) = &member.status_text {
                spans.push(Span::styled(
                    format!(" · {text}"),
                    Style::default().fg(theme.muted),
                ));
            }
            lines.push(Line::from(spans));
        }

        if !offline.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                format!(" OFFLINE — {}", offline.len()),
                Style::default()
                    .fg(theme.muted)
                    .add_modifier(Modifier::BOLD),
            )));
            for member in &offline {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {} ", member.status.indicator()),
                        Style::default().fg(theme.muted),
                    ),
                    Span::styled(
                        member.display_name.clone(),
                        Style::default().fg(theme.muted),
                    ),
                ]));
            }
        }
    }

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// --- status bar ------------------------------------------------------------

/// One line of truth: link state, latency, unread total, key hints.
pub fn status_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let elapsed = app.started.elapsed();

    let (link_text, link_color) = match &app.conn {
        ConnState::Online { .. } => ("LINK:SECURE".to_owned(), theme.success),
        ConnState::Connecting { attempt } => (
            format!("{} LINKING #{attempt}", anim::spinner(elapsed)),
            theme.warning,
        ),
        ConnState::Offline { retry_at, .. } => {
            let secs = retry_at.saturating_duration_since(Instant::now()).as_secs();
            (
                format!("{} RETRY {}s", anim::spinner(elapsed), secs + 1),
                theme.danger,
            )
        }
        ConnState::AuthFailed { .. } => ("AUTH DEAD".to_owned(), theme.danger),
    };

    let mut left = vec![
        Span::styled(
            format!(" {link_text} "),
            Style::default().fg(theme.bg).bg(link_color),
        ),
        Span::raw(" "),
    ];
    if let Some(rtt) = app.rtt_ms {
        left.push(Span::styled(
            format!("{rtt}ms "),
            Style::default().fg(theme.muted),
        ));
    }
    let unread_total: u32 = app.rooms.values().map(|r| r.info.unread).sum();
    if unread_total > 0 {
        left.push(Span::styled(
            format!("✉ {unread_total} unread "),
            Style::default().fg(theme.warning),
        ));
    }

    let hints = Line::from(Span::styled(
        "F1 help · ctrl+w rooms · ctrl+p profile · ctrl+n alerts ",
        Style::default().fg(theme.muted),
    ));

    let base = Style::default().bg(theme.surface);
    frame.render_widget(Paragraph::new(Line::from(left)).style(base), area);
    frame.render_widget(
        Paragraph::new(hints)
            .style(base)
            .alignment(Alignment::Right),
        area,
    );
}

// --- input -----------------------------------------------------------------

/// The bordered prompt. The border glows with the theme's focus colour.
pub fn input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let room_label = app
        .active_room()
        .map_or_else(|| "…".to_owned(), crate::app::RoomState::label);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(Span::styled(
            format!(" {room_label} "),
            Style::default().fg(theme.secondary),
        ))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    let prompt = "❯ ";
    let text = Line::from(vec![
        Span::styled(prompt, Style::default().fg(theme.primary).bold()),
        Span::styled(app.input.text().to_owned(), Style::default().fg(theme.fg)),
    ]);
    frame.render_widget(Paragraph::new(text).block(block), area);

    // Hardware cursor at the edit point (wrapped into the box width).
    let column = app.input.cursor_column() + prompt.len() as u16;
    let width = inner.width.max(1);
    frame.set_cursor_position((
        inner.x + column % width,
        inner.y + (column / width).min(inner.height.saturating_sub(1)),
    ));
}

// --- toasts ----------------------------------------------------------------

/// Corner popups, newest at the top.
pub fn toasts(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let width = 44.min(area.width.saturating_sub(2));
    if width < 10 {
        return;
    }

    let mut y = area.y + 1;
    for toast in app.toasts.iter().rev() {
        let color = match toast.kind {
            ToastKind::Info => theme.secondary,
            ToastKind::Success => theme.success,
            ToastKind::Mention => theme.warning,
            ToastKind::Error => theme.danger,
        };
        let text = Line::from(Span::styled(
            toast.text.clone(),
            Style::default().fg(theme.fg),
        ));
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color))
                .style(Style::default().bg(theme.surface)),
        );

        let height = (paragraph.line_count(width.saturating_sub(2)) as u16 + 2).min(5);
        if y + height >= area.y + area.height {
            break;
        }
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width + 1),
            y,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);
        y += height;
    }
}
