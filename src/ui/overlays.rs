//! Searchable overlays: the bookmarks hub, `:find` / `:grep` results,
//! quick look, help, the open-with prompt, the which-key popup and the
//! command-line suggestion dropdown.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Clear;

use crate::app::state::{
    AppState, BookmarkNavState, HubItem, HubSection, Mode, OpenWithState, ResultsState,
};
use crate::filesystem::DirEntry;
use crate::ui::anim::{AnimKey, SELECT};
use crate::ui::entry;
use crate::ui::format::{display_width, pad_left, pad_right, truncate, truncate_middle};
use crate::ui::glyphs::{Charset, charset, g};
use crate::ui::hit::HitTarget;
use crate::ui::modals::centered_rect;
use crate::ui::palette::*;
use crate::ui::theme::mix;
use crate::ui::{control_glow, drop_shadow, fill, hover_bg, overlay_block, put};

// --- Shared pieces --------------------------------------------------------

/// Shadow, clear and frame a modal; returns the padded interior.
fn open_modal(frame: &mut Frame, rect: Rect, title: &str) -> Rect {
    drop_shadow(frame.buffer_mut(), rect);
    frame.render_widget(Clear, rect);
    let block = overlay_block(title, Style::default().fg(ACCENT));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    )
}

/// A one-line text input: icon, text and a block cursor at `cursor`
/// (chars). The view scrolls horizontally to keep the cursor visible.
pub(crate) fn input_field(
    buf: &mut Buffer,
    rect: Rect,
    icon: &str,
    text: &str,
    cursor: usize,
    placeholder: &str,
) {
    if rect.width < 4 {
        return;
    }
    let bg = SURFACE_1;
    fill(buf, rect, bg);
    let mut x = rect.x + 1;
    if !icon.is_empty() {
        x += put(
            buf,
            x,
            rect.y,
            icon,
            3,
            Style::default()
                .fg(ACCENT)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );
        x += 1;
    }
    let avail = rect.right().saturating_sub(x + 1);
    if avail == 0 {
        return;
    }
    let caret = Style::default().fg(INK_ON_ACCENT).bg(ACCENT_HOVER);
    if text.is_empty() {
        put(buf, x, rect.y, " ", 1, caret);
        put(
            buf,
            x + 1,
            rect.y,
            placeholder,
            avail.saturating_sub(1),
            Style::default()
                .fg(TEXT_MUTED)
                .bg(bg)
                .add_modifier(Modifier::ITALIC),
        );
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let width_of = |a: usize, b: usize| display_width(&chars[a..b].iter().collect::<String>());
    let mut start = 0;
    while start < cursor && width_of(start, cursor) + 1 > avail as usize {
        start += 1;
    }
    let visible: String = chars[start..].iter().collect();
    put(
        buf,
        x,
        rect.y,
        &visible,
        avail,
        Style::default().fg(TEXT_PRIMARY).bg(bg),
    );
    let cx = x + width_of(start, cursor) as u16;
    let under = chars
        .get(cursor)
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    if cx < rect.right() {
        put(buf, cx, rect.y, &under, 1, caret);
    }
}

/// Keycap hints (`⏎ open  Tab section  Esc close`) on one row.
fn hints(buf: &mut Buffer, x: u16, y: u16, right: u16, items: &[(&str, &str)]) {
    let mut x = x;
    for (key, label) in items {
        let need = (display_width(key) + display_width(label) + 4) as u16;
        if x + need > right {
            break;
        }
        x += put(
            buf,
            x,
            y,
            &format!(" {key} "),
            right - x,
            Style::default()
                .fg(TEXT_PRIMARY)
                .bg(SURFACE_1)
                .add_modifier(Modifier::BOLD),
        );
        x += put(
            buf,
            x,
            y,
            &format!(" {label}  "),
            right.saturating_sub(x),
            Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
        );
    }
}

/// Background and foreground for a selectable overlay row: hover tints
/// toward the accent, the selected row fills solid orange.
fn row_colors(
    state: &mut AppState,
    channel: &'static str,
    idx: usize,
    selected: bool,
    target: HitTarget,
) -> (Color, f32) {
    let glow = control_glow(state, target);
    let sel = state.anim.track(
        AnimKey::Indexed(channel, idx as u32),
        if selected { 1.0 } else { 0.0 },
        SELECT,
    );
    (mix(hover_bg(SURFACE_3, glow), SELECTED_BG, sel), sel)
}

fn ink(color: Color, sel: f32) -> Color {
    mix(color, INK_ON_ACCENT, sel)
}

/// First window of `len` rows of height `rows` that keeps `selected` in view.
fn scroll_window(selected: usize, rows: usize, len: usize) -> usize {
    if rows == 0 || len <= rows {
        return 0;
    }
    selected.saturating_sub(rows / 2).min(len - rows)
}

fn home_relative(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn base_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn recent_glyph() -> &'static str {
    match charset() {
        Charset::Unicode => "◷",
        Charset::Ascii => "r",
    }
}

/// Paints the case-insensitive match of `query` inside text already drawn
/// at (x, y).
pub(crate) fn highlight_match(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    shown: &str,
    query: &str,
    style: Style,
) {
    if query.is_empty() {
        return;
    }
    let hay: Vec<char> = shown.chars().collect();
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() || needle.len() > hay.len() {
        return;
    }
    let lower: Vec<char> = hay
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let Some(start) =
        (0..=lower.len() - needle.len()).find(|&i| lower[i..i + needle.len()] == needle[..])
    else {
        return;
    };
    let prefix: String = hay[..start].iter().collect();
    let hit: String = hay[start..start + needle.len()].iter().collect();
    let col = x + display_width(&prefix) as u16;
    put(buf, col, y, &hit, display_width(&hit) as u16, style);
}

// --- Bookmarks hub ----------------------------------------------------------

pub(crate) fn render_hub(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    nav: &BookmarkNavState,
) {
    let width = area.width.saturating_sub(4).clamp(30, 92);
    let height = area.height.saturating_sub(2).clamp(8, 26);
    let rect = centered_rect(area, width, height);
    let title = if nav.picker.is_some() {
        "LINKS"
    } else {
        "BOOKMARKS"
    };
    let inner = open_modal(frame, rect, title);
    if inner.height < 5 {
        return;
    }
    let buf = frame.buffer_mut();
    let count = format!("{} ", nav.matches.len());
    let count_w = display_width(&count) as u16;
    input_field(
        buf,
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(count_w + 1), 1),
        g().search,
        &nav.query,
        nav.query.chars().count(),
        if nav.picker.is_some() {
            "filter links…"
        } else {
            "search folders, links, marks…"
        },
    );
    put(
        buf,
        inner.right().saturating_sub(count_w),
        inner.y,
        &count,
        count_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
    );

    // Section tabs.
    let tabs_y = inner.y + 1;
    if let Some(links) = &nav.picker {
        put(
            buf,
            inner.x,
            tabs_y,
            &format!(
                "{} {} link{} found in this file",
                g().link,
                links.len(),
                if links.len() == 1 { "" } else { "s" }
            ),
            inner.width,
            Style::default().fg(TEXT_SECONDARY).bg(SURFACE_3),
        );
    } else {
        let mut x = inner.x;
        for (i, section) in HubSection::ALL.iter().enumerate() {
            let label = format!(" {} ", section.label());
            let w = display_width(&label) as u16;
            if x + w > inner.right() {
                break;
            }
            let target = HitTarget::HubTab(i);
            let glow = control_glow(state, target);
            let active = *section == nav.section;
            let style = if active {
                Style::default()
                    .fg(INK_ON_ACCENT)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(mix(TEXT_SECONDARY, TEXT_PRIMARY, glow))
                    .bg(hover_bg(SURFACE_2, glow))
            };
            let buf = frame.buffer_mut();
            put(buf, x, tabs_y, &label, w, style);
            state.hit_map.push(Rect::new(x, tabs_y, w, 1), target);
            x += w + 1;
        }
    }
    let buf = frame.buffer_mut();
    put(
        buf,
        inner.x,
        tabs_y + 1,
        &g().h_line.repeat(inner.width as usize),
        inner.width,
        Style::default().fg(BORDER_SUBTLE).bg(SURFACE_3),
    );

    let list = Rect::new(
        inner.x,
        tabs_y + 2,
        inner.width,
        inner.height.saturating_sub(4),
    );
    if nav.matches.is_empty() {
        let (title, hint) = if !nav.query.is_empty() {
            (format!("No matches for “{}”", nav.query), String::new())
        } else {
            match nav.section {
                HubSection::All | HubSection::Folders => (
                    "No bookmarks yet".to_string(),
                    "Ctrl-b bookmarks the current folder".to_string(),
                ),
                HubSection::Links => (
                    "No saved links".to_string(),
                    ":bookmark-url <url> [title] saves a web link".to_string(),
                ),
                HubSection::Marks => (
                    "No marks".to_string(),
                    "m<key> marks the current folder, '<key> jumps back".to_string(),
                ),
                HubSection::Recent => (
                    "Nothing visited yet".to_string(),
                    "folders you open show up here".to_string(),
                ),
            }
        };
        let mid = list.y + list.height / 2;
        put(
            buf,
            list.x,
            mid.saturating_sub(1),
            &crate::ui::format::center(&title, list.width as usize),
            list.width,
            Style::default()
                .fg(TEXT_PRIMARY)
                .bg(SURFACE_3)
                .add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            list.x,
            mid,
            &crate::ui::format::center(&hint, list.width as usize),
            list.width,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
        );
    }
    let rows = list.height as usize;
    let offset = scroll_window(nav.selected, rows, nav.matches.len());
    let home = state.home.clone();
    let title_w = (list.width as usize * 2 / 5).clamp(10, 32);
    for (row, item) in nav.matches.iter().enumerate().skip(offset).take(rows) {
        let y = list.y + (row - offset) as u16;
        let rect = Rect::new(list.x, y, list.width, 1);
        let target = HitTarget::HubRow(row);
        let (bg, sel) = row_colors(state, "hub", row, row == nav.selected, target);
        let buf = frame.buffer_mut();
        fill(buf, rect, bg);
        if sel > 0.5 {
            put(
                buf,
                rect.x,
                y,
                g().bar,
                1,
                Style::default().fg(ACCENT_HOVER).bg(bg),
            );
        }
        let (icon, icon_color, title, detail, kind) = match item {
            HubItem::Folder(p) => (
                g().star.to_string(),
                ACCENT,
                base_name(p),
                home_relative(p, &home),
                "folder",
            ),
            HubItem::Link(link) => (
                g().link.to_string(),
                HUE_WEB,
                link.title.clone(),
                link.url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .to_string(),
                "link",
            ),
            HubItem::Mark(c, p) => (
                c.to_string(),
                HUE_CODE,
                base_name(p),
                home_relative(p, &home),
                "mark",
            ),
            HubItem::Recent(p) => (
                recent_glyph().to_string(),
                TEXT_MUTED,
                base_name(p),
                home_relative(p, &home),
                "recent",
            ),
        };
        let mut x = rect.x + 2;
        let icon_style = if matches!(item, HubItem::Mark(..)) {
            Style::default()
                .fg(ink(INK_ON_ACCENT, sel))
                .bg(mix(icon_color, SURFACE_0, sel * 0.4))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(ink(icon_color, sel))
                .add_modifier(Modifier::BOLD)
        };
        put(buf, x, y, &icon, 1, icon_style);
        x += 3;
        let kind_w = 7u16;
        let shown = truncate(&title, title_w);
        put(
            buf,
            x,
            y,
            &shown,
            title_w as u16,
            Style::default()
                .fg(ink(TEXT_PRIMARY, sel))
                .add_modifier(Modifier::BOLD),
        );
        highlight_match(
            buf,
            x,
            y,
            &shown,
            &nav.query,
            Style::default()
                .fg(ink(ACCENT_HOVER, sel))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        x += title_w as u16 + 2;
        let detail_w = rect.right().saturating_sub(x + kind_w + 1);
        put(
            buf,
            x,
            y,
            &truncate_middle(&detail, detail_w as usize),
            detail_w,
            Style::default().fg(ink(TEXT_MUTED, sel)),
        );
        put(
            buf,
            rect.right().saturating_sub(kind_w),
            y,
            &pad_left(kind, kind_w as usize - 1),
            kind_w,
            Style::default().fg(ink(BORDER_STRONG, sel)),
        );
        state.hit_map.push(rect, target);
    }
    let buf = frame.buffer_mut();
    let mut items = vec![("⏎", "open"), ("↑↓", "move")];
    if nav.picker.is_none() {
        items.push(("Tab", "section"));
        items.push(("Del", "remove"));
    }
    items.push(("Esc", "close"));
    hints(buf, inner.x, inner.bottom() - 1, inner.right(), &items);
}

// --- Find / grep results ----------------------------------------------------

pub(crate) fn render_results(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    r: &ResultsState,
) {
    let width = area.width.saturating_sub(4).clamp(30, 110);
    let height = area.height.saturating_sub(2).clamp(8, 32);
    let rect = centered_rect(area, width, height);
    let inner = open_modal(frame, rect, "RESULTS");
    if inner.height < 5 {
        return;
    }
    let buf = frame.buffer_mut();
    let count = format!("{}/{} ", r.matches.len(), r.hits.len());
    let count_w = display_width(&count) as u16;
    input_field(
        buf,
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(count_w + 1), 1),
        g().search,
        &r.query,
        r.query.chars().count(),
        "narrow the results…",
    );
    put(
        buf,
        inner.right().saturating_sub(count_w),
        inner.y,
        &count,
        count_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
    );
    let home = state.home.clone();
    put(
        buf,
        inner.x,
        inner.y + 1,
        &truncate(
            &format!("{}  in {}", r.title, home_relative(&r.root, &home)),
            inner.width as usize,
        ),
        inner.width,
        Style::default().fg(ACCENT_SOFT).bg(SURFACE_3),
    );
    put(
        buf,
        inner.x,
        inner.y + 2,
        &g().h_line.repeat(inner.width as usize),
        inner.width,
        Style::default().fg(BORDER_SUBTLE).bg(SURFACE_3),
    );
    let list = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(4),
    );
    let rows = list.height as usize;
    let offset = scroll_window(r.selected, rows, r.matches.len());
    for (row, &hit_idx) in r.matches.iter().enumerate().skip(offset).take(rows) {
        let Some(hit) = r.hits.get(hit_idx) else {
            continue;
        };
        let y = list.y + (row - offset) as u16;
        let rect = Rect::new(list.x, y, list.width, 1);
        let target = HitTarget::ResultRow(row);
        let (bg, sel) = row_colors(state, "results", row, row == r.selected, target);
        let buf = frame.buffer_mut();
        fill(buf, rect, bg);
        if sel > 0.5 {
            put(
                buf,
                rect.x,
                y,
                g().bar,
                1,
                Style::default().fg(ACCENT_HOVER).bg(bg),
            );
        }
        let name = base_name(&hit.path);
        let fake = DirEntry::synthetic(std::path::Path::new("/"), &name, hit.is_dir);
        let (badge, hue) = entry::badge(&fake, false);
        let mut x = rect.x + 2;
        put(
            buf,
            x,
            y,
            &badge,
            3,
            Style::default()
                .fg(ink(hue, sel))
                .add_modifier(Modifier::BOLD),
        );
        x += 4;
        let rel = hit
            .path
            .strip_prefix(&r.root)
            .unwrap_or(&hit.path)
            .display()
            .to_string();
        let rel = if hit.is_dir { format!("{rel}/") } else { rel };
        let path_w = match &hit.line {
            Some(_) => (rect.width as usize * 2 / 5).max(12),
            None => rect.right().saturating_sub(x + 1) as usize,
        };
        let shown = truncate_middle(&rel, path_w);
        let used = put(
            buf,
            x,
            y,
            &shown,
            path_w as u16,
            Style::default().fg(ink(
                if hit.is_dir {
                    TEXT_PRIMARY
                } else {
                    TEXT_SECONDARY
                },
                sel,
            )),
        );
        highlight_match(
            buf,
            x,
            y,
            &shown,
            &r.query,
            Style::default()
                .fg(ink(ACCENT_HOVER, sel))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        if let Some((line_no, text)) = &hit.line {
            let grep_query = r.title.strip_prefix("grep ").unwrap_or("");
            let mut lx = x + used.max(path_w as u16) + 1;
            lx += put(
                buf,
                lx,
                y,
                &format!(":{line_no} "),
                8,
                Style::default()
                    .fg(ink(ACCENT, sel))
                    .add_modifier(Modifier::BOLD),
            );
            let line = truncate(text.trim(), rect.right().saturating_sub(lx + 1) as usize);
            put(
                buf,
                lx,
                y,
                &line,
                rect.right().saturating_sub(lx + 1),
                Style::default().fg(ink(TEXT_SECONDARY, sel)),
            );
            highlight_match(
                buf,
                lx,
                y,
                &line,
                grep_query,
                Style::default()
                    .fg(ink(ACCENT_HOVER, sel))
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            );
        }
        state.hit_map.push(rect, target);
    }
    let buf = frame.buffer_mut();
    hints(
        buf,
        inner.x,
        inner.bottom() - 1,
        inner.right(),
        &[
            ("⏎", "jump to"),
            ("↑↓", "move"),
            ("type", "filter"),
            ("Esc", "close"),
        ],
    );
}

// --- Quick look --------------------------------------------------------------

pub(crate) fn render_quick_look(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let rect = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let Some(view) = state.browser.focused().cloned() else {
        return;
    };
    let inner = open_modal(frame, rect, "QUICK LOOK");
    if inner.height < 4 {
        return;
    }
    let buf = frame.buffer_mut();
    let (badge, hue) = entry::badge(&view.entry, false);
    put(
        buf,
        inner.x,
        inner.y,
        &badge,
        3,
        Style::default()
            .fg(hue)
            .bg(SURFACE_3)
            .add_modifier(Modifier::BOLD),
    );
    let name = entry::display_name(&view);
    let meta = format!(
        "{}  {}  {}",
        if view.entry.is_dir_like() {
            String::new()
        } else {
            crate::ui::format::format_size(view.entry.size)
        },
        crate::ui::format::kind_label(&view.entry.kind),
        crate::ui::format::relative_time(view.entry.modified, state.wall_clock),
    );
    let meta_w = (display_width(&meta) as u16).min(inner.width / 2);
    let name_w = inner.width.saturating_sub(meta_w + 6);
    put(
        buf,
        inner.x + 4,
        inner.y,
        &truncate_middle(&name, name_w as usize),
        name_w,
        Style::default()
            .fg(TEXT_PRIMARY)
            .bg(SURFACE_3)
            .add_modifier(Modifier::BOLD),
    );
    put(
        buf,
        inner.right().saturating_sub(meta_w),
        inner.y,
        &truncate(&meta, meta_w as usize),
        meta_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
    );
    let content = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(3),
    );
    let scroll = match &state.mode {
        Mode::QuickLook(q) => q.scroll,
        _ => 0,
    };
    let max_scroll = crate::ui::preview::render_scrolled(frame, content, state, true, scroll);
    if let Mode::QuickLook(q) = &mut state.mode
        && q.scroll > max_scroll
    {
        q.scroll = max_scroll;
    }
    let shown = scroll.min(max_scroll);
    let buf = frame.buffer_mut();
    let position = match &state.preview.content {
        Some(crate::app::state::PreviewContent::Text { lines, .. }) if !lines.is_empty() => {
            let last = (shown + content.height as usize).min(lines.len());
            format!("lines {}–{} of {} ", shown + 1, last, lines.len())
        }
        _ => String::new(),
    };
    let pos_w = display_width(&position) as u16;
    hints(
        buf,
        inner.x,
        inner.bottom() - 1,
        inner.right().saturating_sub(pos_w + 1),
        &[
            ("j/k", "scroll"),
            ("Space", "page"),
            ("e", "edit"),
            ("r", "open with"),
            ("Esc", "close"),
        ],
    );
    put(
        buf,
        inner.right().saturating_sub(pos_w),
        inner.bottom() - 1,
        &position,
        pos_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
    );
}

// --- Help ------------------------------------------------------------------

enum HelpLine {
    Header(String),
    Row(String, String),
    Blank,
}

fn help_lines(query: &str) -> Vec<HelpLine> {
    use crate::input::chords::{Group, help_rows};
    let query = query.to_lowercase();
    let keep = |keys: &str, desc: &str| {
        query.is_empty()
            || keys.to_lowercase().contains(&query)
            || desc.to_lowercase().contains(&query)
    };
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let rows = help_rows();
    // Everyday file actions first; motions are the least surprising.
    let order = [
        Group::Files,
        Group::Motion,
        Group::Select,
        Group::Search,
        Group::View,
        Group::Sort,
        Group::Go,
        Group::Tabs,
        Group::Marks,
        Group::Misc,
    ];
    for group in order {
        let items: Vec<(String, String)> = rows
            .iter()
            .filter(|(g, keys, desc)| *g == group && keep(keys, desc))
            .map(|(_, keys, desc)| (pretty_keys(keys), desc.to_string()))
            .collect();
        if !items.is_empty() {
            sections.push((group.title().to_string(), items));
        }
    }
    let commands: Vec<(String, String)> = crate::input::command::COMMANDS
        .iter()
        .map(|(name, hint, desc)| {
            let label = if hint.is_empty() {
                format!(":{name}")
            } else {
                format!(":{name} {hint}")
            };
            (label, desc.to_string())
        })
        .filter(|(k, d)| keep(k, d))
        .collect();
    if !commands.is_empty() {
        sections.push(("Commands".to_string(), commands));
    }
    let media: Vec<(String, String)> = [
        ("Space / Enter", "play or pause"),
        ("← / →  h / l", "seek 15 s back / forward"),
        ("Shift-← / →  H / L", "seek 60 s"),
        ("0 … 9", "jump to 0 % … 90 %"),
        ("↑ / ↓  + / -", "volume (up to 130 %, remembered)"),
        ("m", "mute / unmute"),
        ("n / p", "next / previous track"),
        ("x / r", "shuffle / repeat (off, all, one)"),
        ("[ / ] / Backspace", "slower / faster / normal speed"),
        ("c", "subtitles: pick a local .srt / .ass file"),
        ("j / v", "next subtitle track / subtitles on-off"),
        ("z / Z", "subtitle delay -/+ 0.1 s"),
        ("a", "next audio track"),
        ("f", "fullscreen video"),
        ("s", "restart from the beginning"),
        ("Esc", "music keeps playing in the mini player"),
        ("M", "bring the player back"),
        ("q", "stop and close the player"),
    ]
    .iter()
    .filter(|(k, d)| keep(k, d))
    .map(|(k, d)| (k.to_string(), d.to_string()))
    .collect();
    if !media.is_empty() {
        sections.push(("Player".to_string(), media));
    }
    let mouse: Vec<(String, String)> = [
        ("click", "focus and select"),
        ("double click", "open"),
        ("right click", "context menu"),
        ("drag", "move onto a folder (Ctrl copies)"),
        ("drag background", "marquee selection"),
        ("wheel", "scroll"),
        ("header", "click a column to sort, a layout to switch"),
    ]
    .iter()
    .filter(|(k, d)| keep(k, d))
    .map(|(k, d)| (k.to_string(), d.to_string()))
    .collect();
    if !mouse.is_empty() {
        sections.push(("Mouse".to_string(), mouse));
    }
    let mut out = Vec::new();
    for (i, (title, items)) in sections.into_iter().enumerate() {
        if i > 0 {
            out.push(HelpLine::Blank);
        }
        out.push(HelpLine::Header(title));
        out.extend(items.into_iter().map(|(k, d)| HelpLine::Row(k, d)));
    }
    out
}

/// `<C-b>` → `Ctrl-b`, `<Space>` → `Space`, for the help screen.
fn pretty_keys(keys: &str) -> String {
    let mut out = String::new();
    let mut rest = keys;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let token = &rest[start + 1..start + end];
        let name = |k: &str| match k {
            "key" => "<key>".to_string(),
            "BS" => "Backspace".to_string(),
            "CR" => "Enter".to_string(),
            "Del" => "Delete".to_string(),
            other => other.to_string(),
        };
        let pretty = if let Some(k) = token.strip_prefix("C-") {
            format!("Ctrl-{}", name(k))
        } else if let Some(k) = token.strip_prefix("A-") {
            format!("Alt-{}", name(k))
        } else if let Some(k) = token.strip_prefix("S-") {
            format!("Shift-{}", name(k))
        } else {
            name(token)
        };
        out.push_str(&pretty);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn render_help(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let width = area.width.saturating_sub(4).clamp(30, 118);
    let height = area.height.saturating_sub(2).max(8);
    let rect = centered_rect(area, width, height);
    let inner = open_modal(frame, rect, "HELP");
    if inner.height < 4 {
        return;
    }
    let buf = frame.buffer_mut();
    input_field(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        g().search,
        &state.help_query,
        state.help_query.chars().count(),
        "type to search keys and commands…",
    );
    let body = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(3),
    );
    let lines = help_lines(&state.help_query);
    let columns: usize = if body.width >= 96 { 2 } else { 1 };
    let gap = 3u16;
    let col_w = (body.width - gap * (columns as u16 - 1)) / columns as u16;
    let per_col = lines.len().div_ceil(columns);
    let rows = body.height as usize;
    let max_scroll = per_col.saturating_sub(rows);
    state.help_scroll = state.help_scroll.min(max_scroll);
    let scroll = state.help_scroll;
    let key_w = (col_w as usize / 3).clamp(10, 22);
    if lines.is_empty() {
        put(
            buf,
            body.x,
            body.y,
            &format!("Nothing matches “{}”", state.help_query),
            body.width,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
        );
    }
    for col in 0..columns {
        let x = body.x + col as u16 * (col_w + gap);
        let start = col * per_col + scroll;
        let end = ((col + 1) * per_col).min(lines.len());
        for (row, line) in lines.iter().enumerate().take(end).skip(start).take(rows) {
            let y = body.y + (row - start) as u16;
            match line {
                HelpLine::Header(title) => {
                    let label = format!(" {title} ");
                    let w = put(
                        buf,
                        x,
                        y,
                        &label,
                        col_w,
                        Style::default()
                            .fg(ACCENT)
                            .bg(SURFACE_3)
                            .add_modifier(Modifier::BOLD),
                    );
                    let rest = col_w.saturating_sub(w + 1);
                    put(
                        buf,
                        x + w + 1,
                        y,
                        &g().h_line.repeat(rest as usize),
                        rest,
                        Style::default().fg(BORDER_SUBTLE).bg(SURFACE_3),
                    );
                }
                HelpLine::Row(keys, desc) => {
                    let shown = pad_right(&truncate(keys, key_w), key_w);
                    put(
                        buf,
                        x + 1,
                        y,
                        &shown,
                        key_w as u16,
                        Style::default()
                            .fg(ACCENT_SOFT)
                            .bg(SURFACE_3)
                            .add_modifier(Modifier::BOLD),
                    );
                    let dx = x + 2 + key_w as u16;
                    let dw = (x + col_w).saturating_sub(dx);
                    let desc_shown = truncate(desc, dw as usize);
                    put(
                        buf,
                        dx,
                        y,
                        &desc_shown,
                        dw,
                        Style::default().fg(TEXT_SECONDARY).bg(SURFACE_3),
                    );
                    let hl = Style::default()
                        .fg(ACCENT_HOVER)
                        .bg(SURFACE_3)
                        .add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
                    highlight_match(buf, x + 1, y, &shown, &state.help_query, hl);
                    highlight_match(buf, dx, y, &desc_shown, &state.help_query, hl);
                }
                HelpLine::Blank => {}
            }
        }
    }
    let position = if max_scroll > 0 {
        format!("{}/{} ", scroll, max_scroll)
    } else {
        String::new()
    };
    let pos_w = display_width(&position) as u16;
    hints(
        buf,
        inner.x,
        inner.bottom() - 1,
        inner.right().saturating_sub(pos_w + 1),
        &[
            ("type", "search"),
            ("↑↓", "scroll"),
            ("PgDn", "page"),
            ("Esc", "close"),
        ],
    );
    put(
        buf,
        inner.right().saturating_sub(pos_w),
        inner.bottom() - 1,
        &position,
        pos_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
    );
}

// --- Open with -------------------------------------------------------------

pub(crate) const OPEN_WITH_HEIGHT: u16 = 13;

pub(crate) fn render_open_with(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    dialog: &OpenWithState,
) {
    let width = area.width.saturating_sub(4).clamp(30, 72);
    let rect = centered_rect(area, width, OPEN_WITH_HEIGHT);
    let inner = open_modal(frame, rect, "OPEN WITH");
    if inner.height < 8 {
        return;
    }
    let buf = frame.buffer_mut();
    let name = base_name(&dialog.target);
    let fake = DirEntry::synthetic(std::path::Path::new("/"), &name, false);
    let (badge, hue) = entry::badge(&fake, false);
    put(
        buf,
        inner.x,
        inner.y,
        &badge,
        3,
        Style::default()
            .fg(hue)
            .bg(SURFACE_3)
            .add_modifier(Modifier::BOLD),
    );
    put(
        buf,
        inner.x + 4,
        inner.y,
        &truncate_middle(&name, inner.width.saturating_sub(4) as usize),
        inner.width.saturating_sub(4),
        Style::default()
            .fg(TEXT_PRIMARY)
            .bg(SURFACE_3)
            .add_modifier(Modifier::BOLD),
    );
    input_field(
        buf,
        Rect::new(inner.x, inner.y + 2, inner.width, 1),
        "$",
        &dialog.input,
        dialog.input.chars().count(),
        "program and arguments (the file is appended)",
    );

    // Suggestion chips detected on PATH.
    let mut x = inner.x;
    let chips_y = inner.y + 4;
    if dialog.suggestions.is_empty() {
        put(
            buf,
            x,
            chips_y,
            "no known openers found on PATH",
            inner.width,
            Style::default()
                .fg(TEXT_MUTED)
                .bg(SURFACE_3)
                .add_modifier(Modifier::ITALIC),
        );
    }
    for (i, suggestion) in dialog.suggestions.iter().enumerate() {
        let label = format!(" {suggestion} ");
        let w = display_width(&label) as u16;
        if x + w > inner.right() {
            break;
        }
        let target = HitTarget::OpenWithChip(i);
        let glow = control_glow(state, target);
        let active = dialog.suggestion == Some(i) || dialog.input == *suggestion;
        let style = if active {
            Style::default()
                .fg(INK_ON_ACCENT)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(mix(TEXT_SECONDARY, TEXT_PRIMARY, glow))
                .bg(hover_bg(SURFACE_1, glow))
        };
        put(frame.buffer_mut(), x, chips_y, &label, w, style);
        state.hit_map.push(Rect::new(x, chips_y, w, 1), target);
        x += w + 1;
    }

    // Remember toggle.
    let ext = crate::settings::association_key(&dialog.target);
    let toggle_y = inner.y + 6;
    let target = HitTarget::OpenWithRemember;
    let glow = control_glow(state, target);
    let checked = dialog.remember && ext.is_some();
    let label = match &ext {
        Some(ext) => format!(" remember for .{ext} files"),
        None => " no extension to remember".to_string(),
    };
    let w = (display_width(&label) as u16 + 3).min(inner.width);
    let box_style = if checked {
        Style::default()
            .fg(INK_ON_ACCENT)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(TEXT_SECONDARY)
            .bg(hover_bg(SURFACE_1, glow))
    };
    let buf = frame.buffer_mut();
    let mark = if checked { g().check } else { " " };
    put(buf, inner.x, toggle_y, &format!(" {mark} "), 3, box_style);
    put(
        buf,
        inner.x + 3,
        toggle_y,
        &label,
        w.saturating_sub(3),
        Style::default()
            .fg(mix(TEXT_SECONDARY, TEXT_PRIMARY, glow))
            .bg(SURFACE_3),
    );
    if ext.is_some() {
        state
            .hit_map
            .push(Rect::new(inner.x, toggle_y, w, 1), target);
    }
    let buf = frame.buffer_mut();
    hints(
        buf,
        inner.x,
        inner.bottom() - 1,
        inner.right(),
        &[
            ("⏎", "run"),
            ("Tab", "suggestion"),
            ("Ctrl-r", "remember"),
            ("Esc", "cancel"),
        ],
    );
}

// --- Subtitle picker ---------------------------------------------------------

pub(crate) fn render_sub_picker(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    picker: &crate::app::state::SubPickerState,
) {
    use crate::app::state::SubChoice;
    let width = area.width.saturating_sub(6).clamp(30, 84);
    let height = area.height.saturating_sub(4).clamp(8, 20);
    let rect = centered_rect(area, width, height);
    state.hit_map.push(rect, HitTarget::Blocker);
    let inner = open_modal(frame, rect, "SUBTITLES");
    if inner.height < 5 {
        return;
    }
    let buf = frame.buffer_mut();
    let count = if picker.searching {
        format!(
            "{} searching ",
            g().spinner[state.anim.tick(80) % g().spinner.len()]
        )
    } else {
        format!("{} found ", picker.found.len())
    };
    if picker.searching {
        state.anim.keep_alive();
    }
    let count_w = display_width(&count) as u16;
    input_field(
        buf,
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(count_w + 1), 1),
        g().search,
        &picker.query,
        picker.query.chars().count(),
        "filter by name or language…",
    );
    put(
        buf,
        inner.right().saturating_sub(count_w),
        inner.y,
        &count,
        count_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
    );
    put(
        buf,
        inner.x,
        inner.y + 1,
        &g().h_line.repeat(inner.width as usize),
        inner.width,
        Style::default().fg(BORDER_SUBTLE).bg(SURFACE_3),
    );
    let list = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(3),
    );
    let rows = list.height as usize;
    let offset = scroll_window(picker.selected, rows, picker.choices.len());
    for (row, choice) in picker.choices.iter().enumerate().skip(offset).take(rows) {
        let y = list.y + (row - offset) as u16;
        let rect = Rect::new(list.x, y, list.width, 1);
        let target = HitTarget::SubRow(row);
        let (bg, sel) = row_colors(state, "subs", row, row == picker.selected, target);
        let buf = frame.buffer_mut();
        fill(buf, rect, bg);
        if sel > 0.5 {
            put(
                buf,
                rect.x,
                y,
                g().bar,
                1,
                Style::default().fg(ACCENT_HOVER).bg(bg),
            );
        }
        let (icon, title, detail) = match choice {
            SubChoice::Off => (
                g().close.to_string(),
                "Off".to_string(),
                "hide subtitles".to_string(),
            ),
            SubChoice::Embedded => (
                g().subtitles.to_string(),
                "Video's own tracks".to_string(),
                "embedded / auto-loaded · j cycles".to_string(),
            ),
            SubChoice::File(file) => {
                let mut detail = file.language.clone().unwrap_or_default();
                for flag in &file.flags {
                    if !detail.is_empty() {
                        detail.push_str(" · ");
                    }
                    detail.push_str(flag);
                }
                let folder = file
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !detail.is_empty() {
                    detail.push_str(" · ");
                }
                detail.push_str(&folder);
                ("cc".to_string(), file.name(), detail)
            }
        };
        put(
            buf,
            rect.x + 2,
            y,
            &icon,
            2,
            Style::default()
                .fg(ink(HUE_VIDEO, sel))
                .add_modifier(Modifier::BOLD),
        );
        let title_w = (rect.width as usize * 3 / 5).max(12);
        let shown = truncate_middle(&title, title_w);
        put(
            buf,
            rect.x + 5,
            y,
            &shown,
            title_w as u16,
            Style::default()
                .fg(ink(TEXT_PRIMARY, sel))
                .add_modifier(Modifier::BOLD),
        );
        highlight_match(
            buf,
            rect.x + 5,
            y,
            &shown,
            &picker.query,
            Style::default()
                .fg(ink(ACCENT_HOVER, sel))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        let dx = rect.x + 6 + title_w as u16;
        put(
            buf,
            dx,
            y,
            &truncate(&detail, rect.right().saturating_sub(dx + 1) as usize),
            rect.right().saturating_sub(dx + 1),
            Style::default().fg(ink(TEXT_MUTED, sel)),
        );
        state.hit_map.push(rect, target);
    }
    if picker.choices.is_empty() {
        put(
            frame.buffer_mut(),
            list.x + 1,
            list.y,
            "no subtitle files match",
            list.width,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_3),
        );
    }
    hints(
        frame.buffer_mut(),
        inner.x,
        inner.bottom() - 1,
        inner.right(),
        &[
            ("⏎", "load"),
            ("↑↓", "move"),
            ("type", "filter"),
            ("Esc", "back"),
        ],
    );
}

// --- Which-key ---------------------------------------------------------------

/// Continuations of the pending chord, bottom-right above the status bar.
pub(crate) fn render_which_key(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if !matches!(state.mode, Mode::Browser) || state.pending_keys.is_empty() {
        return;
    }
    let items = crate::input::chords::continuations(&state.pending_keys);
    if items.is_empty() {
        return;
    }
    let key_w = items
        .iter()
        .map(|(k, _)| display_width(&pretty_keys(k)))
        .max()
        .unwrap_or(1)
        .min(12);
    let desc_w = items
        .iter()
        .map(|(_, d)| display_width(d))
        .max()
        .unwrap_or(10)
        .min(30);
    let cell_w = (key_w + desc_w + 3) as u16;
    let max_h = area.height.saturating_sub(5).max(3);
    let per_col = (max_h.saturating_sub(2)) as usize;
    let columns = items.len().div_ceil(per_col.max(1)).clamp(1, 3);
    let rows = items.len().div_ceil(columns);
    let width = (cell_w * columns as u16 + 2 + (columns as u16 - 1) * 2).min(area.width);
    let height = (rows as u16 + 2).min(max_h);
    let t = state.anim.enter(
        AnimKey::Named("which-key"),
        0.0,
        1.0,
        crate::ui::anim::MODAL,
    );
    let dy = ((1.0 - t) * 2.0).round() as u16;
    let rect = Rect::new(
        area.right().saturating_sub(width + 1),
        area.bottom().saturating_sub(height + 2) + dy,
        width,
        height,
    );
    let mut title: String = state.pending_keys.iter().map(|k| pretty_keys(k)).collect();
    if let Some(n) = state.pending_count {
        title = format!("{n}{title}");
    }
    drop_shadow(frame.buffer_mut(), rect);
    frame.render_widget(Clear, rect);
    let block = overlay_block(
        &format!("{title}{}", g().ellipsis),
        Style::default().fg(ACCENT),
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let buf = frame.buffer_mut();
    for (i, (key, desc)) in items.iter().enumerate() {
        let col = i / rows;
        let row = i % rows;
        if row as u16 >= inner.height {
            continue;
        }
        let x = inner.x + 1 + col as u16 * (cell_w + 2);
        let y = inner.y + row as u16;
        let key = pad_right(&pretty_keys(key), key_w);
        put(
            buf,
            x,
            y,
            &key,
            key_w as u16,
            Style::default()
                .fg(ACCENT)
                .bg(SURFACE_3)
                .add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            x + key_w as u16 + 1,
            y,
            &truncate(desc, desc_w),
            desc_w as u16,
            Style::default().fg(TEXT_SECONDARY).bg(SURFACE_3),
        );
    }
}

// --- Command suggestions -----------------------------------------------------

/// Dropdown of matching commands (or path candidates) above the command
/// line while typing.
pub(crate) fn render_suggestions(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if !matches!(state.mode, Mode::Command) {
        return;
    }
    let suggestions = crate::app::cmdline::suggestions(state);
    if suggestions.is_empty() || area.height < 8 {
        return;
    }
    let max_rows = 8usize.min(area.height as usize / 2);
    let selected = state.completion_index;
    let offset = selected
        .map(|s| scroll_window(s, max_rows, suggestions.len()))
        .unwrap_or(0);
    let shown: Vec<_> = suggestions.iter().skip(offset).take(max_rows).collect();
    let label_w = shown
        .iter()
        .map(|s| display_width(&s.label) + 1 + display_width(&s.hint))
        .max()
        .unwrap_or(8)
        .min(40);
    let desc_w = shown
        .iter()
        .map(|s| display_width(&s.desc))
        .max()
        .unwrap_or(0)
        .min(48);
    // Path candidates (no hint, no description) get a type badge.
    let paths = shown.iter().all(|s| s.hint.is_empty() && s.desc.is_empty());
    let badge_w: u16 = if paths { 4 } else { 0 };
    let width = ((label_w + desc_w + 6) as u16 + badge_w).clamp(20, area.width.saturating_sub(4));
    let height = shown.len() as u16 + 2;
    // The address bar lives in the path bar (row 1): drop down below it.
    let rect = if state.address_bar {
        Rect::new(area.x + 6, area.y + 2, width, height)
    } else {
        Rect::new(
            area.x + 2,
            area.bottom().saturating_sub(height + 1),
            width,
            height,
        )
    };
    drop_shadow(frame.buffer_mut(), rect);
    frame.render_widget(Clear, rect);
    let more = if suggestions.len() > max_rows {
        format!("{} more", suggestions.len() - max_rows)
    } else {
        String::new()
    };
    let title = if more.is_empty() {
        "Tab completes".to_string()
    } else {
        format!("Tab completes · {more}")
    };
    let block = overlay_block(&title, Style::default().fg(BORDER_STRONG));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let typed = {
        let input = state.command_input.trim_start();
        match input.rfind(['/', ' ']) {
            Some(i) => input[i + 1..].to_string(),
            None => input.to_string(),
        }
    };
    let buf = frame.buffer_mut();
    for (row, s) in shown.iter().enumerate() {
        let y = inner.y + row as u16;
        let is_sel = selected == Some(offset + row);
        let bg = if is_sel { ACCENT } else { SURFACE_3 };
        fill(buf, Rect::new(inner.x, y, inner.width, 1), bg);
        let mut x = inner.x + 1;
        if paths {
            let name = s.label.trim_end_matches('/');
            let fake = DirEntry::synthetic(std::path::Path::new("/"), name, s.label.ends_with('/'));
            let (badge, hue) = entry::badge(&fake, false);
            put(
                buf,
                x,
                y,
                &badge,
                3,
                Style::default()
                    .fg(if is_sel { INK_ON_ACCENT } else { hue })
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            );
            x += badge_w;
        }
        let used = put(
            buf,
            x,
            y,
            &s.label,
            label_w as u16,
            Style::default()
                .fg(if is_sel { INK_ON_ACCENT } else { TEXT_PRIMARY })
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );
        if !is_sel && s.label.starts_with(&typed) && !typed.is_empty() {
            put(
                buf,
                x,
                y,
                &typed,
                display_width(&typed) as u16,
                Style::default()
                    .fg(ACCENT)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            );
        }
        if !s.hint.is_empty() {
            put(
                buf,
                x + used + 1,
                y,
                &s.hint,
                (label_w as u16).saturating_sub(used + 1),
                Style::default()
                    .fg(if is_sel { INK_ON_ACCENT } else { ACCENT_SOFT })
                    .bg(bg),
            );
        }
        let dx = x + label_w as u16 + 2;
        put(
            buf,
            dx,
            y,
            &truncate(&s.desc, inner.right().saturating_sub(dx + 1) as usize),
            inner.right().saturating_sub(dx + 1),
            Style::default()
                .fg(if is_sel { INK_ON_ACCENT } else { TEXT_MUTED })
                .bg(bg),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_keys_expands_tokens() {
        assert_eq!(pretty_keys("<C-b>"), "Ctrl-b");
        assert_eq!(pretty_keys("gg"), "gg");
        assert_eq!(pretty_keys("<Space>"), "Space");
        assert_eq!(pretty_keys("j / <Down>"), "j / Down");
    }

    #[test]
    fn help_filter_keeps_matching_rows() {
        let all = help_lines("");
        let some = help_lines("bookmark");
        assert!(some.len() < all.len());
        assert!(
            some.iter()
                .any(|l| matches!(l, HelpLine::Row(_, d) if d.contains("bookmark")))
        );
    }

    #[test]
    fn scroll_window_keeps_selection_visible() {
        assert_eq!(scroll_window(0, 5, 3), 0);
        assert_eq!(scroll_window(9, 5, 10), 5);
        assert_eq!(scroll_window(4, 5, 10), 2);
    }
}
