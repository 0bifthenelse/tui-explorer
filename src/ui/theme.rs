//! Runtime-selectable color themes, all expressed as xterm-256 palette indices.
//!
//! Every field is a `Color::Indexed(u8)` so the UI renders identically on any
//! terminal regardless of truecolor support.

use std::cell::Cell;

use ratatui::style::Color;

/// One theme's resolved color set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    /// Window background, lowest elevation.
    pub surface_0: Color,
    /// Panels and rails resting on `surface_0`.
    pub surface_1: Color,
    /// Raised controls, list rows.
    pub surface_2: Color,
    /// Highest elevation, hover and popups.
    pub surface_3: Color,
    /// Resting border line.
    pub border_subtle: Color,
    /// Focused or active border line.
    pub border_strong: Color,
    /// Body text.
    pub text_primary: Color,
    /// Supporting text.
    pub text_secondary: Color,
    /// De-emphasized text and disabled controls.
    pub text_muted: Color,
    /// Brand accent, primary highlight.
    pub accent: Color,
    /// Accent under the pointer.
    pub accent_hover: Color,
    /// Low emphasis accent wash.
    pub accent_soft: Color,
    /// Error and destructive state.
    pub danger: Color,
    /// Fill of the selected row.
    pub selected_bg: Color,
    /// Fill of the focused control, distinct from `selected_bg`.
    pub focus_bg: Color,
    /// Foreground drawn on top of `accent` and `danger` fills.
    pub ink: Color,
}

/// A named color scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Stable machine identifier, kebab-case.
    pub id: &'static str,
    /// Human readable title-case label.
    pub name: &'static str,
    /// True when the surfaces are dark.
    pub dark: bool,
    /// Resolved colors.
    pub colors: ThemeColors,
}

/// All selectable themes. Index 0 is the default identity theme.
pub const THEMES: [Theme; 16] = [
    Theme {
        id: "ink-orange",
        name: "Ink Orange",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(234),
            surface_1: Color::Indexed(235),
            surface_2: Color::Indexed(236),
            surface_3: Color::Indexed(237),
            border_subtle: Color::Indexed(238),
            border_strong: Color::Indexed(240),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(249),
            text_muted: Color::Indexed(244),
            accent: Color::Indexed(208),
            accent_hover: Color::Indexed(214),
            accent_soft: Color::Indexed(215),
            danger: Color::Indexed(196),
            selected_bg: Color::Indexed(94),
            focus_bg: Color::Indexed(130),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "ink-cyan",
        name: "Ink Cyan",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(234),
            surface_1: Color::Indexed(235),
            surface_2: Color::Indexed(236),
            surface_3: Color::Indexed(237),
            border_subtle: Color::Indexed(238),
            border_strong: Color::Indexed(240),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(249),
            text_muted: Color::Indexed(244),
            accent: Color::Indexed(37),
            accent_hover: Color::Indexed(51),
            accent_soft: Color::Indexed(87),
            danger: Color::Indexed(196),
            selected_bg: Color::Indexed(23),
            focus_bg: Color::Indexed(24),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "ink-green",
        name: "Ink Green",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(234),
            surface_1: Color::Indexed(235),
            surface_2: Color::Indexed(236),
            surface_3: Color::Indexed(237),
            border_subtle: Color::Indexed(238),
            border_strong: Color::Indexed(240),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(249),
            text_muted: Color::Indexed(244),
            accent: Color::Indexed(34),
            accent_hover: Color::Indexed(77),
            accent_soft: Color::Indexed(118),
            danger: Color::Indexed(196),
            selected_bg: Color::Indexed(22),
            focus_bg: Color::Indexed(23),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "ink-magenta",
        name: "Ink Magenta",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(234),
            surface_1: Color::Indexed(235),
            surface_2: Color::Indexed(236),
            surface_3: Color::Indexed(237),
            border_subtle: Color::Indexed(238),
            border_strong: Color::Indexed(240),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(249),
            text_muted: Color::Indexed(244),
            accent: Color::Indexed(164),
            accent_hover: Color::Indexed(170),
            accent_soft: Color::Indexed(213),
            danger: Color::Indexed(196),
            selected_bg: Color::Indexed(53),
            focus_bg: Color::Indexed(90),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "ink-blue",
        name: "Ink Blue",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(234),
            surface_1: Color::Indexed(235),
            surface_2: Color::Indexed(236),
            surface_3: Color::Indexed(237),
            border_subtle: Color::Indexed(238),
            border_strong: Color::Indexed(240),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(249),
            text_muted: Color::Indexed(244),
            accent: Color::Indexed(33),
            accent_hover: Color::Indexed(69),
            accent_soft: Color::Indexed(111),
            danger: Color::Indexed(196),
            selected_bg: Color::Indexed(25),
            focus_bg: Color::Indexed(24),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "ink-violet",
        name: "Ink Violet",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(234),
            surface_1: Color::Indexed(235),
            surface_2: Color::Indexed(236),
            surface_3: Color::Indexed(237),
            border_subtle: Color::Indexed(238),
            border_strong: Color::Indexed(240),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(249),
            text_muted: Color::Indexed(244),
            accent: Color::Indexed(99),
            accent_hover: Color::Indexed(141),
            accent_soft: Color::Indexed(183),
            danger: Color::Indexed(196),
            selected_bg: Color::Indexed(56),
            focus_bg: Color::Indexed(57),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "void-amber",
        name: "Void Amber",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(16),
            surface_1: Color::Indexed(17),
            surface_2: Color::Indexed(18),
            surface_3: Color::Indexed(19),
            border_subtle: Color::Indexed(240),
            border_strong: Color::Indexed(244),
            text_primary: Color::Indexed(231),
            text_secondary: Color::Indexed(250),
            text_muted: Color::Indexed(246),
            accent: Color::Indexed(178),
            accent_hover: Color::Indexed(214),
            accent_soft: Color::Indexed(223),
            danger: Color::Indexed(203),
            selected_bg: Color::Indexed(58),
            focus_bg: Color::Indexed(130),
            ink: Color::Indexed(232),
        },
    },
    Theme {
        id: "graphite-cyan",
        name: "Graphite Cyan",
        dark: true,
        colors: ThemeColors {
            surface_0: Color::Indexed(235),
            surface_1: Color::Indexed(236),
            surface_2: Color::Indexed(237),
            surface_3: Color::Indexed(238),
            border_subtle: Color::Indexed(242),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(255),
            text_secondary: Color::Indexed(251),
            text_muted: Color::Indexed(248),
            accent: Color::Indexed(44),
            accent_hover: Color::Indexed(51),
            accent_soft: Color::Indexed(87),
            danger: Color::Indexed(203),
            selected_bg: Color::Indexed(23),
            focus_bg: Color::Indexed(24),
            ink: Color::Indexed(16),
        },
    },
    Theme {
        id: "paper-orange",
        name: "Paper Orange",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(250),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(94),
            accent_hover: Color::Indexed(130),
            accent_soft: Color::Indexed(173),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(223),
            focus_bg: Color::Indexed(230),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "paper-blue",
        name: "Paper Blue",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(250),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(25),
            accent_hover: Color::Indexed(27),
            accent_soft: Color::Indexed(111),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(189),
            focus_bg: Color::Indexed(153),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "paper-green",
        name: "Paper Green",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(250),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(28),
            accent_hover: Color::Indexed(40),
            accent_soft: Color::Indexed(114),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(194),
            focus_bg: Color::Indexed(158),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "paper-crimson",
        name: "Paper Crimson",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(250),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(88),
            accent_hover: Color::Indexed(160),
            accent_soft: Color::Indexed(176),
            danger: Color::Indexed(52),
            selected_bg: Color::Indexed(224),
            focus_bg: Color::Indexed(210),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "paper-violet",
        name: "Paper Violet",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(250),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(55),
            accent_hover: Color::Indexed(63),
            accent_soft: Color::Indexed(141),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(183),
            focus_bg: Color::Indexed(147),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "paper-teal",
        name: "Paper Teal",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(250),
            border_strong: Color::Indexed(246),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(23),
            accent_hover: Color::Indexed(30),
            accent_soft: Color::Indexed(66),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(159),
            focus_bg: Color::Indexed(123),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "slate-blue",
        name: "Slate Blue",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(253),
            surface_3: Color::Indexed(252),
            border_subtle: Color::Indexed(249),
            border_strong: Color::Indexed(245),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(242),
            accent: Color::Indexed(25),
            accent_hover: Color::Indexed(27),
            accent_soft: Color::Indexed(110),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(189),
            focus_bg: Color::Indexed(153),
            ink: Color::Indexed(231),
        },
    },
    Theme {
        id: "ivory-amber",
        name: "Ivory Amber",
        dark: false,
        colors: ThemeColors {
            surface_0: Color::Indexed(255),
            surface_1: Color::Indexed(254),
            surface_2: Color::Indexed(252),
            surface_3: Color::Indexed(250),
            border_subtle: Color::Indexed(248),
            border_strong: Color::Indexed(244),
            text_primary: Color::Indexed(235),
            text_secondary: Color::Indexed(240),
            text_muted: Color::Indexed(243),
            accent: Color::Indexed(130),
            accent_hover: Color::Indexed(136),
            accent_soft: Color::Indexed(180),
            danger: Color::Indexed(160),
            selected_bg: Color::Indexed(223),
            focus_bg: Color::Indexed(230),
            ink: Color::Indexed(231),
        },
    },
];

thread_local! {
    static CURRENT: Cell<usize> = const { Cell::new(0) };
}

/// Number of selectable themes.
pub fn count() -> usize {
    THEMES.len()
}

/// Display names in menu order.
pub fn names() -> [&'static str; 16] {
    let mut out = [""; 16];
    for (slot, theme) in out.iter_mut().zip(THEMES.iter()) {
        *slot = theme.name;
    }
    out
}

/// Index of the active theme.
pub fn current_index() -> usize {
    CURRENT.with(|cell| cell.get())
}

/// Activates a theme by index. Out-of-range indices leave the current theme unchanged.
pub fn set_current(index: usize) {
    if index < THEMES.len() {
        CURRENT.with(|cell| cell.set(index));
    }
}

/// Colors of the active theme.
pub fn current() -> &'static ThemeColors {
    &THEMES[current_index()].colors
}

/// Restores the default theme. Test-only.
#[cfg(test)]
pub fn reset() {
    CURRENT.with(|cell| cell.set(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::palette;

    /// Canonical xterm values of the 16 system colors.
    const SYSTEM: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];

    /// Cube channel levels.
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    /// All 16 colors of a theme, in declaration order.
    fn palette_of(colors: &ThemeColors) -> [Color; 16] {
        [
            colors.surface_0,
            colors.surface_1,
            colors.surface_2,
            colors.surface_3,
            colors.border_subtle,
            colors.border_strong,
            colors.text_primary,
            colors.text_secondary,
            colors.text_muted,
            colors.accent,
            colors.accent_hover,
            colors.accent_soft,
            colors.danger,
            colors.selected_bg,
            colors.focus_bg,
            colors.ink,
        ]
    }

    /// Resolves an xterm-256 index to its RGB triple.
    fn index_rgb(index: u8) -> (u8, u8, u8) {
        match index {
            0..=15 => SYSTEM[index as usize],
            16..=231 => {
                let i = usize::from(index) - 16;
                (LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6])
            }
            232..=255 => {
                let v = 8 + (usize::from(index) - 232) * 10;
                let v = v as u8;
                (v, v, v)
            }
        }
    }

    /// WCAG 2.1 relative luminance of a linearized 8-bit channel.
    fn channel_luminance(value: u8) -> f64 {
        let v = f64::from(value) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    /// WCAG 2.1 relative luminance of an xterm-256 index.
    fn index_luminance(index: u8) -> f64 {
        let (r, g, b) = index_rgb(index);
        0.2126 * channel_luminance(r)
            + 0.7152 * channel_luminance(g)
            + 0.0722 * channel_luminance(b)
    }

    /// WCAG 2.1 contrast ratio between two xterm-256 indices.
    fn contrast(a: u8, b: u8) -> f64 {
        let (la, lb) = (index_luminance(a), index_luminance(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Contrast ratio of two colors, panicking on a non-indexed color.
    fn ratio(a: Color, b: Color) -> f64 {
        match (a, b) {
            (Color::Indexed(x), Color::Indexed(y)) => contrast(x, y),
            _ => panic!("contrast expects indexed colors, got {a:?} and {b:?}"),
        }
    }

    /// Unwraps a themed color to its palette index.
    fn index_of(color: Color) -> u8 {
        match color {
            Color::Indexed(i) => i,
            other => panic!("expected indexed color, got {other:?}"),
        }
    }

    #[test]
    fn sixteen_themes_present() {
        assert_eq!(THEMES.len(), 16);
        assert_eq!(count(), 16);
        for id in THEMES.iter().map(|t| t.id) {
            assert!(!id.is_empty(), "theme id must not be empty");
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "theme id must be kebab-case: {id}"
            );
        }
        for name in THEMES.iter().map(|t| t.name) {
            assert!(!name.is_empty(), "theme name must not be empty");
        }

        let mut ids: Vec<&str> = THEMES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 16, "theme ids must be unique");

        let mut names: Vec<&str> = THEMES.iter().map(|t| t.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 16, "theme names must be unique");
    }

    #[test]
    fn indexed_only() {
        for theme in THEMES.iter() {
            for color in palette_of(&theme.colors) {
                assert!(
                    matches!(color, Color::Indexed(_)),
                    "theme {} uses non-indexed color {color:?}",
                    theme.id
                );
            }
        }
    }

    #[test]
    fn contrast_floor() {
        for theme in THEMES.iter() {
            let c = &theme.colors;
            let id = theme.id;

            assert!(
                ratio(c.text_primary, c.surface_0) >= 4.5,
                "{id}: text_primary on surface_0 is {}",
                ratio(c.text_primary, c.surface_0)
            );
            assert!(
                ratio(c.text_primary, c.surface_3) >= 4.0,
                "{id}: text_primary on surface_3 is {}",
                ratio(c.text_primary, c.surface_3)
            );
            assert!(
                ratio(c.text_secondary, c.surface_0) >= 3.0,
                "{id}: text_secondary on surface_0 is {}",
                ratio(c.text_secondary, c.surface_0)
            );
            assert!(
                ratio(c.text_muted, c.surface_0) >= 2.5,
                "{id}: text_muted on surface_0 is {}",
                ratio(c.text_muted, c.surface_0)
            );
            assert!(
                ratio(c.accent, c.surface_0) >= 3.0,
                "{id}: accent on surface_0 is {}",
                ratio(c.accent, c.surface_0)
            );
            assert!(
                ratio(c.danger, c.surface_0) >= 3.0,
                "{id}: danger on surface_0 is {}",
                ratio(c.danger, c.surface_0)
            );
            assert!(
                ratio(c.ink, c.accent) >= 4.5,
                "{id}: ink on accent is {}",
                ratio(c.ink, c.accent)
            );
            assert!(
                ratio(c.ink, c.danger) >= 3.0,
                "{id}: ink on danger is {}",
                ratio(c.ink, c.danger)
            );
            assert!(
                ratio(c.text_primary, c.selected_bg) >= 4.5,
                "{id}: text_primary on selected_bg is {}",
                ratio(c.text_primary, c.selected_bg)
            );
            assert!(
                ratio(c.text_primary, c.focus_bg) >= 3.0,
                "{id}: text_primary on focus_bg is {}",
                ratio(c.text_primary, c.focus_bg)
            );
        }
    }

    #[test]
    fn state_fills_are_unambiguous() {
        for theme in THEMES.iter() {
            let c = &theme.colors;
            let id = theme.id;
            let surfaces = [c.surface_0, c.surface_1, c.surface_2, c.surface_3];

            for color in [c.text_primary, c.text_secondary, c.text_muted] {
                assert!(
                    !surfaces.contains(&color),
                    "{id}: text color {color:?} collides with a surface"
                );
            }
            for color in [c.accent, c.danger, c.selected_bg, c.focus_bg] {
                assert!(
                    !surfaces.contains(&color),
                    "{id}: fill {color:?} collides with a surface"
                );
            }
            for color in [c.border_subtle, c.border_strong] {
                assert!(
                    !surfaces.contains(&color),
                    "{id}: border {color:?} collides with a surface"
                );
            }
            assert_ne!(
                c.selected_bg, c.focus_bg,
                "{id}: selected_bg and focus_bg must differ"
            );

            // Every slot must resolve to a distinct index so no two roles share a color.
            for (i, a) in palette_of(c).iter().enumerate() {
                for b in &palette_of(c)[i + 1..] {
                    assert_ne!(index_of(*a), index_of(*b), "{id}: duplicate palette slot");
                }
            }
        }
    }

    #[test]
    fn default_is_ink_orange() {
        reset();
        assert_eq!(current_index(), 0);
        let default = &THEMES[0];
        assert_eq!(default.id, "ink-orange");
        assert_eq!(default.name, "Ink Orange");
        assert!(default.dark);
        assert_eq!(palette::ACCENT, default.colors.accent);
        assert_eq!(palette::ROOT_ORANGE, default.colors.accent);
        assert_eq!(palette::ROOT_INK, default.colors.surface_0);
        assert_eq!(palette::ROOT_PAPER, default.colors.text_primary);
        assert_eq!(palette::SURFACE_0, default.colors.surface_0);
        assert_eq!(palette::TEXT_PRIMARY, default.colors.text_primary);
        assert_eq!(palette::SELECTED_BG, default.colors.selected_bg);
        assert_eq!(palette::FOCUS_BG, default.colors.focus_bg);
        assert_eq!(*current(), default.colors);
    }

    #[test]
    fn current_index_tracks_selection() {
        reset();
        assert_eq!(current(), &THEMES[0].colors);

        set_current(7);
        assert_eq!(current_index(), 7);
        assert_eq!(current(), &THEMES[7].colors);
        assert_eq!(THEMES[7].id, "graphite-cyan");

        // Out-of-range indices are ignored rather than clamped to a wrong theme.
        set_current(16);
        assert_eq!(current_index(), 7);
        set_current(usize::MAX);
        assert_eq!(current_index(), 7);
        set_current(count());
        assert_eq!(current_index(), 7);

        reset();
        assert_eq!(current_index(), 0);
    }

    #[test]
    fn names_match_theme_order() {
        let listed = names();
        assert_eq!(listed.len(), 16);
        for (slot, theme) in listed.iter().zip(THEMES.iter()) {
            assert_eq!(*slot, theme.name);
        }
    }
}
