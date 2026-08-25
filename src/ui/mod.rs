pub mod format;
pub mod hit;
pub mod palette;
pub mod widgets;

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

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

use crate::app::reduce::{breadcrumb_segments, footer_focus_text};
use crate::app::state::{AppState, BookmarkNavState, Mode, PasswordPurpose, PreviewContent};
use crate::icons::{IconResolver, IconVariant, TILE_ART_HEIGHT, TILE_ART_WIDTH, tile_art};
use crate::sidebar::{self, SidebarItem};
use crate::ui::format::{format_mode, format_size, format_time, kind_label, pad_right, truncate};
use crate::ui::hit::{HitMap, HitTarget, LegendAction};
use crate::ui::palette::{
    ACCENT, ACCENT_HOVER, ACCENT_SOFT, BORDER_STRONG, BORDER_SUBTLE, DANGER, FOCUS_BG, ROOT_INK,
    SELECTED_BG, SURFACE_0, SURFACE_1, SURFACE_2, SURFACE_3, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY,
};
use crate::ui::widgets::{Button, ButtonState, button_row, draw_button, rail_geometry};

/// Tile geometry for the icon grid.
const TILE_W: u16 = TILE_ART_WIDTH as u16 + 2;
const TILE_H: u16 = TILE_ART_HEIGHT as u16 + 2;

const SIDEBAR_WIDTH: u16 = 24;
const PREVIEW_WIDTH: u16 = 36;

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

/// Sidebar visibility: the tier sets the default, the user's `b` toggle can
/// override it except at `Narrow`/`TooSmall`, where there is no room for a
/// sidebar at all. `Compact` never auto-shows it; `Standard`/`Wide` do.
pub fn sidebar_visible(width: u16, height: u16, override_: Option<bool>) -> bool {
    match tier_for(width, height) {
        Tier::TooSmall | Tier::Narrow => false,
        Tier::Compact => override_.unwrap_or(false),
        Tier::Standard | Tier::Wide => override_.unwrap_or(true),
    }
}

/// Preview visibility: same override rule as `sidebar_visible`. `Narrow`
/// and `TooSmall` never show it; `Compact`/`Standard` default to hidden;
/// `Wide` is the only tier where it auto-shows.
pub fn preview_visible(width: u16, height: u16, override_: Option<bool>) -> bool {
    match tier_for(width, height) {
        Tier::TooSmall | Tier::Narrow => false,
        Tier::Compact | Tier::Standard => override_.unwrap_or(false),
        Tier::Wide => override_.unwrap_or(true),
    }
}

/// Body text on a surface: the default readable tone for most labels.
fn base_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

/// Directory names and headline text: bold primary text, per the palette
/// contract (no separate directory hue).
fn dir_style() -> Style {
    Style::default()
        .fg(TEXT_PRIMARY)
        .add_modifier(Modifier::BOLD)
}

/// Focused-but-not-selected tile: warm focus fill plus primary text.
fn focused_style() -> Style {
    Style::default().bg(FOCUS_BG).fg(TEXT_PRIMARY)
}

/// Cursor-only grid tile (spec section 8): bright primary text with no
/// selection fill, so a bare navigation cursor never renders orange.
fn cursor_style() -> Style {
    Style::default().fg(TEXT_PRIMARY)
}

/// Selected tile fill.
fn selected_style() -> Style {
    Style::default()
        .bg(SELECTED_BG)
        .fg(TEXT_PRIMARY)
        .add_modifier(Modifier::BOLD)
}

/// Combined focus+selection: keeps the selected fill and swaps the label to
/// the accent color. The accent tile border itself is drawn separately by
/// `render_grid`.
fn focused_selected_style() -> Style {
    Style::default()
        .bg(SELECTED_BG)
        .fg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

/// The signal-orange accent rail: focused tile border, current breadcrumb
/// segment, mode chip, active media control (media reserved for a later
/// phase).
fn accent_border_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

fn tag_style() -> Style {
    Style::default().fg(ACCENT_SOFT)
}

/// Errors are never color-only: pair `DANGER` with bold and a literal
/// `[!]` prefix wherever this style renders a message.
fn error_style() -> Style {
    Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
}

fn muted_style() -> Style {
    Style::default().fg(TEXT_MUTED)
}

/// Legend keys and other hover-adjacent hints.
fn key_style() -> Style {
    Style::default()
        .fg(ACCENT_HOVER)
        .add_modifier(Modifier::BOLD)
}

/// Sidebar/section headings.
fn heading_style() -> Style {
    Style::default().fg(TEXT_MUTED)
}

/// Preview metadata labels (type, size, modified, perms).
fn preview_meta_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

/// The mode chip in the status bar: accent text on root ink, always solid.
fn mode_chip_style() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(ROOT_INK)
        .add_modifier(Modifier::BOLD)
}

/// The current breadcrumb segment: the only breadcrumb touched by the
/// accent rail.
fn breadcrumb_current_style() -> Style {
    Style::default()
        .fg(ACCENT_HOVER)
        .add_modifier(Modifier::BOLD)
}

fn breadcrumb_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

/// Fills `area` with a flat surface color. Later widgets patch their own
/// foreground on top without erasing this fill (styles compose via
/// `Style::patch`, so only fields callers actually set override it).
fn surface_fill(frame: &mut Frame, area: Rect, color: Color) {
    frame.render_widget(Block::default().style(Style::default().bg(color)), area);
}

/// Frame/background/title treatment shared by every modal overlay:
/// `SURFACE_3` fill, `BORDER_STRONG` frame, caller-chosen title accent.
fn overlay_block(title: &str, accent: Style) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(Style::default().fg(BORDER_STRONG))
        .style(Style::default().bg(SURFACE_3))
        .title(Span::styled(format!(" {title} "), accent))
}

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();
    state.width = area.width;
    state.height = area.height;
    state.hit_map.clear();
    surface_fill(frame, area, SURFACE_0);
    let tier = tier_for(area.width, area.height);
    match tier {
        Tier::TooSmall => {
            render_too_small(frame, area);
            state.grid_cols = 1;
            state.list_viewport = 1;
            return;
        }
        // Narrow: one combined header/path row, grid, status, reduced
        // legend. No tip, sidebar, or preview: there is no room, and
        // `legend_items` already drops the sidebar/preview/open-with keys
        // for this tier.
        Tier::Narrow => render_narrow_shell(frame, area, state),
        // Compact: header, a separate path bar, grid, status, legend, and
        // tip. No side panel auto-shows (`sidebar_visible`/`preview_visible`
        // default both to hidden here), but the user's `b`/`p` toggles
        // still work if there is room.
        Tier::Compact => render_chrome_shell(frame, area, state, true),
        // Standard/Wide: header, path bar, grid, status, legend, no tip.
        // `sidebar_visible`/`preview_visible` supply the per-tier default
        // (Standard: sidebar on, preview off; Wide: both on).
        Tier::Standard | Tier::Wide => render_chrome_shell(frame, area, state, false),
    }

    // Pointer hover state shared by every button-style control (spec §7).
    let hovered_control = state.hover.control;
    match &state.mode {
        Mode::Confirm(confirm) => {
            render_confirm(frame, area, confirm, hovered_control, &mut state.hit_map)
        }
        Mode::Conflict(conflict) => {
            render_conflict(frame, area, conflict, hovered_control, &mut state.hit_map)
        }
        Mode::TagPicker(picker) => {
            let picker = picker.clone();
            render_picker(frame, area, state, &picker);
        }
        Mode::ContextMenu(menu) => {
            let menu = menu.clone();
            let cwd = state.browser.cwd.clone();
            render_context_menu(frame, area, &menu, &cwd, &mut state.hit_map);
        }
        Mode::Password(dialog) => {
            let view = PasswordView {
                purpose: dialog.purpose,
                confirming: dialog.confirming(),
                target: dialog.target.display().to_string(),
                input_len: dialog.input.chars().count(),
            };
            render_password(frame, area, &view, hovered_control, &mut state.hit_map);
        }
        Mode::OpenWith(dialog) => {
            let target = dialog.target.display().to_string();
            let input = dialog.input.clone();
            render_open_with(
                frame,
                area,
                &target,
                &input,
                hovered_control,
                &mut state.hit_map,
            );
        }
        Mode::Bookmarks(nav) => {
            let nav = nav.clone();
            let home = state.home.clone();
            render_bookmarks(frame, area, &nav, &home, &mut state.hit_map);
        }
        Mode::Help => render_help(frame, area, &mut state.hit_map),
        Mode::Media(media) => {
            let media = media.clone();
            render_media_modal(frame, area, state, &media);
        }
        Mode::Browser | Mode::Command => {}
    }

    // Active drag feedback renders above panels but below modals.
    if let Some(drag) = &state.drag
        && drag.phase == crate::app::state::DragPhase::Dragging
    {
        render_drag_feedback(frame, area, state);
    }

    // Live marquee band renders above panels but below modals: outline
    // only, so filenames and icons underneath stay readable.
    if let Some(marquee) = &state.marquee
        && marquee.phase == crate::app::state::MarqueePhase::Selecting
    {
        outline_rect(frame, marquee.rect(), Style::default().fg(ACCENT_SOFT));
    }
}
/// Ghost, target border, and status text for an in-flight drag. Renders
/// above panels but below modals; never mutates anything.
fn render_drag_feedback(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let Some(drag) = state.drag.clone() else {
        return;
    };
    if !matches!(state.mode, Mode::Browser) {
        return;
    }
    let (cx, cy) = drag.cursor;

    // Valid-target highlight: accent border around a real directory.
    let valid_target = crate::app::reduce::drag_drop_target_for_ui(state, cx, cy);
    let target_style = if valid_target.is_some() {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    };
    if valid_target.is_some() || state.hit_map.hit(cx, cy).is_some() {
        if let Some(rect) = hovered_row_rect(state, cx, cy) {
            outline_rect(frame, rect, target_style);
        }
    }

    // Ghost label at the cursor.
    let count = drag.sources.len();
    let ghost = format!(" moving {count} item{} ", if count == 1 { "" } else { "s" });
    let ghost_width = ghost.len() as u16;
    let gx = (cx.saturating_add(1)).min(area.width.saturating_sub(ghost_width));
    let gy = cy.saturating_sub(1).min(area.height.saturating_sub(1));
    frame.buffer_mut().set_stringn(
        gx,
        gy,
        &ghost,
        ghost_width as usize,
        Style::default()
            .bg(FOCUS_BG)
            .fg(ACCENT_SOFT)
            .add_modifier(Modifier::BOLD),
    );

    // Status line hint.
    let status_y = area.y + area.height - 3;
    let hint = " drop on a folder to move, Esc to cancel";
    frame.buffer_mut().set_stringn(
        area.x + 1,
        status_y,
        hint,
        (area.width as usize).saturating_sub(2),
        Style::default().fg(TEXT_SECONDARY),
    );
}

fn hovered_row_rect(state: &AppState, x: u16, y: u16) -> Option<Rect> {
    match state.hit_map.hit(x, y)? {
        HitTarget::Row(_) | HitTarget::Sidebar(_) | HitTarget::Breadcrumb(_) => state
            .hit_map
            .regions
            .iter()
            .rev()
            .find(|(rect, _)| {
                x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
            })
            .map(|(rect, _)| *rect),
        _ => None,
    }
}

fn outline_rect(frame: &mut Frame, rect: Rect, style: Style) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(style);
    frame.render_widget(block, rect);
}

/// The `Narrow`-tier shell: a single combined header/path row on top, the
/// grid, a reduced legend, and the status row on the very bottom. Never
/// shows a tip, sidebar, or preview panel.
fn render_narrow_shell(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let header_path = Rect::new(area.x, area.y, area.width, 1);
    // Status is the LAST terminal row (binding media-replay contract);
    // the reduced legend sits directly above it.
    let status = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    let legend = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(2),
        area.width,
        1,
    );
    let grid = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(3),
    );
    render_header_path_narrow(frame, header_path, state);
    render_narrow_grid(frame, grid, state);
    render_status(frame, status, state);
    render_legend(frame, legend, state);
    state.sidebar_items.clear();
    state.preview.key = None;
    state.preview.content = None;
}

/// Combined header + path summary used only by the `Narrow` shell: the app
/// title and the current directory condensed onto one line, since a
/// clickable breadcrumb and a full-width title do not both fit.
fn render_header_path_narrow(frame: &mut Frame, area: Rect, state: &AppState) {
    surface_fill(frame, area, SURFACE_2);
    let cwd = state.browser.cwd.display().to_string();
    let hidden = if state.browser.show_hidden {
        " [.+]"
    } else {
        ""
    };
    let path_budget = (area.width as usize).saturating_sub(15 + hidden.chars().count());
    let spans = vec![
        Span::styled(" tui-explorer ", dir_style()),
        Span::styled(truncate(&cwd, path_budget), breadcrumb_style()),
        Span::styled(hidden.to_string(), muted_style()),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The `Compact`/`Standard`/`Wide` shell: header, path bar, optional
/// sidebar, grid, optional preview, status, legend, and (only when
/// `show_tip` is set, i.e. Compact only) a tip row. `sidebar_visible` and
/// `preview_visible` already encode each tier's default, so this one shell
/// produces the right panels for all three tiers.
fn render_chrome_shell(frame: &mut Frame, area: Rect, state: &mut AppState, show_tip: bool) {
    // Bottom-up: status is the LAST terminal row (binding media-replay
    // contract), then legend, then the Compact-only tip.
    let header = Rect::new(area.x, area.y, area.width, 1);
    let path_bar = Rect::new(area.x, area.y + 1, area.width, 1);
    let (tip, legend, status, main) = if show_tip {
        (
            Some(Rect::new(area.x, area.y + area.height - 3, area.width, 1)),
            Rect::new(area.x, area.y + area.height - 2, area.width, 1),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
            Rect::new(
                area.x,
                area.y + 2,
                area.width,
                area.height.saturating_sub(5),
            ),
        )
    } else {
        (
            None,
            Rect::new(area.x, area.y + area.height - 2, area.width, 1),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
            Rect::new(
                area.x,
                area.y + 2,
                area.width,
                area.height.saturating_sub(4),
            ),
        )
    };

    render_header(frame, header, state);
    render_path_bar(frame, path_bar, state);

    let show_sidebar = sidebar_visible(area.width, area.height, state.show_sidebar)
        && main.width >= SIDEBAR_WIDTH + TILE_W + 4;
    let show_preview = preview_visible(area.width, area.height, state.show_preview)
        && main.width >= PREVIEW_WIDTH + TILE_W + 4;

    let mut grid_area = main;
    if show_sidebar {
        let sb = Rect::new(main.x, main.y, SIDEBAR_WIDTH.min(main.width), main.height);
        render_sidebar(frame, sb, state);
        grid_area.x += sb.width;
        grid_area.width = grid_area.width.saturating_sub(sb.width);
    } else {
        state.sidebar_items.clear();
    }
    if show_preview {
        let pw = PREVIEW_WIDTH.min(grid_area.width.saturating_sub(TILE_W + 2));
        let pv = Rect::new(
            grid_area.x + grid_area.width - pw,
            grid_area.y,
            pw,
            grid_area.height,
        );
        grid_area.width = grid_area.width.saturating_sub(pw);
        render_preview(frame, pv, state);
    } else {
        state.preview.key = None;
        state.preview.content = None;
    }
    render_grid(frame, grid_area, state);
    render_status(frame, status, state);
    render_legend(frame, legend, state);
    if let Some(tip) = tip {
        render_tip(frame, tip, state);
    }
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    surface_fill(frame, area, SURFACE_0);
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

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    surface_fill(frame, area, SURFACE_2);
    let title = format!("tui-explorer {}", env!("CARGO_PKG_VERSION"));
    let help_hint = "Press ? for help";
    let mut spans = vec![Span::styled(format!(" {title}"), dir_style())];
    let used = 1 + title.chars().count() + help_hint.chars().count() + 1;
    let center = "tui-explorer is awesome!";
    if area.width as usize > used + center.chars().count() + 2 {
        let pad = (area.width as usize - center.chars().count()) / 2;
        let left = pad.saturating_sub(1 + title.chars().count());
        spans.push(Span::raw(" ".repeat(left)));
        spans.push(Span::styled(center, muted_style()));
    }
    let right_pad = (area.width as usize).saturating_sub(
        spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum::<usize>()
            + help_hint.chars().count()
            + 1,
    );
    spans.push(Span::raw(" ".repeat(right_pad)));
    spans.push(Span::styled(help_hint, key_style()));
    let _ = state;
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_path_bar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    surface_fill(frame, area, SURFACE_1);
    let segments = breadcrumb_segments(&state.browser.cwd);
    let mut spans: Vec<Span> = vec![Span::styled(" Path: ", base_style())];
    let mut x = area.x + 7;
    // Drop leading segments when the path is too long to fit.
    let mut start = 0usize;
    let boxed_width = |idx: usize| segments[idx].1.chars().count() + 4;
    let total: usize = segments
        .iter()
        .enumerate()
        .map(|(i, _)| boxed_width(i))
        .sum();
    let avail = area.width.saturating_sub(8) as usize;
    if total > avail {
        start = 1;
        while start < segments.len().saturating_sub(2)
            && segments
                .iter()
                .enumerate()
                .skip(start)
                .map(|(i, _)| boxed_width(i))
                .sum::<usize>()
                + 6
                > avail
        {
            start += 1;
        }
        spans.push(Span::styled("[..]", muted_style()));
        spans.push(Span::raw(" "));
        x += 5;
    }
    for (idx, (_, label)) in segments.iter().enumerate().skip(start) {
        if x >= area.x + area.width {
            break;
        }
        let is_last = idx == segments.len() - 1;
        let text = format!(" {label} ");
        let width = text.chars().count() as u16 + 2;
        if x + width > area.x + area.width {
            break;
        }
        let (bracket_style, text_style) = if is_last {
            (muted_style(), breadcrumb_current_style())
        } else {
            (muted_style(), breadcrumb_style())
        };
        spans.push(Span::styled("[", bracket_style));
        spans.push(Span::styled(text, text_style));
        spans.push(Span::styled("]", bracket_style));
        spans.push(Span::styled(if is_last { " " } else { ">" }, muted_style()));
        state
            .hit_map
            .push(Rect::new(x, area.y, width, 1), HitTarget::Breadcrumb(idx));
        x += width + 1;
    }
    if state.browser.show_hidden && x + 6 < area.x + area.width {
        spans.push(Span::styled("[.+]", muted_style()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_sidebar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    surface_fill(frame, area, SURFACE_2);
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_set(ASCII_BORDERS)
        .border_style(Style::default().fg(BORDER_SUBTLE));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = sidebar::build_sections(state);
    let mut flat: Vec<SidebarItem> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut hits: Vec<(u16, usize)> = Vec::new();
    let width = inner.width as usize;

    let section = |title: &str,
                   items: &[SidebarItem],
                   lines: &mut Vec<Line>,
                   flat: &mut Vec<SidebarItem>,
                   hits: &mut Vec<(u16, usize)>,
                   empty: &str| {
        if lines.len() as u16 >= inner.height {
            return;
        }
        let dashes = width.saturating_sub(title.len() + 1);
        lines.push(Line::from(vec![
            Span::styled(title.to_string(), heading_style()),
            Span::styled(format!(" {}", "-".repeat(dashes)), muted_style()),
        ]));
        if items.is_empty() {
            lines.push(Line::from(Span::styled(
                truncate(empty, width.saturating_sub(1)),
                muted_style(),
            )));
            return;
        }
        for item in items {
            if lines.len() as u16 >= inner.height.saturating_sub(1) {
                break;
            }
            let idx = flat.len();
            let spans = match item {
                SidebarItem::Place { label, .. } => vec![
                    Span::styled(" ", base_style()),
                    Span::styled(
                        pad_right(&truncate(label, width - 2), width - 2),
                        base_style(),
                    ),
                ],
                SidebarItem::Mount { .. } => {
                    let m = match item {
                        SidebarItem::Mount {
                            path,
                            fs,
                            used,
                            total,
                        } => sidebar::MountInfo {
                            path: path.clone(),
                            fs: fs.clone(),
                            used: *used,
                            total: *total,
                        },
                        _ => unreachable!(),
                    };
                    vec![Span::styled(
                        pad_right(&truncate(&sidebar::mount_label(&m), width - 1), width - 1),
                        muted_style(),
                    )]
                }
                SidebarItem::Tag { name, token } => vec![
                    Span::styled(" * ", tag_style()),
                    Span::styled(truncate(name, width.saturating_sub(6).max(1)), base_style()),
                    Span::styled(format!(" [{token}]"), muted_style()),
                ],
                SidebarItem::Bookmark { path } => vec![
                    Span::styled(" * ", Style::default().fg(ACCENT)),
                    Span::styled(
                        truncate(
                            &path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string()),
                            width.saturating_sub(4).max(1),
                        ),
                        base_style(),
                    ),
                ],
            };
            lines.push(Line::from(spans));
            hits.push((lines.len() as u16 - 1, idx));
            flat.push(item.clone());
        }
    };

    section(
        "PLACES",
        &sections.places,
        &mut lines,
        &mut flat,
        &mut hits,
        "(none)",
    );
    section(
        "MOUNTS",
        &sections.mounts,
        &mut lines,
        &mut flat,
        &mut hits,
        "(no device mounts)",
    );
    section(
        "TAGS",
        &sections.tags,
        &mut lines,
        &mut flat,
        &mut hits,
        "(no tags yet)",
    );
    section(
        "BOOKMARKS",
        &sections.bookmarks,
        &mut lines,
        &mut flat,
        &mut hits,
        "(Ctrl-b bookmarks cwd)",
    );

    state.sidebar_items = flat;
    for (dy, idx) in hits {
        state.hit_map.push(
            Rect::new(inner.x, inner.y + dy, inner.width, 1),
            HitTarget::Sidebar(idx),
        );
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_narrow_grid(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 4 || area.height < 2 {
        state.grid_cols = 1;
        state.list_viewport = 1;
        return;
    }
    surface_fill(frame, area, SURFACE_1);
    let total = state.browser.visible_len();
    let header = format!(
        "{total} items  Sort: {} ({})",
        state.browser.sort_mode.label(),
        if state.browser.sort_mode.descending() {
            "desc"
        } else {
            "asc"
        }
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate(&header, area.width as usize),
            muted_style(),
        ))),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let viewport = area.height.saturating_sub(1) as usize;
    state.grid_cols = 1;
    state.list_viewport = viewport.max(1);
    state.browser.clamp_scroll(viewport.max(1));
    let indices = state.browser.visible_indices();
    let selected_paths = state.browser.selected_paths_set();
    let scroll = state.browser.scroll;
    let buf = frame.buffer_mut();
    for (row, eidx) in indices.iter().skip(scroll).take(viewport).enumerate() {
        let view = &state.browser.entries[*eidx];
        let position = scroll + row;
        let focused = position == state.browser.selected
            && matches!(state.mode, Mode::Browser | Mode::Command);
        let selected = selected_paths.contains(&view.entry.path);
        // Spec section 8: four-way (selected, focused) split. The bare
        // cursor gets `cursor_style` + a BORDER_STRONG rule, never the
        // orange selection family; hover layers UNDERLINED onto whichever
        // base was picked (`Style::patch` unions modifiers, so it survives
        // every patch below).
        let mut style = if selected && focused {
            focused_selected_style()
        } else if selected {
            selected_style()
        } else if focused {
            cursor_style()
        } else {
            base_style()
        };
        if state.hover.row == Some(position) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        let border = if selected && focused {
            style.patch(accent_border_style())
        } else if focused {
            style.patch(Style::default().fg(BORDER_STRONG))
        } else {
            style.patch(Style::default().fg(BORDER_SUBTLE))
        };
        let y = area.y + 1 + row as u16;
        let row_area = Rect::new(area.x, y, area.width, 1);
        buf.set_style(row_area, style);
        buf.set_stringn(area.x, y, "|", 1, border);
        buf.set_stringn(area.x + area.width - 1, y, "|", 1, border);
        let marker = if selected { "*" } else { " " };
        let detail = if view.entry.kind.is_dir() {
            "dir".to_string()
        } else {
            format_size(view.entry.size)
        };
        let label = format!("{marker} {}  {detail}", view.entry.display_name());
        buf.set_stringn(
            area.x + 1,
            y,
            truncate(&label, area.width.saturating_sub(2) as usize),
            area.width.saturating_sub(2) as usize,
            style,
        );
        state.hit_map.push(row_area, HitTarget::Row(position));
    }
}

fn render_grid(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 4 || area.height < 3 {
        state.grid_cols = 1;
        state.list_viewport = 1;
        return;
    }
    // Header line: entry counts + sort mode.
    let dirs = state
        .browser
        .visible_entries()
        .filter(|(_, e)| e.entry.kind.is_dir())
        .count();
    let total = state.browser.visible_len();
    let listed = state.browser.listed_len();
    let files = total.saturating_sub(dirs);
    let count = if state.browser.filter.is_some() {
        format!("{total}/{listed} items ({dirs} dirs, {files} files)")
    } else {
        format!("{total} items ({dirs} dirs, {files} files)")
    };
    let mut header_spans = vec![
        Span::styled(count, base_style()),
        Span::raw("  "),
        Span::styled(
            format!(
                "Sort: {} ({})",
                state.browser.sort_mode.label(),
                if state.browser.sort_mode.descending() {
                    "desc"
                } else {
                    "asc"
                }
            ),
            muted_style(),
        ),
    ];
    if let Some(filter) = &state.browser.filter {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(format!("Filter: {filter}"), tag_style()));
    }
    let header = Line::from(header_spans);
    frame.render_widget(
        Paragraph::new(header),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let grid = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    surface_fill(frame, grid, SURFACE_1);

    // Blank grid space is a semantic target: a left press here arms a
    // marquee. Registered before the tiles are pushed so reverse-order hit
    // resolution keeps tiles more specific than the background.
    state.hit_map.push(grid, HitTarget::GridBackground);
    if total == 0 && state.browser.filter.is_some() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("No matching files", muted_style())))
                .alignment(ratatui::layout::Alignment::Center),
            grid,
        );
        state.grid_cols = 1;
        state.list_viewport = 1;
        return;
    }
    let cols = (grid.width / TILE_W).max(1) as usize;
    let rows = (grid.height / TILE_H).max(1) as usize;
    state.grid_cols = cols;
    state.list_viewport = cols * rows;
    state.browser.clamp_scroll_grid(cols, rows);

    let indices = state.browser.visible_indices();
    let scroll = state.browser.scroll;
    let per = cols * rows;
    let buf = frame.buffer_mut();
    for (i, eidx) in indices.iter().enumerate().skip(scroll).take(per) {
        let view = &state.browser.entries[*eidx];
        let slot = i - scroll;
        let col = slot % cols;
        let row = slot / cols;
        let tx = grid.x + col as u16 * TILE_W;
        let ty = grid.y + row as u16 * TILE_H;
        let pos = i; // visible position, matches browser.selected semantics
        let focused =
            pos == state.browser.selected && matches!(state.mode, Mode::Browser | Mode::Command);
        let selected = state
            .browser
            .selected_paths_set()
            .contains(&view.entry.path);
        // Spec section 8: four-way (selected, focused) split. The bare
        // cursor gets `cursor_style` + a BORDER_STRONG border, never the
        // orange selection family; hover layers UNDERLINED onto whichever
        // base was picked (`Style::patch` unions modifiers, so the
        // underline survives every patch below).
        let mut base = if selected && focused {
            focused_selected_style()
        } else if selected {
            selected_style()
        } else if focused {
            cursor_style()
        } else {
            base_style()
        };
        if state.hover.row == Some(pos) {
            base = base.add_modifier(Modifier::UNDERLINED);
        }
        // Tile border: subtle rule by default, BORDER_STRONG for the bare
        // cursor, accent only when the tile is both selected and focused.
        let border_style = if selected && focused {
            base.patch(accent_border_style())
        } else if focused {
            base.patch(Style::default().fg(BORDER_STRONG))
        } else {
            base.patch(Style::default().fg(BORDER_SUBTLE))
        };
        for dy in 0..TILE_H {
            buf.set_stringn(tx, ty + dy, "|", 1, border_style);
            buf.set_stringn(tx + TILE_W - 1, ty + dy, "|", 1, border_style);
        }
        let is_dir = view.entry.kind.is_dir();
        let kind = if is_dir {
            if view.entry.hidden {
                crate::icons::IconKind::FolderHidden
            } else if focused {
                crate::icons::IconKind::FolderOpen
            } else {
                crate::icons::IconKind::Folder
            }
        } else {
            IconResolver::default().resolve_with(&view.entry, IconVariant::Normal)
        };
        let art = tile_art(kind, &view.entry);
        let art_style = if is_dir {
            base.patch(dir_style())
        } else {
            base.patch(Style::default().fg(TEXT_PRIMARY))
        };
        for (dy, line) in art.iter().enumerate() {
            buf.set_stringn(tx + 1, ty + dy as u16, line, TILE_ART_WIDTH, art_style);
        }
        // Name line: selection marker + tag badge + name.
        let name_y = ty + TILE_ART_HEIGHT as u16;
        let mark = if selected { "*" } else { " " };
        let name = view.entry.display_name();
        let name_budget = TILE_W as usize - 3;
        let shown = truncate(&name, name_budget);
        let name_style = if is_dir {
            base.patch(dir_style())
        } else {
            base.patch(base_style())
        };
        buf.set_stringn(tx + 1, name_y, mark, 1, base.patch(tag_style()));
        buf.set_stringn(tx + 2, name_y, shown, name_budget, name_style);
        // Size / detail line.
        let detail = if is_dir {
            "dir".to_string()
        } else {
            format_size(view.entry.size)
        };
        let tag_badge = view
            .tags
            .first()
            .map(|t| format!("[{t}]"))
            .unwrap_or_default();
        buf.set_stringn(
            tx + 1,
            name_y + 1,
            &detail,
            TILE_W as usize - 2,
            base.patch(muted_style()),
        );
        if !tag_badge.is_empty() {
            let bx = tx + 1 + detail.chars().count() as u16 + 1;
            let right_edge = tx + TILE_W - 1; // reserve the border column
            if bx < right_edge {
                buf.set_stringn(
                    bx,
                    name_y + 1,
                    &tag_badge,
                    (right_edge - bx) as usize,
                    base.patch(tag_style()),
                );
            }
        }
        state
            .hit_map
            .push(Rect::new(tx, ty, TILE_W, TILE_H), HitTarget::Row(pos));
    }
    if indices.is_empty() {
        buf.set_stringn(
            grid.x,
            grid.y,
            "(empty directory)",
            grid.width as usize,
            muted_style(),
        );
    }
}

fn render_preview(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_set(ASCII_BORDERS)
        .border_style(Style::default().fg(BORDER_SUBTLE));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(view) = state.browser.focused() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no selection", muted_style()))),
            inner,
        );
        return;
    };
    let width = inner.width as usize;
    let entry = &view.entry;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        pad_right(&truncate(&entry.display_name(), width), width),
        dir_style(),
    )));
    lines.push(Line::from(Span::styled(
        format!("Type: {}", kind_label(&entry.kind)),
        preview_meta_style(),
    )));
    if !entry.kind.is_dir() {
        lines.push(Line::from(Span::styled(
            format!("Size: {} ({} B)", format_size(entry.size), entry.size),
            preview_meta_style(),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!("Modified: {}", format_time(entry.modified)),
        preview_meta_style(),
    )));
    lines.push(Line::from(Span::styled(
        format!("Perms: {}", format_mode(&entry.kind, entry.mode)),
        preview_meta_style(),
    )));
    lines.push(Line::from(Span::styled(
        if view.tags.is_empty() {
            "Tags: (none)".to_string()
        } else {
            format!(
                "Tags: {}",
                view.tags
                    .iter()
                    .map(|t| format!("[{t}]"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        },
        tag_style(),
    )));
    let tag_y = inner.y + lines.len() as u16 - 1;
    let title = format!(" Preview ({:?}) ", state.picker.protocol_type());
    let dashes = width.saturating_sub(title.len());
    lines.push(Line::from(vec![
        Span::styled(title, Style::default().fg(TEXT_PRIMARY)),
        Span::styled("-".repeat(dashes), muted_style()),
    ]));
    let meta_height = lines.len() as u16;
    frame.render_widget(Paragraph::new(lines), inner);
    state.hit_map.push(
        Rect::new(inner.x, tag_y, inner.width, 1),
        HitTarget::TagBadge,
    );

    let content_area = Rect::new(
        inner.x,
        inner.y + meta_height,
        inner.width,
        inner.height.saturating_sub(meta_height),
    );
    if content_area.width < 3 || content_area.height < 3 {
        return;
    }
    frame.render_widget(Clear, content_area);
    let content_frame = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(Style::default().fg(BORDER_SUBTLE))
        .style(Style::default().bg(SURFACE_1));
    let content_inner = content_frame.inner(content_area);
    frame.render_widget(content_frame, content_area);
    let content_width = content_inner.width as usize;
    let mut encoding_result = None;
    match &mut state.preview.content {
        Some(PreviewContent::Text { lines, truncated }) => {
            let mut out: Vec<Line> = lines
                .iter()
                .take(content_inner.height as usize)
                .map(|line| {
                    Line::from(Span::styled(
                        truncate(line, content_width),
                        Style::default().fg(TEXT_SECONDARY),
                    ))
                })
                .collect();
            if *truncated {
                out.push(Line::from(Span::styled("... (truncated)", muted_style())));
            }
            frame.render_widget(Paragraph::new(out), content_inner);
        }
        Some(PreviewContent::Directory(names)) => {
            let out: Vec<Line> = names
                .iter()
                .take(content_inner.height as usize)
                .map(|name| Line::from(Span::styled(truncate(name, content_width), base_style())))
                .collect();
            frame.render_widget(Paragraph::new(out), content_inner);
        }
        Some(PreviewContent::Image(proto)) => {
            frame.render_stateful_widget(
                ratatui_image::StatefulImage::new(),
                content_inner,
                proto.as_mut(),
            );
            encoding_result = proto.last_encoding_result();
        }
        Some(PreviewContent::Unavailable(message)) => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate(message, content_width),
                    base_style(),
                ))),
                content_inner,
            );
        }
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("loading...", muted_style()))),
                content_inner,
            );
        }
    }
    apply_image_encoding_result(state, encoding_result);
}

fn apply_image_encoding_result(
    state: &mut AppState,
    result: Option<std::result::Result<(), ratatui_image::errors::Errors>>,
) {
    if let Some(Err(error)) = result {
        state.preview.content = Some(PreviewContent::Unavailable(format!(
            "image preview failed: {error}"
        )));
    }
}

/// Seconds shown as "elapsed" while a rail drag scrubs: the drag position
/// when one is in flight, else the reported position (spec section 1).
fn media_display_position(media: &crate::app::state::MediaState) -> f64 {
    if media.slider_drag_active {
        media.slider_drag_pos.unwrap_or(media.position)
    } else {
        media.position
    }
}

/// Button visual state per spec section 7: Hovered comes from
/// `state.hover.control`; Active marks the transport toggle while the
/// session is live (Playing or Paused, mirroring the pre-existing check)
/// and FULL while fullscreen is engaged. Hover wins over Active so pointer
/// feedback is always visible (`draw_button` treats states as exclusive).
fn media_button_state(
    state: &AppState,
    media: &crate::app::state::MediaState,
    target: HitTarget,
) -> ButtonState {
    use crate::media::{MediaKind, MediaPhase};

    if state.hover.control == Some(target) {
        return ButtonState::Hovered;
    }
    let active = match target {
        HitTarget::MediaTogglePause => {
            matches!(media.phase, MediaPhase::Playing | MediaPhase::Paused)
        }
        HitTarget::MediaFullscreen => media.kind == MediaKind::Video && media.fullscreen,
        _ => false,
    };
    if active {
        ButtonState::Active
    } else {
        ButtonState::Idle
    }
}

/// Draws the seek rail inside `rect` (spec section 1 row 2 draw order):
/// played track ACCENT+BOLD `━`, remainder BORDER_SUBTLE `─`, hover tick
/// `│` in ACCENT_HOVER plus an optional floating timestamp beside the
/// tick, then the ACCENT_HOVER `●` thumb drawn last so it wins where the
/// tick and thumb coincide. Registers `HitTarget::MediaSeekRail`
/// unconditionally — including unknown duration; the reducer gates
/// unknown-duration gestures, not the renderer.
fn draw_seek_rail(
    frame: &mut Frame,
    rect: Rect,
    media: &crate::app::state::MediaState,
    hits: &mut HitMap,
    floating_label: bool,
) {
    let display_position = media_display_position(media);
    let mut geom = rail_geometry(rect, display_position, media.duration);
    if !media.slider_drag_active
        && let Some(hover_secs) = media.slider_hover
    {
        // Same geometry for preview and commit: the tick lands exactly
        // where a click there would seek (spec section 1 row 2).
        geom.hover_x = Some(rail_geometry(rect, hover_secs, media.duration).thumb_x);
    }

    let width = rect.width as usize;
    let played = geom.thumb_x.saturating_sub(rect.x) as usize;
    let buffer = frame.buffer_mut();
    if played > 0 {
        buffer.set_stringn(
            rect.x,
            rect.y,
            "\u{2501}".repeat(played),
            played,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        );
    }
    let rest = width.saturating_sub(played);
    if rest > 0 {
        buffer.set_stringn(
            rect.x + played as u16,
            rect.y,
            "\u{2500}".repeat(rest),
            rest,
            Style::default().fg(BORDER_SUBTLE),
        );
    }
    if let Some(hover_x) = geom.hover_x
        && hover_x >= rect.x
        && hover_x < rect.x + rect.width
    {
        buffer.set_stringn(
            hover_x,
            rect.y,
            "\u{2502}",
            1,
            Style::default().fg(ACCENT_HOVER),
        );
        if floating_label && let Some(hover_secs) = media.slider_hover {
            // Spec section 1: label at hover_x+2 when six columns fit to
            // the right of the tick, else at hover_x-7 when they fit to
            // the left, else omitted.
            let right_edge = rect.x + rect.width;
            let label_x = if hover_x + 8 <= right_edge {
                Some(hover_x + 2)
            } else if hover_x >= rect.x + 7 {
                Some(hover_x - 7)
            } else {
                None
            };
            if let Some(label_x) = label_x {
                let stamp =
                    format_time_duration(std::time::Duration::from_secs_f64(hover_secs.max(0.0)));
                buffer.set_stringn(
                    label_x,
                    rect.y,
                    &stamp,
                    6,
                    Style::default().fg(ACCENT_HOVER),
                );
            }
        }
    }
    // Thumb last: always wins over the hover tick when they coincide.
    if geom.thumb_x >= rect.x && geom.thumb_x < rect.x + rect.width {
        buffer.set_stringn(
            geom.thumb_x,
            rect.y,
            "\u{25CF}",
            1,
            Style::default().fg(ACCENT_HOVER),
        );
    }
    hits.push(rect, HitTarget::MediaSeekRail);
}

fn render_media_modal(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::media::{MediaKind, MediaPhase};

    state.hit_map.push(area, HitTarget::Blocker);
    if media.kind == MediaKind::Video && media.fullscreen {
        render_media_fullscreen(frame, area, state, media);
        return;
    }
    let full = area.width >= 60 && area.height >= 16;
    let modal_height = if full { 22 } else { 12 }.min(area.height);
    let rect = centered_rect(area, area.width.min(96), modal_height);
    frame.render_widget(Clear, rect);
    let title = match media.kind {
        MediaKind::Audio => "NOW PLAYING",
        MediaKind::Video => "VIDEO",
    };
    let block = overlay_block(title, accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width < 8 || inner.height < 7 {
        return;
    }

    // Control tiers key off inner.width (spec section 4): Wide >= 60 one
    // row, Split 40..60 two rows, Pruned < 40 drops NEXT/V-/V+ (video
    // keeps FULL). Clipped windows below the compact two-row budget
    // degrade to one control row; every other size matches the spec table.
    let controls_rows: u16 = if (40..60).contains(&inner.width) && inner.height >= 10 {
        2
    } else {
        1
    };
    let controls_height = controls_rows * 3;
    let controls_top = inner.y + inner.height - controls_height;
    // Row budget (spec section 1): header rows 0-2 fixed; Full mode adds a
    // spacer after the header and before the controls, Compact mode has
    // neither.
    let (surface_top, surface_height) = if full {
        (
            inner.y + 4,
            inner.height.saturating_sub(4 + controls_height + 1),
        )
    } else {
        (
            inner.y + 3,
            inner.height.saturating_sub(3 + controls_height),
        )
    };

    // Row 0: filename (left) + phase chip flush right (spec section 1).
    let phase_text = if media.error.is_some() {
        "ERROR".to_string()
    } else {
        format!("{:?}", media.phase).to_ascii_uppercase()
    };
    let chip = format!(" {phase_text} ");
    let chip_width = (chip.chars().count() as u16).min(inner.width);
    let chip_style = if media.error.is_some() {
        Style::default()
            .fg(DANGER)
            .bg(ROOT_INK)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ACCENT).bg(ROOT_INK)
    };
    frame.buffer_mut().set_stringn(
        inner.x + inner.width - chip_width,
        inner.y,
        &chip,
        chip_width as usize,
        chip_style,
    );
    let filename = media
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| media.path.display().to_string());
    let name_width = inner.width.saturating_sub(chip_width + 1);
    if name_width > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(&filename, name_width as usize),
                dir_style(),
            ))),
            Rect::new(inner.x, inner.y, name_width, 1),
        );
    }

    // Row 1: time row, replaced wholesale by the error message on error.
    // While a drag scrubs, elapsed follows the drag position; remaining
    // stays anchored to the committed position (spec section 1).
    if let Some(error) = media.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(&format!("[!] {error}"), inner.width as usize),
                error_style(),
            ))),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    } else {
        let display_position = media_display_position(media);
        let elapsed_text = format_time_duration(std::time::Duration::from_secs_f64(
            display_position.max(0.0),
        ));
        let duration_text = media
            .duration
            .map(|seconds| {
                format_time_duration(std::time::Duration::from_secs_f64(seconds.max(0.0)))
            })
            .unwrap_or_else(|| "--:--".to_string());
        let remaining_text = media
            .duration
            .map(|seconds| {
                format_time_duration(std::time::Duration::from_secs_f64(
                    (seconds - media.position).max(0.0),
                ))
            })
            .unwrap_or_else(|| "--:--".to_string());
        // Volume is the backend-reported value (sink or observed mpv
        // property), never a UI-only guess.
        let time = format!(
            "{elapsed_text} / {duration_text} / -{remaining_text} | VOL {}%",
            media.volume
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(time, preview_meta_style()))),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }

    // Row 2: the seek rail, drawn through the shared widgets geometry.
    draw_seek_rail(
        frame,
        Rect::new(inner.x, inner.y + 2, inner.width, 1),
        media,
        &mut state.hit_map,
        true,
    );

    let surface_rect = Rect::new(inner.x, surface_top, inner.width, surface_height);
    let surface = crate::app::state::MediaSurface {
        rect: surface_rect,
        terminal_cells: (area.width, area.height),
        cell_pixels: state.picker.font_size(),
    };
    if let Mode::Media(current) = &mut state.mode
        && current.session == media.session
    {
        current.surface = Some(surface);
    }

    // Control strip (spec sections 4 and 7): bordered buttons sized
    // label+2, laid out through `button_row`, hit registration handled by
    // `draw_button`.
    let toggle_label = if matches!(media.phase, MediaPhase::Playing) {
        "PAUSE"
    } else {
        "PLAY"
    };
    let video = media.kind == MediaKind::Video;
    let control_rows: Vec<Vec<(&str, u16, HitTarget)>> = if controls_rows == 2 {
        // Split tier: row 1 transports and exits, row 2 conveniences.
        let row2 = if video {
            vec![
                ("NEXT", 6u16, HitTarget::MediaNext),
                ("V-", 4, HitTarget::MediaVolumeDown),
                ("V+", 4, HitTarget::MediaVolumeUp),
                ("FULL", 6, HitTarget::MediaFullscreen),
            ]
        } else {
            vec![
                ("NEXT", 6u16, HitTarget::MediaNext),
                ("V-", 4, HitTarget::MediaVolumeDown),
                ("V+", 4, HitTarget::MediaVolumeUp),
            ]
        };
        vec![
            vec![
                ("-15", 5u16, HitTarget::MediaSeekBack),
                (toggle_label, 7, HitTarget::MediaTogglePause),
                ("+15", 5, HitTarget::MediaSeekForward),
                ("STOP", 6, HitTarget::MediaStop),
                ("X", 3, HitTarget::MediaClose),
            ],
            row2,
        ]
    } else {
        let mut row = vec![
            ("-15", 5u16, HitTarget::MediaSeekBack),
            (toggle_label, 7, HitTarget::MediaTogglePause),
            ("+15", 5, HitTarget::MediaSeekForward),
        ];
        if inner.width >= 60 {
            row.extend([
                ("NEXT", 6u16, HitTarget::MediaNext),
                ("V-", 4, HitTarget::MediaVolumeDown),
                ("V+", 4, HitTarget::MediaVolumeUp),
            ]);
        }
        if video {
            // FULL survives pruning: the escape hatch for cramped terms.
            row.push(("FULL", 6, HitTarget::MediaFullscreen));
        }
        row.push(("STOP", 6, HitTarget::MediaStop));
        row.push(("X", 3, HitTarget::MediaClose));
        vec![row]
    };
    for (index, specs) in control_rows.iter().enumerate() {
        let y = controls_top + index as u16 * 3;
        let layout: Vec<(&str, u16)> = specs
            .iter()
            .map(|(label, width, _)| (*label, *width))
            .collect();
        // `button_row` lays out horizontally at height 1; bordered buttons
        // are always 3 rows tall (spec section 4).
        for (btn_rect, (label, _, target)) in button_row(inner.x, y, inner.width, &layout)
            .iter()
            .zip(specs)
        {
            let button = Button::new(
                Rect {
                    height: 3,
                    ..*btn_rect
                },
                *label,
                *target,
            )
            .with_state(media_button_state(state, media, *target));
            draw_button(frame, &button, &mut state.hit_map);
        }
    }

    match media.kind {
        MediaKind::Audio => {
            if media.error.is_none() && surface_rect.height > 0 {
                // Spectrum bars fill the surface band bottom-up; mechanics
                // unchanged, only the rect moved (spec section 1).
                let column_width = (inner.width / 24).max(1);
                let bars_height = surface_rect.height.min(10);
                let buffer = frame.buffer_mut();
                for (index, level) in media.spectrum.iter().enumerate() {
                    let x = inner.x + index as u16 * column_width;
                    if x >= inner.x + inner.width {
                        break;
                    }
                    let height = (level.clamp(0.0, 1.0) * bars_height as f32).round() as u16;
                    for offset in 0..height {
                        let y = surface_rect.y + bars_height - 1 - offset;
                        buffer.set_stringn(x, y, "#", 1, Style::default().fg(ACCENT_SOFT));
                    }
                }
            } else if media.error.is_none() {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        if matches!(media.phase, MediaPhase::Preparing | MediaPhase::Starting) {
                            "spectrum starting"
                        } else {
                            "spectrum unavailable at this size"
                        },
                        muted_style(),
                    ))),
                    surface_rect,
                );
            }
        }
        // Video owns the surface: while frames are live (Playing/Paused)
        // nothing is drawn there so mpv's kitty output survives the
        // diff-based redraws. Only startup shows placeholder chrome
        // (spec section 2). Controls live outside `surface_rect` by
        // construction: header/rail/spacers/controls never intersect it.
        MediaKind::Video => {
            if media.error.is_none()
                && surface_rect.height > 0
                && matches!(media.phase, MediaPhase::Preparing | MediaPhase::Starting)
            {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled("loading video", muted_style())))
                        .alignment(ratatui::layout::Alignment::Center),
                    surface_rect,
                );
            }
        }
    }
}

/// Fullscreen video chrome (spec section 3): the whole area clears for
/// mpv and a 2-row bottom strip carries time+rail (row 1) and filename
/// plus flat bracket-text controls (row 2). Bordered buttons need 3 rows,
/// which cannot fit the hard 2-row budget, so this is the one place
/// bracket controls remain (spec section 0).
fn render_media_fullscreen(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::media::MediaPhase;

    frame.render_widget(Clear, area);
    let video_rect = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
    // Surface geometry flows through the awaiting_surface_ready restart
    // cycle: these two rects are what the stop->Preparing->SurfaceReady
    // restart resolves into (spec section 3).
    let surface = crate::app::state::MediaSurface {
        rect: video_rect,
        terminal_cells: (area.width, area.height),
        cell_pixels: state.picker.font_size(),
    };
    if let Mode::Media(current) = &mut state.mode
        && current.session == media.session
    {
        current.surface = Some(surface);
    }
    if area.height < 2 {
        return;
    }
    let strip_y = area.y + area.height - 2;
    surface_fill(frame, Rect::new(area.x, strip_y, area.width, 2), SURFACE_2);

    // Strip row 1: elapsed | rail | -remaining, seven columns reserved at
    // each end; on error the whole row becomes the error text.
    if let Some(error) = media.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(&format!("[!] {error}"), area.width as usize),
                error_style(),
            ))),
            Rect::new(area.x, strip_y, area.width, 1),
        );
    } else {
        let elapsed_text = format_time_duration(std::time::Duration::from_secs_f64(
            media_display_position(media).max(0.0),
        ));
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                elapsed_text,
                Style::default().fg(TEXT_PRIMARY),
            ))),
            Rect::new(area.x, strip_y, 6.min(area.width), 1),
        );
        let remaining_text = match media.duration {
            Some(seconds) => format!(
                "-{}",
                format_time_duration(std::time::Duration::from_secs_f64(
                    (seconds - media.position).max(0.0)
                ))
            ),
            None => "--:--".to_string(),
        };
        let remaining_width = 7u16.min(area.width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                remaining_text,
                Style::default().fg(TEXT_PRIMARY),
            ))),
            Rect::new(
                area.x + area.width - remaining_width,
                strip_y,
                remaining_width,
                1,
            ),
        );
        draw_seek_rail(
            frame,
            Rect::new(area.x + 7, strip_y, area.width.saturating_sub(14), 1),
            media,
            &mut state.hit_map,
            // No floating timestamp here: no spare width; the bare hover
            // tick still renders (spec section 3).
            false,
        );
    }

    // Strip row 2: filename (at most 24 cols) + 1-col gap + flat bracket
    // controls using the overflow-guard loop pattern; colors mirror the
    // ButtonState matrix fg-only (spec section 3).
    let filename = media
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| media.path.display().to_string());
    frame.buffer_mut().set_stringn(
        area.x,
        strip_y + 1,
        truncate(&filename, 24),
        (area.width as usize).min(24),
        dir_style(),
    );
    let toggle_label = if matches!(media.phase, MediaPhase::Playing) {
        "[PAUSE]"
    } else {
        "[PLAY]"
    };
    let controls: [(&str, u16, HitTarget); 9] = [
        ("[-15]", 5, HitTarget::MediaSeekBack),
        (toggle_label, 7, HitTarget::MediaTogglePause),
        ("[+15]", 5, HitTarget::MediaSeekForward),
        ("[NEXT]", 6, HitTarget::MediaNext),
        ("[V-]", 4, HitTarget::MediaVolumeDown),
        ("[V+]", 4, HitTarget::MediaVolumeUp),
        ("[FULL]", 6, HitTarget::MediaFullscreen),
        ("[STOP]", 6, HitTarget::MediaStop),
        ("[X]", 3, HitTarget::MediaClose),
    ];
    let mut control_x = area.x + 25; // 24-col filename + 1-col gap
    for (label, width, target) in controls {
        if control_x + width > area.x + area.width {
            break;
        }
        let style = if state.hover.control == Some(target) {
            Style::default().fg(ACCENT_HOVER)
        } else if target == HitTarget::MediaFullscreen {
            // This branch IS the fullscreen-on state indicator.
            accent_border_style()
        } else if target == HitTarget::MediaTogglePause
            && matches!(media.phase, MediaPhase::Playing)
        {
            accent_border_style()
        } else {
            Style::default().fg(TEXT_SECONDARY)
        };
        frame
            .buffer_mut()
            .set_stringn(control_x, strip_y + 1, label, width as usize, style);
        state
            .hit_map
            .push(Rect::new(control_x, strip_y + 1, width, 1), target);
        control_x += width + 1;
    }
}

fn format_time_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn render_status(frame: &mut Frame, area: Rect, state: &AppState) {
    surface_fill(frame, area, SURFACE_2);
    if matches!(state.mode, Mode::Command) {
        let line = Line::from(vec![
            Span::styled(":", Style::default().fg(ACCENT)),
            Span::styled(state.command_input.clone(), base_style()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let mut spans = vec![
        Span::styled(format!(" {} ", state.mode_name()), mode_chip_style()),
        Span::raw(" "),
    ];
    // Left dynamic segment priority (spec section 6): operation, then any
    // message (error presence suppresses the filename segment), then the
    // hovered/selected focus text.
    if let Some(op) = &state.operation {
        spans.push(Span::styled(
            format!(
                "{:?} {}/{} {}",
                op.kind,
                op.done,
                op.total,
                truncate(&op.current.display().to_string(), 24)
            ),
            Style::default().fg(ACCENT_HOVER),
        ));
    } else if let Some(message) = &state.message {
        let text = if message.is_error {
            format!("[!] {}", message.text)
        } else {
            message.text.clone()
        };
        spans.push(Span::styled(
            truncate(&text, (area.width as usize).saturating_sub(24)),
            if message.is_error {
                error_style()
            } else {
                base_style()
            },
        ));
    } else if let Some(text) = footer_focus_text(state) {
        spans.push(Span::styled(
            format!(
                "{}  ",
                truncate(&text, (area.width as usize).saturating_sub(24))
            ),
            tag_style(),
        ));
    }
    // Right side: focused file size, position and percent.
    let len = state.browser.visible_len();
    let pos_label = if len == 0 {
        "0/0".to_string()
    } else {
        format!("{}/{}", state.browser.selected + 1, len)
    };
    let pct = if len == 0 {
        "0%".to_string()
    } else {
        format!("{}%", ((state.browser.selected + 1) * 100) / len)
    };
    let size_label = state
        .browser
        .focused()
        .filter(|v| !v.entry.kind.is_dir())
        .map(|v| format_size(v.entry.size))
        .unwrap_or_default();
    let metrics = format!("{size_label}  {pct}  {pos_label}");
    // Clipboard chip (spec section 6): its own span ahead of the metrics
    // cluster, shaded per mode — ACCENT for Copy, ACCENT_HOVER for Cut.
    let chip = state.clipboard.chip();
    let chip_len = chip.as_ref().map(|c| c.chars().count() + 2).unwrap_or(0);
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + chip_len + metrics.chars().count() + 1);
    spans.push(Span::raw(" ".repeat(pad)));
    if let Some(chip_text) = &chip {
        let chip_style = match state.clipboard.mode {
            Some(crate::app::state::ClipMode::Cut) => {
                Style::default().fg(ACCENT_HOVER).bg(ROOT_INK)
            }
            _ => Style::default().fg(ACCENT).bg(ROOT_INK),
        };
        spans.push(Span::styled(chip_text.clone(), chip_style));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(metrics, muted_style()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn legend_items(
    state: &AppState,
    tier: Tier,
) -> Vec<(&'static str, &'static str, Option<LegendAction>)> {
    match &state.mode {
        Mode::Command => vec![
            ("Enter", "run", None),
            ("Esc", "cancel", Some(LegendAction::Cancel)),
        ],
        Mode::Confirm(_) => vec![("y", "confirm", None), ("n", "cancel", None)],
        Mode::Conflict(_) => vec![
            ("c", "cancel", None),
            ("s", "skip", None),
            ("r", "replace", None),
        ],
        Mode::TagPicker(_) => vec![
            ("Enter", "toggle", None),
            ("n", "new", Some(LegendAction::TagPicker)),
            ("d", "delete", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::ContextMenu(_) => vec![
            ("Enter", "choose", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::Password(_) => vec![
            ("Enter", "submit", None),
            ("Esc", "cancel", Some(LegendAction::Cancel)),
        ],
        Mode::OpenWith(_) => vec![
            ("Enter", "run", None),
            ("Esc", "cancel", Some(LegendAction::Cancel)),
        ],
        Mode::Bookmarks(_) => vec![
            ("Enter", "go", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::Help => vec![("Esc", "close", Some(LegendAction::Cancel))],
        Mode::Browser => {
            let mut items = vec![
                ("e/Enter", "Open", Some(LegendAction::Open)),
                ("X", "Crypt", Some(LegendAction::Encrypt)),
                ("Bsp", "Up", Some(LegendAction::Parent)),
                ("v", "Visual", Some(LegendAction::Select)),
                ("t", "Tag", Some(LegendAction::QuickTag)),
                (":", "Command", Some(LegendAction::Command)),
                ("?", "Help", Some(LegendAction::Help)),
            ];
            if tier != Tier::Narrow {
                items.push(("r", "OpenWith", Some(LegendAction::OpenWith)));
                items.push(("b", "Sidebar", Some(LegendAction::Sidebar)));
                items.push(("p", "Preview", Some(LegendAction::Preview)));
            }
            if tier == Tier::Wide {
                items.push(("B", "Bookmarks", Some(LegendAction::Bookmarks)));
                items.push(("q", "Quit", Some(LegendAction::Quit)));
            }
            items
        }
        Mode::Media(_) => vec![],
    }
}

fn render_legend(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let tier = tier_for(area.width, state.height);
    let items = legend_items(state, tier);
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x;
    for (key, label, action) in items {
        let text = format!(" {key} {label} ");
        let width = text.chars().count() as u16;
        if x + width + 1 > area.x + area.width {
            break;
        }
        spans.push(Span::styled("|", muted_style()));
        spans.push(Span::styled(format!(" {key} "), key_style()));
        spans.push(Span::styled(
            format!("{label} "),
            Style::default().fg(TEXT_PRIMARY).bg(SURFACE_3),
        ));
        if let Some(action) = action {
            state.hit_map.push(
                Rect::new(x + 1, area.y, width, 1),
                HitTarget::Legend(action),
            );
        }
        x += width + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_tip(frame: &mut Frame, area: Rect, state: &AppState) {
    let tip = match &state.mode {
        Mode::Browser => {
            "TIP  Double-click or Enter/e to open * Right-click for menu * Mouse wheel to scroll"
        }
        Mode::Password(_) => "TIP  Password input is masked and never stored",
        Mode::OpenWith(_) => {
            "TIP  Type a command, e.g. mupdf, then Enter to run it on the focused entry"
        }
        Mode::Bookmarks(_) => "TIP  Type to fuzzy-search bookmarks, Up/Down to select, Enter to go",
        Mode::Media(_) => "TIP  Space plays/pauses, arrows seek and set volume, Esc closes",
        _ => "TIP  Esc goes back",
    };
    let _ = state;
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate(tip, area.width as usize),
            muted_style(),
        ))),
        area,
    );
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn push_blocker(area: Rect, hits: &mut HitMap) {
    hits.push(area, HitTarget::Blocker);
}

/// Context-menu overlay width (spec section 5): fits "Open with" plus the
/// cursor prefix.
const MENU_WIDTH: u16 = 20;

/// Lays out a row of bordered modal buttons (spec sections 0 and 7):
/// widths are label+2, height 3, hover feedback from the pointer state,
/// hit registration through `draw_button`. `y_rel` is measured from the
/// modal's inner top.
fn draw_modal_buttons(
    frame: &mut Frame,
    inner: Rect,
    y_rel: u16,
    specs: &[(&str, HitTarget, bool)], // (label, target, danger)
    hover: Option<HitTarget>,
    hits: &mut HitMap,
) {
    let y = inner.y + y_rel;
    let layout: Vec<(&str, u16)> = specs
        .iter()
        .map(|(label, _, _)| (*label, label.chars().count() as u16 + 2))
        .collect();
    for (btn_rect, (label, target, danger)) in button_row(inner.x, y, inner.width, &layout)
        .iter()
        .zip(specs)
    {
        let state = if hover == Some(*target) {
            ButtonState::Hovered
        } else {
            ButtonState::Idle
        };
        let mut button = Button::new(
            Rect {
                height: 3,
                ..*btn_rect
            },
            *label,
            *target,
        )
        .with_state(state);
        if *danger {
            button = button.danger();
        }
        draw_button(frame, &button, hits);
    }
}

/// Title for the context-menu overlay (spec section 5): Single/Bulk
/// delegate to the captured target; Background names the current
/// directory instead of the placeholder in `ContextTarget::title`.
fn context_menu_title(target: &crate::app::state::ContextTarget, cwd: &Path) -> String {
    match target {
        crate::app::state::ContextTarget::Background => cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string()),
        other => other.title(),
    }
}

fn render_confirm(
    frame: &mut Frame,
    area: Rect,
    confirm: &crate::app::state::ConfirmState,
    hover: Option<HitTarget>,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let rect = centered_rect(area, 56, 8);
    frame.render_widget(Clear, rect);
    let block = overlay_block("CONFIRM", error_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines = vec![
        Line::from(Span::styled(
            truncate(&confirm.title, inner.width as usize),
            error_style(),
        )),
        Line::from(Span::styled(
            truncate(&confirm.detail, inner.width as usize),
            base_style(),
        )),
    ];
    // Keyboard y/n still works; the buttons carry the same ModalConfirm /
    // ModalCancel targets as before (plan item f).
    frame.render_widget(Paragraph::new(lines), inner);
    draw_modal_buttons(
        frame,
        inner,
        3,
        &[
            ("Delete forever", HitTarget::ModalConfirm, true),
            ("Cancel", HitTarget::ModalCancel, false),
        ],
        hover,
        hits,
    );
}

fn render_conflict(
    frame: &mut Frame,
    area: Rect,
    conflict: &crate::app::state::ConflictState,
    hover: Option<HitTarget>,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let height = (conflict.conflicts.len() as u16 + 6).clamp(8, area.height.max(8));
    let rect = centered_rect(area, 60, height);
    frame.render_widget(Clear, rect);
    let block = overlay_block("CONFLICT", accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut lines = vec![Line::from(Span::styled(
        format!("{} destination(s) already exist:", conflict.conflicts.len()),
        base_style(),
    ))];
    for (_, dst) in conflict.conflicts.iter().take(inner.height as usize - 5) {
        lines.push(Line::from(Span::styled(
            truncate(&dst.display().to_string(), inner.width as usize),
            muted_style(),
        )));
    }
    lines.push(Line::from(""));
    // Buttons occupy the bottom three rows; the list paragraph gets the
    // rest. Same Conflict* targets as before (plan item f).
    let list_height = inner.height.saturating_sub(3);
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_height,
        },
    );
    draw_modal_buttons(
        frame,
        inner,
        inner.height.saturating_sub(3),
        &[
            ("Cancel", HitTarget::ConflictCancel, false),
            ("Skip", HitTarget::ConflictSkip, false),
            ("Replace", HitTarget::ConflictReplace, false),
        ],
        hover,
        hits,
    );
}

fn render_picker(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    picker: &crate::app::state::TagPickerState,
) {
    state.hit_map.push(area, HitTarget::Blocker);
    let height = (picker.defs.len() as u16 + 7).clamp(9, area.height.max(9));
    let rect = centered_rect(area, 44, height);
    frame.render_widget(Clear, rect);
    let block = overlay_block("TAGS", accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut lines: Vec<Line> = Vec::new();
    let targets_label = format!("targets: {}", picker.targets.len());
    lines.push(Line::from(Span::styled(targets_label, muted_style())));
    let assigned: Vec<String> = picker
        .targets
        .first()
        .and_then(|t| {
            state
                .browser
                .entries
                .iter()
                .find(|e| e.entry.path == *t)
                .map(|e| e.tags.clone())
        })
        .unwrap_or_default();
    for (idx, def) in picker.defs.iter().enumerate() {
        if lines.len() as u16 >= inner.height - 3 {
            break;
        }
        let focused = idx == picker.selected;
        let has = assigned.contains(&def.name);
        let mark = if has { "[x]" } else { "[ ]" };
        let cursor = if focused { ">" } else { " " };
        let style = if focused {
            focused_style()
        } else {
            base_style()
        };
        lines.push(Line::from(vec![
            Span::styled(cursor, style),
            Span::styled(format!("{mark} "), tag_style()),
            Span::styled(pad_right(&def.name, 16), style),
            Span::styled(format!("[{}]", def.display_token), muted_style()),
        ]));
        state.hit_map.push(
            Rect::new(inner.x, inner.y + lines.len() as u16 - 1, inner.width, 1),
            HitTarget::PickerItem(idx),
        );
    }
    if picker.defs.is_empty() {
        lines.push(Line::from(Span::styled(
            "no tags defined, press n",
            muted_style(),
        )));
    }
    lines.push(Line::from(""));
    if let Some(input) = &picker.input {
        lines.push(Line::from(vec![
            Span::styled("new tag: ", base_style()),
            Span::styled(input.clone(), tag_style()),
            Span::styled("_", muted_style()),
        ]));
        frame.render_widget(Paragraph::new(lines), inner);
    } else {
        // Keyboard n/d/Esc still work; buttons carry the same Picker*
        // targets as before (plan item f). Delete keeps DANGER per spec
        // section 7 (word plus confirm flow carry the meaning).
        let list_height = inner.height.saturating_sub(3);
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: list_height,
            },
        );
        let hover = state.hover.control;
        draw_modal_buttons(
            frame,
            inner,
            inner.height.saturating_sub(3),
            &[
                ("New", HitTarget::PickerNew, false),
                ("Delete", HitTarget::PickerDelete, true),
                ("Close", HitTarget::PickerClose, false),
            ],
            hover,
            &mut state.hit_map,
        );
    }
}

fn render_context_menu(
    frame: &mut Frame,
    area: Rect,
    menu: &crate::app::state::ContextMenuState,
    cwd: &Path,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    // Chrome matches every other overlay: SURFACE_3 fill, BORDER_STRONG
    // frame, accent-styled title in the top border (spec section 5).
    let title = truncate(
        &context_menu_title(&menu.target, cwd),
        (MENU_WIDTH - 4) as usize,
    );
    let width = MENU_WIDTH;
    let height = menu.items.len() as u16 + 2;
    let x = menu.x.min(area.width.saturating_sub(width));
    let y = menu.y.min(area.height.saturating_sub(height));
    let rect = Rect::new(
        area.x + x,
        area.y + y,
        width.min(area.width),
        height.min(area.height),
    );
    frame.render_widget(Clear, rect);
    let block = overlay_block(&title, accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    // Flat rows reuse the button state-color vocabulary, not the widget.
    // Precedence per spec section 5: disabled (TEXT_MUTED) beats the
    // Delete DANGER rule beats selected (ACCENT_HOVER + bold) beats plain.
    // The background never varies per row: it inherits the menu fill.
    for (idx, item) in menu.items.iter().enumerate() {
        let selected = idx == menu.selected;
        let cursor = if selected { ">" } else { " " };
        let style = if !item.enabled {
            muted_style()
        } else if item.action == crate::app::state::ContextItem::Delete {
            if selected {
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DANGER)
            }
        } else if selected {
            Style::default()
                .fg(ACCENT_HOVER)
                .add_modifier(Modifier::BOLD)
        } else {
            base_style()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(
                    &format!("{cursor} {}", item.action.label()),
                    inner.width as usize,
                ),
                style,
            ))),
            Rect::new(inner.x, inner.y + idx as u16, inner.width, 1),
        );
        if item.enabled {
            hits.push(
                Rect::new(inner.x, inner.y + idx as u16, inner.width, 1),
                HitTarget::ContextItem(idx),
            );
        }
    }
}

/// Values render_password needs from the password dialog, extracted before
/// the hit map is mutably borrowed.
struct PasswordView {
    purpose: PasswordPurpose,
    confirming: bool,
    target: String,
    input_len: usize,
}

fn render_password(
    frame: &mut Frame,
    area: Rect,
    view: &PasswordView,
    hover: Option<HitTarget>,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let rect = centered_rect(area, 56, 9);
    frame.render_widget(Clear, rect);
    let purpose = view.purpose;
    let confirming = view.confirming;
    let title = match purpose {
        PasswordPurpose::Encrypt => "ENCRYPT",
        PasswordPurpose::Decrypt => "DECRYPT",
    };
    let style = accent_border_style();
    let block = overlay_block(title, style);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let prompt = match (purpose, confirming) {
        (PasswordPurpose::Encrypt, false) => "new password:",
        (PasswordPurpose::Encrypt, true) => "confirm password:",
        (PasswordPurpose::Decrypt, _) => "password:",
    };
    // The password is masked; its length is the only thing rendered.
    let masked = "*".repeat(view.input_len);
    let lines = vec![
        Line::from(Span::styled(
            truncate(&view.target, inner.width as usize),
            base_style(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{prompt} "), base_style()),
            Span::styled(masked, Style::default().fg(TEXT_PRIMARY)),
            Span::styled("_", muted_style()),
        ]),
        Line::from(""),
    ];
    // Keyboard Enter/Esc still work; same ModalConfirm / ModalCancel
    // targets as before (plan item f).
    frame.render_widget(Paragraph::new(lines), inner);
    draw_modal_buttons(
        frame,
        inner,
        4,
        &[
            ("Submit", HitTarget::ModalConfirm, false),
            ("Cancel", HitTarget::ModalCancel, false),
        ],
        hover,
        hits,
    );
}

fn render_open_with(
    frame: &mut Frame,
    area: Rect,
    target: &str,
    input: &str,
    hover: Option<HitTarget>,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let rect = centered_rect(area, 56, 9);
    frame.render_widget(Clear, rect);
    let style = accent_border_style();
    let block = overlay_block("OPEN WITH", style);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines = vec![
        Line::from(Span::styled(
            truncate(target, inner.width as usize),
            base_style(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("command: ", base_style()),
            Span::styled(input.to_string(), Style::default().fg(TEXT_PRIMARY)),
            Span::styled("_", muted_style()),
        ]),
        Line::from(""),
    ];
    // Keyboard Enter/Esc still work; same ModalConfirm / ModalCancel
    // targets as before (plan item f).
    frame.render_widget(Paragraph::new(lines), inner);
    draw_modal_buttons(
        frame,
        inner,
        4,
        &[
            ("Run", HitTarget::ModalConfirm, false),
            ("Cancel", HitTarget::ModalCancel, false),
        ],
        hover,
        hits,
    );
}

fn render_bookmarks(
    frame: &mut Frame,
    area: Rect,
    nav: &BookmarkNavState,
    home: &Path,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let rect = centered_rect(
        area,
        64.min(area.width),
        (nav.matches.len() as u16 + 6).clamp(9, area.height.max(9)),
    );
    frame.render_widget(Clear, rect);
    let block = overlay_block("BOOKMARKS", accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("search: ", base_style()),
        Span::styled(nav.query.clone(), Style::default().fg(TEXT_PRIMARY)),
        Span::styled("_", muted_style()),
    ]));
    lines.push(Line::from(""));
    if nav.matches.is_empty() {
        let text = if nav.query.is_empty() {
            "no bookmarks yet, press Ctrl-b to bookmark the current directory"
        } else {
            "no bookmarks match this search"
        };
        lines.push(Line::from(Span::styled(
            truncate(text, width),
            muted_style(),
        )));
    }
    for (idx, path) in nav.matches.iter().enumerate() {
        if lines.len() as u16 >= inner.height - 2 {
            break;
        }
        let focused = idx == nav.selected;
        let cursor = if focused { ">" } else { " " };
        let cursor_style = if focused {
            focused_style()
        } else {
            base_style()
        };
        let basename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let shown = path
            .strip_prefix(home)
            .map(|rest| format!("~/{rest}", rest = rest.display()))
            .unwrap_or_else(|_| path.display().to_string());
        let text = truncate(
            &format!("{cursor}{} {shown}", pad_right(&basename, 20)),
            width,
        );
        let chars: Vec<char> = text.chars().collect();
        let mut spans: Vec<Span> = Vec::new();
        if let Some(&c) = chars.first() {
            spans.push(Span::styled(c.to_string(), cursor_style));
        }
        let base_end = chars.len().min(1 + 20);
        if base_end > 1 {
            spans.push(Span::styled(
                chars[1..base_end].iter().collect::<String>(),
                if focused {
                    focused_style()
                } else {
                    base_style()
                },
            ));
        }
        if chars.len() > base_end {
            spans.push(Span::styled(
                chars[base_end..].iter().collect::<String>(),
                muted_style(),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" [Enter] go ", accent_border_style()),
        Span::raw(" "),
        Span::styled(" [Esc] close ", base_style()),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_help(frame: &mut Frame, area: Rect, hits: &mut HitMap) {
    push_blocker(area, hits);
    let rect = centered_rect(area, 100.min(area.width), (area.height * 4 / 5).max(16));
    frame.render_widget(Clear, rect);
    let block = overlay_block("HELP", accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let entries: &[(&str, &str)] = &[
        ("j / Down", "next row of tiles"),
        ("k / Up", "previous row of tiles"),
        ("h / Left", "tile left"),
        ("l / Right", "tile right"),
        ("Backspace", "parent directory"),
        ("F5", "refresh current directory"),
        ("e / Enter", "open: folder, media, or open-with"),
        ("r", "open with: prompt for a command to run"),
        ("double left click", "open: enter folder or open file"),
        ("single left click", "select and focus (never opens)"),
        ("g g", "first entry"),
        ("G", "last entry"),
        ("Ctrl-u / Ctrl-d", "half page up / down"),
        ("PageUp / PageDown", "full page up / down"),
        ("Space", "toggle entry selection"),
        ("v", "visual multi-selection mode"),
        (".", "toggle hidden files"),
        ("X", "encrypt / decrypt focused entry"),
        ("b", "toggle sidebar"),
        ("p", "toggle preview panel"),
        ("B", "search bookmarks (fuzzy)"),
        ("Ctrl-b", "bookmark / unbookmark current directory"),
        ("t", "toggle last used tag"),
        ("T", "tag picker and manager"),
        ("/ / Ctrl-f", "filter current directory filenames"),
        (":", "command mode"),
        ("Esc", "cancel mode or modal"),
        ("?", "this help"),
        ("q", "quit"),
        ("Space (media)", "play or pause the focused audio"),
        ("Left/Right (media)", "seek +/-15 seconds"),
        ("Up/Down (media)", "volume up/down"),
        ("f (media)", "toggle fullscreen video"),
        ("n (media)", "next playlist track"),
        ("s (media)", "restart from the beginning"),
        ("Esc/q (media)", "close the media modal"),
        (":copy <dest>", "copy selection"),
        (":move <dest>", "move selection"),
        (":rename <name>", "rename entry"),
        (":delete", "delete selection (confirmed)"),
        (":tag <name>", "assign tag"),
        (":untag <name>", "remove tag"),
        (":tags", "open tag picker"),
        (":open", "open entry"),
        (
            ":open-with <cmd> [args]",
            "run a command on the entry (alias :ow)",
        ),
        (":cd <path>", "change directory"),
        (":mkdir <name>", "create a directory"),
        (":touch <name>", "create an empty file / update mtime"),
        (":selectall", "select every entry"),
        (":invert", "invert the current selection"),
        (":deselect", "clear the current selection"),
        (":sort <field>", "sort by name, size, or modified time"),
        (":refresh", "reload current directory"),
        (":quit", "quit"),
        (":help", "this help"),
        ("mouse right", "context menu (bulk acts on the selection)"),
        (
            "right-click background",
            "menu with paste for copied/cut files",
        ),
        ("mouse wheel", "scroll grid"),
    ];
    let max_rows = inner.height as usize;
    let two_col = inner.width >= 70 && entries.len() > max_rows;
    let mut lines: Vec<Line> = Vec::new();
    if two_col {
        let col_width = (inner.width as usize - 1) / 2;
        let half = entries.len().div_ceil(2);
        for row in 0..half {
            if lines.len() >= max_rows {
                break;
            }
            let mut spans = help_entry_spans(&entries[row], col_width);
            if let Some(second) = entries.get(row + half) {
                spans.push(Span::raw(" "));
                spans.extend(help_entry_spans(second, col_width));
            }
            lines.push(Line::from(spans));
        }
    } else {
        for entry in entries.iter().take(max_rows) {
            lines.push(Line::from(help_entry_spans(entry, inner.width as usize)));
        }
        if entries.len() > max_rows {
            lines.push(Line::from(Span::styled(
                format!("(+{} more, widen window)", entries.len() - max_rows),
                muted_style(),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn help_entry_spans(entry: &(&str, &str), width: usize) -> Vec<Span<'static>> {
    let key_width = 18usize.min(width.saturating_sub(8));
    let desc_width = width.saturating_sub(key_width);
    let desc = pad_right(&truncate(entry.1, desc_width), desc_width);
    vec![
        Span::styled(pad_right(entry.0, key_width), tag_style()),
        Span::styled(desc, base_style()),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::apply_image_encoding_result;
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

/// Render-contract tests for the designer-spec media chrome (spec §1-§5).
#[cfg(test)]
mod media_render_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    use crate::app::state::MediaState;
    use crate::media::{MediaKind, MediaPhase};

    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        line
    }

    fn rendered(state: &mut AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        state.width = width;
        state.height = height;
        let backend = TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, state)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn playing_audio(duration: Option<f64>) -> (AppState, MediaState) {
        let state = AppState::new(PathBuf::from("/"), PathBuf::from("/tmp"));
        let mut media = MediaState::preparing(7, PathBuf::from("/tmp/song.mp3"), MediaKind::Audio);
        media.phase = MediaPhase::Playing;
        media.position = 30.0;
        media.duration = duration;
        (state, media)
    }

    #[test]
    fn audio_modal_shows_chip_time_rail_and_button_labels() {
        use crate::app::state::Mode;
        let (mut state, media) = playing_audio(Some(90.0));
        state.mode = Mode::Media(Box::new(media));
        let buffer = rendered(&mut state, 100, 30);
        let text: String = (0..30)
            .map(|y| format!("{}|", row_text(&buffer, y, 100)))
            .collect();
        assert!(text.contains("PLAYING"), "phase chip: {text}");
        assert!(
            text.contains("00:30 / 01:30 / -01:00 | VOL 100%"),
            "time row format per spec section 1: {text}"
        );
        assert!(text.contains('\u{25CF}'), "rail thumb drawn");
        assert!(text.contains('\u{2501}'), "played rail segment drawn");
        for label in ["-15", "PAUSE", "+15", "NEXT", "V-", "V+", "STOP", "X"] {
            assert!(text.contains(label), "button label {label}: {text}");
        }
        // Wide tier keeps the video-only FULL button out of audio modals.
        assert!(!text.contains("FULL "), "no FULL button on audio");
    }

    #[test]
    fn rail_hit_registered_even_when_duration_is_unknown() {
        use crate::app::state::Mode;
        let (mut state, mut media) = playing_audio(None);
        media.duration = None;
        state.mode = Mode::Media(Box::new(media));
        let _buffer = rendered(&mut state, 100, 30);
        let rail = state.hit_map.rect_for(HitTarget::MediaSeekRail);
        assert!(
            rail.is_some_and(|rect| rect.width > 0),
            "MediaSeekRail must be registered without a duration"
        );
    }

    #[test]
    fn split_tier_lays_two_control_rows_and_pruned_tier_drops_conveniences() {
        use crate::app::state::Mode;
        // Split tier: 40 <= inner.width < 60.
        let (mut state, media) = playing_audio(Some(90.0));
        state.mode = Mode::Media(Box::new(media));
        rendered(&mut state, 50, 24);
        assert!(
            state.hit_map.rect_for(HitTarget::MediaNext).is_some(),
            "split tier registers NEXT"
        );

        // Pruned tier: inner.width < 40 drops NEXT/V-/V+, keeps STOP/X.
        let (mut state, media) = playing_audio(Some(90.0));
        state.mode = Mode::Media(Box::new(media));
        let _buffer = rendered(&mut state, 36, 20);
        assert!(
            state.hit_map.rect_for(HitTarget::MediaNext).is_none(),
            "pruned tier drops NEXT"
        );
        assert!(
            state.hit_map.rect_for(HitTarget::MediaVolumeDown).is_none(),
            "pruned tier drops V-"
        );
        assert!(state.hit_map.rect_for(HitTarget::MediaStop).is_some());
        assert!(state.hit_map.rect_for(HitTarget::MediaClose).is_some());
    }

    #[test]
    fn fullscreen_video_reserves_two_row_strip_and_clears_the_rest() {
        use crate::app::state::Mode;
        let (mut state, mut media) = playing_audio(Some(90.0));
        media.kind = MediaKind::Video;
        media.fullscreen = true;
        state.mode = Mode::Media(Box::new(media));
        let width = 90u16;
        let height = 26u16;
        let buffer = rendered(&mut state, width, height);
        // Surface gets everything above the strip.
        if let Mode::Media(current) = &state.mode {
            let surface = current.surface.expect("surface registered");
            assert_eq!(surface.rect.height, height - 2, "video rect is area-2");
            assert_eq!(surface.rect.y, 0);
        } else {
            panic!("media mode expected");
        }
        // Strip rows hold transport and flat bracket controls.
        let strip_text = format!(
            "{}\n{}",
            row_text(&buffer, height - 2, width),
            row_text(&buffer, height - 1, width)
        );
        assert!(
            strip_text.contains("[PAUSE]"),
            "flat controls: {strip_text}"
        );
        assert!(strip_text.contains("[X]"), "close control: {strip_text}");
        assert!(
            strip_text.contains("-01:00"),
            "remaining time: {strip_text}"
        );
        // The cleared region above the strip must not leak modal borders.
        assert_ne!(
            buffer[(0, 0)].symbol(),
            "+",
            "fullscreen clears windowed chrome"
        );
    }

    #[test]
    fn context_menu_background_titles_cwd_and_disabled_paste_has_no_hit() {
        use crate::app::state::{
            ClipboardState, ContextItem, ContextMenuState, ContextTarget, MenuItem, Mode,
        };
        let mut state = AppState::new(PathBuf::from("/home/u/docs"), PathBuf::from("/"));
        state.mode = Mode::ContextMenu(Box::new(ContextMenuState {
            target: ContextTarget::Background,
            items: vec![MenuItem {
                action: ContextItem::Paste,
                enabled: false,
            }],
            selected: 0,
            x: 2,
            y: 2,
        }));
        state.clipboard = ClipboardState::default();
        let buffer = rendered(&mut state, 60, 20);
        // Title row (menu top border) carries the cwd basename.
        let top = row_text(&buffer, 2, 60);
        assert!(
            top.contains("docs"),
            "background title is cwd basename: {top}"
        );
        // Disabled Paste renders but registers no hit.
        assert!(
            !state
                .hit_map
                .regions
                .iter()
                .any(|(_, t)| matches!(t, HitTarget::ContextItem(_))),
            "disabled paste registers no hit"
        );
    }
}
