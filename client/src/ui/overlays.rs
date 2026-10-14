//! Modal overlays: help, profile, notifications, the room switcher, quit and
//! auth-failure dialogs.

use std::fmt::Write as _;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::update::{switcher_entries, SwitcherEntry};
use crate::app::{App, Overlay};

use super::centered;

/// Draws whichever overlay is open.
pub fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.overlay.as_ref() {
        Some(Overlay::Help) => help(frame, app, area),
        Some(Overlay::Profile) => profile(frame, app, area),
        Some(Overlay::Notifications) => notifications(frame, app, area),
        Some(Overlay::RoomSwitcher { filter, selected }) => {
            switcher(frame, app, area, filter, *selected);
        }
        Some(Overlay::ConfirmQuit) => confirm_quit(frame, app, area),
        Some(Overlay::SaveAs { card, from, path }) => save_as(frame, app, area, card, from, path),
        Some(Overlay::AuthFailed { message }) => auth_failed(frame, app, area, message),
        None => {}
    }
}

/// A themed modal frame.
fn modal<'a>(app: &App, title: &'a str) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(app.theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(app.theme.border_focus))
        .style(Style::default().bg(app.theme.surface))
}

fn help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let key = |k: &str| Span::styled(format!("{k:<18}"), Style::default().fg(theme.secondary));
    let txt = |t: &str| Span::styled(t.to_owned(), Style::default().fg(theme.fg));
    let head = |t: &str| {
        Line::from(Span::styled(
            t.to_owned(),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let lines = vec![
        head("── COMMANDS ────────────────────────────"),
        Line::from(vec![
            key("/join <room>"),
            txt("enter a channel (or switch)"),
        ]),
        Line::from(vec![
            key("/create <room>"),
            txt("new channel; --private / --invite-only"),
        ]),
        Line::from(vec![key("/delete [room]"), txt("delete a channel you own")]),
        Line::from(vec![key("/dm <user>"), txt("open a direct line")]),
        Line::from(vec![
            key("/send [@u] <path>"),
            txt("share a file or folder (folders zip)"),
        ]),
        Line::from(vec![key("/zip [@u] <path>"), txt("same, compressed")]),
        Line::from(vec![
            key("/get [n|name]"),
            txt("pull a file — asks where to save"),
        ]),
        Line::from(vec![
            key("/downloads [dir]"),
            txt("show or set the default folder"),
        ]),
        Line::from(vec![key("/files"), txt("list files shared here")]),
        Line::from(vec![
            key("/admin [op]"),
            txt("admin cleanup: stats·files·messages·compact"),
        ]),
        Line::from(vec![
            key("/admin users"),
            txt("admin: list accounts · remove @user"),
        ]),
        Line::from(vec![key("/invite <user>"), txt("invite into this room")]),
        Line::from(vec![key("/leave"), txt("leave this room")]),
        Line::from(vec![key("/me <action>"), txt("emote")]),
        Line::from(vec![
            key("/shoot <user>"),
            txt("gun someone down — 💥 on their name"),
        ]),
        Line::from(vec![key(""), txt("also /slap /hug /highfive /hack /nuke")]),
        Line::from(vec![key(""), txt("     /coffee /fire /rekt")]),
        Line::from(vec![key("/nick <name>"), txt("change display name")]),
        Line::from(vec![
            key("/status <s> [txt]"),
            txt("online·busy·coding·reviewing·away"),
        ]),
        Line::from(vec![
            key("/theme [name]"),
            txt("switch colours; bare = list"),
        ]),
        Line::from(vec![
            key("/rooms /users"),
            txt("list channels · list members"),
        ]),
        Line::from(vec![key("/whoami /version"), txt("identity · versions")]),
        Line::from(vec![key("/ping"), txt("round-trip time")]),
        Line::from(vec![key("/clear"), txt("clear this scrollback")]),
        Line::from(vec![key("/logout /quit"), txt("end session · leave app")]),
        Line::default(),
        head("── KEYS ────────────────────────────────"),
        Line::from(vec![key("Ctrl+W"), txt("room switchboard")]),
        Line::from(vec![
            key("Alt+↑/↓ · Alt+1-9"),
            txt("cycle rooms · jump to room"),
        ]),
        Line::from(vec![key("PgUp/PgDn · wheel"), txt("scrollback")]),
        Line::from(vec![key("Tab"), txt("complete names and /commands")]),
        Line::from(vec![key("↑/↓"), txt("input history")]),
        Line::from(vec![key("Ctrl+K"), txt("clear scrollback")]),
        Line::from(vec![key("Ctrl+R"), txt("reconnect now")]),
        Line::from(vec![key("Ctrl+P · Ctrl+N"), txt("profile · notifications")]),
        Line::from(vec![key("Ctrl+L"), txt("repaint")]),
        Line::from(vec![key("Esc"), txt("close overlay / jump to newest")]),
        Line::from(vec![key("Ctrl+C"), txt("quit (asks first)")]),
    ];

    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = centered(area, 64.min(area.width.saturating_sub(2)), height);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(modal(app, "MANUAL")),
        rect,
    );
}

fn profile(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let mut lines = Vec::new();

    match &app.me {
        Some(me) => {
            lines.push(Line::from(vec![
                Span::styled("  identity  ", Style::default().fg(theme.muted)),
                Span::styled(
                    me.display_name.clone(),
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  @{}", me.username),
                    Style::default().fg(theme.muted),
                ),
            ]));
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::styled("  presence  ", Style::default().fg(theme.muted)),
                Span::styled(
                    format!("{} {}", me.status.indicator(), me.status),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(
                    me.status_text
                        .as_ref()
                        .map(|t| format!(" — {t}"))
                        .unwrap_or_default(),
                    Style::default().fg(theme.fg),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  clearance ", Style::default().fg(theme.muted)),
                Span::styled(me.role.to_string(), Style::default().fg(theme.tertiary)),
            ]));
            if let Some(server) = &app.server {
                lines.push(Line::from(vec![
                    Span::styled("  node      ", Style::default().fg(theme.muted)),
                    Span::styled(
                        format!("{} v{}", server.name, server.version),
                        Style::default().fg(theme.fg),
                    ),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "  /nick and /status change these",
                Style::default().fg(theme.muted),
            )));
        }
        None => lines.push(Line::from(Span::styled(
            "  not connected",
            Style::default().fg(theme.muted),
        ))),
    }

    let rect = centered(
        area,
        52.min(area.width.saturating_sub(2)),
        lines.len() as u16 + 2,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(modal(app, "OPERATIVE FILE")),
        rect,
    );
}

fn notifications(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let mut lines = Vec::new();

    if app.notice_log.is_empty() {
        lines.push(Line::from(Span::styled(
            "  all quiet on the wire",
            Style::default().fg(theme.muted),
        )));
    }
    for (at, text) in app.notice_log.iter().take(20) {
        let age = at.elapsed().as_secs();
        let stamp = if age < 60 {
            format!("{age:>3}s")
        } else if age < 3600 {
            format!("{:>3}m", age / 60)
        } else {
            format!("{:>3}h", age / 3600)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {stamp} "), Style::default().fg(theme.muted)),
            Span::styled(text.clone(), Style::default().fg(theme.fg)),
        ]));
    }

    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = centered(area, 60.min(area.width.saturating_sub(2)), height);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(modal(app, "SIGNALS")),
        rect,
    );
}

fn switcher(frame: &mut Frame<'_>, app: &App, area: Rect, filter: &str, selected: usize) {
    let theme = &app.theme;
    let entries = switcher_entries(app);
    let selected = selected.min(entries.len().saturating_sub(1));

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(theme.secondary)),
            Span::styled(filter.to_owned(), Style::default().fg(theme.fg)),
            Span::styled("▏", Style::default().fg(theme.primary)),
        ]),
        Line::default(),
    ];

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  nothing matches",
            Style::default().fg(theme.muted),
        )));
    }

    for (index, entry) in entries.iter().enumerate() {
        let is_selected = index == selected;
        let marker = if is_selected { "  ▸ " } else { "    " };
        let mut spans = vec![Span::styled(marker, Style::default().fg(theme.primary))];

        match entry {
            SwitcherEntry::Joined(id) => {
                if let Some(state) = app.rooms.get(id) {
                    spans.push(Span::styled(
                        state.label(),
                        Style::default().fg(theme.fg).add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ));
                    if state.info.unread > 0 {
                        spans.push(Span::styled(
                            format!("  {} unread", state.info.unread),
                            Style::default().fg(theme.warning),
                        ));
                    }
                }
            }
            SwitcherEntry::Joinable(name) => {
                spans.push(Span::styled(
                    format!("#{name}"),
                    Style::default().fg(theme.muted),
                ));
                spans.push(Span::styled(
                    "  (join)",
                    Style::default().fg(theme.secondary),
                ));
            }
        }

        let mut line = Line::from(spans);
        if is_selected {
            line = line.style(Style::default().bg(theme.highlight_bg));
        }
        lines.push(line);
    }

    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "  type to filter · enter to go · esc to close",
        Style::default().fg(theme.muted),
    )));

    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = centered(area, 48.min(area.width.saturating_sub(2)), height);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(modal(app, "SWITCHBOARD")),
        rect,
    );
}

/// The `/get` prompt: which file, from whom, and the folder it will land in.
fn save_as(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    card: &teresa_proto::FileCard,
    from: &str,
    path: &str,
) {
    let theme = &app.theme;
    let width = 66.min(area.width.saturating_sub(2));
    let inner = usize::from(width.saturating_sub(4));
    let key = |k: &'static str| Span::styled(k, Style::default().fg(theme.primary).bold());
    let hint = |t: &'static str| Span::styled(t, Style::default().fg(theme.muted));

    // Long paths scroll: the end of what is being typed stays in view.
    let shown: String = {
        let chars: Vec<char> = path.chars().collect();
        let room = inner.saturating_sub(4);
        if chars.len() > room {
            std::iter::once('…')
                .chain(chars[chars.len() - room + 1..].iter().copied())
                .collect()
        } else {
            path.to_owned()
        }
    };
    let mut detail = teresa_proto::file::human_bytes(card.size);
    if let Some(n) = card.entries {
        let _ = write!(detail, " · folder of {n} files, unpacked on arrival");
    }

    let lines = vec![
        Line::from(vec![
            Span::styled(" ⬢ ", Style::default().fg(theme.primary)),
            Span::styled(
                card.landing_name().to_owned(),
                Style::default().fg(theme.fg).bold(),
            ),
        ]),
        Line::from(Span::styled(
            format!("   {detail} · from {from}"),
            Style::default().fg(theme.muted),
        )),
        Line::default(),
        Line::from(Span::styled(
            " save to",
            Style::default().fg(theme.secondary),
        )),
        Line::from(vec![
            Span::styled(" ❯ ", Style::default().fg(theme.primary).bold()),
            Span::styled(shown, Style::default().fg(theme.fg)),
            Span::styled("▏", Style::default().fg(theme.primary)),
        ]),
        Line::default(),
        Line::from(vec![
            key(" Enter"),
            hint(" save · "),
            key("Tab"),
            hint(" complete · "),
            key("Ctrl+U"),
            hint(" clear"),
        ]),
        Line::from(vec![
            key(" Ctrl+D"),
            hint(" save & make default · "),
            key("Esc"),
            hint(" cancel"),
        ]),
    ];

    let rect = centered(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(modal(app, "SAVE FILE")),
        rect,
    );
}

fn confirm_quit(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let text = Text::from(vec![
        Line::from(Span::styled(
            "cut the line?",
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled(
                "y",
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("es · ", Style::default().fg(theme.muted)),
            Span::styled(
                "n",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("o", Style::default().fg(theme.muted)),
        ]),
    ]);

    let rect = centered(area, 28, 5);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(modal(app, "DISCONNECT")),
        rect,
    );
}

fn auth_failed(frame: &mut Frame<'_>, app: &App, area: Rect, message: &str) {
    let theme = &app.theme;
    let text = Text::from(vec![
        Line::from(Span::styled(
            message.to_owned(),
            Style::default().fg(theme.danger),
        )),
        Line::default(),
        Line::from(Span::styled(
            "run teresa again to log back in",
            Style::default().fg(theme.fg),
        )),
        Line::from(Span::styled(
            "press enter to exit",
            Style::default().fg(theme.muted),
        )),
    ]);

    let rect = centered(area, 44.min(area.width.saturating_sub(2)), 6);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(modal(app, "SESSION TERMINATED")),
        rect,
    );
}
