//! Frozen color constants bound to the default theme.
//!
//! New rendering code should read [`crate::ui::theme::current`] instead, so it
//! follows the user's theme choice rather than always using theme 0.

use ratatui::style::Color;

use crate::ui::theme::THEMES;

/// Default theme: orange on dark ink.
const DEFAULT: Color = THEMES[0].colors.accent;

pub const ROOT_ORANGE: Color = DEFAULT;
pub const ROOT_INK: Color = THEMES[0].colors.surface_0;
pub const ROOT_PAPER: Color = THEMES[0].colors.text_primary;

pub const SURFACE_0: Color = THEMES[0].colors.surface_0;
pub const SURFACE_1: Color = THEMES[0].colors.surface_1;
pub const SURFACE_2: Color = THEMES[0].colors.surface_2;
pub const SURFACE_3: Color = THEMES[0].colors.surface_3;

pub const BORDER_SUBTLE: Color = THEMES[0].colors.border_subtle;
pub const BORDER_STRONG: Color = THEMES[0].colors.border_strong;

pub const TEXT_PRIMARY: Color = THEMES[0].colors.text_primary;
pub const TEXT_SECONDARY: Color = THEMES[0].colors.text_secondary;
pub const TEXT_MUTED: Color = THEMES[0].colors.text_muted;

pub const ACCENT: Color = DEFAULT;
pub const ACCENT_HOVER: Color = THEMES[0].colors.accent_hover;
pub const ACCENT_SOFT: Color = THEMES[0].colors.accent_soft;
pub const DANGER: Color = THEMES[0].colors.danger;
pub const SELECTED_BG: Color = THEMES[0].colors.selected_bg;
pub const FOCUS_BG: Color = THEMES[0].colors.focus_bg;
