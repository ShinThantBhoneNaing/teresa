//! Colour schemes.
//!
//! A theme is pure data — no widget knows a theme's name, and adding a theme
//! is adding one constant to the roster. Colours are authored in 24-bit RGB
//! and downconverted to the xterm-256 cube at startup when the terminal does
//! not advertise truecolor, so every theme works everywhere and merely gets
//! subtler the better the terminal is.

use ratatui::style::Color;

/// A complete colour scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Roster name, lowercase.
    pub name: &'static str,
    /// Whole-screen background.
    pub bg: Color,
    /// Panel background, one step lifted from `bg`.
    pub surface: Color,
    /// Body text.
    pub fg: Color,
    /// De-emphasised text: timestamps, hints, offline members.
    pub muted: Color,
    /// Unfocused borders.
    pub border: Color,
    /// The focused / active border.
    pub border_focus: Color,
    /// The signature colour: logo, own nick, active room.
    pub primary: Color,
    /// The counterpart accent: links, room sigils, key hints.
    pub secondary: Color,
    /// A third voice for variety: emotes, invites.
    pub tertiary: Color,
    /// Good news.
    pub success: Color,
    /// Caution: unread badges, reconnect countdowns.
    pub warning: Color,
    /// Bad news: errors, mentions of failure.
    pub danger: Color,
    /// Nick colours for other users, assigned by stable hash.
    pub nicks: [Color; 6],
    /// Code block background.
    pub code_bg: Color,
    /// Selection / highlighted row background.
    pub highlight_bg: Color,
}

/// `Color::Rgb` shorthand for the theme tables.
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The Matrix: phosphor green on near-black. The default; the film canon.
const MATRIX: Theme = Theme {
    name: "matrix",
    bg: rgb(3, 9, 4),
    surface: rgb(7, 18, 9),
    fg: rgb(158, 222, 166),
    muted: rgb(64, 110, 72),
    border: rgb(24, 58, 30),
    border_focus: rgb(0, 255, 102),
    primary: rgb(0, 255, 102),
    secondary: rgb(0, 208, 255),
    tertiary: rgb(180, 255, 140),
    success: rgb(0, 230, 118),
    warning: rgb(255, 214, 0),
    danger: rgb(255, 82, 82),
    nicks: [
        rgb(102, 255, 153),
        rgb(0, 208, 255),
        rgb(199, 255, 112),
        rgb(64, 224, 208),
        rgb(148, 255, 220),
        rgb(0, 255, 200),
    ],
    code_bg: rgb(10, 26, 13),
    highlight_bg: rgb(14, 46, 22),
};

/// Cyberpunk: neon magenta and electric cyan over midnight violet.
const CYBERPUNK: Theme = Theme {
    name: "cyberpunk",
    bg: rgb(13, 6, 24),
    surface: rgb(24, 12, 42),
    fg: rgb(234, 214, 255),
    muted: rgb(122, 96, 160),
    border: rgb(58, 32, 92),
    border_focus: rgb(255, 0, 170),
    primary: rgb(255, 0, 170),
    secondary: rgb(0, 229, 255),
    tertiary: rgb(255, 230, 0),
    success: rgb(0, 255, 159),
    warning: rgb(255, 184, 0),
    danger: rgb(255, 55, 95),
    nicks: [
        rgb(0, 229, 255),
        rgb(255, 122, 214),
        rgb(190, 129, 255),
        rgb(255, 230, 0),
        rgb(0, 255, 159),
        rgb(255, 158, 100),
    ],
    code_bg: rgb(30, 16, 52),
    highlight_bg: rgb(52, 24, 84),
};

/// Who Am I (2014): CLAY/FR1ENDS terminal white-on-black with warning amber.
const WHOAMI: Theme = Theme {
    name: "whoami",
    bg: rgb(8, 8, 8),
    surface: rgb(18, 18, 18),
    fg: rgb(219, 219, 219),
    muted: rgb(112, 112, 112),
    border: rgb(48, 48, 48),
    border_focus: rgb(255, 255, 255),
    primary: rgb(255, 255, 255),
    secondary: rgb(255, 176, 0),
    tertiary: rgb(0, 194, 255),
    success: rgb(148, 226, 118),
    warning: rgb(255, 176, 0),
    danger: rgb(235, 77, 75),
    nicks: [
        rgb(255, 176, 0),
        rgb(0, 194, 255),
        rgb(148, 226, 118),
        rgb(255, 122, 122),
        rgb(196, 148, 255),
        rgb(120, 220, 220),
    ],
    code_bg: rgb(24, 24, 24),
    highlight_bg: rgb(40, 40, 40),
};

/// Dracula, by its published palette.
const DRACULA: Theme = Theme {
    name: "dracula",
    bg: rgb(40, 42, 54),
    surface: rgb(48, 51, 66),
    fg: rgb(248, 248, 242),
    muted: rgb(98, 114, 164),
    border: rgb(68, 71, 90),
    border_focus: rgb(189, 147, 249),
    primary: rgb(189, 147, 249),
    secondary: rgb(139, 233, 253),
    tertiary: rgb(255, 121, 198),
    success: rgb(80, 250, 123),
    warning: rgb(241, 250, 140),
    danger: rgb(255, 85, 85),
    nicks: [
        rgb(139, 233, 253),
        rgb(255, 121, 198),
        rgb(80, 250, 123),
        rgb(255, 184, 108),
        rgb(241, 250, 140),
        rgb(189, 147, 249),
    ],
    code_bg: rgb(33, 34, 44),
    highlight_bg: rgb(68, 71, 90),
};

/// Nord, by its published palette.
const NORD: Theme = Theme {
    name: "nord",
    bg: rgb(46, 52, 64),
    surface: rgb(59, 66, 82),
    fg: rgb(236, 239, 244),
    muted: rgb(97, 110, 136),
    border: rgb(67, 76, 94),
    border_focus: rgb(136, 192, 208),
    primary: rgb(136, 192, 208),
    secondary: rgb(129, 161, 193),
    tertiary: rgb(180, 142, 173),
    success: rgb(163, 190, 140),
    warning: rgb(235, 203, 139),
    danger: rgb(191, 97, 106),
    nicks: [
        rgb(136, 192, 208),
        rgb(180, 142, 173),
        rgb(163, 190, 140),
        rgb(208, 135, 112),
        rgb(235, 203, 139),
        rgb(94, 129, 172),
    ],
    code_bg: rgb(39, 44, 54),
    highlight_bg: rgb(67, 76, 94),
};

/// Gruvbox dark, by its published palette.
const GRUVBOX: Theme = Theme {
    name: "gruvbox",
    bg: rgb(40, 40, 40),
    surface: rgb(50, 48, 47),
    fg: rgb(235, 219, 178),
    muted: rgb(146, 131, 116),
    border: rgb(80, 73, 69),
    border_focus: rgb(250, 189, 47),
    primary: rgb(250, 189, 47),
    secondary: rgb(131, 165, 152),
    tertiary: rgb(211, 134, 155),
    success: rgb(184, 187, 38),
    warning: rgb(254, 128, 25),
    danger: rgb(251, 73, 52),
    nicks: [
        rgb(131, 165, 152),
        rgb(211, 134, 155),
        rgb(184, 187, 38),
        rgb(254, 128, 25),
        rgb(250, 189, 47),
        rgb(142, 192, 124),
    ],
    code_bg: rgb(29, 32, 33),
    highlight_bg: rgb(60, 56, 54),
};

/// Catppuccin Mocha, by its published palette.
const CATPPUCCIN: Theme = Theme {
    name: "catppuccin",
    bg: rgb(30, 30, 46),
    surface: rgb(24, 24, 37),
    fg: rgb(205, 214, 244),
    muted: rgb(108, 112, 134),
    border: rgb(49, 50, 68),
    border_focus: rgb(203, 166, 247),
    primary: rgb(203, 166, 247),
    secondary: rgb(137, 220, 235),
    tertiary: rgb(245, 194, 231),
    success: rgb(166, 227, 161),
    warning: rgb(249, 226, 175),
    danger: rgb(243, 139, 168),
    nicks: [
        rgb(137, 220, 235),
        rgb(245, 194, 231),
        rgb(166, 227, 161),
        rgb(250, 179, 135),
        rgb(249, 226, 175),
        rgb(180, 190, 254),
    ],
    code_bg: rgb(17, 17, 27),
    highlight_bg: rgb(49, 50, 68),
};

/// Every theme, in `/theme` listing order.
const ROSTER: [&Theme; 7] = [
    &MATRIX,
    &CYBERPUNK,
    &WHOAMI,
    &DRACULA,
    &NORD,
    &GRUVBOX,
    &CATPPUCCIN,
];

impl Theme {
    /// A theme by name, falling back to Matrix for anything unknown — a bad
    /// config value should degrade, not crash the chat.
    #[must_use]
    pub fn by_name(name: &str) -> Self {
        Self::by_name_checked(name).unwrap_or_else(|| MATRIX.clone())
    }

    /// A theme by name, or `None` — `/theme` uses this to refuse typos.
    #[must_use]
    pub fn by_name_checked(name: &str) -> Option<Self> {
        let wanted = name.trim().to_lowercase();
        ROSTER
            .iter()
            .find(|t| t.name == wanted)
            .map(|t| (*t).clone())
    }

    /// The names for `/theme` and docs.
    #[must_use]
    pub fn roster() -> Vec<&'static str> {
        ROSTER.iter().map(|t| t.name).collect()
    }

    /// A stable nick colour for a user.
    #[must_use]
    pub fn nick_color(&self, key: &str) -> Color {
        // FNV-1a: tiny, stable across runs, good enough spread for 6 buckets.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in key.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.nicks[(hash % self.nicks.len() as u64) as usize]
    }

    /// Downconverts every colour to the xterm-256 palette, for terminals
    /// without truecolor.
    #[must_use]
    pub fn to_indexed(&self) -> Self {
        let mut theme = self.clone();
        for slot in [
            &mut theme.bg,
            &mut theme.surface,
            &mut theme.fg,
            &mut theme.muted,
            &mut theme.border,
            &mut theme.border_focus,
            &mut theme.primary,
            &mut theme.secondary,
            &mut theme.tertiary,
            &mut theme.success,
            &mut theme.warning,
            &mut theme.danger,
            &mut theme.code_bg,
            &mut theme.highlight_bg,
        ] {
            *slot = indexed(*slot);
        }
        for nick in &mut theme.nicks {
            *nick = indexed(*nick);
        }
        theme
    }
}

/// Nearest xterm-256 colour for an RGB value: greys go to the grey ramp,
/// everything else to the 6×6×6 cube.
fn indexed(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };

    // Near-grey? The 24-step grey ramp (232–255) is much finer than the cube.
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max - min < 12 {
        let level = u16::from(r) + u16::from(g) + u16::from(b);
        let average = (level / 3) as u8;
        let step = ((f32::from(average) - 8.0) / 10.0).round().clamp(0.0, 23.0) as u8;
        return Color::Indexed(232 + step);
    }

    let scale = |v: u8| -> u8 {
        // Cube levels: 0, 95, 135, 175, 215, 255.
        match v {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    };
    Color::Indexed(16 + 36 * scale(r) + 6 * scale(g) + scale(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roster_has_all_seven_advertised_themes() {
        let names = Theme::roster();
        for wanted in [
            "matrix",
            "cyberpunk",
            "whoami",
            "dracula",
            "nord",
            "gruvbox",
            "catppuccin",
        ] {
            assert!(names.contains(&wanted), "missing theme {wanted}");
        }
        assert_eq!(names.len(), 7);
    }

    #[test]
    fn lookup_is_case_insensitive_and_falls_back_to_matrix() {
        assert_eq!(Theme::by_name("DRACULA").name, "dracula");
        assert_eq!(Theme::by_name(" Nord ").name, "nord");
        assert_eq!(Theme::by_name("no-such-theme").name, "matrix");
        assert!(Theme::by_name_checked("no-such-theme").is_none());
    }

    #[test]
    fn nick_colours_are_stable_and_within_palette() {
        let theme = Theme::by_name("matrix");
        let a1 = theme.nick_color("alice");
        let a2 = theme.nick_color("alice");
        assert_eq!(a1, a2, "same user, same colour, every run");
        assert!(theme.nicks.contains(&a1));
    }

    #[test]
    fn indexed_conversion_leaves_no_rgb_behind() {
        for name in Theme::roster() {
            let theme = Theme::by_name(name).to_indexed();
            let all = [
                theme.bg,
                theme.surface,
                theme.fg,
                theme.muted,
                theme.border,
                theme.border_focus,
                theme.primary,
                theme.secondary,
                theme.tertiary,
                theme.success,
                theme.warning,
                theme.danger,
                theme.code_bg,
                theme.highlight_bg,
            ];
            for color in all.into_iter().chain(theme.nicks) {
                assert!(
                    matches!(color, Color::Indexed(_)),
                    "{name}: {color:?} escaped conversion"
                );
            }
        }
    }

    #[test]
    fn grey_conversion_uses_the_grey_ramp() {
        let grey = indexed(rgb(128, 128, 128));
        assert!(matches!(grey, Color::Indexed(i) if (232..=255).contains(&i)));

        let saturated = indexed(rgb(255, 0, 170));
        assert!(matches!(saturated, Color::Indexed(i) if (16..232).contains(&i)));
    }
}
