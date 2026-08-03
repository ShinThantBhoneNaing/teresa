//! Animation timing: pure functions from elapsed time to frames.
//!
//! Nothing here holds state — the renderer passes `app.started.elapsed()` (or
//! any instant) and gets glyphs back. That keeps every animation testable and
//! guarantees a dropped tick can never wedge an animation mid-frame.

use std::time::Duration;

/// How long the boot splash owns the whole screen.
pub const BOOT_DURATION: Duration = Duration::from_millis(1400);

/// The spinner used for connecting/reconnecting states.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Braille spinner frame for a moment in time.
#[must_use]
pub fn spinner(elapsed: Duration) -> char {
    let index = (elapsed.as_millis() / 80) as usize % SPINNER.len();
    SPINNER[index]
}

/// Typing-indicator dots: `·`, `··`, `···` cycling.
#[must_use]
pub fn typing_dots(elapsed: Duration) -> &'static str {
    match (elapsed.as_millis() / 350) % 3 {
        0 => "·  ",
        1 => "·· ",
        _ => "···",
    }
}

/// A one-line progress bar of `width` cells, filled to `ratio` (0..=1).
#[must_use]
pub fn progress_bar(width: u16, ratio: f32) -> String {
    // Eighth-block glyphs give 8 sub-steps per cell — the smoothness is the point.
    const BLOCKS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

    let width = usize::from(width);
    let clamped = ratio.clamp(0.0, 1.0);
    let cells_8 = (clamped * (width as f32) * 8.0).round() as usize;
    let (full, rem) = (cells_8 / 8, cells_8 % 8);

    let mut bar = String::with_capacity(width * 3);
    for _ in 0..full.min(width) {
        bar.push('█');
    }
    if full < width && rem > 0 {
        bar.push(BLOCKS[rem - 1]);
    }
    while bar.chars().count() < width {
        bar.push(' ');
    }
    bar
}

/// The boot splash's staged reveal: which of `total` lines are visible at
/// `elapsed`, ramped so the last line lands well before [`BOOT_DURATION`].
#[must_use]
pub fn boot_lines_visible(elapsed: Duration, total: usize) -> usize {
    let reveal_window = BOOT_DURATION.mul_f32(0.6);
    if elapsed >= reveal_window {
        return total;
    }
    let ratio = elapsed.as_secs_f32() / reveal_window.as_secs_f32();
    ((ratio * total as f32).floor() as usize).min(total)
}

/// Boot progress 0..=1 for the splash's progress bar.
#[must_use]
pub fn boot_progress(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() / BOOT_DURATION.as_secs_f32()).clamp(0.0, 1.0)
}

/// A slow pulse (0..=1..=0) for cursor-ish glows, 1.2s period.
#[must_use]
pub fn pulse(elapsed: Duration) -> f32 {
    let phase = (elapsed.as_millis() % 1200) as f32 / 1200.0;
    if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_cycles_through_every_frame() {
        let mut seen = std::collections::HashSet::new();
        for ms in (0..800).step_by(80) {
            seen.insert(spinner(Duration::from_millis(ms)));
        }
        assert_eq!(seen.len(), SPINNER.len());
    }

    #[test]
    fn progress_bar_is_exactly_the_requested_width() {
        for ratio in [0.0, 0.33, 0.5, 0.99, 1.0, 7.0, -3.0] {
            let bar = progress_bar(20, ratio);
            assert_eq!(bar.chars().count(), 20, "ratio {ratio}: {bar:?}");
        }
        assert_eq!(progress_bar(10, 1.0), "██████████");
        assert_eq!(progress_bar(10, 0.0), "          ");
    }

    #[test]
    fn progress_bar_uses_partial_blocks_between_cells() {
        // Half of one cell: a single half-block glyph.
        let bar = progress_bar(1, 0.5);
        assert_eq!(bar, "▌");
    }

    #[test]
    fn boot_reveal_reaches_the_last_line_early() {
        assert_eq!(boot_lines_visible(Duration::ZERO, 8), 0);
        assert_eq!(boot_lines_visible(BOOT_DURATION.mul_f32(0.61), 8), 8);
        assert_eq!(boot_lines_visible(BOOT_DURATION, 8), 8);
    }

    #[test]
    fn pulse_peaks_mid_period_and_stays_in_range() {
        assert!(pulse(Duration::from_millis(600)) > 0.95);
        for ms in (0..2400).step_by(100) {
            let value = pulse(Duration::from_millis(ms));
            assert!((0.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn typing_dots_cycle() {
        assert_eq!(typing_dots(Duration::from_millis(0)), "·  ");
        assert_eq!(typing_dots(Duration::from_millis(400)), "·· ");
        assert_eq!(typing_dots(Duration::from_millis(800)), "···");
        assert_eq!(typing_dots(Duration::from_millis(1100)), "·  ");
    }
}
