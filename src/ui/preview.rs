//! Preview: the side panel (badge, name, metadata, framed content) and the
//! bare content renderer reused by the third Miller column.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::state::{AppState, PreviewContent};
use crate::filesystem::{DirEntry, EntryKind};
use crate::ui::entry;
use crate::ui::format::{
    display_width, format_mode, format_size, format_time, kind_label, relative_time, truncate,
};
use crate::ui::glyphs::g;
use crate::ui::hit::HitTarget;
use crate::ui::palette::*;
use crate::ui::theme::hue_for;
use crate::ui::{fill, put};

/// Side panel: identity header, metadata table, then framed content.
pub fn render_panel(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_1);
    for y in area.top()..area.bottom() {
        put(
            frame.buffer_mut(),
            area.x,
            y,
            g().v_line,
            1,
            Style::default().fg(BORDER_SUBTLE).bg(SURFACE_1),
        );
    }
    let inner = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(3),
        area.height,
    );
    let Some(view) = state.browser.focused().cloned() else {
        put(
            frame.buffer_mut(),
            inner.x,
            inner.y + 1,
            "nothing focused",
            inner.width,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_1),
        );
        return;
    };
    let entry = &view.entry;
    let width = inner.width;
    let mut y = inner.y;

    // Identity: badge + bold name.
    let (badge, hue) = entry::badge(entry, false);
    let buf = frame.buffer_mut();
    put(
        buf,
        inner.x,
        y,
        &badge,
        3,
        Style::default()
            .fg(INK_ON_ACCENT)
            .bg(hue)
            .add_modifier(Modifier::BOLD),
    );
    put(
        buf,
        inner.x + 4,
        y,
        &truncate(&entry.display_name(), width.saturating_sub(4) as usize),
        width.saturating_sub(4),
        Style::default()
            .fg(TEXT_PRIMARY)
            .bg(SURFACE_1)
            .add_modifier(Modifier::BOLD),
    );
    y += 2;

    let mut rows: Vec<(&str, String)> = vec![("Type:", type_line(entry))];
    if !entry.is_dir_like() {
        rows.push((
            "Size:",
            format!("{} ({} B)", format_size(entry.size), entry.size),
        ));
    } else if let Some(n) = state.browser.child_counts.get(&entry.path) {
        rows.push(("Items:", n.to_string()));
    }
    rows.push((
        "Modified:",
        format!(
            "{} {}",
            format_time(entry.modified),
            relative_time(entry.modified, state.wall_clock)
        ),
    ));
    rows.push(("Perms:", format_mode(&entry.kind, entry.mode)));
    if let Some(target) = &entry.link_target {
        rows.push(("Target:", target.display().to_string()));
    }
    let tag_row = rows.len();
    rows.push((
        "Tags:",
        if view.tags.is_empty() {
            "(none)".to_string()
        } else {
            view.tags
                .iter()
                .map(|t| format!("[{t}]"))
                .collect::<Vec<_>>()
                .join(" ")
        },
    ));
    let label_w = 10u16;
    for (i, (label, value)) in rows.iter().enumerate() {
        if y >= inner.bottom() {
            break;
        }
        let buf = frame.buffer_mut();
        put(
            buf,
            inner.x,
            y,
            label,
            label_w,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_1),
        );
        let style = if i == tag_row {
            Style::default().fg(ACCENT_SOFT).bg(SURFACE_1)
        } else {
            Style::default().fg(TEXT_SECONDARY).bg(SURFACE_1)
        };
        put(
            buf,
            inner.x + label_w,
            y,
            &truncate(value, width.saturating_sub(label_w) as usize),
            width.saturating_sub(label_w),
            style,
        );
        if i == tag_row {
            state
                .hit_map
                .push(Rect::new(inner.x, y, width, 1), HitTarget::TagBadge);
        }
        y += 1;
    }
    y += 1;
    let content = Rect::new(inner.x, y, inner.width, inner.bottom().saturating_sub(y));
    if content.width < 4 || content.height < 3 {
        return;
    }
    let title = match &state.preview.content {
        Some(PreviewContent::Image(_)) => format!(" Preview · {:?} ", state.picker.protocol_type()),
        _ => " Preview ".to_string(),
    };
    frame.render_widget(Clear, content);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(g().rounded)
        .border_style(Style::default().fg(BORDER_SUBTLE).bg(SURFACE_0))
        .style(Style::default().bg(SURFACE_0))
        .title(ratatui::text::Span::styled(
            title,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_0),
        ));
    let content_inner = block.inner(content);
    frame.render_widget(block, content);
    render_content(frame, content_inner, state, true);
}

fn type_line(entry: &DirEntry) -> String {
    let kind = kind_label(&entry.kind).to_string();
    if entry.kind == EntryKind::File {
        let ext = entry
            .display_name()
            .rsplit_once('.')
            .filter(|(base, _)| !base.is_empty())
            .map(|(_, ext)| ext.to_uppercase());
        return match ext {
            Some(ext) => format!("{kind} {} {ext}", g().dot),
            None if entry.executable => format!("{kind} {} executable", g().dot),
            None => kind,
        };
    }
    if entry.link_dir {
        return format!("{kind} {} folder", g().dot);
    }
    kind
}

/// Draws the focused entry's preview content into `area`.
pub fn render_content(frame: &mut Frame, area: Rect, state: &mut AppState, numbered: bool) {
    render_scrolled(frame, area, state, numbered, 0);
}

/// Like `render_content`, starting `scroll` lines down. Returns the
/// largest useful scroll offset for the content.
pub fn render_scrolled(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    numbered: bool,
    scroll: usize,
) -> usize {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    let bg = SURFACE_0;
    fill(frame.buffer_mut(), area, bg);
    let mut encoding_result = None;
    let mut max_scroll = 0;
    match &mut state.preview.content {
        Some(PreviewContent::Text { lines, truncated }) => {
            max_scroll =
                (lines.len() + usize::from(*truncated)).saturating_sub(area.height as usize);
            let scroll = scroll.min(max_scroll);
            let gutter = if numbered && area.width > 12 {
                (lines.len().max(1).to_string().len() + 1) as u16
            } else {
                0
            };
            let text_w = area.width.saturating_sub(gutter + u16::from(gutter > 0));
            let buf = frame.buffer_mut();
            let mut y = area.y;
            for (i, line) in lines.iter().enumerate().skip(scroll) {
                if y >= area.bottom() {
                    break;
                }
                if gutter > 0 {
                    let num = format!("{:>w$} ", i + 1, w = gutter as usize - 1);
                    put(
                        buf,
                        area.x,
                        y,
                        &num,
                        gutter,
                        Style::default().fg(BORDER_STRONG).bg(bg),
                    );
                }
                let x = area.x + gutter + u16::from(gutter > 0);
                draw_text_line(buf, x, y, line, text_w);
                y += 1;
            }
            if *truncated && y < area.bottom() {
                put(
                    buf,
                    area.x,
                    y,
                    &format!("{} truncated", g().ellipsis),
                    area.width,
                    Style::default().fg(TEXT_MUTED).bg(bg),
                );
            }
        }
        Some(PreviewContent::Directory(names)) => {
            if names.is_empty() {
                put(
                    frame.buffer_mut(),
                    area.x,
                    area.y,
                    "empty folder",
                    area.width,
                    Style::default()
                        .fg(TEXT_MUTED)
                        .bg(bg)
                        .add_modifier(Modifier::ITALIC),
                );
            }
            max_scroll = names.len().saturating_sub(area.height as usize);
            let scroll = scroll.min(max_scroll);
            let buf = frame.buffer_mut();
            for (i, name) in names
                .iter()
                .skip(scroll)
                .take(area.height as usize)
                .enumerate()
            {
                let y = area.y + i as u16;
                let is_dir = name.ends_with('/');
                let fake = DirEntry::synthetic(
                    std::path::Path::new("/"),
                    name.trim_end_matches('/'),
                    is_dir,
                );
                let kind = entry::icon_kind(&fake, false);
                let (badge, _) = entry::badge(&fake, false);
                put(
                    buf,
                    area.x,
                    y,
                    &badge,
                    3,
                    Style::default()
                        .fg(hue_for(kind))
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                );
                let style = if is_dir {
                    Style::default()
                        .fg(TEXT_PRIMARY)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(TEXT_SECONDARY).bg(bg)
                };
                put(
                    buf,
                    area.x + 4,
                    y,
                    &truncate(name, area.width.saturating_sub(4) as usize),
                    area.width.saturating_sub(4),
                    style,
                );
            }
        }
        Some(PreviewContent::Image(proto)) => {
            frame.render_stateful_widget(ratatui_image::StatefulImage::new(), area, proto.as_mut());
            encoding_result = proto.last_encoding_result();
        }
        Some(PreviewContent::Unavailable(message)) => {
            let msg = truncate(message, area.width as usize);
            put(
                frame.buffer_mut(),
                area.x,
                area.y,
                &msg,
                area.width,
                Style::default()
                    .fg(TEXT_MUTED)
                    .bg(bg)
                    .add_modifier(Modifier::ITALIC),
            );
        }
        None => {
            let spin = g().spinner[state.anim.tick(80) % g().spinner.len()];
            put(
                frame.buffer_mut(),
                area.x,
                area.y,
                &format!("{spin} loading"),
                area.width,
                Style::default().fg(ACCENT).bg(bg),
            );
            state.anim.keep_alive();
        }
    }
    apply_image_encoding_result(state, encoding_result);
    max_scroll
}

/// Draws one preview text line, underlining URLs so they stand out.
fn draw_text_line(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, line: &str, width: u16) {
    let shown = truncate(line, width as usize);
    let base = Style::default().fg(TEXT_SECONDARY).bg(SURFACE_0);
    put(buf, x, y, &shown, width, base);
    for (start, end) in crate::urls::find_urls(&shown) {
        let col = display_width(&shown[..start]) as u16;
        let url = &shown[start..end];
        put(
            buf,
            x + col,
            y,
            url,
            display_width(url) as u16,
            Style::default()
                .fg(HUE_CODE)
                .bg(SURFACE_0)
                .add_modifier(Modifier::UNDERLINED),
        );
    }
}

pub(crate) fn apply_image_encoding_result(
    state: &mut AppState,
    result: Option<std::result::Result<(), ratatui_image::errors::Errors>>,
) {
    if let Some(Err(error)) = result {
        state.preview.content = Some(PreviewContent::Unavailable(format!(
            "image preview failed: {error}"
        )));
    }
}
