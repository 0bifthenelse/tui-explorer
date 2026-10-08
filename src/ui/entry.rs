//! Shared entry presentation: animated state colors (hover tint, solid
//! selection, focus glow, cascade fade) and the file-type badges used by
//! every layout.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::app::state::{AppState, Mode};
use crate::browser::EntryView;
use crate::filesystem::{DirEntry, EntryKind};
use crate::icons::{IconKind, IconRegistry, IconResolver, IconSize, IconVariant};
use crate::ui::anim::{AnimKey, HOVER_IN, HOVER_OUT, SELECT, path_key};
use crate::ui::glyphs::{IconStyle, g, icon_style};
use crate::ui::palette::*;
use crate::ui::theme::{hue_for, mix};

/// Resolved colors for one entry this frame.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// Background after hover and selection blending.
    pub bg: Color,
    /// Focus glow strength (0..1): accent rail plus left-to-right gradient.
    pub focus: f32,
    /// Selection strength (0..1).
    pub selected: f32,
    /// Cascade appear progress (0..1).
    pub appear: f32,
}

impl Look {
    /// Foreground adjusted for selection (ink on orange) and cascade fade.
    pub fn fg(&self, color: Color) -> Color {
        let on_fill = mix(color, INK_ON_ACCENT, self.selected);
        mix(self.bg, on_fill, self.appear)
    }

    /// Muted metadata foreground.
    pub fn meta(&self) -> Color {
        let base = mix(
            TEXT_MUTED,
            mix(INK_ON_ACCENT, ACCENT_DEEP, 0.35),
            self.selected,
        );
        mix(self.bg, base, self.appear)
    }
}

/// Whether the navigation cursor should render at all (it hides behind
/// overlays so modal focus is unambiguous).
pub fn cursor_visible(state: &AppState) -> bool {
    matches!(
        state.mode,
        Mode::Browser | Mode::Command | Mode::Rename(_) | Mode::Search(_)
    )
}

/// Computes the animated look for an entry.
pub fn look(
    state: &mut AppState,
    entry: &DirEntry,
    base: Color,
    hovered: bool,
    selected: bool,
    focus: f32,
    appear: f32,
) -> Look {
    let key = path_key(&entry.path);
    let hover_t = state.anim.track_asym(
        AnimKey::RowHover(key),
        if hovered { 1.0 } else { 0.0 },
        HOVER_IN,
        HOVER_OUT,
    );
    let sel_t = state.anim.track(
        AnimKey::RowSelect(key),
        if selected { 1.0 } else { 0.0 },
        SELECT,
    );
    let hover_tint = mix(base, ACCENT, 0.20);
    let mut bg = mix(base, hover_tint, hover_t);
    bg = mix(bg, SELECTED_BG, sel_t);
    if sel_t > 0.0 && focus > 0.0 {
        // Focused selection glows slightly brighter than plain selection.
        bg = mix(bg, ACCENT_HOVER, 0.35 * focus * sel_t);
    }
    Look {
        bg,
        focus,
        selected: sel_t,
        appear,
    }
}

/// Fills `rect` with the look's background plus the focus glow: a
/// horizontal gradient from a warm accent tint into the base color.
pub fn paint_bg(buf: &mut Buffer, rect: Rect, look: &Look) {
    let rect = rect.intersection(buf.area);
    let glow = look.focus * (1.0 - look.selected);
    let width = rect.width.max(1) as f32;
    for x in rect.left()..rect.right() {
        let t = (x - rect.left()) as f32 / width;
        let strength = glow * 0.42 * (1.0 - t).powf(1.6);
        let color = mix(look.bg, ACCENT, strength);
        for y in rect.top()..rect.bottom() {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.bg = color;
            cell.modifier = Modifier::empty();
        }
    }
}

/// Draws the accent rail at the left edge of `rect` when focused.
pub fn paint_rail(buf: &mut Buffer, rect: Rect, look: &Look) {
    if look.focus < 0.35 || rect.width == 0 {
        return;
    }
    let color = if look.selected > 0.5 {
        mix(TEXT_PRIMARY, ROOT_PAPER, 0.5)
    } else {
        mix(look.bg, ACCENT, look.focus.min(1.0))
    };
    for y in rect.top()..rect.bottom() {
        if y < buf.area.bottom() && rect.x < buf.area.right() {
            let cell = &mut buf[(rect.x, y)];
            cell.set_symbol(g().bar);
            cell.fg = color;
        }
    }
}

pub fn icon_kind(entry: &DirEntry, open: bool) -> IconKind {
    if entry.kind.is_dir() {
        return if entry.hidden {
            IconKind::FolderHidden
        } else if open {
            IconKind::FolderOpen
        } else {
            IconKind::Folder
        };
    }
    IconResolver::default().resolve_with(entry, IconVariant::Normal)
}

/// Badge text (3 cells, centered) and hue for an entry.
pub fn badge(entry: &DirEntry, open: bool) -> (String, Color) {
    let kind = icon_kind(entry, open);
    let hue = if matches!(entry.kind, EntryKind::Symlink { broken: true }) {
        DANGER
    } else if entry.link_dir {
        HUE_LINK
    } else {
        hue_for(kind)
    };
    let text = match icon_style() {
        IconStyle::Nerd => format!(" {} ", nerd_glyph(kind, entry)),
        IconStyle::Badges => {
            let small = if entry.link_dir {
                "ln/"
            } else {
                IconRegistry::new().glyph(kind, IconSize::Small)
            };
            crate::ui::format::center(small, 3)
        }
    };
    (text, hue)
}

/// Nerd Font codepoint per kind (requires a patched font).
fn nerd_glyph(kind: IconKind, entry: &DirEntry) -> &'static str {
    use IconKind::*;
    if entry.link_dir {
        return "\u{f0c1}";
    }
    match kind {
        Folder => "\u{f07b}",
        FolderOpen => "\u{f07c}",
        FolderHidden => "\u{f114}",
        Symlink => "\u{f0c1}",
        Executable => "\u{f489}",
        SourceFile => "\u{f121}",
        Rust => "\u{e7a8}",
        TypeScript => "\u{e628}",
        JavaScript => "\u{e74e}",
        C => "\u{e61e}",
        Cpp => "\u{e61d}",
        Python => "\u{e73c}",
        Shell => "\u{f489}",
        Html => "\u{e736}",
        Css => "\u{e749}",
        Json => "\u{e60b}",
        Toml | Yaml | Config => "\u{e615}",
        Markdown => "\u{e73e}",
        Text => "\u{f15c}",
        WebLink => "\u{f0ac}",
        Subtitle => "\u{f20a}",
        Image => "\u{f1c5}",
        Audio => "\u{f001}",
        Video => "\u{f03d}",
        Archive => "\u{f410}",
        Pdf => "\u{f1c1}",
        Database => "\u{f1c0}",
        Git => "\u{e702}",
        CargoToml | CargoLock => "\u{e7a8}",
        PackageJson | Lockfile => "\u{e71e}",
        Makefile => "\u{f0ad}",
        Docker => "\u{f308}",
        Socket | Pipe | Device => "\u{f0a0}",
        Unknown => "\u{f15b}",
    }
}

/// Display name plus a trailing slash for directory-like entries.
pub fn display_name(view: &EntryView) -> String {
    let name = view.entry.display_name();
    if view.entry.is_dir_like() {
        format!("{name}/")
    } else {
        name
    }
}

/// Name color by entry type (exec green, links teal, hidden muted).
pub fn name_color(entry: &DirEntry) -> Color {
    match entry.kind {
        EntryKind::Symlink { broken: true } => DANGER,
        EntryKind::Symlink { .. } => HUE_LINK,
        EntryKind::Directory if entry.hidden => mix(TEXT_PRIMARY, TEXT_MUTED, 0.5),
        EntryKind::Directory => TEXT_PRIMARY,
        _ if entry.hidden => TEXT_MUTED,
        _ if entry.executable => HUE_EXEC,
        _ => TEXT_SECONDARY,
    }
}

/// Permission string with per-bit colors (read amber, write rose, exec
/// green, unset muted).
pub fn perm_spans(entry: &DirEntry, look: &Look) -> Vec<(String, Style)> {
    let text = crate::ui::format::format_mode(&entry.kind, entry.mode);
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            let color = match (i, c) {
                (0, _) => TEXT_MUTED,
                (_, 'r') => HUE_ARCHIVE,
                (_, 'w') => DANGER,
                (_, 'x') => HUE_EXEC,
                _ => BORDER_STRONG,
            };
            (
                c.to_string(),
                Style::default()
                    .fg(mix(
                        look.bg,
                        mix(color, INK_ON_ACCENT, look.selected),
                        look.appear,
                    ))
                    .bg(look.bg),
            )
        })
        .collect()
}
