//! Markdown → styled terminal lines.
//!
//! Renders the subset of markdown that makes sense in chat: emphasis, inline
//! code, fenced code blocks (syntax highlighted), lists, quotes, headings,
//! links and simple tables. Everything unknown degrades to its literal text —
//! a renderer that can lose characters has no place under someone's words.
//!
//! Also expands `:shortcode:` emoji, because `:rocket:` is muscle memory.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::emoji;
use super::highlight;
use super::theme::Theme;

/// Replaces `:name:` tokens with their emoji; unknown names stay literal.
///
/// Names resolve through [`emoji::find`], the same table Tab-completion and
/// the picker offer — so anything the client suggests is something it renders.
#[must_use]
pub fn expand_emoji(text: &str) -> String {
    if !text.contains(':') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find(':') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let matched = after
            .find(':')
            .and_then(|end| emoji::find(&after[..end]).map(|glyph| (glyph, end)));
        if let Some((glyph, end)) = matched {
            out.push_str(glyph);
            rest = &after[end + 1..];
        } else {
            // Not a shortcode; keep the colon, resume after it so the closing
            // colon can open the next candidate.
            out.push(':');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Renders a message body into themed lines.
#[must_use]
pub fn render(body: &str, theme: &Theme) -> Vec<Line<'static>> {
    let expanded = expand_emoji(body);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    Renderer::new(theme).run(Parser::new_ext(&expanded, options))
}

/// The walking state for one body.
struct Renderer<'t> {
    theme: &'t Theme,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    /// Nested inline styles (bold inside italic, etc.).
    styles: Vec<Style>,
    /// Prefix applied at the start of each new line (quotes, list bullets).
    prefix: Vec<Span<'static>>,
    /// Fenced block capture, when inside one.
    code: Option<CodeCapture>,
    /// Ordered-list counters, one per nesting level.
    list_counters: Vec<Option<u64>>,
    /// Table capture, when inside one.
    table: Option<TableCapture>,
    /// Destination of the link currently open, applied at `End(Link)`.
    pending_link: Option<String>,
}

struct CodeCapture {
    language: String,
    text: String,
}

struct TableCapture {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_header: bool,
}

impl<'t> Renderer<'t> {
    fn new(theme: &'t Theme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            current: Vec::new(),
            styles: vec![Style::default()],
            prefix: Vec::new(),
            code: None,
            list_counters: Vec::new(),
            table: None,
            pending_link: None,
        }
    }

    fn style(&self) -> Style {
        *self.styles.last().expect("style stack never empties")
    }

    fn push_style(&mut self, patch: impl FnOnce(Style) -> Style) {
        self.styles.push(patch(self.style()));
    }

    fn pop_style(&mut self) {
        if self.styles.len() > 1 {
            self.styles.pop();
        }
    }

    /// Ends the current visual line.
    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.current);
        // Skip lines that are entirely empty *and* have no prefix context —
        // block separations are added explicitly instead.
        if spans.iter().all(|s| s.content.is_empty()) && self.prefix.is_empty() {
            if !spans.is_empty() {
                self.lines.push(Line::default());
            }
            return;
        }
        let mut line = self.prefix.clone();
        line.extend(spans);
        self.lines.push(Line::from(line));
    }

    fn text(&mut self, text: &str) {
        if let Some(code) = &mut self.code {
            code.text.push_str(text);
            return;
        }
        if let Some(table) = &mut self.table {
            table.current_cell.push_str(text);
            return;
        }
        let style = self.style();
        // Text events can span newlines in loose contexts.
        let mut first = true;
        for piece in text.split('\n') {
            if !first {
                self.flush_line();
            }
            first = false;
            if !piece.is_empty() {
                self.current.push(Span::styled(piece.to_owned(), style));
            }
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per markdown construct
    fn run(mut self, parser: Parser<'_>) -> Vec<Line<'static>> {
        for event in parser {
            match event {
                Event::Text(text) => self.text(&text),

                Event::Code(code) => {
                    let style = Style::default()
                        .fg(self.theme.tertiary)
                        .bg(self.theme.code_bg);
                    self.current.push(Span::styled(format!(" {code} "), style));
                }

                Event::SoftBreak | Event::HardBreak => self.flush_line(),

                Event::Rule => {
                    self.flush_line();
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(24),
                        Style::default().fg(self.theme.border),
                    )));
                }

                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),

                Event::TaskListMarker(done) => {
                    let (glyph, color) = if done {
                        ("☑ ", self.theme.success)
                    } else {
                        ("☐ ", self.theme.muted)
                    };
                    self.current
                        .push(Span::styled(glyph, Style::default().fg(color)));
                }

                // Raw HTML has no terminal meaning; show it as-is, muted.
                Event::Html(html) | Event::InlineHtml(html) => {
                    let style = Style::default().fg(self.theme.muted);
                    self.current.push(Span::styled(html.into_string(), style));
                }

                Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
            }
        }
        self.flush_line();

        // Trim trailing blank lines a final paragraph break can leave.
        while self
            .lines
            .last()
            .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        {
            self.lines.pop();
        }
        self.lines
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if !self.lines.is_empty() {
                    self.flush_line();
                }
            }
            Tag::Heading { .. } => {
                self.flush_line();
                self.push_style(|s| s.fg(self.theme.primary).add_modifier(Modifier::BOLD));
                self.current
                    .push(Span::styled("§ ", Style::default().fg(self.theme.primary)));
            }
            Tag::BlockQuote(_) => {
                self.flush_line();
                self.prefix.push(Span::styled(
                    "▎ ",
                    Style::default().fg(self.theme.secondary),
                ));
                self.push_style(|s| s.fg(self.theme.muted).add_modifier(Modifier::ITALIC));
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                let language = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        lang.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some(CodeCapture {
                    language,
                    text: String::new(),
                });
            }
            Tag::List(start) => {
                self.flush_line();
                self.list_counters.push(start);
            }
            Tag::Item => {
                self.flush_line();
                let depth = self.list_counters.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let marker = match self.list_counters.last_mut() {
                    Some(Some(counter)) => {
                        let text = format!("{indent}{counter}. ");
                        *counter += 1;
                        text
                    }
                    _ => format!("{indent}• "),
                };
                self.current.push(Span::styled(
                    marker,
                    Style::default().fg(self.theme.secondary),
                ));
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.push_style(|s| {
                    s.fg(self.theme.secondary)
                        .add_modifier(Modifier::UNDERLINED)
                });
                // Remembered until End(Link), where it is appended after the
                // link text.
                self.pending_link = Some(dest_url.into_string());
            }
            Tag::Table(_) => {
                self.flush_line();
                self.table = Some(TableCapture {
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    current_cell: String::new(),
                    in_header: false,
                });
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_header = true;
                }
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_line();
                self.lines.push(Line::default());
            }
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush_line();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_line();
                self.prefix.pop();
                self.pop_style();
            }
            TagEnd::CodeBlock => self.finish_code_block(),
            TagEnd::List(_) => {
                self.flush_line();
                self.list_counters.pop();
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                if let Some(url) = self.pending_link.take() {
                    let shown: String = self.current.iter().map(|s| s.content.as_ref()).collect();
                    // Only append the target when the text is not already it.
                    if !shown.ends_with(&url) {
                        self.current.push(Span::styled(
                            format!(" ({url})"),
                            Style::default().fg(self.theme.muted),
                        ));
                    }
                }
                self.pop_style();
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    table
                        .current_row
                        .push(std::mem::take(&mut table.current_cell));
                }
            }
            TagEnd::TableRow | TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.rows.push(std::mem::take(&mut table.current_row));
                    if matches!(tag, TagEnd::TableHead) {
                        table.in_header = false;
                    }
                }
            }
            TagEnd::Table => self.finish_table(),
            _ => {}
        }
    }

    fn finish_code_block(&mut self) {
        let Some(capture) = self.code.take() else {
            return;
        };
        let gutter = Span::styled("▌ ", Style::default().fg(self.theme.border_focus));

        // Fence header names the language, dimly.
        if !capture.language.is_empty() {
            self.lines.push(Line::from(vec![
                gutter.clone(),
                Span::styled(
                    format!("⌈{}⌉", capture.language),
                    Style::default().fg(self.theme.muted),
                ),
            ]));
        }

        for spans in highlight::highlight(capture.text.trim_end_matches('\n'), &capture.language) {
            let mut line = vec![gutter.clone()];
            if spans.is_empty() {
                line.push(Span::styled(
                    String::new(),
                    Style::default().bg(self.theme.code_bg),
                ));
            }
            for span in spans {
                line.push(Span::styled(
                    span.content.into_owned(),
                    span.style.bg(self.theme.code_bg),
                ));
            }
            self.lines.push(Line::from(line));
        }
        self.lines.push(Line::default());
    }

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        if table.rows.is_empty() {
            return;
        }

        // Column widths from content, capped so one long cell cannot push the
        // table off every screen.
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0usize; columns];
        for row in &table.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.trim().width()).min(28);
            }
        }

        for (index, row) in table.rows.iter().enumerate() {
            let mut spans = vec![Span::styled("│ ", Style::default().fg(self.theme.border))];
            for (i, width) in widths.iter().enumerate() {
                let cell = row.get(i).map_or("", String::as_str).trim();
                let clipped: String = cell.chars().take(28).collect();
                let padding = width.saturating_sub(clipped.width());
                let style = if index == 0 {
                    Style::default()
                        .fg(self.theme.secondary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.fg)
                };
                spans.push(Span::styled(clipped, style));
                spans.push(Span::raw(" ".repeat(padding)));
                spans.push(Span::styled(" │ ", Style::default().fg(self.theme.border)));
            }
            self.lines.push(Line::from(spans));

            if index == 0 {
                let total: usize = widths.iter().map(|w| w + 3).sum::<usize>() + 2;
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(total.min(120)),
                    Style::default().fg(self.theme.border),
                )));
            }
        }
        self.lines.push(Line::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::by_name("matrix")
    }

    fn flat(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn plain_text_stays_plain() {
        let lines = render("just words", &theme());
        assert_eq!(flat(&lines), vec!["just words"]);
    }

    #[test]
    fn emoji_shortcodes_expand_and_unicode_passes_through() {
        assert_eq!(expand_emoji("ship it :rocket::fire:"), "ship it 🚀🔥");
        assert_eq!(expand_emoji("no such :nope: thing"), "no such :nope: thing");
        assert_eq!(expand_emoji("plain 🐧 stays"), "plain 🐧 stays");
        assert_eq!(expand_emoji("a:b:rocket:"), "a:b🚀");
        assert_eq!(expand_emoji("time 12:30:45"), "time 12:30:45");
    }

    #[test]
    fn bold_and_italic_set_modifiers() {
        let lines = render("**bold** and *lean*", &theme());
        let spans = &lines[0].spans;
        let bold = spans
            .iter()
            .find(|s| s.content == "bold")
            .expect("bold text present");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let lean = spans
            .iter()
            .find(|s| s.content == "lean")
            .expect("italic text present");
        assert!(lean.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_code_is_padded_and_tinted() {
        let lines = render("run `cargo test` now", &theme());
        let code = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("cargo test"))
            .expect("code span present");
        assert_eq!(code.content, " cargo test ");
        assert_eq!(code.style.bg, Some(theme().code_bg));
    }

    #[test]
    fn fenced_code_blocks_keep_every_line_and_carry_the_gutter() {
        let body = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        let lines = render(body, &theme());
        let text = flat(&lines).join("\n");
        assert!(text.contains("fn main() {"));
        assert!(text.contains("println!(\"hi\");"));
        assert!(text.contains("⌈rust⌉"), "language tag shown");
        // Every non-empty rendered line of the block carries the gutter glyph.
        let gutters = lines
            .iter()
            .filter(|l| !l.spans.is_empty())
            .filter(|l| l.spans[0].content.starts_with('▌'))
            .count();
        assert!(gutters >= 4, "code lines carry the gutter, got {gutters}");
    }

    #[test]
    fn multiline_messages_keep_their_breaks() {
        let lines = render("first\nsecond\nthird", &theme());
        assert_eq!(flat(&lines), vec!["first", "second", "third"]);
    }

    #[test]
    fn lists_get_bullets_and_numbers() {
        let lines = render("- alpha\n- beta\n\n1. one\n2. two", &theme());
        let text = flat(&lines);
        assert!(text.iter().any(|l| l.starts_with("• alpha")));
        assert!(text.iter().any(|l| l.starts_with("• beta")));
        assert!(text.iter().any(|l| l.starts_with("1. one")));
        assert!(text.iter().any(|l| l.starts_with("2. two")));
    }

    #[test]
    fn quotes_carry_their_bar() {
        let lines = render("> wisdom here", &theme());
        assert!(flat(&lines).iter().any(|l| l.starts_with("▎ wisdom")));
    }

    #[test]
    fn tables_align_and_keep_headers() {
        let body = "| name | role |\n|---|---|\n| neo | admin |\n| bob | member |";
        let lines = render(body, &theme());
        let text = flat(&lines);
        let header = text
            .iter()
            .find(|l| l.contains("name"))
            .expect("header row");
        assert!(header.contains("role"));
        assert!(text
            .iter()
            .any(|l| l.contains("neo") && l.contains("admin")));
    }

    #[test]
    fn links_show_text_then_target() {
        let lines = render("see [the docs](https://example.com)", &theme());
        let text = flat(&lines).join(" ");
        assert!(text.contains("the docs"));
        assert!(text.contains("https://example.com"));
    }

    #[test]
    fn nothing_is_ever_lost() {
        // A grab-bag of constructs; the visible text must survive them all.
        let body = "# head\n**b** *i* ~~s~~ `c`\n> q\n- l1\n\ntail";
        let text = flat(&render(body, &theme())).join(" ");
        for needle in ["head", "b", "i", "s", "c", "q", "l1", "tail"] {
            assert!(text.contains(needle), "{needle:?} lost in {text:?}");
        }
    }
}
