//! Grid layout. Large tiles carry a box-drawn pictogram (folder or a file
//! card with its extension and a category motif) over the name and size;
//! small tiles are a dense multi-column listing. Tiles share the list's
//! animated hover/selection/focus treatment and spread leftover width
//! evenly so the grid never hugs the left edge.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::state::AppState;
use crate::browser::EntryView;
use crate::filesystem::EntryKind;
use crate::icons::IconKind;
use crate::settings::GridSize;
use crate::ui::anim::{AnimKey, GLIDE, path_key};
use crate::ui::entry::{self, Look};
use crate::ui::format::{center, display_width, format_size, truncate_middle};
use crate::ui::glyphs::{Charset, charset};
use crate::ui::hit::HitTarget;
use crate::ui::palette::*;
use crate::ui::theme::mix;
use crate::ui::{fill, put};

/// Large tile footprint including the 1-cell gutter.
pub const TILE_W: u16 = 16;
pub const TILE_H: u16 = 8;
const ART_H: usize = 4;

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 8 || area.height < 3 {
        state.grid_cols = 1;
        state.list_viewport = 1;
        return;
    }
    fill(frame.buffer_mut(), area, SURFACE_1);
    state.hit_map.push(area, HitTarget::GridBackground);
    let indices = state.browser.visible_indices();
    if indices.is_empty() {
        crate::ui::list::render_empty(frame, area, state);
        state.grid_cols = 1;
        state.list_viewport = 1;
        return;
    }
    let small = state.settings.grid_size == GridSize::Small;
    let (tile_w, tile_h) = if small {
        let longest = indices
            .iter()
            .map(|&i| display_width(&state.browser.entries[i].entry.display_name()))
            .max()
            .unwrap_or(8) as u16;
        ((longest + 9).clamp(20, 36), 1)
    } else {
        (TILE_W, TILE_H)
    };
    let inner = Rect::new(
        area.x + 1,
        area.y + u16::from(!small),
        area.width.saturating_sub(2),
        area.height.saturating_sub(u16::from(!small)),
    );
    let cols = (inner.width / tile_w).max(1) as usize;
    let rows = (inner.height / tile_h).max(1) as usize;
    let leftover = inner.width.saturating_sub(cols as u16 * tile_w);
    let gap_extra = if cols > 1 { leftover / cols as u16 } else { 0 };
    state.grid_cols = cols;
    state.list_viewport = cols * rows;
    state.browser.clamp_scroll_grid(cols, rows);

    let scroll = state.browser.scroll;
    let per = cols * rows;
    let show_cursor = entry::cursor_visible(state);
    let selected_pos = state.browser.selected;
    for (i, eidx) in indices.iter().enumerate().skip(scroll).take(per) {
        let view = state.browser.entries[*eidx].clone();
        let slot = i - scroll;
        let col = (slot % cols) as u16;
        let row = (slot / cols) as u16;
        let tx = inner.x + col * (tile_w + gap_extra);
        let ty = inner.y + row * tile_h;
        let w = tile_w.saturating_sub(1);
        let h = if small { 1 } else { tile_h - 1 };
        let rect = Rect::new(tx, ty, w, h).intersection(area);
        let focused = show_cursor && i == selected_pos;
        let focus = state.anim.track(
            AnimKey::RowFocus(path_key(&view.entry.path)),
            if focused { 1.0 } else { 0.0 },
            GLIDE,
        );
        let selected = state.browser.selection.contains(&view.entry.path);
        let hovered = state.hover.row == Some(i);
        let appear = state.anim.cascade(slot);
        let look = entry::look(
            state,
            &view.entry,
            SURFACE_1,
            hovered,
            selected,
            focus,
            appear,
        );
        if small {
            draw_small(frame, rect, &view, &look, focused);
        } else {
            draw_large(frame, rect, &view, &look, focused);
        }
        state.hit_map.push(rect, HitTarget::Row(i));
    }
}

fn draw_small(frame: &mut Frame, rect: Rect, view: &EntryView, look: &Look, focused: bool) {
    let buf = frame.buffer_mut();
    entry::paint_bg(buf, rect, look);
    entry::paint_rail(buf, Rect::new(rect.x, rect.y, 1, 1), look);
    let (badge, hue) = entry::badge(&view.entry, focused);
    put(
        buf,
        rect.x + 1,
        rect.y,
        &badge,
        3,
        Style::default()
            .fg(look.fg(hue))
            .add_modifier(Modifier::BOLD),
    );
    let name = entry::display_name(view);
    let budget = rect.width.saturating_sub(5);
    let mut style = Style::default().fg(look.fg(entry::name_color(&view.entry)));
    if view.entry.is_dir_like() || focused {
        style = style.add_modifier(Modifier::BOLD);
    }
    put(
        buf,
        rect.x + 5,
        rect.y,
        &truncate_middle(&name, budget as usize),
        budget,
        style,
    );
}

fn draw_large(frame: &mut Frame, rect: Rect, view: &EntryView, look: &Look, focused: bool) {
    let buf = frame.buffer_mut();
    entry::paint_bg(buf, rect, look);
    entry::paint_rail(buf, rect, look);
    let kind = entry::icon_kind(&view.entry, focused);
    let (_, hue) = entry::badge(&view.entry, focused);
    let (art, mask) = pictogram(kind, &view.entry);
    let art_w = art.iter().map(|l| display_width(l)).max().unwrap_or(0) as u16;
    let ax = rect.x + rect.width.saturating_sub(art_w) / 2;
    let ay = rect.y + 1;
    let interior = mix(
        look.bg,
        hue,
        0.30 * (1.0 - look.selected) + 0.18 * look.selected,
    );
    for (dy, (line, mask_line)) in art.iter().zip(mask.iter()).enumerate() {
        let y = ay + dy as u16;
        if y >= rect.bottom() {
            break;
        }
        for (dx, (ch, m)) in line.chars().zip(mask_line.chars()).enumerate() {
            let x = ax + dx as u16;
            if x >= rect.right() {
                break;
            }
            let bg = if m == '#' { interior } else { look.bg };
            let is_label = m == '#' && ch != ' ' && dy == 1;
            let fg = if is_label {
                look.fg(mix(hue, TEXT_PRIMARY, 0.25))
            } else if m == '#' {
                look.fg(mix(hue, TEXT_MUTED, 0.35))
            } else {
                look.fg(hue)
            };
            let mut style = Style::default().fg(fg).bg(bg);
            if is_label {
                style = style.add_modifier(Modifier::BOLD);
            }
            let cell = &mut buf[(x, y)];
            cell.set_symbol(&ch.to_string());
            cell.set_style(style);
        }
    }
    let name_y = ay + ART_H as u16;
    let inner_w = rect.width.saturating_sub(2) as usize;
    if name_y < rect.bottom() {
        let name = entry::display_name(view);
        let mut style = Style::default().fg(look.fg(entry::name_color(&view.entry)));
        if view.entry.is_dir_like() || focused {
            style = style.add_modifier(Modifier::BOLD);
        }
        put(
            buf,
            rect.x + 1,
            name_y,
            &center(&truncate_middle(&name, inner_w), inner_w),
            inner_w as u16,
            style,
        );
    }
    let meta_y = name_y + 1;
    if meta_y < rect.bottom() {
        let detail = if view.entry.is_dir_like() {
            "folder".to_string()
        } else {
            format_size(view.entry.size)
        };
        let text = match view.tags.first() {
            Some(tag) => format!("{detail} [{tag}]"),
            None => detail,
        };
        let shown = center(&text, inner_w);
        let style = Style::default().fg(look.meta());
        put(buf, rect.x + 1, meta_y, &shown, inner_w as u16, style);
        if let Some(tag) = view.tags.first() {
            let badge = format!("[{tag}]");
            if let Some(offset) = shown.find(&badge) {
                let col = display_width(&shown[..offset]) as u16;
                put(
                    buf,
                    rect.x + 1 + col,
                    meta_y,
                    &badge,
                    display_width(&badge) as u16,
                    Style::default().fg(look.fg(ACCENT_SOFT)),
                );
            }
        }
    }
}

/// Box-drawn pictogram plus an interior mask (`#` = tinted fill).
fn pictogram(
    kind: IconKind,
    entry: &crate::filesystem::DirEntry,
) -> ([String; ART_H], [String; ART_H]) {
    let ascii = charset() == Charset::Ascii;
    let folder = matches!(
        kind,
        IconKind::Folder | IconKind::FolderOpen | IconKind::FolderHidden
    ) || entry.link_dir;
    if folder {
        let art = if ascii {
            [" ___    ", "|   \\__ ", "|      |", "|______|"]
        } else if kind == IconKind::FolderOpen {
            ["╭───╮   ", "│   ╰──╮", "│ ▁▁▁▁ │", "╰──────╯"]
        } else {
            ["╭───╮   ", "│   ╰──╮", "│      │", "╰──────╯"]
        };
        let mask = ["        ", " ###    ", " ###### ", "        "];
        return (art.map(String::from), mask.map(String::from));
    }
    let label: String = match kind {
        IconKind::Symlink => "LNK".to_string(),
        IconKind::Executable => "EXE".to_string(),
        IconKind::Socket => "SOCK".to_string(),
        IconKind::Pipe => "PIPE".to_string(),
        IconKind::Device => "DEV".to_string(),
        _ => {
            let name = entry.name.to_string_lossy();
            name.rsplit_once('.')
                .filter(|(base, _)| !base.is_empty())
                .map(|(_, ext)| ext.chars().take(4).collect::<String>().to_uppercase())
                .unwrap_or_else(|| {
                    if matches!(entry.kind, EntryKind::File) && entry.executable {
                        "EXE".to_string()
                    } else {
                        "FILE".to_string()
                    }
                })
        }
    };
    let motif = motif(kind, ascii);
    let label = format!(" {:<4}", label);
    let art = if ascii {
        [
            " _____ ".to_string(),
            format!("|{label}|"),
            format!("|{motif}|"),
            "|_____|".to_string(),
        ]
    } else {
        [
            "╭─────╮".to_string(),
            format!("│{label}│"),
            format!("│{motif}│"),
            "╰─────╯".to_string(),
        ]
    };
    let mask = ["       ", " ##### ", " ##### ", "       "].map(String::from);
    (art, mask)
}

/// Five-cell category motif drawn under the extension label.
fn motif(kind: IconKind, ascii: bool) -> &'static str {
    use IconKind::*;
    if ascii {
        return match kind {
            Image => " /\\/ ",
            Audio => " ~~~ ",
            Video => " |>  ",
            Archive => " === ",
            Executable | Shell => " >_  ",
            Rust | TypeScript | JavaScript | C | Cpp | Python | SourceFile | Html | Css => " </> ",
            _ => " --- ",
        };
    }
    match kind {
        Image => " ◢◣◢ ",
        Audio => " ♪ ♫ ",
        Video => " ▶   ",
        Archive => " ▤▤▤ ",
        Executable | Shell => " >_  ",
        Rust | TypeScript | JavaScript | C | Cpp | Python | SourceFile | Html | Css => " </> ",
        Json | Toml | Yaml | Config | CargoToml | CargoLock | PackageJson | Lockfile => " {…} ",
        Database => " ◫◫◫ ",
        Pdf | Markdown => " ≡≡≡ ",
        Symlink => " ↗   ",
        _ => " ─── ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::builders::entry as mk;
    use std::path::Path;

    #[test]
    fn pictograms_have_uniform_width() {
        let root = Path::new("/x");
        for (name, kind) in [
            ("a.rs", EntryKind::File),
            ("dir", EntryKind::Directory),
            ("b.tar.gz", EntryKind::File),
            ("noext", EntryKind::File),
        ] {
            let e = mk(root, name, kind, 1, 0o644, 0);
            let k = entry::icon_kind(&e, false);
            let (art, mask) = pictogram(k, &e);
            let w = display_width(&art[0]);
            for (line, m) in art.iter().zip(mask.iter()) {
                assert_eq!(display_width(line), w, "{name}: {line:?}");
                assert_eq!(m.chars().count(), w, "{name} mask");
            }
        }
    }
}
