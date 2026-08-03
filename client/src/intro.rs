//! The animated welcome — everything you see before the alt screen.
//!
//! The login flow gets the same care as the TUI: the logo materialises line
//! by line, a ✦ mark breathes beside shimmering status text while *real*
//! work happens behind it (probing the node, forging the identity), and the
//! narration types itself out. All of it degrades honestly: no TTY, `NO_COLOR`
//! or `TERM=dumb` turns every animation into plain instantaneous text, so
//! scripts and pipes never wait on theatre.
//!
//! Frame *content* is computed by pure functions (tested below); only the
//! printing and sleeping live in the async helpers.

use std::io::{IsTerminal, Write};
use std::time::Duration;

/// The product mark. One glyph, single-width everywhere.
pub const MARK: char = '✦';

/// The TERESA letterform, 49 columns.
pub const LOGO: [&str; 6] = [
    "████████╗███████╗██████╗ ███████╗███████╗ █████╗ ",
    "╚══██╔══╝██╔════╝██╔══██╗██╔════╝██╔════╝██╔══██╗",
    "   ██║   █████╗  ██████╔╝█████╗  ███████╗███████║",
    "   ██║   ██╔══╝  ██╔══██╗██╔══╝  ╚════██║██╔══██║",
    "   ██║   ███████╗██║  ██║███████╗███████║██║  ██║",
    "   ╚═╝   ╚══════╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝",
];

/// The gradient the brand lives in: violet → orchid → sky.
const STOPS: [(u8, u8, u8); 3] = [(178, 132, 255), (232, 148, 255), (120, 220, 255)];

/// The spinner frames, shared with the TUI's vocabulary.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What the terminal can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    /// 24-bit colour.
    True,
    /// xterm-256 approximations.
    Indexed,
    /// No colour at all.
    Plain,
}

impl Palette {
    /// Reads the environment the way the TUI does, plus `NO_COLOR`.
    #[must_use]
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some()
            || std::env::var("TERM").is_ok_and(|t| t == "dumb")
        {
            return Self::Plain;
        }
        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
        if truecolor {
            Self::True
        } else {
            Self::Indexed
        }
    }
}

/// Linear blend across the three gradient stops at `t` ∈ [0, 1].
fn blend(t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0) * 2.0;
    let (from, to, local) = if t < 1.0 {
        (STOPS[0], STOPS[1], t)
    } else {
        (STOPS[1], STOPS[2], t - 1.0)
    };
    let mix = |a: u8, b: u8| -> u8 {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * local).round() as u8
    };
    (mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

/// Nearest 256-palette index for a gradient colour (6×6×6 cube).
fn indexed(rgb: (u8, u8, u8)) -> u8 {
    let scale = |v: u8| -> u8 {
        match v {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    };
    16 + 36 * scale(rgb.0) + 6 * scale(rgb.1) + scale(rgb.2)
}

/// One character wrapped in its gradient colour.
fn paint(glyph: char, position: f32, palette: Palette) -> String {
    match palette {
        Palette::Plain => glyph.to_string(),
        Palette::Indexed => format!("\x1b[38;5;{}m{glyph}", indexed(blend(position))),
        Palette::True => {
            let (red, green, blue) = blend(position);
            format!("\x1b[38;2;{red};{green};{blue}m{glyph}")
        }
    }
}

/// Renders `text` with the gradient swept to `phase` — the shimmer is the
/// phase advancing frame over frame, exactly like a light moving across the
/// word. Pure: same inputs, same bytes.
#[must_use]
pub fn shimmer(text: &str, phase: f32, palette: Palette) -> String {
    if palette == Palette::Plain {
        return text.to_owned();
    }
    let count = text.chars().count().max(1) as f32;
    let mut out = String::with_capacity(text.len() * 12);
    for (i, c) in text.chars().enumerate() {
        // A moving triangular highlight over a base gradient.
        let base = i as f32 / count;
        let sweep = ((base - phase).rem_euclid(1.0) - 0.5).abs() * 2.0;
        out.push_str(&paint(c, sweep, palette));
    }
    out.push_str("\x1b[0m");
    out
}

/// One spinner frame for a moment in time.
#[must_use]
pub fn spinner(elapsed: Duration) -> char {
    SPINNER[(elapsed.as_millis() / 80) as usize % SPINNER.len()]
}

/// A logo row painted top-to-bottom through the gradient.
#[must_use]
pub fn logo_row(row: usize, palette: Palette) -> String {
    let t = row as f32 / (LOGO.len() - 1) as f32;
    if palette == Palette::Plain {
        return LOGO[row].to_owned();
    }
    let painted: String = LOGO[row].chars().map(|c| paint(c, t, palette)).collect();
    format!("{painted}\x1b[0m")
}

/// The animated welcome, or its instantaneous plain fallback.
pub struct Intro {
    palette: Palette,
    animate: bool,
}

impl Intro {
    /// Detects terminal capabilities.
    #[must_use]
    pub fn new() -> Self {
        let tty = std::io::stdout().is_terminal();
        let palette = if tty {
            Palette::detect()
        } else {
            Palette::Plain
        };
        Self {
            palette,
            animate: tty && palette != Palette::Plain,
        }
    }

    /// A fixed configuration, for tests.
    #[must_use]
    pub const fn with(palette: Palette, animate: bool) -> Self {
        Self { palette, animate }
    }

    fn sleep(&self, ms: u64) {
        if self.animate {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }

    /// The logo, materialising one row at a time.
    pub fn reveal_logo(&self) {
        println!();
        for row in 0..LOGO.len() {
            println!("  {}", logo_row(row, self.palette));
            let _ = std::io::stdout().flush();
            self.sleep(55);
        }
    }

    /// The welcome card under the logo.
    pub fn welcome(&self, version: &str, node: &str, tls: bool) {
        let title = format!("{MARK} Welcome to TERESA v{version}");
        let sub = format!(
            "node {node} · {}",
            if tls { "tls" } else { "plaintext (local)" }
        );
        let width = title.chars().count().max(sub.chars().count()) + 4;
        let bar = "─".repeat(width);

        println!();
        println!("  ╭{bar}╮");
        println!(
            "  │  {}{}  │",
            shimmer(&title, 0.25, self.palette),
            " ".repeat(width - 4 - title.chars().count())
        );
        println!(
            "  │  {}{}  │",
            self.dim(&sub),
            " ".repeat(width - 4 - sub.chars().count())
        );
        println!("  ╰{bar}╯");
        println!();
    }

    /// Types a narration line out character by character.
    pub fn narrate(&self, text: &str) {
        if !self.animate {
            println!("  {text}");
            return;
        }
        print!("  ");
        for c in text.chars() {
            print!("{c}");
            let _ = std::io::stdout().flush();
            std::thread::sleep(Duration::from_millis(9));
        }
        println!();
    }

    /// Runs `work` while a ✦ spinner and shimmering label hold the line;
    /// replaces the line with ✔ (or ✖) when the work resolves.
    pub async fn working<T, E, F>(&self, label: &str, work: F) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        if !self.animate {
            println!("  {MARK} {label}…");
            return work.await;
        }

        print!("\x1b[?25l"); // cursor away while we own the line
        let started = std::time::Instant::now();
        let mut ticks = tokio::time::interval(Duration::from_millis(66));
        tokio::pin!(work);

        let result = loop {
            tokio::select! {
                result = &mut work => break result,
                _ = ticks.tick() => {
                    let elapsed = started.elapsed();
                    let phase = (elapsed.as_millis() % 1200) as f32 / 1200.0;
                    print!(
                        "\r  {} {}…\x1b[K",
                        self.accent(&spinner(elapsed).to_string()),
                        shimmer(label, phase, self.palette),
                    );
                    let _ = std::io::stdout().flush();
                }
            }
        };

        let glyph = if result.is_ok() {
            self.good("✔")
        } else {
            self.bad("✖")
        };
        println!("\r  {glyph} {label}\x1b[K\x1b[?25h");
        let _ = std::io::stdout().flush();
        result
    }

    /// The success beat at the end of login.
    pub fn accepted(&self, text: &str) {
        if self.animate {
            // A brief sparkle: the mark breathes twice.
            for frame in ["✧", "✦", "✧", "✦"] {
                print!("\r  {} {text}", self.accent(frame));
                let _ = std::io::stdout().flush();
                std::thread::sleep(Duration::from_millis(110));
            }
            println!();
        } else {
            println!("  {MARK} {text}");
        }
    }

    /// A prompt label: `✦ username ›`.
    #[must_use]
    pub fn prompt(&self, label: &str) -> String {
        format!(
            "  {} {} › ",
            self.accent(&MARK.to_string()),
            self.bold(label)
        )
    }

    /// An error line.
    pub fn refused(&self, text: &str) {
        println!("  {} {text}", self.bad("✖"));
        println!();
    }

    // -- tiny stylers ------------------------------------------------------

    fn accent(&self, s: &str) -> String {
        match self.palette {
            Palette::Plain => s.to_owned(),
            Palette::Indexed => format!("\x1b[38;5;141m{s}\x1b[0m"),
            Palette::True => format!("\x1b[38;2;178;132;255m{s}\x1b[0m"),
        }
    }

    fn dim(&self, s: &str) -> String {
        match self.palette {
            Palette::Plain => s.to_owned(),
            _ => format!("\x1b[2m{s}\x1b[0m"),
        }
    }

    fn bold(&self, s: &str) -> String {
        match self.palette {
            Palette::Plain => s.to_owned(),
            _ => format!("\x1b[1m{s}\x1b[0m"),
        }
    }

    fn good(&self, s: &str) -> String {
        match self.palette {
            Palette::Plain => s.to_owned(),
            Palette::Indexed => format!("\x1b[38;5;114m{s}\x1b[0m"),
            Palette::True => format!("\x1b[38;2;120;230;160m{s}\x1b[0m"),
        }
    }

    fn bad(&self, s: &str) -> String {
        match self.palette {
            Palette::Plain => s.to_owned(),
            Palette::Indexed => format!("\x1b[38;5;203m{s}\x1b[0m"),
            Palette::True => format!("\x1b[38;2;255;110;120m{s}\x1b[0m"),
        }
    }
}

impl Default for Intro {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn the_logo_is_rectangular_and_paints_every_row() {
        let widths: std::collections::HashSet<usize> =
            LOGO.iter().map(|l| l.chars().count()).collect();
        assert_eq!(widths.len(), 1, "all rows equal width");

        for (row, expected) in LOGO.iter().enumerate() {
            let painted = logo_row(row, Palette::True);
            assert_eq!(strip_ansi(&painted), *expected, "row {row} text intact");
        }
    }

    #[test]
    fn shimmer_never_alters_the_text_itself() {
        for phase in [0.0, 0.33, 0.5, 0.99] {
            for palette in [Palette::True, Palette::Indexed, Palette::Plain] {
                let out = shimmer("Waking the node", phase, palette);
                assert_eq!(strip_ansi(&out), "Waking the node");
            }
        }
    }

    #[test]
    fn shimmer_actually_moves() {
        let a = shimmer("TERESA", 0.0, Palette::True);
        let b = shimmer("TERESA", 0.5, Palette::True);
        assert_ne!(a, b, "the highlight must sweep");
    }

    #[test]
    fn plain_palette_emits_no_escapes() {
        let out = shimmer("hello", 0.4, Palette::Plain);
        assert!(!out.contains('\x1b'));
        assert_eq!(logo_row(0, Palette::Plain), LOGO[0]);
    }

    #[test]
    fn gradient_endpoints_hit_the_stops() {
        assert_eq!(blend(0.0), STOPS[0]);
        assert_eq!(blend(1.0), STOPS[2]);
        // And the middle passes through the orchid stop.
        assert_eq!(blend(0.5), STOPS[1]);
    }

    #[test]
    fn prompts_carry_the_mark() {
        let intro = Intro::with(Palette::Plain, false);
        assert_eq!(intro.prompt("username"), "  ✦ username › ");
    }

    #[tokio::test]
    async fn working_returns_the_futures_result_without_animation() {
        let intro = Intro::with(Palette::Plain, false);
        let ok: Result<i32, &str> = intro.working("thinking", async { Ok(41 + 1) }).await;
        assert_eq!(ok, Ok(42));
        let err: Result<i32, &str> = intro.working("failing", async { Err("nope") }).await;
        assert_eq!(err, Err("nope"));
    }
}
