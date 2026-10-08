//! Application chrome: the header (brand, layout switcher, help chip), the
//! breadcrumb path bar with live directory stats, the pill status bar, the
//! contextual key legend, and drag feedback. Also lays out the body:
//! sidebar | active layout | preview panel.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::app::reduce::{breadcrumb_segments, footer_focus_text};
use crate::app::state::{AppState, ClipMode, Mode};
use crate::settings::ViewMode;
use crate::ui::anim::{AnimKey, Ease};
use crate::ui::format::{display_width, format_size, truncate};
use crate::ui::glyphs::{Charset, charset, g};
use crate::ui::hit::{HitTarget, LegendAction};
use crate::ui::list::ListOpts;
use crate::ui::palette::*;
use crate::ui::theme::{accent_gradient, mix};
use crate::ui::{
    PREVIEW_WIDTH, SIDEBAR_WIDTH, Tier, control_glow, fill, hover_bg, outline_rect,
    preview_visible, put, sidebar_visible, tier_for,
};

/// Compact/Standard/Wide shell.
pub fn render_shell(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let header = Rect::new(area.x, area.y, area.width, 1);
    let path_bar = Rect::new(area.x, area.y + 1, area.width, 1);
    // The status bar is the LAST terminal row (binding media contract);
    // the legend sits directly above it.
    let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let legend = Rect::new(area.x, area.bottom() - 2, area.width, 1);
    let body = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(4),
    );

    render_header(frame, header, state);
    render_path_bar(frame, path_bar, state);
    render_body(frame, body, state);
    render_legend(frame, legend, state);
    render_status(frame, status, state);
}

fn render_body(frame: &mut Frame, body: Rect, state: &mut AppState) {
    let view = state.view();
    let min_main = 24;
    let show_sidebar = sidebar_visible(state.width, state.height, state.show_sidebar)
        && body.width >= SIDEBAR_WIDTH + min_main + 4;
    let show_preview = view != ViewMode::Columns
        && preview_visible(state.width, state.height, state.show_preview)
        && body.width >= PREVIEW_WIDTH + min_main + 4;

    let mut main = body;
    if show_sidebar {
        let sb = Rect::new(body.x, body.y, SIDEBAR_WIDTH, body.height);
        crate::ui::side::render(frame, sb, state);
        main.x += sb.width;
        main.width -= sb.width;
    } else {
        state.sidebar_items.clear();
    }
    if show_preview {
        let pw = PREVIEW_WIDTH.min(main.width.saturating_sub(min_main));
        let pv = Rect::new(main.right() - pw, main.y, pw, main.height);
        main.width -= pw;
        crate::ui::preview::render_panel(frame, pv, state);
    } else if view != ViewMode::Columns && !matches!(state.mode, Mode::QuickLook(_)) {
        state.preview.key = None;
        state.preview.content = None;
    }
    if view != ViewMode::Columns {
        state.parent_rows.clear();
    }
    match view {
        ViewMode::List => crate::ui::list::render(
            frame,
            main,
            state,
            ListOpts {
                header: true,
                meta: true,
                channel: 0,
            },
        ),
        ViewMode::Grid => crate::ui::grid::render(frame, main, state),
        ViewMode::Columns => crate::ui::columns::render(frame, main, state),
    }
}

/// Narrow shell: one combined header/path row, a compact list, the legend
/// and the status row. No sidebar or preview: there is no room.
pub fn render_narrow_shell(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let top = Rect::new(area.x, area.y, area.width, 1);
    let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let legend = Rect::new(area.x, area.bottom() - 2, area.width, 1);
    let body = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(3),
    );
    fill(frame.buffer_mut(), top, SURFACE_2);
    let buf = frame.buffer_mut();
    let mut x = top.x + 1;
    x += put(
        buf,
        x,
        top.y,
        g().pill_left,
        1,
        Style::default().fg(ACCENT).bg(SURFACE_2),
    );
    let cwd = state.browser.cwd.display().to_string();
    let budget = top.width.saturating_sub(x - top.x + 2);
    put(
        buf,
        x,
        top.y,
        &truncate(&cwd, budget as usize),
        budget,
        Style::default()
            .fg(TEXT_PRIMARY)
            .bg(SURFACE_2)
            .add_modifier(Modifier::BOLD),
    );
    state.sidebar_items.clear();
    state.parent_rows.clear();
    if !matches!(state.mode, Mode::QuickLook(_)) {
        state.preview.key = None;
        state.preview.content = None;
    }
    crate::ui::list::render(
        frame,
        body,
        state,
        ListOpts {
            header: false,
            meta: false,
            channel: 0,
        },
    );
    render_legend(frame, legend, state);
    render_status(frame, status, state);
}

// --- Header --------------------------------------------------------------

fn render_header(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_2);
    let buf = frame.buffer_mut();
    let mut x = area.x + 1;
    let mark = if charset() == Charset::Ascii {
        "<>"
    } else {
        "◢◤"
    };
    x += put(
        buf,
        x,
        area.y,
        mark,
        2,
        Style::default()
            .fg(ACCENT)
            .bg(SURFACE_2)
            .add_modifier(Modifier::BOLD),
    );
    x += 1;
    for (i, ch) in "tui".chars().enumerate() {
        x += put(
            buf,
            x,
            area.y,
            &ch.to_string(),
            1,
            Style::default()
                .fg(accent_gradient(i as f32 / 2.0))
                .bg(SURFACE_2)
                .add_modifier(Modifier::BOLD),
        );
    }
    x += put(
        buf,
        x,
        area.y,
        "·explorer",
        9,
        Style::default()
            .fg(TEXT_PRIMARY)
            .bg(SURFACE_2)
            .add_modifier(Modifier::BOLD),
    );
    let version = format!(" {}", env!("CARGO_PKG_VERSION"));
    x += put(
        buf,
        x,
        area.y,
        &version,
        display_width(&version) as u16,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_2),
    );
    let brand_end = x;

    // Right cluster: help chip, then the layout switcher to its left.
    let help_label = " ? help ";
    let help_w = display_width(help_label) as u16;
    let help_x = area.right().saturating_sub(help_w + 1);
    let glow = control_glow(state, HitTarget::HelpChip);
    let help_rect = Rect::new(help_x, area.y, help_w, 1);
    put(
        frame.buffer_mut(),
        help_x,
        area.y,
        help_label,
        help_w,
        Style::default()
            .fg(mix(ACCENT_HOVER, INK_ON_ACCENT, glow))
            .bg(mix(SURFACE_3, ACCENT, glow))
            .add_modifier(Modifier::BOLD),
    );
    if help_x > brand_end {
        state.hit_map.push(help_rect, HitTarget::HelpChip);
    }

    let modes = [ViewMode::List, ViewMode::Grid, ViewMode::Columns];
    let ascii = charset() == Charset::Ascii;
    let labels: Vec<String> = modes
        .iter()
        .map(|m| {
            let icon = match (m, ascii) {
                (ViewMode::List, false) => "≡",
                (ViewMode::Grid, false) => "⊞",
                (ViewMode::Columns, false) => "▥",
                (ViewMode::List, true) => "=",
                (ViewMode::Grid, true) => "#",
                (ViewMode::Columns, true) => "|",
            };
            format!(" {icon} {} ", m.label())
        })
        .collect();
    let total: u16 = labels.iter().map(|l| display_width(l) as u16).sum();
    let mut sx = help_x.saturating_sub(total + 2);
    render_tabs(frame, area, state, brand_end + 2, sx.saturating_sub(1));
    if sx <= brand_end + 2 {
        return;
    }
    let active = state.view();
    for (mode, label) in modes.iter().zip(labels.iter()) {
        let w = display_width(label) as u16;
        let target = HitTarget::ViewSwitch(*mode);
        let glow = control_glow(state, target);
        let is_active = *mode == active;
        let sel = state.anim.track_with(
            AnimKey::Indexed("view-switch", *mode as u32),
            if is_active { 1.0 } else { 0.0 },
            crate::ui::anim::SELECT,
            crate::ui::anim::SELECT,
            Ease::OutCubic,
        );
        let bg = mix(hover_bg(SURFACE_3, glow), ACCENT, sel);
        let fg = mix(mix(TEXT_SECONDARY, TEXT_PRIMARY, glow), INK_ON_ACCENT, sel);
        let mut style = Style::default().fg(fg).bg(bg);
        if is_active {
            style = style.add_modifier(Modifier::BOLD);
        }
        put(frame.buffer_mut(), sx, area.y, label, w, style);
        state.hit_map.push(Rect::new(sx, area.y, w, 1), target);
        sx += w;
    }
}

/// Tab chips (only with two or more tabs): number and folder name; the
/// active tab is a solid accent chip.
fn render_tabs(frame: &mut Frame, area: Rect, state: &mut AppState, left: u16, right: u16) {
    if state.tabs.len() < 2 || right <= left + 4 {
        return;
    }
    let names: Vec<String> = (0..state.tabs.len())
        .map(|i| {
            let cwd = if i == state.active_tab {
                &state.browser.cwd
            } else {
                &state.tabs[i].browser.cwd
            };
            cwd.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".to_string())
        })
        .collect();
    let mut x = left;
    for (i, name) in names.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, truncate(name, 14));
        let w = display_width(&label) as u16;
        if x + w > right {
            let more = format!(" +{} ", names.len() - i);
            put(
                frame.buffer_mut(),
                x,
                area.y,
                &more,
                right.saturating_sub(x),
                Style::default().fg(TEXT_MUTED).bg(SURFACE_2),
            );
            break;
        }
        let target = HitTarget::Tab(i);
        let glow = control_glow(state, target);
        let active = i == state.active_tab;
        let sel = state.anim.track(
            AnimKey::Indexed("tab", i as u32),
            if active { 1.0 } else { 0.0 },
            crate::ui::anim::SELECT,
        );
        let bg = mix(hover_bg(SURFACE_3, glow), ACCENT, sel);
        let fg = mix(mix(TEXT_SECONDARY, TEXT_PRIMARY, glow), INK_ON_ACCENT, sel);
        let mut style = Style::default().fg(fg).bg(bg);
        if active {
            style = style.add_modifier(Modifier::BOLD);
        }
        put(frame.buffer_mut(), x, area.y, &label, w, style);
        state.hit_map.push(Rect::new(x, area.y, w, 1), target);
        x += w + 1;
    }
}

// --- Path bar ------------------------------------------------------------

fn render_address_bar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let buf = frame.buffer_mut();
    let used = pill(
        buf,
        area.x,
        area.y,
        &format!(" {} GO ", g().arrow_right),
        (INK_ON_ACCENT, ACCENT, SURFACE_1),
        area.right(),
    );
    let text = state
        .command_input
        .strip_prefix("cd ")
        .unwrap_or(&state.command_input)
        .to_string();
    let field = Rect::new(area.x + used, area.y, area.width.saturating_sub(used), 1);
    crate::ui::overlays::input_field(
        buf,
        field,
        "",
        &text,
        text.chars().count(),
        "a folder, ~/path, file:// or https:// link",
    );
}

fn render_path_bar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_1);
    if state.address_bar && matches!(state.mode, Mode::Command) {
        render_address_bar(frame, area, state);
        return;
    }
    // The whole bar opens the address bar; segments (pushed later) win.
    state.hit_map.push(area, HitTarget::PathBar);
    let info = path_info(state);
    let info_w = display_width(&info) as u16;
    let crumbs_end = area.right().saturating_sub(info_w + 2);

    let segments = breadcrumb_segments(&state.browser.cwd);
    let sep = format!(" {} ", g().sep);
    let sep_w = display_width(&sep) as u16;
    let seg_w = |label: &str| display_width(label) as u16 + 2;
    let avail = crumbs_end.saturating_sub(area.x + 1);
    let mut start = 0usize;
    let total = |from: usize| -> u16 {
        segments[from..]
            .iter()
            .map(|(_, l)| seg_w(l) + sep_w)
            .sum::<u16>()
    };
    let ellipsis = format!("{}{}", g().ellipsis, sep);
    if total(0) > avail {
        start = 1;
        while start + 1 < segments.len() && total(start) + display_width(&ellipsis) as u16 > avail {
            start += 1;
        }
    }
    let mut x = area.x + 1;
    if start > 0 {
        x += put(
            frame.buffer_mut(),
            x,
            area.y,
            &ellipsis,
            avail,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_1),
        );
    }
    for (idx, (_, label)) in segments.iter().enumerate().skip(start) {
        let last = idx == segments.len() - 1;
        let text = format!(" {label} ");
        let w = seg_w(label);
        if x + w > crumbs_end {
            break;
        }
        let target = HitTarget::Breadcrumb(idx);
        let glow = control_glow(state, target);
        let (fg, bg) = if last {
            (INK_ON_ACCENT, mix(ACCENT, ACCENT_HOVER, glow))
        } else {
            (
                mix(TEXT_SECONDARY, TEXT_PRIMARY, glow),
                hover_bg(SURFACE_1, glow),
            )
        };
        let mut style = Style::default().fg(fg).bg(bg);
        if last || glow > 0.5 {
            style = style.add_modifier(Modifier::BOLD);
        }
        put(frame.buffer_mut(), x, area.y, &text, w, style);
        state.hit_map.push(Rect::new(x, area.y, w, 1), target);
        x += w;
        if !last {
            x += put(
                frame.buffer_mut(),
                x,
                area.y,
                &sep,
                sep_w,
                Style::default().fg(ACCENT_DEEP).bg(SURFACE_1),
            );
        }
    }

    let buf = frame.buffer_mut();
    let ix = area.right().saturating_sub(info_w + 1);
    if ix > x {
        put(
            buf,
            ix,
            area.y,
            &info,
            info_w,
            Style::default().fg(TEXT_MUTED).bg(SURFACE_1),
        );
        for chip in info_chips(state) {
            if let Some(off) = info.find(&chip) {
                let cx = ix + display_width(&info[..off]) as u16;
                put(
                    buf,
                    cx,
                    area.y,
                    &chip,
                    display_width(&chip) as u16,
                    Style::default().fg(ACCENT_SOFT).bg(SURFACE_1),
                );
            }
        }
    }
}

fn info_chips(state: &AppState) -> Vec<String> {
    let mut chips = Vec::new();
    if let Some(filter) = &state.browser.filter {
        chips.push(format!("Filter: {filter}"));
    }
    if let Some(search) = &state.browser.search {
        chips.push(format!("{} {search}", g().search));
    }
    if state.browser.show_hidden {
        chips.push(".*".to_string());
    }
    chips
}

/// "12 items · 3 dirs · 4.2M · Sort: name ▲" plus filter/search chips.
fn path_info(state: &AppState) -> String {
    let total = state.browser.visible_len();
    let listed = state.browser.listed_len();
    let dirs = state
        .browser
        .visible_entries()
        .filter(|(_, e)| e.entry.is_dir_like())
        .count();
    let bytes: u64 = state
        .browser
        .visible_entries()
        .filter(|(_, e)| !e.entry.is_dir_like())
        .map(|(_, e)| e.entry.size)
        .sum();
    let count = if state.browser.filter.is_some() {
        format!("{total}/{listed} items")
    } else {
        format!("{total} item{}", if total == 1 { "" } else { "s" })
    };
    let arrow = if state.browser.sort_mode.desc {
        g().arrow_down
    } else {
        g().arrow_up
    };
    let mut parts = info_chips(state);
    parts.push(count);
    if dirs > 0 {
        parts.push(format!("{dirs} dir{}", if dirs == 1 { "" } else { "s" }));
    }
    if bytes > 0 {
        parts.push(format_size(bytes));
    }
    parts.push(format!("Sort: {} {arrow}", state.browser.sort_mode.label()));
    parts.join(&format!(" {} ", g().dot))
}

// --- Status bar ----------------------------------------------------------

/// A rounded "pill" label: half-block caps in `bg` around bold text.
/// `colors` is (text, fill, bar background).
fn pill(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    colors: (Color, Color, Color),
    max_right: u16,
) -> u16 {
    let (fg, bg, bar) = colors;
    if x >= max_right {
        return 0;
    }
    let mut used = put(buf, x, y, g().pill_left, 1, Style::default().fg(bg).bg(bar));
    used += put(
        buf,
        x + used,
        y,
        text,
        max_right.saturating_sub(x + used + 1),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    );
    used += put(
        buf,
        x + used,
        y,
        g().pill_right,
        1,
        Style::default().fg(bg).bg(bar),
    );
    used
}

pub fn render_status(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_2);
    if matches!(state.mode, Mode::Command) && state.address_bar {
        let used = pill(
            frame.buffer_mut(),
            area.x,
            area.y,
            " ADDRESS ",
            (INK_ON_ACCENT, ACCENT, SURFACE_2),
            area.right(),
        );
        put(
            frame.buffer_mut(),
            area.x + used + 1,
            area.y,
            "type or paste a path or link · Tab completes · Enter goes · Esc cancels",
            area.width.saturating_sub(used + 1),
            Style::default().fg(TEXT_MUTED).bg(SURFACE_2),
        );
        return;
    }
    if matches!(state.mode, Mode::Command) {
        render_command_line(frame, area, state);
        return;
    }
    if matches!(state.mode, Mode::Rename(_) | Mode::Search(_)) {
        render_prompt_line(frame, area, state);
        return;
    }
    let y = area.y;
    let mode_name = format!(" {} ", state.mode_name());
    let used = pill(
        frame.buffer_mut(),
        area.x,
        y,
        &mode_name,
        (INK_ON_ACCENT, ACCENT, SURFACE_2),
        area.right(),
    );
    let mut x = area.x + used + 1;

    // Right cluster first so the left text knows its budget.
    let len = state.browser.visible_len();
    let pos = if len == 0 {
        "0/0".to_string()
    } else {
        format!("{}/{}", state.browser.selected + 1, len)
    };
    let pct = (state.browser.selected + 1)
        .checked_mul(100)
        .and_then(|v| v.checked_div(len))
        .map(|p| format!("{p}%"))
        .unwrap_or_else(|| "0%".to_string());
    let size = state
        .browser
        .focused()
        .filter(|v| !v.entry.is_dir_like())
        .map(|v| format_size(v.entry.size))
        .unwrap_or_default();
    let metrics = format!("{size}  {pct}  {pos} ");
    let metrics_w = display_width(&metrics) as u16;
    let mut right = area.right().saturating_sub(metrics_w);
    put(
        frame.buffer_mut(),
        right,
        y,
        &metrics,
        metrics_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_2),
    );
    if let Some(chip) = state.clipboard.chip() {
        let text = format!(" {chip} ");
        let w = display_width(&text) as u16 + 2;
        if right > x + w + 2 {
            right -= w + 1;
            let fg = match state.clipboard.mode {
                Some(ClipMode::Cut) => ACCENT_HOVER,
                _ => ACCENT,
            };
            pill(
                frame.buffer_mut(),
                right,
                y,
                &text,
                (fg, SURFACE_3, SURFACE_2),
                area.right(),
            );
        }
    }
    if let Some(mini) = state.mini.as_deref().cloned()
        && !matches!(state.mode, Mode::Media(_))
    {
        let width = 52u16.min(right.saturating_sub(x + 12));
        if width >= 24 {
            right -= width + 1;
            render_mini_player(frame, Rect::new(right, y, width, 1), state, &mini);
        }
    }

    let budget = right.saturating_sub(x + 1);
    if let Some(op) = state.operation.clone() {
        let label = format!("{:?} {}/{} ", op.kind, op.done, op.total);
        x += put(
            frame.buffer_mut(),
            x,
            y,
            &label,
            budget,
            Style::default()
                .fg(ACCENT_HOVER)
                .bg(SURFACE_2)
                .add_modifier(Modifier::BOLD),
        );
        let ratio = if op.total == 0 {
            0.0
        } else {
            op.done as f64 / op.total as f64
        };
        let shown = state.anim.track(
            AnimKey::Named("op-meter"),
            ratio as f32,
            crate::ui::anim::SELECT,
        );
        let meter_w = 12u16.min(right.saturating_sub(x + 2));
        if meter_w > 2 {
            let (filled, rest) = crate::ui::glyphs::meter(meter_w as usize, f64::from(shown));
            let w = put(
                frame.buffer_mut(),
                x,
                y,
                &filled,
                meter_w,
                Style::default().fg(ACCENT).bg(SURFACE_3),
            );
            put(
                frame.buffer_mut(),
                x + w,
                y,
                &" ".repeat(rest),
                rest as u16,
                Style::default().bg(SURFACE_3),
            );
            x += meter_w + 1;
        }
        let current = op
            .current
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        put(
            frame.buffer_mut(),
            x,
            y,
            &truncate(&current, right.saturating_sub(x + 1) as usize),
            right.saturating_sub(x + 1),
            Style::default().fg(TEXT_SECONDARY).bg(SURFACE_2),
        );
        state.anim.keep_alive();
    } else if let Some(message) = state.message.clone() {
        let text = if message.is_error {
            format!("[!] {}", message.text)
        } else {
            format!("{} {}", g().info, message.text)
        };
        // New messages flash in: their background eases from the accent.
        let key = AnimKey::Indexed(
            "msg",
            (crate::ui::anim::path_key(std::path::Path::new(&message.text)) & 0xffff_ffff) as u32,
        );
        let flash = state
            .anim
            .enter(key, 1.0, 0.0, std::time::Duration::from_millis(700));
        let tone = if message.is_error { DANGER } else { ACCENT };
        let bg = mix(SURFACE_2, mix(SURFACE_2, tone, 0.35), flash);
        let style = if message.is_error {
            Style::default()
                .fg(DANGER)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT_SECONDARY).bg(bg)
        };
        let shown = truncate(&text, budget as usize);
        put(frame.buffer_mut(), x, y, &shown, budget, style);
        if !message.is_error {
            put(
                frame.buffer_mut(),
                x,
                y,
                g().info,
                1,
                Style::default().fg(ACCENT).bg(bg),
            );
        }
    } else if let Some(text) = footer_focus_text(state) {
        put(
            frame.buffer_mut(),
            x,
            y,
            &truncate(&text, budget as usize),
            budget,
            Style::default().fg(ACCENT_SOFT).bg(SURFACE_2),
        );
    }
}

fn render_command_line(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let buf = frame.buffer_mut();
    let used = pill(
        buf,
        area.x,
        area.y,
        " : ",
        (INK_ON_ACCENT, ACCENT, SURFACE_2),
        area.right(),
    );
    let x = area.x + used + 1;
    let input = state.command_input.clone();
    let budget = area.right().saturating_sub(x + 1);
    let shown_w = put(
        buf,
        x,
        area.y,
        &input,
        budget,
        Style::default().fg(TEXT_PRIMARY).bg(SURFACE_2),
    );
    let cx = x + shown_w;
    if cx < area.right() {
        put(buf, cx, area.y, " ", 1, Style::default().bg(ACCENT_HOVER));
    }
}

/// Status-bar mini player: transport buttons, title (click expands) and
/// an eased progress meter.
fn render_mini_player(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::media::MediaPhase;
    let bg = SURFACE_3;
    fill(frame.buffer_mut(), area, bg);
    let mut x = area.x;
    let note = if charset() == Charset::Ascii {
        "~"
    } else {
        "♪"
    };
    x += put(
        frame.buffer_mut(),
        x,
        area.y,
        &format!(" {note} "),
        3,
        Style::default()
            .fg(INK_ON_ACCENT)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    let playing = media.phase == MediaPhase::Playing;
    let buttons = [
        (g().prev, HitTarget::MediaPrev),
        (
            if playing { g().pause } else { g().play },
            HitTarget::MediaTogglePause,
        ),
        (g().next, HitTarget::MediaNext),
    ];
    for (label, target) in buttons {
        let text = format!(" {label} ");
        let w = display_width(&text) as u16;
        let glow = control_glow(state, target);
        put(
            frame.buffer_mut(),
            x,
            area.y,
            &text,
            w,
            Style::default()
                .fg(mix(ACCENT_HOVER, INK_ON_ACCENT, glow))
                .bg(mix(bg, ACCENT, glow))
                .add_modifier(Modifier::BOLD),
        );
        state.hit_map.push(Rect::new(x, area.y, w, 1), target);
        x += w;
    }
    x += 1;
    let time = match media.duration {
        Some(total) => format!(
            " {} / {} ",
            crate::ui::format::format_clock(media.position),
            crate::ui::format::format_clock(total)
        ),
        None => format!(" {} ", crate::ui::format::format_clock(media.position)),
    };
    let time_w = display_width(&time) as u16;
    let meter_w = 8u16;
    let title_w = area.right().saturating_sub(x + meter_w + time_w + 1);
    let title_rect = Rect::new(x, area.y, title_w, 1);
    let glow = control_glow(state, HitTarget::MiniPlayer);
    put(
        frame.buffer_mut(),
        x,
        area.y,
        &truncate(&media.title(), title_w as usize),
        title_w,
        Style::default()
            .fg(mix(TEXT_PRIMARY, ACCENT_HOVER, glow))
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    );
    state.hit_map.push(title_rect, HitTarget::MiniPlayer);
    x += title_w + 1;
    let ratio = media
        .duration
        .filter(|d| *d > 0.0)
        .map(|d| (media.position / d).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let shown = state.anim.track(
        AnimKey::Named("mini-progress"),
        ratio as f32,
        crate::ui::anim::SELECT,
    );
    let (filled, rest) = crate::ui::glyphs::meter(meter_w as usize, f64::from(shown));
    let w = put(
        frame.buffer_mut(),
        x,
        area.y,
        &filled,
        meter_w,
        Style::default().fg(ACCENT).bg(SURFACE_1),
    );
    put(
        frame.buffer_mut(),
        x + w,
        area.y,
        &" ".repeat(rest),
        rest as u16,
        Style::default().bg(SURFACE_1),
    );
    put(
        frame.buffer_mut(),
        x + meter_w,
        area.y,
        &time,
        time_w,
        Style::default().fg(TEXT_SECONDARY).bg(bg),
    );
    state.hit_map.push(
        Rect::new(x, area.y, meter_w + time_w, 1),
        HitTarget::MiniPlayer,
    );
}

/// Inline rename / search / find / filter prompt in the status row.
fn render_prompt_line(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let (label, edit, info) = match &state.mode {
        Mode::Rename(r) => {
            let name = r
                .target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (" RENAME ", r.edit.clone(), format!("was {name} "))
        }
        Mode::Search(s) => {
            let label = match s.kind {
                crate::app::state::SearchKind::Search => " / ",
                crate::app::state::SearchKind::Find => " FIND ",
                crate::app::state::SearchKind::Filter => " FILTER ",
            };
            let info = match s.kind {
                crate::app::state::SearchKind::Filter => {
                    format!("{} shown ", state.browser.visible_len())
                }
                _ if s.edit.text.is_empty() => String::new(),
                _ => {
                    let n = crate::app::ranger::search_matches(state).len();
                    match n {
                        0 => "no matches ".to_string(),
                        1 => "1 match ".to_string(),
                        n => format!("{n} matches "),
                    }
                }
            };
            (label, s.edit.clone(), info)
        }
        _ => return,
    };
    let buf = frame.buffer_mut();
    let used = pill(
        buf,
        area.x,
        area.y,
        label,
        (INK_ON_ACCENT, ACCENT, SURFACE_2),
        area.right(),
    );
    let info_w = display_width(&info) as u16;
    put(
        buf,
        area.right().saturating_sub(info_w),
        area.y,
        &info,
        info_w,
        Style::default().fg(TEXT_MUTED).bg(SURFACE_2),
    );
    let field = Rect::new(
        area.x + used,
        area.y,
        area.width.saturating_sub(used + info_w + 1),
        1,
    );
    crate::ui::overlays::input_field(buf, field, "", &edit.text, edit.cursor, "");
}

// --- Legend --------------------------------------------------------------

fn legend_items(
    state: &AppState,
    tier: Tier,
) -> Vec<(&'static str, &'static str, Option<LegendAction>)> {
    match &state.mode {
        Mode::Command => vec![
            ("Enter", "run", None),
            ("Tab", "complete", None),
            ("Esc", "cancel", Some(LegendAction::Cancel)),
        ],
        Mode::Confirm(_) => vec![("y", "confirm", None), ("n", "cancel", None)],
        Mode::Conflict(_) => vec![
            ("c", "cancel", None),
            ("s", "skip", None),
            ("r", "replace", None),
            ("k", "keep both", None),
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
            ("Tab", "suggestion", None),
            ("C-r", "remember", None),
            ("Esc", "cancel", Some(LegendAction::Cancel)),
        ],
        Mode::Bookmarks(_) => vec![
            ("Enter", "go", None),
            ("Tab", "section", None),
            ("Del", "remove", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::Help => vec![
            ("type", "search", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::Media(media) if media.sub_picker.is_some() => vec![
            ("Enter", "load", None),
            ("type", "filter", None),
            ("Esc", "back", None),
        ],
        Mode::Media(media) => {
            let mut items = vec![
                ("Space", "play/pause", None),
                ("←→", "seek", None),
                ("↑↓", "volume", None),
                ("m", "mute", None),
                ("n/p", "next/prev", None),
            ];
            if media.kind == crate::media::MediaKind::Video {
                items.push(("c", "subtitles", None));
                items.push(("f", "fullscreen", None));
                items.push(("z/Z", "sub delay", None));
            } else {
                items.push(("x", "shuffle", None));
                items.push(("r", "repeat", None));
                items.push(("Esc", "background", None));
            }
            items.push(("[ ]", "speed", None));
            items.push(("0-9", "jump", None));
            items.push(("q", "stop", None));
            items
        }
        Mode::Rename(_) => vec![
            ("Enter", "rename", None),
            ("C-w", "delete word", None),
            ("C-u", "clear", None),
            ("Esc", "cancel", Some(LegendAction::Cancel)),
        ],
        Mode::Search(search) => match search.kind {
            crate::app::state::SearchKind::Search => vec![
                ("Enter", "done", None),
                ("Tab", "next match", None),
                ("S-Tab", "previous", None),
                ("Esc", "cancel", Some(LegendAction::Cancel)),
            ],
            crate::app::state::SearchKind::Find => vec![
                ("Enter", "open", None),
                ("type", "jump", None),
                ("Esc", "cancel", Some(LegendAction::Cancel)),
            ],
            crate::app::state::SearchKind::Filter => vec![
                ("Enter", "keep filter", None),
                ("Esc", "clear", Some(LegendAction::Cancel)),
            ],
        },
        Mode::QuickLook(_) => vec![
            ("j/k", "scroll", None),
            ("e", "edit", None),
            ("r", "open with", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::Results(_) => vec![
            ("Enter", "jump to", None),
            ("type", "filter", None),
            ("Esc", "close", Some(LegendAction::Cancel)),
        ],
        Mode::Browser => {
            let focused = state.browser.focused();
            let open_label = match focused {
                Some(v) if v.entry.is_dir_like() => "open",
                Some(v) if crate::media::classify_path(&v.entry.path).is_some() => "play",
                _ => "open",
            };
            let mut items = vec![
                ("Enter", open_label, Some(LegendAction::Open)),
                ("Space", "select", Some(LegendAction::Select)),
                ("/", "search", Some(LegendAction::Search)),
                (":", "command", Some(LegendAction::Command)),
            ];
            if !state.clipboard.is_empty() {
                items.push(("pp", "paste", Some(LegendAction::Paste)));
            }
            items.push(("Bsp", "up", Some(LegendAction::Parent)));
            if tier != Tier::Narrow {
                items.push(("zv", "layout", Some(LegendAction::View)));
                items.push(("B", "bookmarks", Some(LegendAction::Bookmarks)));
                items.push(("i", "quick look", None));
                items.push(("r", "open with", Some(LegendAction::OpenWith)));
                items.push(("cw", "rename", None));
                items.push(("+", "new", None));
                items.push(("t", "tag", Some(LegendAction::QuickTag)));
                items.push(("X", "crypt", Some(LegendAction::Encrypt)));
                items.push(("b", "sidebar", Some(LegendAction::Sidebar)));
                items.push(("zp", "preview", Some(LegendAction::Preview)));
            }
            items.push(("?", "help", Some(LegendAction::Help)));
            items.push(("q", "quit", Some(LegendAction::Quit)));
            items
        }
    }
}

pub fn render_legend(frame: &mut Frame, area: Rect, state: &mut AppState) {
    fill(frame.buffer_mut(), area, SURFACE_0);
    let tier = tier_for(state.width, state.height);
    let items = legend_items(state, tier);
    let mut x = area.x + 1;
    for (key, label, action) in items {
        let key_text = format!(" {key} ");
        let label_text = format!(" {label} ");
        let w = display_width(&key_text) as u16 + display_width(&label_text) as u16;
        if x + w + 1 > area.right() {
            break;
        }
        let glow = match action {
            Some(a) => control_glow(state, HitTarget::Legend(a)),
            None => 0.0,
        };
        let buf = frame.buffer_mut();
        let kw = put(
            buf,
            x,
            area.y,
            &key_text,
            w,
            Style::default()
                .fg(mix(ACCENT_HOVER, INK_ON_ACCENT, glow))
                .bg(mix(SURFACE_3, ACCENT, glow))
                .add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            x + kw,
            area.y,
            &label_text,
            w - kw,
            Style::default()
                .fg(mix(TEXT_MUTED, TEXT_PRIMARY, glow))
                .bg(SURFACE_0),
        );
        if let Some(action) = action {
            state
                .hit_map
                .push(Rect::new(x, area.y, w, 1), HitTarget::Legend(action));
        }
        x += w + 1;
    }
}

// --- Drag feedback -------------------------------------------------------

pub fn render_drag_feedback(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let Some(drag) = state.drag.clone() else {
        return;
    };
    if !matches!(state.mode, Mode::Browser) {
        return;
    }
    let (cx, cy) = drag.cursor;
    let valid = crate::app::reduce::drag_drop_target_for_ui(state, cx, cy);
    let target_style = if valid.is_some() {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
    };
    if valid.is_some() || state.hit_map.hit(cx, cy).is_some() {
        if let Some(rect) = hovered_row_rect(state, cx, cy) {
            outline_rect(frame, rect, target_style);
        }
    }
    let count = drag.sources.len();
    let ghost = format!(
        " {} moving {count} item{} ",
        g().arrow_right,
        if count == 1 { "" } else { "s" }
    );
    let ghost_w = display_width(&ghost) as u16;
    let gx = cx.saturating_add(2).min(area.width.saturating_sub(ghost_w));
    let gy = cy.saturating_sub(1).min(area.height.saturating_sub(1));
    put(
        frame.buffer_mut(),
        gx,
        gy,
        &ghost,
        ghost_w,
        Style::default()
            .bg(ACCENT)
            .fg(INK_ON_ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    let hint = " drop on a folder to move · Esc cancels";
    let status_y = area.bottom().saturating_sub(2);
    put(
        frame.buffer_mut(),
        area.x + 1,
        status_y,
        hint,
        area.width.saturating_sub(2),
        Style::default().fg(TEXT_SECONDARY).bg(SURFACE_0),
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
