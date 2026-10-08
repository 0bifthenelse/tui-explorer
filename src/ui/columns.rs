//! Miller columns (ranger-style): parent | current | preview at 2:3:4.
//! The parent pane highlights where you came from and is clickable; the
//! current pane is the compact list; the third pane previews the focused
//! entry (folder contents, text, or image).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::state::AppState;
use crate::browser::{EntryView, sort_entries};
use crate::ui::entry;
use crate::ui::format::truncate_middle;
use crate::ui::glyphs::g;
use crate::ui::hit::HitTarget;
use crate::ui::list::ListOpts;
use crate::ui::palette::*;
use crate::ui::theme::mix;
use crate::ui::{fill, put};

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 30 {
        state.parent_rows.clear();
        crate::ui::list::render(
            frame,
            area,
            state,
            ListOpts {
                header: false,
                meta: false,
                channel: 1,
            },
        );
        return;
    }
    let parent_w = (area.width * 2 / 9).max(12);
    let current_w = (area.width * 3 / 9).max(18);
    let preview_w = area.width.saturating_sub(parent_w + current_w);
    let parent = Rect::new(area.x, area.y, parent_w, area.height);
    let current = Rect::new(area.x + parent_w, area.y, current_w, area.height);
    let preview = Rect::new(current.right(), area.y, preview_w, area.height);

    render_parent(frame, parent, state);
    separator(frame, Rect::new(current.x, area.y, 1, area.height));
    crate::ui::list::render(
        frame,
        Rect::new(current.x + 1, current.y, current.width - 1, current.height),
        state,
        ListOpts {
            header: false,
            meta: false,
            channel: 1,
        },
    );
    if preview.width > 4 {
        separator(frame, Rect::new(preview.x, area.y, 1, area.height));
        let inner = Rect::new(preview.x + 2, preview.y, preview.width - 3, preview.height);
        fill(
            frame.buffer_mut(),
            Rect::new(preview.x + 1, preview.y, preview.width - 1, preview.height),
            SURFACE_0,
        );
        if state.browser.focused().is_some() {
            crate::ui::preview::render_content(frame, inner, state, false);
        }
    }
}

fn separator(frame: &mut Frame, rect: Rect) {
    for y in rect.top()..rect.bottom() {
        put(
            frame.buffer_mut(),
            rect.x,
            y,
            g().v_line,
            1,
            Style::default().fg(BORDER_SUBTLE).bg(SURFACE_1),
        );
    }
}

/// Visible parent-directory entries in the active sort order.
fn parent_entries(state: &AppState) -> Option<Vec<EntryView>> {
    let parent = state.browser.cwd.parent()?;
    let mut entries: Vec<EntryView> = state
        .side_listings
        .get(parent)?
        .iter()
        .filter(|e| state.browser.show_hidden || !e.entry.hidden)
        .cloned()
        .collect();
    sort_entries(&mut entries, state.browser.sort_mode);
    Some(entries)
}

fn render_parent(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_2);
    state.parent_rows.clear();
    let Some(entries) = parent_entries(state) else {
        let label = if state.browser.cwd.parent().is_none() {
            "/".to_string()
        } else {
            format!(
                "{} loading",
                g().spinner[state.anim.tick(80) % g().spinner.len()]
            )
        };
        put(
            frame.buffer_mut(),
            area.x + 1,
            area.y,
            &label,
            area.width.saturating_sub(1),
            Style::default().fg(TEXT_MUTED).bg(SURFACE_2),
        );
        if state.browser.cwd.parent().is_some() {
            state.anim.keep_alive();
        }
        return;
    };
    let cwd = state.browser.cwd.clone();
    let current = entries
        .iter()
        .position(|e| e.entry.path == cwd)
        .unwrap_or(0);
    let height = area.height as usize;
    // Keep the current directory roughly centered.
    let scroll = current
        .saturating_sub(height / 2)
        .min(entries.len().saturating_sub(height));
    for (row, view) in entries.iter().enumerate().skip(scroll).take(height) {
        let y = area.y + (row - scroll) as u16;
        let rect = Rect::new(area.x, y, area.width, 1);
        let is_current = view.entry.path == cwd;
        let idx = state.parent_rows.len();
        let hovered = state.hover.control == Some(HitTarget::ParentRow(idx));
        let glow = crate::ui::control_glow(state, HitTarget::ParentRow(idx));
        let _ = hovered;
        let base = if is_current {
            mix(SURFACE_2, ACCENT, 0.22)
        } else {
            SURFACE_2
        };
        let bg = mix(base, mix(SURFACE_2, ACCENT, 0.3), glow);
        let buf = frame.buffer_mut();
        fill(buf, rect, bg);
        if is_current {
            put(
                buf,
                rect.x,
                y,
                g().bar,
                1,
                Style::default().fg(ACCENT).bg(bg),
            );
        }
        let (badge, hue) = entry::badge(&view.entry, is_current);
        put(
            buf,
            rect.x + 1,
            y,
            &badge,
            3,
            Style::default().fg(hue).bg(bg).add_modifier(Modifier::BOLD),
        );
        let mut style = Style::default()
            .fg(if is_current {
                TEXT_PRIMARY
            } else {
                mix(entry::name_color(&view.entry), TEXT_MUTED, 0.25)
            })
            .bg(bg);
        if is_current || view.entry.is_dir_like() {
            style = style.add_modifier(Modifier::BOLD);
        }
        let budget = rect.width.saturating_sub(6);
        put(
            buf,
            rect.x + 5,
            y,
            &truncate_middle(&entry::display_name(view), budget as usize),
            budget,
            style,
        );
        state.hit_map.push(rect, HitTarget::ParentRow(idx));
        state.parent_rows.push(view.entry.path.clone());
    }
}
