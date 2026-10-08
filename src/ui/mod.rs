//! Rendering. `render` lays out the shell for the current size tier, draws
//! the active layout (list, grid, or Miller columns), then overlays, and
//! finally applies whole-frame effects (scrim, 256-color fallback).
//!
//! The renderer reads `AppState` directly and writes back exactly what the
//! reducer needs to interpret input: the hit map, the grid geometry, the
//! sidebar/parent row lists, and the media surface.

pub mod anim;
pub mod format;
pub mod glyphs;
pub mod hit;
pub mod palette;
pub mod theme;
pub mod widgets;

mod chrome;
mod columns;
mod entry;
mod grid;
mod list;
mod media;
pub(crate) mod modals;
pub(crate) mod overlays;
mod preview;
mod side;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::state::{AppState, DragPhase, MarqueePhase, Mode};
use crate::settings::ViewMode;
use crate::ui::glyphs::g;
use crate::ui::hit::HitTarget;
use crate::ui::palette::*;
use crate::ui::theme::{dim_area, mix};

pub(crate) const SIDEBAR_WIDTH: u16 = 26;
pub(crate) const PREVIEW_WIDTH: u16 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    TooSmall,
    Narrow,
    Compact,
    Standard,
    Wide,
}

pub fn tier_for(width: u16, height: u16) -> Tier {
    if width < 24 || height < 6 {
        Tier::TooSmall
    } else if width < 70 || height < 12 {
        Tier::Narrow
    } else if width < 100 || height < 20 {
        Tier::Compact
    } else if width < 130 || height < 28 {
        Tier::Standard
    } else {
        Tier::Wide
    }
}

/// Sidebar visibility: the tier sets the default, the user's toggle can
/// override it except at `Narrow`/`TooSmall`, where there is no room.
pub fn sidebar_visible(width: u16, height: u16, override_: Option<bool>) -> bool {
    match tier_for(width, height) {
        Tier::TooSmall | Tier::Narrow => false,
        Tier::Compact => override_.unwrap_or(false),
        Tier::Standard | Tier::Wide => override_.unwrap_or(true),
    }
}

/// Preview-panel visibility, same override rule as `sidebar_visible`.
/// `Narrow`/`TooSmall` never show it; `Wide` is the only tier where it
/// auto-shows.
pub fn preview_visible(width: u16, height: u16, override_: Option<bool>) -> bool {
    match tier_for(width, height) {
        Tier::TooSmall | Tier::Narrow => false,
        Tier::Compact | Tier::Standard => override_.unwrap_or(false),
        Tier::Wide => override_.unwrap_or(true),
    }
}

/// Whether the focused entry's preview content is shown anywhere right
/// now: the side panel, or the third Miller column.
pub fn preview_needed(state: &AppState) -> bool {
    match state.view() {
        ViewMode::Columns => !matches!(
            tier_for(state.width, state.height),
            Tier::TooSmall | Tier::Narrow
        ),
        _ => preview_visible(state.width, state.height, state.show_preview),
    }
}

// --- Shared styles -------------------------------------------------------

pub(crate) fn base_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

pub(crate) fn dir_style() -> Style {
    Style::default()
        .fg(TEXT_PRIMARY)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn focused_style() -> Style {
    Style::default().bg(FOCUS_BG).fg(TEXT_PRIMARY)
}

pub(crate) fn accent_border_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub(crate) fn tag_style() -> Style {
    Style::default().fg(ACCENT_SOFT)
}

/// Errors are never color-only: `DANGER` pairs with bold and a literal
/// `[!]` prefix wherever this style renders a message.
pub(crate) fn error_style() -> Style {
    Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
}

pub(crate) fn muted_style() -> Style {
    Style::default().fg(TEXT_MUTED)
}

pub(crate) fn preview_meta_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

pub(crate) fn surface_fill(frame: &mut Frame, area: Rect, color: Color) {
    fill(frame.buffer_mut(), area, color);
}

/// Paints `area` with a flat background, keeping nothing underneath.
pub(crate) fn fill(buf: &mut Buffer, area: Rect, color: Color) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.bg = color;
            cell.fg = TEXT_SECONDARY;
            cell.modifier = Modifier::empty();
        }
    }
}

/// Writes `text` at (x, y) clipped to `max` cells; returns the cells used.
pub(crate) fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16, style: Style) -> u16 {
    if max == 0 || y >= buf.area.bottom() || x >= buf.area.right() {
        return 0;
    }
    let max = max.min(buf.area.right() - x);
    let (end, _) = buf.set_stringn(x, y, text, max as usize, style);
    end.saturating_sub(x)
}

/// Standard raised-surface frame used by every overlay: rounded border,
/// `SURFACE_3` interior, and a title chip.
pub(crate) fn overlay_block(title: &str, accent: Style) -> Block<'static> {
    let tone = accent.fg.unwrap_or(ACCENT);
    Block::default()
        .borders(Borders::ALL)
        .border_set(g().rounded)
        .border_style(
            Style::default()
                .fg(mix(BORDER_STRONG, tone, 0.35))
                .bg(SURFACE_3),
        )
        .style(Style::default().bg(SURFACE_3))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(INK_ON_ACCENT)
                .bg(tone)
                .add_modifier(Modifier::BOLD),
        ))
}

/// Soft drop shadow to the right of and below `rect`.
pub(crate) fn drop_shadow(buf: &mut Buffer, rect: Rect) {
    let right = Rect::new(rect.right(), rect.y + 1, 2, rect.height.saturating_sub(1));
    let below = Rect::new(rect.x + 2, rect.bottom(), rect.width, 1);
    dim_area(buf, right, ROOT_INK, 0.55);
    dim_area(buf, below, ROOT_INK, 0.55);
}

// --- Frame ---------------------------------------------------------------

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();
    state.width = area.width;
    state.height = area.height;
    state.hit_map.clear();
    let now = state.now;
    state.anim.begin_frame(now);
    fill(frame.buffer_mut(), area, SURFACE_0);

    let tier = tier_for(area.width, area.height);
    match tier {
        Tier::TooSmall => {
            render_too_small(frame, area);
            state.grid_cols = 1;
            state.list_viewport = 1;
            state.sidebar_items.clear();
            finish_frame(frame, state);
            return;
        }
        Tier::Narrow => chrome::render_narrow_shell(frame, area, state),
        Tier::Compact | Tier::Standard | Tier::Wide => chrome::render_shell(frame, area, state),
    }

    // Overlays sit above a scrim that eases in as they open.
    if state.mode.is_overlay() {
        let fullscreen_video =
            matches!(&state.mode, Mode::Media(m) if m.fullscreen && m.backend.in_terminal());
        if !fullscreen_video {
            let t = state
                .anim
                .enter(anim::AnimKey::Named("scrim"), 0.0, 1.0, anim::MODAL);
            let amount = if matches!(state.mode, Mode::ContextMenu(_)) {
                0.18
            } else {
                0.42
            };
            dim_area(frame.buffer_mut(), area, ROOT_INK, amount * t);
        }
    }
    modals::render_modal(frame, area, state);
    overlays::render_which_key(frame, area, state);
    overlays::render_suggestions(frame, area, state);

    if let Some(drag) = &state.drag
        && drag.phase == DragPhase::Dragging
    {
        chrome::render_drag_feedback(frame, area, state);
    }
    if let Some(marquee) = &state.marquee
        && marquee.phase == MarqueePhase::Selecting
    {
        outline_rect(frame, marquee.rect(), Style::default().fg(ACCENT_SOFT));
    }
    finish_frame(frame, state);
}

fn finish_frame(frame: &mut Frame, state: &mut AppState) {
    state.anim.end_frame();
    state.anim.settle();
    if !state.truecolor {
        theme::downsample_256(frame.buffer_mut());
    }
}

pub(crate) fn outline_rect(frame: &mut Frame, rect: Rect, style: Style) {
    let rect = rect.intersection(frame.area());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(g().rounded)
        .border_style(style);
    frame.render_widget(block, rect);
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("resize terminal", error_style())),
        Line::from(Span::styled("24x6 minimum", muted_style())),
    ];
    let height = lines.len() as u16;
    let top = area.y + area.height.saturating_sub(height) / 2;
    let rect = Rect::new(area.x, top, area.width, height.min(area.height));
    frame.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        rect,
    );
}

/// Hover intensity (0..1, animated) for an interactive control.
pub(crate) fn control_glow(state: &mut AppState, target: HitTarget) -> f32 {
    let hovered = state.hover.control == Some(target);
    state.anim.track_asym(
        anim::AnimKey::Control(target),
        if hovered { 1.0 } else { 0.0 },
        anim::HOVER_IN,
        anim::HOVER_OUT,
    )
}

/// Background for a hoverable chrome element over `base`.
pub(crate) fn hover_bg(base: Color, glow: f32) -> Color {
    mix(base, mix(base, ACCENT, 0.28), glow)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::preview::apply_image_encoding_result;
    use crate::app::state::{AppState, PreviewContent};

    #[test]
    fn image_encoding_failure_replaces_preview_on_next_frame() {
        let mut state = AppState::new(PathBuf::from("/"), PathBuf::from("/"));
        state.preview.content = Some(PreviewContent::Text {
            lines: vec!["old preview".to_string()],
            truncated: false,
        });
        apply_image_encoding_result(
            &mut state,
            Some(Err(ratatui_image::errors::Errors::Sixel(
                "encoder stopped".to_string(),
            ))),
        );
        assert!(matches!(
            &state.preview.content,
            Some(PreviewContent::Unavailable(message))
                if message == "image preview failed: Sixel error: encoder stopped"
        ));
    }

    #[test]
    fn successful_image_encoding_keeps_preview_content() {
        let mut state = AppState::new(PathBuf::from("/"), PathBuf::from("/"));
        state.preview.content = Some(PreviewContent::Text {
            lines: vec!["current preview".to_string()],
            truncated: false,
        });
        apply_image_encoding_result(&mut state, Some(Ok(())));
        assert!(matches!(
            &state.preview.content,
            Some(PreviewContent::Text { .. })
        ));
    }
}
