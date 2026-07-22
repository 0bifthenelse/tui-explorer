pub mod format;
pub mod hit;

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

use crate::app::reduce::breadcrumb_segments;
use crate::app::state::{AppState, Mode};
use crate::browser::EntryView;
use crate::icons::{IconResolver, IconSize, IconVariant};
use crate::ui::format::{format_mode, format_size, format_time, kind_label, pad_right, truncate};
use crate::ui::hit::{HitMap, HitTarget, LegendAction};

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
    } else if width < 50 || height < 12 {
        Tier::Narrow
    } else if width < 80 || height < 20 {
        Tier::Compact
    } else if width < 120 || height < 30 {
        Tier::Standard
    } else {
        Tier::Wide
    }
}

fn base_style() -> Style {
    Style::default().fg(Color::Gray)
}

fn dir_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn selected_style() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

fn tag_style() -> Style {
    Style::default().fg(Color::Yellow)
}

fn error_style() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

fn muted_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn icon_size_for(tier: Tier) -> IconSize {
    match tier {
        Tier::TooSmall | Tier::Narrow | Tier::Compact => IconSize::Compact,
        Tier::Standard | Tier::Wide => IconSize::Small,
    }
}

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();
    state.width = area.width;
    state.height = area.height;
    state.hit_map.clear();
    let tier = tier_for(area.width, area.height);
    if tier == Tier::TooSmall {
        render_too_small(frame, area);
        state.list_viewport = 1;
        return;
    }
    let breadcrumb = Rect::new(area.x, area.y, area.width, 1);
    let legend = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let status = Rect::new(area.x, area.y + area.height - 2, area.width, 1);
    let main = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(3),
    );
    render_breadcrumb(frame, breadcrumb, state);
    let resolver = IconResolver::default();
    if tier == Tier::Wide {
        let list_width = (main.width as u32 * 3 / 5) as u16;
        let list_area = Rect::new(
            main.x,
            main.y,
            list_width.max(20).min(main.width),
            main.height,
        );
        let details_area = Rect::new(
            main.x + list_area.width,
            main.y,
            main.width - list_area.width,
            main.height,
        );
        render_list(frame, list_area, state, tier, &resolver);
        render_details(frame, details_area, state, &resolver);
    } else {
        render_list(frame, main, state, tier, &resolver);
    }
    render_status(frame, status, state);
    render_legend(frame, legend, state);
    match &state.mode {
        Mode::Confirm(confirm) => render_confirm(frame, area, confirm, &mut state.hit_map),
        Mode::Conflict(conflict) => render_conflict(frame, area, conflict, &mut state.hit_map),
        Mode::TagPicker(picker) => {
            let picker = picker.clone();
            render_picker(frame, area, state, &picker);
        }
        Mode::ContextMenu(menu) => {
            let menu = menu.clone();
            render_context_menu(frame, area, &menu, &mut state.hit_map);
        }
        Mode::Help => render_help(frame, area, &mut state.hit_map),
        _ => {}
    }
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("terminal too small", error_style())),
        Line::from(Span::styled("resize or q to quit", muted_style())),
    ];
    let height = lines.len() as u16;
    let top = area.y + area.height.saturating_sub(height) / 2;
    let rect = Rect::new(area.x, top, area.width, height.min(area.height));
    frame.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        rect,
    );
}

fn render_breadcrumb(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let segments = breadcrumb_segments(&state.browser.cwd);
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x;
    for (idx, (_, label)) in segments.iter().enumerate() {
        if x >= area.x + area.width {
            break;
        }
        let text = if idx == 0 {
            "/".to_string()
        } else if idx == 1 {
            label.clone()
        } else {
            format!("/{label}")
        };
        let remaining = (area.x + area.width - x) as usize;
        let shown = truncate(&text, remaining);
        let width = shown.chars().count() as u16;
        let is_last = idx == segments.len() - 1;
        let style = if is_last {
            dir_style()
        } else {
            Style::default().fg(Color::Blue)
        };
        spans.push(Span::styled(shown, style));
        state
            .hit_map
            .push(Rect::new(x, area.y, width, 1), HitTarget::Breadcrumb(idx));
        x += width;
    }
    if state.browser.show_hidden && x + 4 < area.x + area.width {
        spans.push(Span::styled("  [.+]", muted_style()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn row_mark(state: &AppState, view: &EntryView, focused: bool) -> (&'static str, Style) {
    let selected = state
        .browser
        .selected_paths_set()
        .contains(&view.entry.path);
    if selected {
        ("*", tag_style())
    } else if focused {
        (">", selected_style())
    } else {
        (" ", base_style())
    }
}

fn badge_text(tags: &[String], compact: bool) -> String {
    if tags.is_empty() {
        return String::new();
    }
    if compact {
        let mut out = String::new();
        for tag in tags.iter().take(2) {
            let short = truncate(tag, 3);
            out.push_str(&format!("[{short}]"));
        }
        if tags.len() > 2 {
            out.push_str(&format!("+{}", tags.len() - 2));
        }
        out
    } else {
        tags.iter()
            .map(|t| format!("[{t}]"))
            .collect::<Vec<_>>()
            .join("")
    }
}

fn render_list(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    tier: Tier,
    resolver: &IconResolver,
) {
    state.list_viewport = area.height as usize;
    state.browser.clamp_scroll(area.height as usize);
    let icon_size = icon_size_for(tier);
    let icon_width = resolver
        .registry()
        .rendered_size(crate::icons::IconKind::Folder, icon_size)
        .0 as usize;
    let indices = state.browser.visible_indices();
    let scroll = state.browser.scroll;
    let mut lines: Vec<Line> = Vec::new();
    for (row, pos) in indices
        .iter()
        .enumerate()
        .skip(scroll)
        .take(area.height as usize)
        .map(|(p, i)| (p, *i))
        .collect::<Vec<_>>()
    {
        let view = &state.browser.entries[pos];
        let focused =
            state.browser.selected == row && matches!(state.mode, Mode::Browser | Mode::Command);
        let (mark, mark_style) = row_mark(state, view, focused);
        let kind = resolver.resolve_with(
            &view.entry,
            if focused && view.entry.kind.is_dir() {
                IconVariant::Open
            } else {
                IconVariant::Normal
            },
        );
        let icon = resolver.registry().glyph(kind, icon_size);
        let name = view.entry.display_name();
        let is_dir = view.entry.kind.is_dir();
        let name_style = if is_dir { dir_style() } else { base_style() };
        let width = area.width as usize;
        let line = match tier {
            Tier::Narrow => {
                let budget = width.saturating_sub(2 + icon_width + 1);
                let shown = truncate(&name, budget);
                Line::from(vec![
                    Span::styled(mark, mark_style),
                    Span::styled(format!("{icon} "), muted_style()),
                    Span::styled(shown, name_style),
                ])
            }
            Tier::Compact => {
                let size_text = if is_dir {
                    "-".to_string()
                } else {
                    format_size(view.entry.size)
                };
                let badges = badge_text(&view.tags, true);
                let fixed = 2 + icon_width + 1 + 1 + 6 + badges.chars().count();
                let budget = width.saturating_sub(fixed);
                let shown = truncate(&name, budget);
                Line::from(vec![
                    Span::styled(mark, mark_style),
                    Span::styled(format!("{icon} "), muted_style()),
                    Span::styled(pad_right(&shown, budget), name_style),
                    Span::styled(badges, tag_style()),
                    Span::styled(format!(" {size_text:>5}"), muted_style()),
                ])
            }
            _ => {
                let size_text = if is_dir {
                    "-".to_string()
                } else {
                    format_size(view.entry.size)
                };
                let perms = format_mode(&view.entry.kind, view.entry.mode);
                let mtime = format_time(view.entry.modified);
                let badges = badge_text(&view.tags, false);
                let fixed = 2 + icon_width + 1 + 1 + 6 + 1 + 10 + 1 + 16 + badges.chars().count();
                let budget = width.saturating_sub(fixed);
                let shown = truncate(&name, budget);
                Line::from(vec![
                    Span::styled(mark, mark_style),
                    Span::styled(format!("{icon} "), muted_style()),
                    Span::styled(pad_right(&shown, budget), name_style),
                    Span::styled(badges, tag_style()),
                    Span::styled(format!(" {size_text:>5}"), muted_style()),
                    Span::styled(format!(" {perms}"), muted_style()),
                    Span::styled(format!(" {mtime}"), muted_style()),
                ])
            }
        };
        let line = if focused {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| {
                        Span::styled(
                            span.content.into_owned(),
                            span.style.patch(selected_style()),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            line
        };
        lines.push(line);
        state.hit_map.push(
            Rect::new(area.x, area.y + lines.len() as u16 - 1, area.width, 1),
            HitTarget::Row(row),
        );
    }
    if indices.is_empty() {
        lines.push(Line::from(Span::styled("(empty directory)", muted_style())));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_details(frame: &mut Frame, area: Rect, state: &mut AppState, resolver: &IconResolver) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_set(ASCII_BORDERS)
        .border_style(muted_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(view) = state.browser.focused() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no selection", muted_style()))),
            inner,
        );
        return;
    };
    let kind = resolver.resolve(&view.entry);
    let art = resolver.registry().glyph(kind, IconSize::Large);
    let mut lines: Vec<Line> = Vec::new();
    for art_line in art.lines() {
        lines.push(Line::from(Span::styled(
            art_line.to_string(),
            Style::default().fg(Color::Cyan),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        truncate(&view.entry.display_name(), inner.width as usize),
        dir_style(),
    )));
    lines.push(Line::from(Span::styled(
        format!("type: {}", kind_label(&view.entry.kind)),
        base_style(),
    )));
    if !view.entry.kind.is_dir() {
        lines.push(Line::from(Span::styled(
            format!("size: {}", format_size(view.entry.size)),
            base_style(),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!("mode: {}", format_mode(&view.entry.kind, view.entry.mode)),
        base_style(),
    )));
    lines.push(Line::from(Span::styled(
        format!("mtime: {}", format_time(view.entry.modified)),
        base_style(),
    )));
    lines.push(Line::from(Span::styled("tags:", base_style())));
    let tag_y = inner.y + lines.len() as u16;
    if view.tags.is_empty() {
        lines.push(Line::from(Span::styled("  (none)", muted_style())));
    } else {
        for tag in &view.tags {
            lines.push(Line::from(Span::styled(format!("  [{tag}]"), tag_style())));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
    state.hit_map.push(
        Rect::new(inner.x, tag_y, inner.width, view.tags.len().max(1) as u16),
        HitTarget::TagBadge,
    );
}

fn render_status(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if matches!(state.mode, Mode::Command) {
        let line = Line::from(vec![
            Span::styled(":", Style::default().fg(Color::Green)),
            Span::styled(state.command_input.clone(), base_style()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let mut spans = vec![
        Span::styled(
            format!(" {} ", state.mode_name()),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    let selected_count = state.browser.selection.len();
    if selected_count > 0 {
        spans.push(Span::styled(
            format!("{selected_count} selected "),
            tag_style(),
        ));
    }
    if let Some(op) = &state.operation {
        spans.push(Span::styled(
            format!(
                "{:?} {}/{} {}",
                op.kind,
                op.done,
                op.total,
                truncate(&op.current.display().to_string(), 30)
            ),
            Style::default().fg(Color::Magenta),
        ));
    } else if let Some(message) = &state.message {
        spans.push(Span::styled(
            truncate(&message.text, area.width as usize),
            if message.is_error {
                error_style()
            } else {
                base_style()
            },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn legend_items(state: &AppState, tier: Tier) -> Vec<(&'static str, Option<LegendAction>)> {
    match &state.mode {
        Mode::Command => vec![
            ("Enter run", None),
            ("Esc cancel", Some(LegendAction::Cancel)),
        ],
        Mode::Confirm(_) => vec![("y confirm", None), ("n cancel", None)],
        Mode::Conflict(_) => vec![("c cancel", None), ("s skip", None), ("r replace", None)],
        Mode::TagPicker(_) => vec![
            ("Enter toggle", None),
            ("n new", Some(LegendAction::TagPicker)),
            ("d delete", None),
            ("Esc close", Some(LegendAction::Cancel)),
        ],
        Mode::ContextMenu(_) => vec![
            ("Enter choose", None),
            ("Esc close", Some(LegendAction::Cancel)),
        ],
        Mode::Help => vec![("Esc close", Some(LegendAction::Cancel))],
        Mode::Browser => {
            if tier == Tier::Narrow || tier == Tier::Compact {
                vec![
                    ("q quit", Some(LegendAction::Quit)),
                    (": cmd", Some(LegendAction::Command)),
                    ("? help", Some(LegendAction::Help)),
                    ("t tag", Some(LegendAction::QuickTag)),
                    ("T tags", Some(LegendAction::TagPicker)),
                ]
            } else {
                vec![
                    ("q quit", Some(LegendAction::Quit)),
                    (": cmd", Some(LegendAction::Command)),
                    ("? help", Some(LegendAction::Help)),
                    ("v sel", Some(LegendAction::Select)),
                    (". hid", Some(LegendAction::Hidden)),
                    ("t tag", Some(LegendAction::QuickTag)),
                    ("T tags", Some(LegendAction::TagPicker)),
                    ("Enter open", Some(LegendAction::Open)),
                    ("h parent", Some(LegendAction::Parent)),
                ]
            }
        }
    }
}

fn render_legend(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let tier = tier_for(area.width, state.height);
    let items = legend_items(state, tier);
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x;
    for (label, action) in items {
        let text = format!(" {label} ");
        let width = text.chars().count() as u16;
        if x + width > area.x + area.width {
            break;
        }
        spans.push(Span::styled("|", muted_style()));
        spans.push(Span::styled(
            text,
            Style::default().fg(Color::White).bg(Color::Indexed(236)),
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

fn render_confirm(
    frame: &mut Frame,
    area: Rect,
    confirm: &crate::app::state::ConfirmState,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let rect = centered_rect(area, 56, 8);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(error_style())
        .title(Span::styled(" CONFIRM ", error_style()));
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
        Line::from(""),
        Line::from(vec![
            Span::styled(" [y] delete forever ", error_style()),
            Span::raw("  "),
            Span::styled(" [n] cancel ", base_style()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
    let button_y = inner.y + 3;
    hits.push(Rect::new(inner.x, button_y, 21, 1), HitTarget::ModalConfirm);
    hits.push(
        Rect::new(inner.x + 23, button_y, 12, 1),
        HitTarget::ModalCancel,
    );
}

fn render_conflict(
    frame: &mut Frame,
    area: Rect,
    conflict: &crate::app::state::ConflictState,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let height = (conflict.conflicts.len() as u16 + 6).clamp(8, area.height.max(8));
    let rect = centered_rect(area, 60, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(tag_style())
        .title(Span::styled(" CONFLICT ", tag_style()));
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
    lines.push(Line::from(vec![
        Span::styled(" [c] cancel ", base_style()),
        Span::raw(" "),
        Span::styled(" [s] skip existing ", base_style()),
        Span::raw(" "),
        Span::styled(" [r] replace ", error_style()),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
    let button_y = inner.y + inner.height - 1;
    hits.push(
        Rect::new(inner.x, button_y, 12, 1),
        HitTarget::ConflictCancel,
    );
    hits.push(
        Rect::new(inner.x + 13, button_y, 18, 1),
        HitTarget::ConflictSkip,
    );
    hits.push(
        Rect::new(inner.x + 32, button_y, 12, 1),
        HitTarget::ConflictReplace,
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(tag_style())
        .title(Span::styled(" TAGS ", tag_style()));
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
            selected_style()
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
    } else {
        lines.push(Line::from(vec![
            Span::styled(" [n] new ", tag_style()),
            Span::styled(" [d] delete ", error_style()),
            Span::styled(" [Esc] close ", base_style()),
        ]));
        let button_y = inner.y + lines.len() as u16 - 1;
        state
            .hit_map
            .push(Rect::new(inner.x, button_y, 9, 1), HitTarget::PickerNew);
        state.hit_map.push(
            Rect::new(inner.x + 9, button_y, 12, 1),
            HitTarget::PickerDelete,
        );
        state.hit_map.push(
            Rect::new(inner.x + 21, button_y, 13, 1),
            HitTarget::PickerClose,
        );
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_context_menu(
    frame: &mut Frame,
    area: Rect,
    menu: &crate::app::state::ContextMenuState,
    hits: &mut HitMap,
) {
    push_blocker(area, hits);
    let width = 16u16;
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(base_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines: Vec<Line> = menu
        .items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let focused = idx == menu.selected;
            let style = if focused {
                selected_style()
            } else {
                base_style()
            };
            Line::from(Span::styled(
                pad_right(item.label(), inner.width as usize),
                style,
            ))
        })
        .collect();
    for (idx, _) in menu.items.iter().enumerate() {
        hits.push(
            Rect::new(inner.x, inner.y + idx as u16, inner.width, 1),
            HitTarget::ContextItem(idx),
        );
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_help(frame: &mut Frame, area: Rect, hits: &mut HitMap) {
    push_blocker(area, hits);
    let rect = centered_rect(area, 100.min(area.width), (area.height * 4 / 5).max(16));
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ASCII_BORDERS)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" HELP ", dir_style()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let entries: &[(&str, &str)] = &[
        ("j / Down", "move down"),
        ("k / Up", "move up"),
        ("h / Left", "parent directory"),
        ("l / Right / Enter", "enter directory or open file"),
        ("g g", "first entry"),
        ("G", "last entry"),
        ("Ctrl-u / Ctrl-d", "half page up / down"),
        ("PageUp / PageDown", "full page up / down"),
        ("Space", "toggle entry selection"),
        ("v", "visual multi-selection mode"),
        (".", "toggle hidden files"),
        ("t", "toggle last used tag"),
        ("T", "tag picker and manager"),
        (":", "command mode"),
        ("Esc", "cancel mode or modal"),
        ("?", "this help"),
        ("q", "quit"),
        (":copy <dest>", "copy selection"),
        (":move <dest>", "move selection"),
        (":rename <name>", "rename entry"),
        (":delete", "delete selection (confirmed)"),
        (":tag <name>", "assign tag"),
        (":untag <name>", "remove tag"),
        (":tags", "open tag picker"),
        (":open", "open entry"),
        (":cd <path>", "change directory"),
        (":quit", "quit"),
        (":help", "this help"),
        ("mouse left", "select row, legend, breadcrumb"),
        ("mouse double", "enter directory or open file"),
        ("mouse right", "context menu"),
        ("mouse wheel", "scroll list"),
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
