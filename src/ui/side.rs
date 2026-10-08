//! Sidebar: places, bookmarks, tags and devices. Every row is clickable
//! (and a drop target), hover fades in, and the row matching the current
//! directory carries the accent rail.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::app::state::AppState;
use crate::sidebar::{self, SidebarItem};
use crate::ui::anim::{AnimKey, HOVER_IN, HOVER_OUT};
use crate::ui::format::{display_width, truncate};
use crate::ui::glyphs::{Charset, charset, g};
use crate::ui::hit::HitTarget;
use crate::ui::palette::*;
use crate::ui::theme::mix;
use crate::ui::{fill, put};

const TAG_HUES: [Color; 8] = [
    HUE_CODE,
    HUE_IMAGE,
    HUE_ARCHIVE,
    HUE_EXEC,
    HUE_AUDIO,
    HUE_LINK,
    HUE_VIDEO,
    HUE_DATA,
];

pub fn tag_hue(name: &str) -> Color {
    let sum: u32 = name.bytes().map(u32::from).sum();
    TAG_HUES[(sum as usize) % TAG_HUES.len()]
}

fn place_glyph(label: &str) -> &'static str {
    let ascii = charset() == Charset::Ascii;
    match (label, ascii) {
        ("Home", false) => "⌂",
        ("Root", false) => "/",
        ("Desktop", false) => "▣",
        ("Documents", false) => "≡",
        ("Downloads", false) => "↓",
        ("Music", false) => "♪",
        ("Pictures", false) => "◩",
        ("Videos", false) => "▶",
        ("Home", true) => "~",
        ("Root", true) => "/",
        (_, true) => ">",
        _ => "•",
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_2);
    for y in area.top()..area.bottom() {
        put(
            frame.buffer_mut(),
            area.right() - 1,
            y,
            g().v_line,
            1,
            Style::default().fg(BORDER_SUBTLE).bg(SURFACE_2),
        );
    }
    let inner = Rect::new(area.x, area.y, area.width - 1, area.height);
    let sections = sidebar::build_sections(state);
    let mut flat: Vec<SidebarItem> = Vec::new();
    let mut y = inner.y;
    let cwd = state.browser.cwd.clone();
    let bottom = inner.bottom();

    let groups: [(&str, &[SidebarItem], &str); 5] = [
        ("PLACES", &sections.places, "(none)"),
        (
            "BOOKMARKS",
            &sections.bookmarks,
            "Ctrl-b bookmarks this folder",
        ),
        ("LINKS", &sections.links, ""),
        ("TAGS", &sections.tags, "T creates a tag"),
        ("DEVICES", &sections.mounts, "no device mounts"),
    ];
    for (title, items, empty) in groups {
        if y >= bottom {
            break;
        }
        if items.is_empty() && empty.is_empty() {
            continue;
        }
        let buf = frame.buffer_mut();
        put(
            buf,
            inner.x + 1,
            y,
            title,
            inner.width.saturating_sub(2),
            Style::default()
                .fg(TEXT_MUTED)
                .bg(SURFACE_2)
                .add_modifier(Modifier::BOLD),
        );
        if !items.is_empty() {
            let count = items.len().to_string();
            let cw = display_width(&count) as u16;
            put(
                buf,
                inner.right().saturating_sub(cw + 1),
                y,
                &count,
                cw,
                Style::default().fg(BORDER_STRONG).bg(SURFACE_2),
            );
        }
        y += 1;
        if items.is_empty() {
            if y < bottom {
                put(
                    frame.buffer_mut(),
                    inner.x + 2,
                    y,
                    &truncate(empty, inner.width.saturating_sub(3) as usize),
                    inner.width.saturating_sub(3),
                    Style::default()
                        .fg(BORDER_STRONG)
                        .bg(SURFACE_2)
                        .add_modifier(Modifier::ITALIC),
                );
                y += 1;
            }
        } else {
            for item in items {
                if y >= bottom {
                    break;
                }
                let idx = flat.len();
                let rect = Rect::new(inner.x, y, inner.width, 1);
                draw_item(frame, rect, state, item, idx, &cwd);
                state.hit_map.push(rect, HitTarget::Sidebar(idx));
                flat.push(item.clone());
                y += 1;
            }
        }
        y += 1; // breathing room between sections
    }
    state.sidebar_items = flat;
}

fn draw_item(
    frame: &mut Frame,
    rect: Rect,
    state: &mut AppState,
    item: &SidebarItem,
    idx: usize,
    cwd: &std::path::Path,
) {
    let hovered = state.hover.control == Some(HitTarget::Sidebar(idx));
    let glow = state.anim.track_asym(
        AnimKey::Indexed("side", idx as u32),
        if hovered { 1.0 } else { 0.0 },
        HOVER_IN,
        HOVER_OUT,
    );
    let current = match item {
        SidebarItem::Place { path, .. }
        | SidebarItem::Mount { path, .. }
        | SidebarItem::Bookmark { path } => path == cwd,
        SidebarItem::Tag { .. } | SidebarItem::Link { .. } => false,
    };
    let base = if current {
        mix(SURFACE_2, ACCENT, 0.16)
    } else {
        SURFACE_2
    };
    let bg = mix(base, mix(SURFACE_2, ACCENT, 0.26), glow);
    let buf = frame.buffer_mut();
    fill(buf, rect, bg);
    if current {
        put(
            buf,
            rect.x,
            rect.y,
            g().bar,
            1,
            Style::default().fg(ACCENT).bg(bg),
        );
    }
    let text_fg = if current {
        TEXT_PRIMARY
    } else {
        mix(TEXT_SECONDARY, TEXT_PRIMARY, glow)
    };
    let mut label_style = Style::default().fg(text_fg).bg(bg);
    if current {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    let width = rect.width.saturating_sub(4);
    match item {
        SidebarItem::Place { label, .. } => {
            put(
                buf,
                rect.x + 2,
                rect.y,
                place_glyph(label),
                1,
                Style::default().fg(ACCENT).bg(bg),
            );
            put(
                buf,
                rect.x + 4,
                rect.y,
                &truncate(label, width as usize),
                width,
                label_style,
            );
        }
        SidebarItem::Bookmark { path } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            put(
                buf,
                rect.x + 2,
                rect.y,
                g().star,
                1,
                Style::default().fg(ACCENT_AMBER).bg(bg),
            );
            put(
                buf,
                rect.x + 4,
                rect.y,
                &truncate(&name, width as usize),
                width,
                label_style,
            );
        }
        SidebarItem::Link { title, .. } => {
            put(
                buf,
                rect.x + 2,
                rect.y,
                g().link,
                1,
                Style::default().fg(HUE_WEB).bg(bg),
            );
            put(
                buf,
                rect.x + 4,
                rect.y,
                &truncate(title, width as usize),
                width,
                label_style,
            );
        }
        SidebarItem::Tag { name, token } => {
            put(
                buf,
                rect.x + 2,
                rect.y,
                g().thumb,
                1,
                Style::default().fg(tag_hue(name)).bg(bg),
            );
            let token = format!("[{token}]");
            let tw = display_width(&token) as u16;
            let nw = width.saturating_sub(tw + 1);
            let used = put(
                buf,
                rect.x + 4,
                rect.y,
                &truncate(name, nw as usize),
                nw,
                label_style,
            );
            put(
                buf,
                rect.x + 5 + used,
                rect.y,
                &token,
                tw,
                Style::default().fg(TEXT_MUTED).bg(bg),
            );
        }
        SidebarItem::Mount {
            path, used, total, ..
        } => {
            let ratio = if *total == 0 {
                0.0
            } else {
                *used as f64 / *total as f64
            };
            let meter_w = 6u16;
            let label_w = width.saturating_sub(meter_w + 1);
            let label = path.display().to_string();
            put(
                buf,
                rect.x + 2,
                rect.y,
                if charset() == Charset::Ascii {
                    "#"
                } else {
                    "▤"
                },
                1,
                Style::default().fg(HUE_DATA).bg(bg),
            );
            put(
                buf,
                rect.x + 4,
                rect.y,
                &truncate(&label, label_w as usize),
                label_w,
                label_style,
            );
            let mx = rect.right().saturating_sub(meter_w + 1);
            let (filled, rest) = crate::ui::glyphs::meter(meter_w as usize, ratio);
            let color = if ratio > 0.9 {
                DANGER
            } else if ratio > 0.75 {
                ACCENT_AMBER
            } else {
                HUE_EXEC
            };
            let w = put(
                buf,
                mx,
                rect.y,
                &filled,
                meter_w,
                Style::default().fg(color).bg(SURFACE_3),
            );
            put(
                buf,
                mx + w,
                rect.y,
                &" ".repeat(rest),
                rest as u16,
                Style::default().bg(SURFACE_3),
            );
        }
    }
}
