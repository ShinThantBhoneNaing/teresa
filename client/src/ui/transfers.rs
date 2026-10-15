//! File transfer visuals: the card a shared file becomes in the scrollback,
//! and the dock of live transfers above the status bar.
//!
//! The dock is deliberately an instrument panel — direction, stage, a
//! sub-cell progress bar, live throughput, ETA, and a hex readout of "the
//! wire" while bytes move — because watching a 2 GiB dump cross the LAN
//! should feel like something is happening, and should tell you exactly what.

use std::fmt::Write as _;
use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use teresa_proto::file::human_bytes;
use teresa_proto::{FileCard, FileKind};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Transfer};
use crate::net::transfer::{Outcome, Stage};

use super::{anim, theme::Theme};

/// Widest a file card grows, in columns.
const CARD_MAX: usize = 62;
/// Narrowest it is drawn at all.
const CARD_MIN: usize = 26;
/// Most transfers the dock shows at once.
pub const DOCK_ROWS: usize = 4;
/// The clickable stop button at the right end of a running transfer's row.
pub const CANCEL_BUTTON: &str = " ✖ cancel ";

// --- the card ----------------------------------------------------------------

/// The lines a file message renders as. `index` is its `/get` number.
#[must_use]
pub fn card_lines(app: &App, card: &FileCard, index: usize, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let width = width.min(CARD_MAX);
    if width < CARD_MIN {
        // Too narrow for a box: one honest line.
        return vec![Line::from(vec![
            Span::styled(
                format!("{} ", glyph(card.kind)),
                Style::default().fg(theme.primary),
            ),
            Span::styled(card.name.clone(), Style::default().fg(theme.fg).bold()),
            Span::styled(
                format!(" {} · /get {index}", human_bytes(card.size)),
                Style::default().fg(theme.muted),
            ),
        ])];
    }
    let inner = width - 4; // "│ " + " │"
    let edge = Style::default().fg(theme.border);
    let accent = kind_color(theme, card.kind);

    // ╭─▣ FILE ─────────── #1 ─╮
    let badge = format!("▣ {}", card.kind.badge());
    let tag = format!(" #{index} ");
    let fill = width.saturating_sub(badge.width() + tag.width() + 6);
    let top = Line::from(vec![
        Span::styled("╭─", edge),
        Span::styled(
            format!(" {badge} "),
            Style::default().fg(theme.bg).bg(accent).bold(),
        ),
        Span::styled("─".repeat(fill), edge),
        Span::styled(tag, Style::default().fg(theme.muted)),
        Span::styled("─╮", edge),
    ]);

    let name = vec![
        Span::styled(
            format!("{} ", glyph(card.kind)),
            Style::default().fg(accent),
        ),
        Span::styled(
            fit(&card.name, inner.saturating_sub(2)),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
    ];

    let mut facts = human_bytes(card.size);
    if let Some(entries) = card.entries {
        let _ = write!(facts, " · {entries} files");
    }
    let sha = &card.sha256[..card.sha256.len().min(12)];
    let facts = vec![
        Span::styled(format!("  {facts}"), Style::default().fg(theme.secondary)),
        Span::styled(
            format!(" · sha256:{sha}…"),
            Style::default().fg(theme.muted),
        ),
    ];

    let status = card_status(app, card, index, inner);

    let mut lines = vec![top];
    for row in [name, facts, status] {
        lines.push(boxed(row, inner, edge));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(width - 2)),
        edge,
    )));
    lines
}

/// The card's last row: how to get it, or how getting it is going.
fn card_status(app: &App, card: &FileCard, index: usize, inner: usize) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let Some(transfer) = app.transfer_for(card.id).filter(|t| !t.upload) else {
        return vec![
            Span::styled("  ❯ ", Style::default().fg(theme.primary)),
            Span::styled(
                format!("/get {index}"),
                Style::default().fg(theme.primary).bold(),
            ),
            Span::styled(
                if card.kind == FileKind::Folder {
                    "  pull + unpack"
                } else {
                    "  pull — you pick where"
                },
                Style::default().fg(theme.muted),
            ),
        ];
    };
    match &transfer.stage {
        Stage::Receiving { done, total } => {
            let ratio = ratio(*done, *total);
            let bar_width = inner.saturating_sub(22).clamp(6, 24) as u16;
            vec![
                Span::styled("  ▼ ", Style::default().fg(theme.secondary)),
                Span::styled(
                    anim::progress_bar(bar_width, ratio),
                    Style::default().fg(theme.secondary).bg(theme.surface),
                ),
                Span::styled(
                    format!(" {:>3.0}% {}", ratio * 100.0, rate(transfer)),
                    Style::default().fg(theme.fg),
                ),
            ]
        }
        Stage::Verifying => vec![Span::styled(
            format!(
                "  {} verifying sha256…",
                anim::spinner(transfer.started.elapsed())
            ),
            Style::default().fg(theme.warning),
        )],
        Stage::Unpacking => vec![Span::styled(
            format!("  {} unpacking…", anim::spinner(transfer.started.elapsed())),
            Style::default().fg(theme.tertiary),
        )],
        Stage::Done(_) => vec![Span::styled(
            "  ✔ pulled · integrity verified",
            Style::default().fg(theme.success),
        )],
        Stage::Failed(_) => vec![Span::styled(
            format!("  ✖ failed — /get {index} to retry"),
            Style::default().fg(theme.danger),
        )],
        Stage::Cancelled => vec![Span::styled(
            format!("  ⊘ cancelled — /get {index} to pull it again"),
            Style::default().fg(theme.muted),
        )],
        Stage::Packing { .. } | Stage::Sending { .. } => Vec::new(),
    }
}

/// Wraps a row in the card's side rails, padded to the inner width.
fn boxed(mut row: Vec<Span<'static>>, inner: usize, edge: Style) -> Line<'static> {
    let used: usize = row.iter().map(|s| s.content.width()).sum();
    let mut spans = vec![Span::styled("│ ", edge)];
    if used > inner {
        // Clip from the right; only the facts row can get here, on a tight fit.
        let mut budget = inner;
        row = row
            .into_iter()
            .filter_map(|span| {
                if budget == 0 {
                    return None;
                }
                let text = fit(&span.content, budget);
                budget = budget.saturating_sub(text.width());
                Some(Span::styled(text, span.style))
            })
            .collect();
    }
    let used: usize = row.iter().map(|s| s.content.width()).sum();
    spans.extend(row);
    spans.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
    spans.push(Span::styled(" │", edge));
    Line::from(spans)
}

// --- the dock ----------------------------------------------------------------

/// How many rows the dock wants right now.
#[must_use]
pub fn dock_height(app: &App) -> u16 {
    app.transfers.len().min(DOCK_ROWS) as u16
}

/// The transfers the dock shows, top row first.
pub fn dock_visible(app: &App) -> &[Transfer] {
    &app.transfers[app.transfers.len().saturating_sub(DOCK_ROWS)..]
}

/// One row per transfer, newest at the bottom, just above the status bar.
/// Running ones end in a [`CANCEL_BUTTON`] the mouse can hit.
pub fn dock(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = &app.theme;
    let width = usize::from(area.width);
    let button = CANCEL_BUTTON.width();
    app.dock_area.set(area);
    let lines: Vec<Line<'static>> = dock_visible(app)
        .iter()
        .map(|t| {
            if !t.active() || width < button + 30 {
                return dock_row(theme, t, width);
            }
            let mut line = dock_row(theme, t, width - button);
            let used = line.width();
            line.spans
                .push(Span::raw(" ".repeat((width - button).saturating_sub(used))));
            line.spans.push(Span::styled(
                CANCEL_BUTTON,
                Style::default().fg(theme.bg).bg(theme.danger).bold(),
            ));
            line
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.surface)),
        area,
    );
}

#[allow(clippy::too_many_lines)] // one arm per stage; splitting it scatters the row's layout
fn dock_row(theme: &Theme, t: &Transfer, width: usize) -> Line<'static> {
    let arrow = if t.upload { "▲" } else { "▼" };
    let (tag, color) = match &t.stage {
        Stage::Packing { .. } => ("ZIP", theme.tertiary),
        Stage::Sending { .. } => ("TX", theme.primary),
        Stage::Receiving { .. } => ("RX", theme.secondary),
        Stage::Verifying => ("SHA", theme.warning),
        Stage::Unpacking => ("UNZIP", theme.tertiary),
        Stage::Done(_) => ("OK", theme.success),
        Stage::Failed(_) => ("FAIL", theme.danger),
        Stage::Cancelled => ("STOP", theme.muted),
    };
    let route = if t.upload {
        format!(" → {}", t.place)
    } else {
        format!(" ← {}", t.place)
    };

    let mut spans = vec![
        Span::styled(
            format!(" {arrow} {tag:<5}"),
            Style::default().fg(theme.bg).bg(color).bold(),
        ),
        Span::raw(" "),
        Span::styled(fit(&t.name, 26), Style::default().fg(theme.fg).bold()),
        Span::styled(route, Style::default().fg(theme.muted)),
        Span::raw("  "),
    ];
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let room = width.saturating_sub(used);

    match &t.stage {
        Stage::Packing { done, total }
        | Stage::Sending { done, total }
        | Stage::Receiving { done, total } => {
            let indeterminate = *total == 0;
            let ratio = ratio(*done, *total);
            let numbers = if indeterminate {
                "scanning…".to_owned()
            } else {
                format!(
                    "{:>5.1}%  {}/{}  {}  ETA {}",
                    ratio * 100.0,
                    human_bytes(*done),
                    human_bytes(*total),
                    rate(t),
                    eta(t, *done, *total)
                )
            };
            // Leave ~20 columns for the wire readout when the row is wide enough.
            let bar_width = room.saturating_sub(numbers.width() + 4 + 20).clamp(8, 28);
            let bar = if indeterminate {
                sweep(bar_width, t.started.elapsed())
            } else {
                anim::progress_bar(bar_width as u16, ratio)
            };
            spans.push(Span::styled("▕", Style::default().fg(theme.border)));
            spans.push(Span::styled(bar, Style::default().fg(color).bg(theme.bg)));
            spans.push(Span::styled("▏ ", Style::default().fg(theme.border)));
            spans.push(Span::styled(numbers, Style::default().fg(theme.fg)));
            wire_readout(&mut spans, theme, t, width);
        }
        Stage::Verifying | Stage::Unpacking => {
            let label = if t.stage == Stage::Verifying {
                "checking sha256 against the server's"
            } else {
                "unpacking folder"
            };
            spans.push(Span::styled("▕", Style::default().fg(theme.border)));
            spans.push(Span::styled(
                sweep(16, t.stage_started.elapsed()),
                Style::default().fg(color).bg(theme.bg),
            ));
            spans.push(Span::styled("▏ ", Style::default().fg(theme.border)));
            spans.push(Span::styled(
                format!("{} {label}", anim::spinner(t.stage_started.elapsed())),
                Style::default().fg(color),
            ));
        }
        Stage::Done(outcome) => {
            let mut text = match t.wire {
                Some((bytes, took)) => format!(
                    "{} in {} · {} avg",
                    human_bytes(bytes),
                    secs(took),
                    per_second(bytes, took)
                ),
                None => "done".to_owned(),
            };
            text.push_str(match outcome {
                Outcome::Shared(_) => " · delivered",
                Outcome::Saved(_) => " · sha256 ✔ verified",
            });
            spans.push(Span::styled(text, Style::default().fg(theme.success)));
        }
        Stage::Failed(reason) => {
            spans.push(Span::styled(
                fit(reason, room),
                Style::default().fg(theme.danger),
            ));
        }
        Stage::Cancelled => {
            spans.push(Span::styled(
                if t.upload {
                    "cancelled — nothing was shared"
                } else {
                    "cancelled — partial download removed"
                },
                Style::default().fg(theme.muted),
            ));
        }
    }
    Line::from(spans)
}

/// Appends a spinner and a scrolling hex readout of "the wire", filling
/// whatever width is left. Pure function of time and id: no state to leak.
fn wire_readout(spans: &mut Vec<Span<'static>>, theme: &Theme, t: &Transfer, width: usize) {
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let room = width.saturating_sub(used + 4);
    if room < 6 {
        return;
    }
    let elapsed = t.started.elapsed();
    spans.push(Span::styled(
        format!("  {} ", anim::spinner(elapsed)),
        Style::default().fg(theme.primary),
    ));
    let bytes = room / 3;
    let frame = elapsed.as_millis() as u64 / 70;
    let mut state = (t.id.wrapping_mul(0x9E37_79B9_7F4A_7C15)) ^ frame;
    let mut hex = String::with_capacity(bytes * 3);
    for _ in 0..bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let _ = write!(hex, "{:02x} ", state & 0xff);
    }
    spans.push(Span::styled(
        hex.trim_end().to_owned(),
        Style::default().fg(theme.muted).add_modifier(Modifier::DIM),
    ));
}

// --- small pieces ---------------------------------------------------------------

/// A bright block sweeping back and forth: progress of unknown size.
fn sweep(width: usize, elapsed: Duration) -> String {
    const HEAD: [char; 5] = ['░', '▒', '▓', '▒', '░'];
    let span = width.saturating_sub(HEAD.len()).max(1);
    let step = (elapsed.as_millis() / 45) as usize % (span * 2);
    let at = if step < span { step } else { span * 2 - step };
    (0..width)
        .map(|i| {
            i.checked_sub(at)
                .and_then(|k| HEAD.get(k).copied())
                .unwrap_or(' ')
        })
        .collect()
}

fn ratio(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

/// Throughput of the current stage.
fn rate(t: &Transfer) -> String {
    let done = match t.stage {
        Stage::Packing { done, .. }
        | Stage::Sending { done, .. }
        | Stage::Receiving { done, .. } => done,
        _ => 0,
    };
    per_second(done, t.stage_started.elapsed())
}

fn per_second(bytes: u64, took: Duration) -> String {
    let secs = took.as_secs_f64();
    if secs < 0.05 {
        return "— /s".to_owned();
    }
    format!("{}/s", human_bytes((bytes as f64 / secs) as u64))
}

fn eta(t: &Transfer, done: u64, total: u64) -> String {
    let secs = t.stage_started.elapsed().as_secs_f64();
    if done == 0 || secs < 0.2 {
        return "--:--".to_owned();
    }
    let remaining = (total.saturating_sub(done) as f64) / (done as f64 / secs);
    let remaining = remaining.min(359_999.0) as u64;
    format!("{}:{:02}", remaining / 60, remaining % 60)
}

fn secs(took: Duration) -> String {
    let s = took.as_secs_f64();
    if s < 60.0 {
        format!("{s:.1}s")
    } else {
        format!("{}:{:02}", took.as_secs() / 60, took.as_secs() % 60)
    }
}

const fn glyph(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "⬢",
        FileKind::Folder => "▤",
        FileKind::Zip => "▦",
    }
}

const fn kind_color(theme: &Theme, kind: FileKind) -> Color {
    match kind {
        FileKind::File => theme.primary,
        FileKind::Folder => theme.secondary,
        FileKind::Zip => theme.tertiary,
    }
}

/// Clips to `max` display columns with an ellipsis.
fn fit(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_respects_display_width_not_bytes() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("abcdefghij", 5), "abcd…");
        assert!(fit("ဓာတ်ပုံများ-final.png", 8).width() <= 8);
    }

    #[test]
    fn the_sweep_stays_inside_its_width() {
        for ms in (0..5000).step_by(37) {
            assert_eq!(sweep(16, Duration::from_millis(ms)).chars().count(), 16);
        }
    }

    #[test]
    fn eta_and_rate_are_calm_before_there_is_data() {
        assert_eq!(per_second(0, Duration::ZERO), "— /s");
        assert_eq!(
            per_second(10 * 1024 * 1024, Duration::from_secs(1)),
            "10.0 MiB/s"
        );
    }
}
