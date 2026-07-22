use std::path::{Path, PathBuf};

use crate::app::action::{Action, ConflictDecision, DirectorySnapshot, MouseKind};
use crate::app::effects::Effect;
use crate::app::state::{
    AppState, ConfirmAction, ConfirmState, ConflictState, ContextItem, ContextMenuState, Mode,
    OperationState, StatusMessage, TagPickerState,
};
use crate::filesystem::EntryKind;
use crate::input::command::{self, Command};
use crate::operations::{
    ConflictPolicy, OpOutcome, OperationKind, OperationPlan, OperationReport, validate,
    validate_rename,
};
use crate::tags::validate_name;
use crate::ui::hit::{HitTarget, LegendAction};

pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    let was_pending_g = state.pending_g;
    state.pending_g = false;
    match action {
        Action::LoadInitial => vec![Effect::LoadDirectory(state.browser.cwd.clone())],
        Action::KeyG => {
            if was_pending_g {
                state.browser.goto_first();
            } else {
                state.pending_g = true;
            }
            Vec::new()
        }
        Action::MoveDown => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.move_down(vp);
        }),
        Action::MoveUp => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.move_up(vp);
        }),
        Action::PageDown => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.page_down(vp);
        }),
        Action::PageUp => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.page_up(vp);
        }),
        Action::HalfPageDown => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.half_page_down(vp);
        }),
        Action::HalfPageUp => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.half_page_up(vp);
        }),
        Action::GotoFirst => browser_only(state, |s| s.browser.goto_first()),
        Action::GotoLast => browser_only(state, |s| {
            let vp = s.list_viewport;
            s.browser.goto_last(vp);
        }),
        Action::OpenFocused => open_focused(state),
        Action::OpenParent => browser_only_fx(state, |s| {
            let parent = s.browser.cwd.parent().map(Path::to_path_buf);
            match parent {
                Some(p) if p != s.browser.cwd => navigate(s, p),
                _ => Vec::new(),
            }
        }),
        Action::ToggleSelect => browser_only(state, |s| {
            s.browser.toggle_select_focused();
            let vp = s.list_viewport;
            s.browser.move_down(vp);
        }),
        Action::ToggleVisual => browser_only(state, |s| {
            s.browser.visual = !s.browser.visual;
            if s.browser.visual {
                s.browser.toggle_select_focused();
            }
        }),
        Action::ToggleHidden => browser_only(state, |s| s.browser.toggle_hidden()),
        Action::QuickTag => quick_tag(state),
        Action::OpenTagPicker => open_picker(state),
        Action::EnterCommand => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                state.command_input.clear();
            }
            Vec::new()
        }
        Action::CommandChar(c) => {
            if matches!(state.mode, Mode::Command) {
                state.command_input.push(c);
            }
            Vec::new()
        }
        Action::CommandBackspace => {
            if matches!(state.mode, Mode::Command) {
                state.command_input.pop();
            }
            Vec::new()
        }
        Action::CommandSubmit => submit_command(state),
        Action::Cancel => cancel(state),
        Action::ToggleHelp => {
            state.mode = if matches!(state.mode, Mode::Help) {
                Mode::Browser
            } else {
                Mode::Help
            };
            Vec::new()
        }
        Action::Quit => {
            if matches!(state.mode, Mode::Browser) {
                vec![Effect::Quit]
            } else {
                Vec::new()
            }
        }
        Action::Confirm => confirm(state),
        Action::Reject => {
            if matches!(state.mode, Mode::Confirm(_) | Mode::Conflict(_)) {
                state.mode = Mode::Browser;
                state.message = Some(StatusMessage::info("cancelled"));
            }
            Vec::new()
        }
        Action::ConflictChoice(decision) => conflict_choice(state, decision),
        Action::PickerMove(delta) => {
            if let Mode::TagPicker(picker) = &mut state.mode {
                let len = picker.defs.len();
                if len > 0 {
                    let next = (picker.selected as isize + delta).clamp(0, len as isize - 1);
                    picker.selected = next as usize;
                }
            }
            Vec::new()
        }
        Action::PickerToggle => picker_toggle(state),
        Action::PickerNew => {
            if let Mode::TagPicker(picker) = &mut state.mode {
                picker.input = Some(String::new());
            }
            Vec::new()
        }
        Action::PickerChar(c) => {
            if let Mode::TagPicker(picker) = &mut state.mode {
                if let Some(input) = &mut picker.input {
                    input.push(c);
                }
            }
            Vec::new()
        }
        Action::PickerBackspace => {
            if let Mode::TagPicker(picker) = &mut state.mode {
                if let Some(input) = &mut picker.input {
                    input.pop();
                }
            }
            Vec::new()
        }
        Action::PickerSubmitNew => picker_submit_new(state),
        Action::PickerCancelInput => {
            if let Mode::TagPicker(picker) = &mut state.mode {
                if picker.input.is_some() {
                    picker.input = None;
                } else {
                    state.mode = Mode::Browser;
                }
            }
            Vec::new()
        }
        Action::PickerDelete => {
            if let Mode::TagPicker(picker) = &state.mode {
                if let Some(def) = picker.defs.get(picker.selected) {
                    return vec![Effect::TagDelete(def.name.clone())];
                }
            }
            Vec::new()
        }
        Action::ContextMove(delta) => {
            if let Mode::ContextMenu(menu) = &mut state.mode {
                let len = menu.items.len();
                if len > 0 {
                    let next = (menu.selected as isize + delta).clamp(0, len as isize - 1);
                    menu.selected = next as usize;
                }
            }
            Vec::new()
        }
        Action::ContextChoose => {
            if let Mode::ContextMenu(menu) = &state.mode {
                let item = menu.items[menu.selected];
                return context_apply(state, item);
            }
            Vec::new()
        }
        Action::Mouse { kind, x, y } => mouse(state, kind, x, y),
        Action::Resize { width, height } => {
            state.width = width;
            state.height = height;
            Vec::new()
        }
        Action::DirectoryLoaded { result } => directory_loaded(state, result),
        Action::OperationProgress {
            current,
            done,
            total,
        } => {
            if let Some(op) = &mut state.operation {
                op.current = current;
                op.done = done;
                op.total = total;
            }
            Vec::new()
        }
        Action::OperationFinished { report } => operation_finished(state, report),
        Action::ConflictsFound { plan, conflicts } => {
            state.mode = Mode::Conflict(Box::new(ConflictState { plan, conflicts }));
            Vec::new()
        }
        Action::OpenFailed(err) => {
            state.message = Some(StatusMessage::error(err));
            Vec::new()
        }
        Action::ErrorMessage(err) => {
            state.message = Some(StatusMessage::error(err));
            Vec::new()
        }
        Action::TagsApplied { message, last_tag } => {
            state.message = Some(StatusMessage::info(message));
            if let Some(name) = last_tag {
                state.last_tag = Some(name);
            }
            vec![Effect::LoadDirectory(state.browser.cwd.clone())]
        }
    }
}

fn browser_only(state: &mut AppState, f: impl FnOnce(&mut AppState)) -> Vec<Effect> {
    if matches!(state.mode, Mode::Browser) {
        f(state);
    }
    Vec::new()
}

fn browser_only_fx(
    state: &mut AppState,
    f: impl FnOnce(&mut AppState) -> Vec<Effect>,
) -> Vec<Effect> {
    if matches!(state.mode, Mode::Browser) {
        return f(state);
    }
    Vec::new()
}

fn navigate(state: &mut AppState, dir: PathBuf) -> Vec<Effect> {
    state.pending_nav = Some(state.browser.cwd.clone());
    state.browser.enter(&dir);
    vec![Effect::LoadDirectory(dir)]
}

fn open_focused(state: &mut AppState) -> Vec<Effect> {
    browser_only_fx(state, |s| {
        let Some(view) = s.browser.focused() else {
            return Vec::new();
        };
        let path = view.entry.path.clone();
        match &view.entry.kind {
            EntryKind::Directory => navigate(s, path),
            _ => vec![Effect::OpenPath(path)],
        }
    })
}

fn quick_tag(state: &mut AppState) -> Vec<Effect> {
    browser_only_fx(state, |s| {
        let name = match s
            .last_tag
            .clone()
            .or_else(|| s.tag_defs.first().map(|d| d.name.clone()))
        {
            Some(n) => n,
            None => {
                s.message = Some(StatusMessage::info("no tags yet, press T to create one"));
                return Vec::new();
            }
        };
        let targets = s.browser.targets();
        if targets.is_empty() {
            return Vec::new();
        }
        let all_have = targets.iter().all(|t| {
            s.browser
                .entries
                .iter()
                .find(|e| e.entry.path == *t)
                .map(|e| e.tags.contains(&name))
                .unwrap_or(false)
        });
        if all_have {
            vec![Effect::TagUnassign {
                name,
                paths: targets,
            }]
        } else {
            vec![Effect::TagAssign {
                name,
                paths: targets,
                create: false,
            }]
        }
    })
}

fn open_picker(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.mode, Mode::Browser) {
        return Vec::new();
    }
    let targets = state.browser.targets();
    state.mode = Mode::TagPicker(Box::new(TagPickerState {
        defs: state.tag_defs.clone(),
        selected: 0,
        input: None,
        targets,
    }));
    Vec::new()
}

fn picker_toggle(state: &mut AppState) -> Vec<Effect> {
    let Mode::TagPicker(picker) = &state.mode else {
        return Vec::new();
    };
    if picker.input.is_some() {
        return Vec::new();
    }
    let Some(def) = picker.defs.get(picker.selected) else {
        return Vec::new();
    };
    let name = def.name.clone();
    let targets = picker.targets.clone();
    if targets.is_empty() {
        state.message = Some(StatusMessage::info("no entry focused"));
        return Vec::new();
    }
    let all_have = targets.iter().all(|t| {
        state
            .browser
            .entries
            .iter()
            .find(|e| e.entry.path == *t)
            .map(|e| e.tags.contains(&name))
            .unwrap_or(false)
    });
    if all_have {
        vec![Effect::TagUnassign {
            name,
            paths: targets,
        }]
    } else {
        vec![Effect::TagAssign {
            name,
            paths: targets,
            create: false,
        }]
    }
}

fn picker_submit_new(state: &mut AppState) -> Vec<Effect> {
    let Mode::TagPicker(picker) = &mut state.mode else {
        return Vec::new();
    };
    let Some(name) = picker.input.take() else {
        return Vec::new();
    };
    if let Err(e) = validate_name(&name) {
        state.message = Some(StatusMessage::error(e.to_string()));
        return Vec::new();
    }
    if picker.defs.iter().any(|d| d.name == name) {
        state.message = Some(StatusMessage::error(format!("tag exists: {name}")));
        return Vec::new();
    }
    let targets = picker.targets.clone();
    let mut effects = vec![Effect::TagCreate(name.clone())];
    if !targets.is_empty() {
        effects.push(Effect::TagAssign {
            name,
            paths: targets,
            create: false,
        });
    }
    effects
}

fn resolve_user_path(state: &AppState, input: &str) -> PathBuf {
    let expanded = if let Some(rest) = input.strip_prefix("~/") {
        state.home.join(rest)
    } else if input == "~" {
        state.home.clone()
    } else {
        PathBuf::from(input)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        state.browser.cwd.join(expanded)
    }
}

fn delete_confirm(state: &mut AppState) -> Vec<Effect> {
    let targets = state.browser.targets();
    if targets.is_empty() {
        state.message = Some(StatusMessage::info("nothing selected"));
        return Vec::new();
    }
    let recursive = targets.iter().any(|t| {
        state
            .browser
            .entries
            .iter()
            .find(|e| e.entry.path == *t)
            .map(|e| e.entry.kind.is_dir())
            .unwrap_or(false)
    });
    let plan = OperationPlan {
        kind: OperationKind::Delete,
        sources: targets.clone(),
        dest_dir: None,
        rename_to: None,
        policy: ConflictPolicy::Ask,
    };
    let count = targets.len();
    state.mode = Mode::Confirm(Box::new(ConfirmState {
        title: format!(
            "Delete {count} entr{} permanently?",
            if count == 1 { "y" } else { "ies" }
        ),
        detail: targets
            .iter()
            .take(5)
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        stage: 1,
        recursive,
        action: ConfirmAction::Delete {
            plan: Box::new(plan),
        },
    }));
    Vec::new()
}

fn submit_command(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.mode, Mode::Command) {
        return Vec::new();
    }
    let input = state.command_input.clone();
    state.mode = Mode::Browser;
    state.command_input.clear();
    let parsed = match command::parse(&input) {
        Ok(c) => c,
        Err(e) => {
            state.message = Some(StatusMessage::error(e.to_string()));
            return Vec::new();
        }
    };
    match parsed {
        Command::Copy { dest } => start_copy_move(state, OperationKind::Copy, dest),
        Command::Move { dest } => start_copy_move(state, OperationKind::Move, dest),
        Command::Rename { name } => {
            let sources = state.browser.targets();
            let plan = OperationPlan {
                kind: OperationKind::Move,
                sources,
                dest_dir: None,
                rename_to: Some(std::ffi::OsString::from(name)),
                policy: ConflictPolicy::Ask,
            };
            match validate_rename(&plan) {
                Ok(_) => vec![Effect::RunRename(Box::new(plan))],
                Err(e) => {
                    state.message = Some(StatusMessage::error(e.to_string()));
                    Vec::new()
                }
            }
        }
        Command::Delete => delete_confirm(state),
        Command::Tag { name } => {
            if let Err(e) = validate_name(&name) {
                state.message = Some(StatusMessage::error(e.to_string()));
                return Vec::new();
            }
            let targets = state.browser.targets();
            if targets.is_empty() {
                state.message = Some(StatusMessage::info("nothing selected"));
                return Vec::new();
            }
            vec![Effect::TagAssign {
                name,
                paths: targets,
                create: true,
            }]
        }
        Command::Untag { name } => {
            let targets = state.browser.targets();
            if targets.is_empty() {
                state.message = Some(StatusMessage::info("nothing selected"));
                return Vec::new();
            }
            vec![Effect::TagUnassign {
                name,
                paths: targets,
            }]
        }
        Command::Tags => open_picker(state),
        Command::Open => open_focused(state),
        Command::Cd { path } => {
            let dir = resolve_user_path(state, &path);
            navigate(state, dir)
        }
        Command::Quit => vec![Effect::Quit],
        Command::Help => {
            state.mode = Mode::Help;
            Vec::new()
        }
    }
}

fn start_copy_move(state: &mut AppState, kind: OperationKind, dest: String) -> Vec<Effect> {
    let sources = state.browser.targets();
    let plan = OperationPlan {
        kind,
        sources,
        dest_dir: Some(resolve_user_path(state, &dest)),
        rename_to: None,
        policy: ConflictPolicy::Ask,
    };
    match validate(&plan) {
        Ok(()) => start_operation(state, plan),
        Err(e) => {
            state.message = Some(StatusMessage::error(e.to_string()));
            Vec::new()
        }
    }
}

fn start_operation(state: &mut AppState, plan: OperationPlan) -> Vec<Effect> {
    let total = plan.sources.len();
    state.operation = Some(OperationState {
        kind: plan.kind,
        current: PathBuf::new(),
        done: 0,
        total,
    });
    vec![Effect::RunOperation(Box::new(plan))]
}

fn confirm(state: &mut AppState) -> Vec<Effect> {
    let Mode::Confirm(confirm_state) = &mut state.mode else {
        return Vec::new();
    };
    if confirm_state.stage == 1 && confirm_state.recursive {
        confirm_state.stage = 2;
        confirm_state.title = "Really delete directories recursively?".to_string();
        confirm_state.detail = "this cannot be undone".to_string();
        return Vec::new();
    }
    let Mode::Confirm(confirm_state) = std::mem::replace(&mut state.mode, Mode::Browser) else {
        return Vec::new();
    };
    match confirm_state.action {
        ConfirmAction::Delete { plan } => start_operation(state, *plan),
    }
}

fn conflict_choice(state: &mut AppState, decision: ConflictDecision) -> Vec<Effect> {
    let Mode::Conflict(conflict) = std::mem::replace(&mut state.mode, Mode::Browser) else {
        return Vec::new();
    };
    match decision {
        ConflictDecision::Cancel => {
            state.operation = None;
            state.message = Some(StatusMessage::info("operation cancelled"));
            Vec::new()
        }
        ConflictDecision::Skip => {
            let mut plan = *conflict.plan;
            plan.policy = ConflictPolicy::Skip;
            vec![Effect::RunOperation(Box::new(plan))]
        }
        ConflictDecision::Replace => {
            let mut plan = *conflict.plan;
            plan.policy = ConflictPolicy::Replace;
            vec![Effect::RunOperation(Box::new(plan))]
        }
    }
}

fn context_apply(state: &mut AppState, item: ContextItem) -> Vec<Effect> {
    let Mode::ContextMenu(menu) = std::mem::replace(&mut state.mode, Mode::Browser) else {
        return Vec::new();
    };
    let target = menu.target.clone();
    if let Some(pos) = state
        .browser
        .visible_indices()
        .iter()
        .position(|&i| state.browser.entries[i].entry.path == target)
    {
        state.browser.selected = pos;
    }
    match item {
        ContextItem::Open => open_focused(state),
        ContextItem::Rename => {
            state.mode = Mode::Command;
            state.command_input = "rename ".to_string();
            Vec::new()
        }
        ContextItem::Copy => {
            state.mode = Mode::Command;
            state.command_input = "copy ".to_string();
            Vec::new()
        }
        ContextItem::Move => {
            state.mode = Mode::Command;
            state.command_input = "move ".to_string();
            Vec::new()
        }
        ContextItem::Delete => delete_confirm(state),
        ContextItem::Tags => open_picker(state),
    }
}

fn cancel(state: &mut AppState) -> Vec<Effect> {
    match &state.mode {
        Mode::Command
        | Mode::Confirm(_)
        | Mode::Conflict(_)
        | Mode::TagPicker(_)
        | Mode::ContextMenu(_)
        | Mode::Help => {
            state.mode = Mode::Browser;
            state.command_input.clear();
        }
        Mode::Browser => {
            if !state.browser.selection.is_empty() || state.browser.visual {
                state.browser.clear_selection();
            }
        }
    }
    Vec::new()
}

fn directory_loaded(
    state: &mut AppState,
    result: Result<DirectorySnapshot, String>,
) -> Vec<Effect> {
    match result {
        Ok(snapshot) => {
            if snapshot.path == state.browser.cwd {
                state.browser.set_entries(snapshot.entries);
            }
            state.tag_defs = snapshot.defs;
            state.pending_nav = None;
            if let Mode::TagPicker(picker) = &mut state.mode {
                picker.defs = state.tag_defs.clone();
                if picker.selected >= picker.defs.len() {
                    picker.selected = picker.defs.len().saturating_sub(1);
                }
            }
        }
        Err(err) => {
            if let Some(prev) = state.pending_nav.take() {
                state.browser.enter(&prev);
                state.message = Some(StatusMessage::error(err));
                return vec![Effect::LoadDirectory(prev)];
            }
            state.message = Some(StatusMessage::error(err));
        }
    }
    Vec::new()
}

fn operation_finished(state: &mut AppState, report: OperationReport) -> Vec<Effect> {
    state.operation = None;
    let done = report.done_count();
    let skipped = report.skipped_count();
    let failed = report.failed();
    let total = report.results.len();
    let mut parts = vec![format!("{done}/{total} done")];
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    if !failed.is_empty() {
        parts.push(format!("{} failed", failed.len()));
    }
    let mut text = parts.join(", ");
    if let Some(first) = failed.first() {
        if let OpOutcome::Failed(err) = &first.outcome {
            text.push_str(&format!(": {err}"));
        }
    }
    state.message = Some(if failed.is_empty() {
        StatusMessage::info(text)
    } else {
        StatusMessage::error(text)
    });
    let mut effects: Vec<Effect> = report
        .moves
        .iter()
        .map(|(from, to)| Effect::TagMove {
            from: from.clone(),
            to: to.clone(),
        })
        .collect();
    effects.push(Effect::LoadDirectory(state.browser.cwd.clone()));
    effects
}

fn mouse(state: &mut AppState, kind: MouseKind, x: u16, y: u16) -> Vec<Effect> {
    let Some(target) = state.hit_map.hit(x, y) else {
        return Vec::new();
    };
    match target {
        HitTarget::Blocker => match kind {
            MouseKind::Left => cancel(state),
            _ => Vec::new(),
        },
        HitTarget::Row(pos) => match kind {
            MouseKind::Left => {
                if matches!(state.mode, Mode::Browser) {
                    state.browser.selected = pos;
                    let vp = state.list_viewport;
                    state.browser.clamp_scroll(vp);
                }
                Vec::new()
            }
            MouseKind::DoubleLeft => {
                if matches!(state.mode, Mode::Browser) {
                    state.browser.selected = pos;
                    let vp = state.list_viewport;
                    state.browser.clamp_scroll(vp);
                }
                open_focused(state)
            }
            MouseKind::Right => {
                if matches!(state.mode, Mode::Browser) {
                    state.browser.selected = pos;
                    let vp = state.list_viewport;
                    state.browser.clamp_scroll(vp);
                    if let Some(view) = state.browser.focused() {
                        let target_path = view.entry.path.clone();
                        state.mode = Mode::ContextMenu(Box::new(ContextMenuState {
                            target: target_path,
                            items: ContextItem::all(),
                            selected: 0,
                            x,
                            y,
                        }));
                    }
                }
                Vec::new()
            }
            MouseKind::ScrollUp => browser_only(state, |s| {
                let vp = s.list_viewport;
                s.browser.scroll_by(-3, vp);
            }),
            MouseKind::ScrollDown => browser_only(state, |s| {
                let vp = s.list_viewport;
                s.browser.scroll_by(3, vp);
            }),
        },
        HitTarget::Breadcrumb(idx) => match kind {
            MouseKind::Left => browser_only_fx(state, |s| breadcrumb_nav(s, idx)),
            _ => Vec::new(),
        },
        HitTarget::Legend(action) => match kind {
            MouseKind::Left => legend_action(state, action),
            _ => Vec::new(),
        },
        HitTarget::TagBadge => match kind {
            MouseKind::Left => open_picker(state),
            _ => Vec::new(),
        },
        HitTarget::ModalConfirm => match kind {
            MouseKind::Left => confirm(state),
            _ => Vec::new(),
        },
        HitTarget::ModalCancel => match kind {
            MouseKind::Left => reduce(state, Action::Reject),
            _ => Vec::new(),
        },
        HitTarget::ConflictCancel => match kind {
            MouseKind::Left => conflict_choice(state, ConflictDecision::Cancel),
            _ => Vec::new(),
        },
        HitTarget::ConflictSkip => match kind {
            MouseKind::Left => conflict_choice(state, ConflictDecision::Skip),
            _ => Vec::new(),
        },
        HitTarget::ConflictReplace => match kind {
            MouseKind::Left => conflict_choice(state, ConflictDecision::Replace),
            _ => Vec::new(),
        },
        HitTarget::PickerItem(idx) => match kind {
            MouseKind::Left => {
                if let Mode::TagPicker(picker) = &mut state.mode {
                    picker.selected = idx;
                }
                picker_toggle(state)
            }
            _ => Vec::new(),
        },
        HitTarget::PickerNew => match kind {
            MouseKind::Left => reduce(state, Action::PickerNew),
            _ => Vec::new(),
        },
        HitTarget::PickerDelete => match kind {
            MouseKind::Left => reduce(state, Action::PickerDelete),
            _ => Vec::new(),
        },
        HitTarget::PickerClose => match kind {
            MouseKind::Left => cancel(state),
            _ => Vec::new(),
        },
        HitTarget::ContextItem(idx) => match kind {
            MouseKind::Left => {
                if let Mode::ContextMenu(menu) = &mut state.mode {
                    menu.selected = idx;
                }
                reduce(state, Action::ContextChoose)
            }
            _ => Vec::new(),
        },
        HitTarget::Details => Vec::new(),
    }
}

fn breadcrumb_nav(state: &mut AppState, idx: usize) -> Vec<Effect> {
    let segments = breadcrumb_segments(&state.browser.cwd);
    if idx >= segments.len() {
        return Vec::new();
    }
    let target = segments[idx].0.clone();
    if target == state.browser.cwd {
        return Vec::new();
    }
    navigate(state, target)
}

pub fn breadcrumb_segments(cwd: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut current = Some(cwd);
    while let Some(path) = current {
        let label = if path.parent().is_none() {
            "/".to_string()
        } else {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        };
        out.push((path.to_path_buf(), label));
        current = path.parent();
    }
    out.reverse();
    out
}

fn legend_action(state: &mut AppState, action: LegendAction) -> Vec<Effect> {
    match action {
        LegendAction::Quit => reduce(state, Action::Quit),
        LegendAction::Help => reduce(state, Action::ToggleHelp),
        LegendAction::Command => reduce(state, Action::EnterCommand),
        LegendAction::Hidden => reduce(state, Action::ToggleHidden),
        LegendAction::Select => reduce(state, Action::ToggleSelect),
        LegendAction::QuickTag => reduce(state, Action::QuickTag),
        LegendAction::TagPicker => reduce(state, Action::OpenTagPicker),
        LegendAction::Open => reduce(state, Action::OpenFocused),
        LegendAction::Parent => reduce(state, Action::OpenParent),
        LegendAction::Cancel => reduce(state, Action::Cancel),
    }
}
