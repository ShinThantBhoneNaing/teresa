//! Syntax highlighting for fenced code blocks, via syntect.
//!
//! The syntax set costs real milliseconds to load, so it is initialised
//! lazily on the first code block ever rendered — a session with no code
//! pays nothing, and startup stays under budget.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme as SynTheme, ThemeSet};
use syntect::parsing::SyntaxSet;

/// The loaded syntax definitions and the one dark theme we use.
struct Engine {
    syntaxes: SyntaxSet,
    theme: SynTheme,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults();
        // base16-eighties reads well on every one of our dark backgrounds;
        // per-app-theme syntax themes would double the palette maintenance
        // for a distinction nobody can see in a chat window.
        let theme = themes
            .themes
            .remove("base16-eighties.dark")
            .unwrap_or_else(|| {
                themes
                    .themes
                    .into_values()
                    .next()
                    .expect("syntect ships at least one theme")
            });
        Engine {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            theme,
        }
    })
}

/// Highlights one code block into per-line spans.
///
/// `token` is whatever followed the fence backticks; matching is by syntect
/// token first, then file extension. Unknown languages come back unstyled —
/// text is never lost to a bad fence tag.
#[must_use]
pub fn highlight(code: &str, token: &str) -> Vec<Vec<Span<'static>>> {
    let engine = engine();
    let syntax = engine
        .syntaxes
        .find_syntax_by_token(token)
        .or_else(|| engine.syntaxes.find_syntax_by_extension(token))
        .unwrap_or_else(|| engine.syntaxes.find_syntax_plain_text());

    let mut highlighter = HighlightLines::new(syntax, &engine.theme);
    let mut lines = Vec::new();

    for line in code.lines() {
        // Syntect wants the newline for correct state transitions.
        let with_newline = format!("{line}\n");
        let spans = match highlighter.highlight_line(&with_newline, &engine.syntaxes) {
            Ok(regions) => regions
                .into_iter()
                .map(|(style, text)| {
                    Span::styled(
                        text.trim_end_matches('\n').to_owned(),
                        Style::default().fg(Color::Rgb(
                            style.foreground.r,
                            style.foreground.g,
                            style.foreground.b,
                        )),
                    )
                })
                .filter(|span| !span.content.is_empty())
                .collect(),
            Err(_) => vec![Span::raw(line.to_owned())],
        };
        lines.push(spans);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_text(lines: &[Vec<Span<'static>>]) -> String {
        lines
            .iter()
            .map(|spans| spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rust_code_keeps_its_text_and_gains_colour() {
        let code = "fn main() {\n    println!(\"hello\");\n}";
        let lines = highlight(code, "rust");

        assert_eq!(flat_text(&lines), code, "no character may be lost");
        let coloured = lines
            .iter()
            .flatten()
            .filter(|s| s.style.fg.is_some())
            .count();
        assert!(coloured > 0, "rust code should get at least one colour");
    }

    #[test]
    fn unknown_languages_pass_through_unharmed() {
        let code = "SOME ??? gibberish ///";
        let lines = highlight(code, "no-such-language");
        assert_eq!(flat_text(&lines), code);
    }

    #[test]
    fn python_matches_by_extension_token() {
        let lines = highlight("print('hello')", "py");
        assert_eq!(flat_text(&lines), "print('hello')");
    }

    #[test]
    fn empty_code_produces_no_lines() {
        assert!(highlight("", "rust").is_empty());
    }
}
