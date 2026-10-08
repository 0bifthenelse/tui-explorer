//! Color tokens. Deep graphite surfaces with a single signal accent
//! (`#ff7d27`); everything interactive derives its states from the accent
//! through [`crate::ui::theme::mix`], so hover/selection transitions can
//! interpolate between any two of these tokens.

use ratatui::style::Color;

pub const ROOT_ORANGE: Color = Color::Rgb(0xff, 0x7d, 0x27);
pub const ROOT_INK: Color = Color::Rgb(0x0d, 0x0f, 0x13);
pub const ROOT_PAPER: Color = Color::Rgb(0xec, 0xef, 0xf5);

/// App background.
pub const SURFACE_0: Color = Color::Rgb(0x11, 0x13, 0x18);
/// Main content panes.
pub const SURFACE_1: Color = Color::Rgb(0x15, 0x17, 0x1d);
/// Chrome bars (header, status) and the sidebar.
pub const SURFACE_2: Color = Color::Rgb(0x1a, 0x1d, 0x24);
/// Raised surfaces: modals, popovers, menus.
pub const SURFACE_3: Color = Color::Rgb(0x22, 0x26, 0x2f);
/// Zebra stripe for alternating list rows.
pub const SURFACE_ZEBRA: Color = Color::Rgb(0x17, 0x19, 0x20);

pub const BORDER_SUBTLE: Color = Color::Rgb(0x2b, 0x2f, 0x39);
pub const BORDER_STRONG: Color = Color::Rgb(0x45, 0x4a, 0x57);

pub const TEXT_PRIMARY: Color = Color::Rgb(0xec, 0xef, 0xf5);
pub const TEXT_SECONDARY: Color = Color::Rgb(0xa6, 0xac, 0xba);
pub const TEXT_MUTED: Color = Color::Rgb(0x68, 0x6f, 0x7e);

pub const ACCENT: Color = Color::Rgb(0xff, 0x7d, 0x27);
pub const ACCENT_HOVER: Color = Color::Rgb(0xff, 0x9e, 0x5e);
pub const ACCENT_SOFT: Color = Color::Rgb(0xff, 0xc6, 0x9e);
/// Deep ember used as the far end of accent gradients.
pub const ACCENT_DEEP: Color = Color::Rgb(0x8a, 0x3a, 0x0c);
/// Amber partner hue for two-stop accent gradients.
pub const ACCENT_AMBER: Color = Color::Rgb(0xff, 0xb3, 0x2e);
/// Text drawn on top of a solid accent fill.
pub const INK_ON_ACCENT: Color = Color::Rgb(0x1a, 0x0d, 0x04);

pub const DANGER: Color = Color::Rgb(0xff, 0x4d, 0x5e);
pub const SUCCESS: Color = Color::Rgb(0x5f, 0xd6, 0x8f);
pub const INFO: Color = Color::Rgb(0x5e, 0xc8, 0xff);

/// Solid selection fill: the signal orange itself.
pub const SELECTED_BG: Color = Color::Rgb(0xff, 0x7d, 0x27);
/// Warm tint used by drag ghosts and picker cursors.
pub const FOCUS_BG: Color = Color::Rgb(0x4d, 0x2c, 0x18);

// File category hues (icons and badges; never used as the only signal).
pub const HUE_DIR: Color = Color::Rgb(0xff, 0x9a, 0x4d);
pub const HUE_CODE: Color = Color::Rgb(0x5e, 0xc8, 0xff);
pub const HUE_WEB: Color = Color::Rgb(0x82, 0xa8, 0xff);
pub const HUE_DATA: Color = Color::Rgb(0xc7, 0x92, 0xea);
pub const HUE_IMAGE: Color = Color::Rgb(0xff, 0x7a, 0xc0);
pub const HUE_AUDIO: Color = Color::Rgb(0xb4, 0x8c, 0xff);
pub const HUE_VIDEO: Color = Color::Rgb(0xff, 0x6b, 0x7a);
pub const HUE_ARCHIVE: Color = Color::Rgb(0xff, 0xd1, 0x66);
pub const HUE_DOC: Color = Color::Rgb(0xd8, 0xdc, 0xe6);
pub const HUE_EXEC: Color = Color::Rgb(0x7e, 0xe7, 0x87);
pub const HUE_LINK: Color = Color::Rgb(0x4f, 0xd6, 0xbe);
pub const HUE_SPECIAL: Color = Color::Rgb(0xf0, 0x71, 0x78);
