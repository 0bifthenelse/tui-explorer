//! Reusable interactive controls: bordered buttons and media seek-rail
//! geometry shared by the renderer and the mouse hit-resolution path.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::ui::hit::{HitMap, HitTarget};
use crate::ui::palette::{
    ACCENT, ACCENT_HOVER, BORDER_SUBTLE, DANGER, SURFACE_2, SURFACE_3, TEXT_MUTED, TEXT_PRIMARY,
};

/// Same ASCII border set every modal in [`crate::ui`] uses.
const ASCII_BORDERS: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// Visual state of a [`Button`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    /// Resting appearance.
    Idle,
    /// Pointer rests on the button.
    Hovered,
    /// Pressed / toggled-on appearance (e.g. PAUSE while held).
    Active,
    /// Unavailable: rendered muted and registers NO mouse hit.
    Disabled,
}

/// A bordered, labelled clickable region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub rect: Rect,
    pub label: String,
    pub target: HitTarget,
    /// Destructive action: label foreground becomes DANGER (never colour
    /// alone — wording plus confirm dialogs carry the meaning).
    pub danger: bool,
    pub state: ButtonState,
}

impl Button {
    pub fn new(rect: Rect, label: impl Into<String>, target: HitTarget) -> Self {
        Self {
            rect,
            label: label.into(),
            target,
            danger: false,
            state: ButtonState::Idle,
        }
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn with_state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }
}

/// Draw one button and register its hit region (unless disabled).
pub fn draw_button(frame: &mut ratatui::Frame, btn: &Button, hits: &mut HitMap) {
    if btn.rect.width == 0 || btn.rect.height == 0 {
        return;
    }

    let (bg, border_fg, label_fg, bold) = match btn.state {
        ButtonState::Idle => (SURFACE_2, BORDER_SUBTLE, TEXT_PRIMARY, false),
        ButtonState::Hovered => (SURFACE_3, ACCENT_HOVER, TEXT_PRIMARY, false),
        ButtonState::Active => (ACCENT, ACCENT_HOVER, Color::Black, true),
        ButtonState::Disabled => (SURFACE_2, BORDER_SUBTLE, TEXT_MUTED, false),
    };
    let label_fg = if btn.danger && btn.state != ButtonState::Disabled {
        DANGER
    } else {
        label_fg
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(Style::default().fg(border_fg))
        .style(Style::default().bg(bg));
    let inner = block.inner(btn.rect);
    frame.render_widget(block, btn.rect);

    if inner.width > 0 && inner.height > 0 {
        let mut style = Style::default().fg(label_fg);
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        let line = Line::from(Span::styled(btn.label.as_str(), style)).centered();
        let y = inner.y + (inner.height.saturating_sub(1)) / 2;
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
    }

    if btn.state != ButtonState::Disabled {
        hits.push(btn.rect, btn.target);
    }
}

/// Lay out `(label, width)` specs left-to-right with 1-cell gaps inside
/// `max_w` columns. Stops at the first spec that does not fit (overflow is
/// dropped along with everything after it).
pub fn button_row(x: u16, y: u16, max_w: u16, specs: &[(&str, u16)]) -> Vec<Rect> {
    let mut rects = Vec::with_capacity(specs.len());
    let mut cursor = x as u32;
    let limit = x as u32 + max_w as u32;
    for &(_, w) in specs {
        let gap = u32::from(!rects.is_empty());
        let start = cursor + gap;
        if start + w as u32 > limit {
            break;
        }
        rects.push(Rect {
            x: start as u16,
            y,
            width: w,
            height: 1,
        });
        cursor = start + w as u32;
    }
    rects
}

/// Geometry snapshot of a seek rail for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailGeom {
    /// Full rail rectangle (single row).
    pub rect: Rect,
    /// `position / duration` clamped to `0.0..=1.0`; `None` when the duration
    /// is unknown or zero.
    pub ratio: Option<f64>,
    /// Column of the progress thumb within the inner track
    /// (`rect.x + 1 ..= rect.x + width - 2`).
    pub thumb_x: u16,
    /// Column under the pointer, filled in by the renderer when hovering.
    pub hover_x: Option<u16>,
}

impl RailGeom {
    /// Seconds represented by column `x` using this rail's exact draw
    /// geometry; endpoints clamp. `None` for zero-width rails.
    pub fn seconds_at(&self, x: u16, duration: f64) -> Option<f64> {
        rail_seconds_at(self.rect, x, duration)
    }
}

/// Compute rail geometry shared by drawing and mouse mapping.
///
/// The track spans the INNER columns of `rect` (one border cell trimmed from
/// each side), matching the reducer's mouse mapping exactly:
/// `inner_left = rect.x + 1`, `inner_width = width - 2`.
pub fn rail_geometry(rect: Rect, position: f64, duration: Option<f64>) -> RailGeom {
    let ratio = duration
        .filter(|d| *d > 0.0)
        .map(|d| (position / d).clamp(0.0, 1.0));
    let thumb_x = match ratio {
        Some(r) => column_for_ratio(rect, r),
        None => rect.x,
    };
    RailGeom {
        rect,
        ratio,
        thumb_x,
        hover_x: None,
    }
}

/// Exact inverse of the [`rail_geometry`] thumb mapping: column `x` back to
/// seconds over `duration`.
pub fn rail_ratio_to_seconds(rail: &RailGeom, x: u16, duration: f64) -> Option<f64> {
    rail_seconds_at(rail.rect, x, duration)
}

fn rail_seconds_at(rect: Rect, x: u16, duration: f64) -> Option<f64> {
    if rect.width == 0 || !duration.is_finite() || duration < 0.0 {
        return None;
    }
    let (inner_left, inner_width) = inner_track(rect);
    let clamped = x.clamp(inner_left, inner_left.saturating_add(inner_width));
    let offset = clamped.saturating_sub(inner_left);
    let ratio = if inner_width == 0 {
        0.0
    } else {
        offset as f64 / inner_width as f64
    };
    Some((ratio * duration).clamp(0.0, duration))
}

/// `(left column, column count)` of the drawable track inside `rect`.
fn inner_track(rect: Rect) -> (u16, u16) {
    (rect.x.saturating_add(1), rect.width.saturating_sub(2))
}

fn column_for_ratio(rect: Rect, ratio: f64) -> u16 {
    if rect.width == 0 {
        return rect.x;
    }
    let (inner_left, inner_width) = inner_track(rect);
    inner_left + (ratio * inner_width as f64).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn button_row_places_rects_with_one_cell_gaps() {
        let rects = button_row(4, 7, 40, &[("A", 5), ("BB", 6)]);
        assert_eq!(
            rects,
            vec![rect(4, 7, 5, 1), rect(10, 7, 6, 1)],
            "second button starts one cell after the first ends"
        );
    }

    #[test]
    fn button_row_drops_overflowing_tail() {
        let rects = button_row(0, 0, 10, &[("A", 5), ("B", 5), ("C", 5)]);
        assert_eq!(
            rects,
            vec![rect(0, 0, 5, 1)],
            "B would end at column 11, past max_w=10; the tail is dropped"
        );

        let wider = button_row(0, 0, 12, &[("A", 5), ("B", 5), ("C", 5)]);
        assert_eq!(
            wider,
            vec![rect(0, 0, 5, 1), rect(6, 0, 5, 1)],
            "C needs columns 12..17, past max_w=12"
        );
    }

    #[test]
    fn button_row_exact_fit_is_kept() {
        let rects = button_row(2, 0, 11, &[("A", 5), ("B", 5)]);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[1].x + rects[1].width, 13);
    }

    #[test]
    fn button_row_zero_width_yields_nothing() {
        assert!(button_row(0, 0, 0, &[("A", 1)]).is_empty());
    }

    #[test]
    fn rail_round_trip_matches_draw_geometry() {
        let area = rect(5, 3, 21, 1);
        // Inner track spans columns 6..=24.
        let duration = 90.0;
        for x in [6, 13, 20, 24] {
            let secs =
                rail_ratio_to_seconds(&rail_geometry(area, 0.0, Some(duration)), x, duration)
                    .expect("rail maps column");
            let geom = rail_geometry(area, secs, Some(duration));
            assert_eq!(geom.thumb_x, x, "draw(mouse({x})) must land back on {x}");
        }
    }

    #[test]
    fn rail_endpoints_and_middle() {
        let area = rect(0, 0, 42, 1);
        let geom = rail_geometry(area, 0.0, Some(100.0));
        assert_eq!(geom.ratio, Some(0.0));
        assert_eq!(
            geom.thumb_x,
            area.x + 1,
            "thumb sits on the inner track start"
        );

        let mid = rail_ratio_to_seconds(&geom, area.x + 21, 100.0);
        assert_eq!(mid, Some(50.0));

        let end_geom = rail_geometry(area, 100.0, Some(100.0));
        assert_eq!(end_geom.ratio, Some(1.0));
        assert_eq!(
            end_geom.thumb_x,
            area.x + 41,
            "thumb reaches the inner track end"
        );
        assert_eq!(
            rail_ratio_to_seconds(&geom, area.x + 41, 100.0),
            Some(100.0)
        );
    }

    #[test]
    fn rail_clamps_position_and_pointer() {
        let area = rect(3, 0, 11, 1);
        let over = rail_geometry(area, 999.0, Some(10.0));
        assert_eq!(over.ratio, Some(1.0));
        let under = rail_geometry(area, -5.0, Some(10.0));
        assert_eq!(under.ratio, Some(0.0));

        let geom = rail_geometry(area, 0.0, Some(10.0));
        // Pointer on a border cell or beyond the rail clamps to the nearest
        // inner endpoint (track spans columns 4..=12).
        assert_eq!(rail_ratio_to_seconds(&geom, area.x - 2, 10.0), Some(0.0));
        assert_eq!(
            rail_ratio_to_seconds(&geom, area.x + area.width + 3, 10.0),
            Some(10.0)
        );
    }

    #[test]
    fn rail_unknown_duration_has_no_ratio() {
        let area = rect(2, 2, 16, 1);
        let geom = rail_geometry(area, 4.2, None);
        assert_eq!(geom.ratio, None);
        assert_eq!(geom.thumb_x, area.x);
        assert_eq!(rail_ratio_to_seconds(&geom, area.x + 8, 60.0), Some(30.0));
    }

    #[test]
    fn rail_zero_duration_treated_as_unknown() {
        let geom = rail_geometry(rect(0, 0, 10, 1), 1.0, Some(0.0));
        assert_eq!(geom.ratio, None);
    }

    #[test]
    fn rail_zero_width_maps_nothing() {
        let area = rect(3, 3, 0, 1);
        let geom = rail_geometry(area, 1.0, Some(10.0));
        assert_eq!(rail_ratio_to_seconds(&geom, 3, 10.0), None);
    }

    fn render_button(state: ButtonState, danger: bool) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(7, 3);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        let mut hits = HitMap::default();
        let btn = Button {
            rect: rect(0, 0, 7, 3),
            label: "OK".into(),
            target: HitTarget::MediaStop,
            danger,
            state,
        };
        terminal
            .draw(|frame| draw_button(frame, &btn, &mut hits))
            .expect("draw");
        if state != ButtonState::Disabled {
            assert_eq!(hits.regions, vec![(rect(0, 0, 7, 3), HitTarget::MediaStop)]);
        } else {
            assert!(hits.regions.is_empty(), "disabled registers NO hit");
        }
        terminal.backend().buffer().clone()
    }

    #[test]
    fn draw_button_state_colors() {
        let idle = render_button(ButtonState::Idle, false);
        assert_eq!(idle[(0, 0)].fg, BORDER_SUBTLE);
        assert_eq!(idle[(1, 1)].bg, SURFACE_2);
        assert_eq!(idle[(2, 1)].fg, TEXT_PRIMARY);
        assert_eq!(idle[(2, 1)].symbol(), "O");

        let hovered = render_button(ButtonState::Hovered, false);
        assert_eq!(hovered[(0, 0)].fg, ACCENT_HOVER);
        assert_eq!(hovered[(1, 1)].bg, SURFACE_3);

        let active = render_button(ButtonState::Active, false);
        assert_eq!(active[(1, 1)].bg, ACCENT);
        assert_eq!(active[(2, 1)].fg, Color::Black);
        assert!(active[(2, 1)].modifier.contains(Modifier::BOLD));

        let disabled = render_button(ButtonState::Disabled, false);
        assert_eq!(disabled[(2, 1)].fg, TEXT_MUTED);
        assert_eq!(disabled[(1, 1)].bg, SURFACE_2);
    }

    #[test]
    fn draw_button_danger_label_uses_danger_color() {
        let buf = render_button(ButtonState::Idle, true);
        assert_eq!(buf[(2, 1)].fg, DANGER);
        // Border styling stays state-driven.
        assert_eq!(buf[(0, 0)].fg, BORDER_SUBTLE);

        // Disabled wins: muted label even when marked dangerous.
        let off = render_button(ButtonState::Disabled, true);
        assert_eq!(off[(2, 1)].fg, TEXT_MUTED);
    }
}
