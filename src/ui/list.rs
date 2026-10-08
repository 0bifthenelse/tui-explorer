//! List layout: one row per entry with a gliding focus highlight, colored
//! type badges, tags, and right-aligned metadata columns that drop out as
//! the pane narrows. Also used (compact, header-less) as the current pane
//! of the Miller-columns layout.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::state::AppState;
use crate::browser::SortKey;
use crate::ui::anim::{AnimKey, GLIDE};
use crate::ui::entry::{self, Look};
use crate::ui::format::{display_width, format_size, pad_left, relative_time, truncate};
use crate::ui::glyphs::g;
use crate::ui::hit::HitTarget;
use crate::ui::palette::*;
use crate::ui::theme::mix;
use crate::ui::{control_glow, fill, hover_bg, put};

#[derive(Clone, Copy, Debug)]
pub struct ListOpts {
    /// Sticky, clickable column header.
    pub header: bool,
    /// Metadata columns (size, modified, permissions) when they fit.
    pub meta: bool,
    /// Cursor-glide channel for this pane.
    pub channel: u8,
}

struct Columns {
    name: u16,
    size: u16,
    time: u16,
    perms: u16,
}

const LEAD: u16 = 7; // rail(1) + mark(2) + badge(3) + gap(1)
const SIZE_W: u16 = 9;
const TIME_W: u16 = 10;
const PERMS_W: u16 = 11;

fn columns(width: u16, meta: bool) -> Columns {
    let mut size = 0;
    let mut time = 0;
    let mut perms = 0;
    if meta {
        if width >= 36 {
            size = SIZE_W;
        }
        if width >= 56 {
            time = TIME_W;
        }
        if width >= 74 {
            perms = PERMS_W;
        }
    } else if width >= 30 {
        size = SIZE_W;
    }
    let name = width.saturating_sub(LEAD + size + time + perms + 1);
    Columns {
        name,
        size,
        time,
        perms,
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState, opts: ListOpts) {
    if area.width < 8 || area.height < 2 {
        state.grid_cols = 1;
        state.list_viewport = 1;
        return;
    }
    fill(frame.buffer_mut(), area, SURFACE_1);
    let mut body = area;
    let has_scroll = {
        let rows = area.height.saturating_sub(u16::from(opts.header)) as usize;
        state.browser.visible_len() > rows
    };
    let content_w = area.width.saturating_sub(u16::from(has_scroll));
    let cols = columns(content_w, opts.meta);
    if opts.header {
        render_header(frame, Rect::new(area.x, area.y, content_w, 1), state, &cols);
        body = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
    }

    let viewport = body.height.max(1) as usize;
    state.grid_cols = 1;
    state.list_viewport = viewport;
    state.browser.clamp_scroll(viewport);
    // Blank space below the rows arms the marquee / background menu;
    // pushed first so rows (pushed later) win the reverse hit scan.
    state.hit_map.push(body, HitTarget::GridBackground);

    let indices = state.browser.visible_indices();
    if indices.is_empty() {
        render_empty(frame, body, state);
        return;
    }
    let scroll = state.browser.scroll;
    let selected_pos = state.browser.selected;
    let show_cursor = entry::cursor_visible(state);
    let target_y = selected_pos.saturating_sub(scroll) as f32;
    let glide_y = state
        .anim
        .track(AnimKey::CursorY(opts.channel), target_y, GLIDE);

    for (row, eidx) in indices.iter().skip(scroll).take(viewport).enumerate() {
        let pos = scroll + row;
        let view = state.browser.entries[*eidx].clone();
        let selected = state.browser.selection.contains(&view.entry.path);
        let hovered = state.hover.row == Some(pos);
        let focus = if show_cursor {
            (1.0 - (row as f32 - glide_y).abs()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let base = if row % 2 == 1 {
            SURFACE_ZEBRA
        } else {
            SURFACE_1
        };
        let appear = state.anim.cascade(row);
        let look = entry::look(state, &view.entry, base, hovered, selected, focus, appear);
        let rect = Rect::new(body.x, body.y + row as u16, content_w, 1);
        draw_row(
            frame,
            rect,
            state,
            &view,
            &look,
            &cols,
            pos == selected_pos && show_cursor,
        );
        state.hit_map.push(rect, HitTarget::Row(pos));
    }
    if has_scroll {
        render_scrollbar(
            frame,
            Rect::new(area.right() - 1, body.y, 1, body.height),
            scroll,
            viewport,
            indices.len(),
        );
    }
}

fn draw_row(
    frame: &mut Frame,
    rect: Rect,
    state: &AppState,
    view: &crate::browser::EntryView,
    look: &Look,
    cols: &Columns,
    is_cursor: bool,
) {
    let buf = frame.buffer_mut();
    entry::paint_bg(buf, rect, look);
    entry::paint_rail(buf, Rect::new(rect.x, rect.y, 1, 1), look);
    let y = rect.y;
    let mut x = rect.x + 1;
    let mark = if look.selected > 0.5 { g().check } else { " " };
    put(
        buf,
        x,
        y,
        mark,
        1,
        Style::default().fg(look.fg(INK_ON_ACCENT)),
    );
    x += 2;
    let (badge, hue) = entry::badge(&view.entry, is_cursor);
    put(
        buf,
        x,
        y,
        &badge,
        3,
        Style::default()
            .fg(look.fg(hue))
            .add_modifier(Modifier::BOLD),
    );
    x += 4;

    // Name + tag badges share the flexible column.
    let mut name_style = Style::default().fg(look.fg(entry::name_color(&view.entry)));
    if view.entry.kind.is_dir() || is_cursor {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    if matches!(
        view.entry.kind,
        crate::filesystem::EntryKind::Symlink { .. }
    ) {
        name_style = name_style.add_modifier(Modifier::ITALIC);
    }
    // Inline rename: an edit field replaces the name cell.
    if let crate::app::state::Mode::Rename(r) = &state.mode
        && r.target == view.entry.path
    {
        let field = Rect::new(x.saturating_sub(1), y, cols.name + 1, 1);
        crate::ui::overlays::input_field(buf, field, "", &r.edit.text, r.edit.cursor, "");
    } else {
        draw_name(buf, x, y, state, view, look, cols, name_style);
    }
    x += cols.name + 1;

    if cols.size > 0 {
        let size = if view.entry.is_dir_like() {
            match state.browser.child_counts.get(&view.entry.path) {
                Some(n) => format!("{n} item{}", if *n == 1 { "" } else { "s" }),
                None => "—".to_string(),
            }
        } else {
            format_size(view.entry.size)
        };
        put(
            buf,
            x,
            y,
            &pad_left(&size, (cols.size - 1) as usize),
            cols.size,
            Style::default().fg(look.meta()),
        );
        x += cols.size;
    }
    if cols.time > 0 {
        let when = relative_time(view.entry.modified, state.wall_clock);
        put(
            buf,
            x + 1,
            y,
            &pad_left(&when, (cols.time - 1) as usize),
            cols.time,
            Style::default().fg(look.meta()),
        );
        x += cols.time;
    }
    if cols.perms > 0 {
        let mut px = x + 1;
        for (text, style) in entry::perm_spans(&view.entry, look) {
            px += put(buf, px, y, &text, 1, style);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_name(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    state: &AppState,
    view: &crate::browser::EntryView,
    look: &Look,
    cols: &Columns,
    name_style: Style,
) {
    let name = entry::display_name(view);
    let tags: String = view.tags.iter().map(|t| format!(" [{t}]")).collect();
    let link: String = view
        .entry
        .link_target
        .as_ref()
        .map(|t| format!(" {} {}", g().arrow_right, t.display()))
        .unwrap_or_default();
    let budget = cols.name as usize;
    let name_budget = budget
        .saturating_sub(display_width(&tags).min(budget / 2))
        .max(budget.min(8));
    let shown = crate::ui::format::truncate_middle(&name, name_budget);
    let used = put(buf, x, y, &shown, cols.name, name_style);
    if let Some(query) = state.browser.search.as_deref() {
        let hl = if look.selected > 0.5 {
            Style::default()
                .fg(INK_ON_ACCENT)
                .bg(ACCENT_HOVER)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(INK_ON_ACCENT)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        };
        crate::ui::overlays::highlight_match(buf, x, y, &shown, query, hl);
    }
    let mut cursor = x + used;
    let remaining = (x + cols.name).saturating_sub(cursor);
    if !tags.is_empty() && remaining > 2 {
        cursor += put(
            buf,
            cursor,
            y,
            &tags,
            remaining,
            Style::default().fg(look.fg(ACCENT_SOFT)),
        );
    }
    let remaining = (x + cols.name).saturating_sub(cursor);
    if !link.is_empty() && remaining > 4 {
        put(
            buf,
            cursor,
            y,
            &link,
            remaining,
            Style::default().fg(look.meta()),
        );
    }
}

fn render_header(frame: &mut Frame, area: Rect, state: &mut AppState, cols: &Columns) {
    let sort = state.browser.sort_mode;
    let arrow = if sort.desc {
        g().arrow_down
    } else {
        g().arrow_up
    };
    let mut x = area.x + 7;
    let mut specs: Vec<(SortKey, &str, u16, bool)> =
        vec![(SortKey::Name, "Name", cols.name, false)];
    if cols.size > 0 {
        specs.push((SortKey::Size, "Size", cols.size, true));
    }
    if cols.time > 0 {
        specs.push((SortKey::Modified, "Modified", cols.time, true));
    }
    fill(frame.buffer_mut(), area, SURFACE_1);
    for (key, label, width, right) in specs {
        let active = sort.key == key;
        let target = HitTarget::SortBy(key);
        let glow = control_glow(state, target);
        let text = if active {
            format!("{label} {arrow}")
        } else {
            label.to_string()
        };
        let fg = if active {
            ACCENT
        } else {
            mix(TEXT_MUTED, TEXT_PRIMARY, glow)
        };
        let cell_x = if key == SortKey::Name { x } else { x + 1 };
        let rect = Rect::new(cell_x, area.y, width.saturating_sub(1).max(1), 1);
        let buf = frame.buffer_mut();
        fill(buf, rect, hover_bg(SURFACE_1, glow));
        let shown = if right {
            pad_left(&text, rect.width as usize)
        } else {
            truncate(&text, rect.width as usize)
        };
        put(
            buf,
            rect.x,
            rect.y,
            &shown,
            rect.width,
            Style::default()
                .fg(fg)
                .bg(hover_bg(SURFACE_1, glow))
                .add_modifier(Modifier::BOLD),
        );
        state.hit_map.push(rect, target);
        x = if key == SortKey::Name {
            x + width + 1
        } else {
            x + width
        };
    }
    if cols.perms > 0 {
        put(
            frame.buffer_mut(),
            x + 1,
            area.y,
            "Perms",
            cols.perms,
            Style::default().fg(TEXT_MUTED).add_modifier(Modifier::BOLD),
        );
    }
}

pub fn render_scrollbar(
    frame: &mut Frame,
    rect: Rect,
    scroll: usize,
    viewport: usize,
    total: usize,
) {
    if rect.height == 0 || total == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let h = rect.height as usize;
    let thumb = ((viewport * h) / total).clamp(1, h);
    let max_scroll = total.saturating_sub(viewport).max(1);
    let top = ((scroll.min(max_scroll) * (h - thumb)) / max_scroll).min(h - thumb);
    for i in 0..h {
        let in_thumb = i >= top && i < top + thumb;
        let (sym, color) = if in_thumb {
            (g().scroll_thumb, ACCENT)
        } else {
            (g().scroll_track, BORDER_SUBTLE)
        };
        put(
            buf,
            rect.x,
            rect.y + i as u16,
            sym,
            1,
            Style::default().fg(color).bg(SURFACE_1),
        );
    }
}

/// Empty states: a filter with no hits, or a genuinely empty folder.
pub fn render_empty(frame: &mut Frame, area: Rect, state: &AppState) {
    let filtered = state.browser.filter.is_some() || state.browser.search.is_some();
    let (title, hint) = if filtered {
        (
            "No matching files".to_string(),
            "Esc clears the filter".to_string(),
        )
    } else if state.browser.listed_len() == 0 && !state.browser.entries.is_empty() {
        (
            "Only hidden files here".to_string(),
            "zh or . shows hidden files".to_string(),
        )
    } else {
        (
            "This folder is empty".to_string(),
            "+ creates a file or folder · :mkdir · :touch".to_string(),
        )
    };
    let buf = frame.buffer_mut();
    let mid = area.y + area.height / 2;
    let glyph = if filtered { g().search } else { g().dot };
    let art_y = mid.saturating_sub(2);
    if art_y >= area.y {
        let w = 9u16.min(area.width);
        let ax = area.x + (area.width.saturating_sub(w)) / 2;
        let line = crate::ui::format::center(glyph, w as usize);
        put(
            buf,
            ax,
            art_y,
            &line,
            w,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        );
    }
    let center = |text: &str| crate::ui::format::center(text, area.width as usize);
    put(
        buf,
        area.x,
        mid,
        &center(&title),
        area.width,
        Style::default()
            .fg(TEXT_PRIMARY)
            .add_modifier(Modifier::BOLD),
    );
    if mid + 1 < area.bottom() {
        put(
            buf,
            area.x,
            mid + 1,
            &center(&hint),
            area.width,
            Style::default().fg(TEXT_MUTED),
        );
    }
}
