//! Modal overlays: confirmations, conflicts, the tag picker, context
//! menus, password and open-with prompts, the bookmark navigator and help.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::state::{AppState, PasswordPurpose};
use crate::ui::format::{pad_right, truncate};
use crate::ui::hit::{HitMap, HitTarget};
use crate::ui::palette::*;
use crate::ui::widgets::{Button, ButtonState, button_row, draw_button};
use crate::ui::{
    accent_border_style, base_style, error_style, focused_style, muted_style, overlay_block,
    tag_style,
};

/// Draws the overlay for the current mode (if any). Modals ease in by
/// sliding up two rows as the scrim fades in.
pub(crate) fn render_modal(frame: &mut Frame, area: Rect, state: &mut AppState) {
    use crate::app::state::Mode;
    if !state.mode.is_overlay() {
        return;
    }
    let t = state.anim.enter(
        crate::ui::anim::AnimKey::Named("modal"),
        0.0,
        1.0,
        crate::ui::anim::MODAL,
    );
    let dy = ((1.0 - t) * 2.0).round() as u16;
    state.hit_map.push(area, HitTarget::Blocker);
    let shifted = Rect::new(
        area.x,
        area.y + dy,
        area.width,
        area.height.saturating_sub(dy),
    );
    let hovered_control = state.hover.control;
    match &state.mode {
        Mode::Confirm(confirm) => {
            let confirm = confirm.clone();
            render_confirm(
                frame,
                shifted,
                &confirm,
                hovered_control,
                &mut state.hit_map,
            )
        }
        Mode::Conflict(conflict) => {
            let conflict = conflict.clone();
            render_conflict(
                frame,
                shifted,
                &conflict,
                hovered_control,
                &mut state.hit_map,
            )
        }
        Mode::TagPicker(picker) => {
            let picker = picker.clone();
            render_picker(frame, shifted, state, &picker);
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
            render_password(frame, shifted, &view, hovered_control, &mut state.hit_map);
        }
        Mode::OpenWith(dialog) => {
            let dialog = dialog.as_ref().clone();
            crate::ui::overlays::render_open_with(frame, shifted, state, &dialog);
            draw_modal_buttons(
                frame,
                open_with_buttons_row(shifted),
                0,
                &[
                    ("Run", HitTarget::ModalConfirm, false),
                    ("Cancel", HitTarget::ModalCancel, false),
                ],
                hovered_control,
                &mut state.hit_map,
            );
        }
        Mode::Bookmarks(nav) => {
            let nav = nav.as_ref().clone();
            crate::ui::overlays::render_hub(frame, shifted, state, &nav);
        }
        Mode::Help => crate::ui::overlays::render_help(frame, shifted, state),
        Mode::Media(media) => {
            let media = media.clone();
            crate::ui::media::render_media_modal(frame, area, state, &media);
            if let Some(picker) = &media.sub_picker {
                crate::ui::overlays::render_sub_picker(frame, area, state, picker);
            }
        }
        Mode::QuickLook(_) => crate::ui::overlays::render_quick_look(frame, shifted, state),
        Mode::Results(results) => {
            let results = results.as_ref().clone();
            crate::ui::overlays::render_results(frame, shifted, state, &results);
        }
        Mode::Browser | Mode::Command | Mode::Rename(_) | Mode::Search(_) => {}
    }
}

pub(crate) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
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
    crate::ui::drop_shadow(frame.buffer_mut(), rect);
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
    crate::ui::drop_shadow(frame.buffer_mut(), rect);
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
            ("Keep both", HitTarget::ConflictKeepBoth, false),
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
    crate::ui::drop_shadow(frame.buffer_mut(), rect);
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
    crate::ui::drop_shadow(frame.buffer_mut(), rect);
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
    crate::ui::drop_shadow(frame.buffer_mut(), rect);
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

/// Row (inside the open-with modal) where its Run / Cancel buttons sit,
/// to the right of the remember toggle.
fn open_with_buttons_row(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(30, 72);
    let rect = centered_rect(area, width, crate::ui::overlays::OPEN_WITH_HEIGHT);
    // Inner area minus padding; buttons occupy rows 7..10 of the inner box.
    let x = rect.x + 2;
    let w = rect.width.saturating_sub(4);
    Rect::new(x + w.saturating_sub(24), rect.y + 7, 24.min(w), 3)
}
