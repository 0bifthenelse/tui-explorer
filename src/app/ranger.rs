//! Ranger-style behaviors: chord resolution, go-to locations, history,
//! marks, tabs, clipboard keys, trash/undo, inline rename, live search /
//! find / filter, quick look, and system-clipboard yanks.

use std::path::{Path, PathBuf};

use crate::app::action::{Action, RenameCursor, YankKind};
use crate::app::effects::Effect;
use crate::app::reduce::{
    delete_confirm_targets, grid_dims, navigate_with, reduce_inner, resolve_user_path,
    start_operation,
};
use crate::app::state::{
    AppState, ClipMode, ClipboardState, Mode, RenameState, SearchKind, SearchState, StatusMessage,
    Tab, UndoEntry, UndoStep,
};
use crate::input::chords::{self, Resolution};
use crate::input::line::{Edit, LineEdit};
use crate::operations::{ConflictPolicy, OperationKind, OperationPlan, validate};
use crate::settings::ViewMode;

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn info(state: &mut AppState, text: impl Into<String>) {
    state.message = Some(StatusMessage::info(text));
}

// --- Chords --------------------------------------------------------------

pub fn clear_pending(state: &mut AppState) {
    state.pending_keys.clear();
    state.pending_count = None;
}

pub fn chord_key(state: &mut AppState, token: String) -> Vec<Effect> {
    if !matches!(state.mode, Mode::Browser) {
        clear_pending(state);
        return Vec::new();
    }
    let is_digit = token.len() == 1 && token.chars().all(|c| c.is_ascii_digit());
    if state.pending_keys.is_empty() && is_digit && (token != "0" || state.pending_count.is_some())
    {
        let d = token.parse::<usize>().unwrap_or(0);
        let count = state
            .pending_count
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(d);
        state.pending_count = Some(count.min(99_999));
        return Vec::new();
    }
    state.pending_keys.push(token);
    let tokens = state.pending_keys.clone();
    let count = state.pending_count;
    // Grid layouts keep h/l (and arrows) as spatial moves.
    if state.view() == ViewMode::Grid && tokens.len() == 1 {
        let spatial = match tokens[0].as_str() {
            "h" | "<Left>" => Some(Action::MoveLeft),
            "l" | "<Right>" => Some(Action::MoveRight),
            _ => None,
        };
        if let Some(action) = spatial {
            clear_pending(state);
            let action = match count {
                Some(n) if n > 1 => Action::Repeat(n, Box::new(action)),
                _ => action,
            };
            return reduce_inner(state, action);
        }
    }
    match chords::resolve(&tokens, count) {
        Resolution::Run(action) => {
            clear_pending(state);
            reduce_inner(state, action)
        }
        Resolution::Pending => Vec::new(),
        Resolution::Unknown => {
            clear_pending(state);
            let seq: String = tokens.concat();
            info(state, format!("no binding for {seq}"));
            Vec::new()
        }
    }
}

// --- Locations, history, marks -------------------------------------------

pub fn goto_index(state: &mut AppState, index: usize) {
    let len = state.browser.visible_len();
    if len == 0 {
        return;
    }
    state.browser.selected = index.min(len - 1);
    let (c, r) = grid_dims(state);
    state.browser.clamp_scroll_grid(c, r);
}

pub fn goto_spec(state: &mut AppState, spec: &str) -> Vec<Effect> {
    let path = resolve_user_path(state, spec);
    if path == state.browser.cwd {
        return Vec::new();
    }
    navigate_with(state, path, true)
}

pub fn history_back(state: &mut AppState) -> Vec<Effect> {
    let Some(prev) = state.history.back.pop() else {
        info(state, "no earlier folder");
        return Vec::new();
    };
    let cwd = state.browser.cwd.clone();
    state.history.forward.push(cwd.clone());
    let fx = navigate_with(state, prev, false);
    state.pending_focus = Some(cwd);
    fx
}

pub fn history_forward(state: &mut AppState) -> Vec<Effect> {
    let Some(next) = state.history.forward.pop() else {
        info(state, "no later folder");
        return Vec::new();
    };
    let cwd = state.browser.cwd.clone();
    state.history.back.push(cwd);
    navigate_with(state, next, false)
}

pub fn previous_dir(state: &mut AppState) -> Vec<Effect> {
    match state.previous_dir.clone() {
        Some(prev) if prev != state.browser.cwd => navigate_with(state, prev, true),
        _ => {
            info(state, "no previous folder");
            Vec::new()
        }
    }
}

pub fn set_mark(state: &mut AppState, key: char) -> Vec<Effect> {
    let cwd = state.browser.cwd.clone();
    state.settings.marks.insert(key.to_string(), cwd.clone());
    state.settings_dirty = true;
    info(state, format!("mark {key} → {}", cwd.display()));
    Vec::new()
}

pub fn jump_mark(state: &mut AppState, key: char) -> Vec<Effect> {
    match state.settings.marks.get(&key.to_string()).cloned() {
        Some(path) => navigate_with(state, path, true),
        None => {
            info(state, format!("mark {key} is not set (m{key} sets it)"));
            Vec::new()
        }
    }
}

pub fn delete_mark(state: &mut AppState, key: char) -> Vec<Effect> {
    if state.settings.marks.remove(&key.to_string()).is_some() {
        state.settings_dirty = true;
        info(state, format!("mark {key} deleted"));
    } else {
        info(state, format!("mark {key} is not set"));
    }
    Vec::new()
}

pub fn follow_link(state: &mut AppState) -> Vec<Effect> {
    let Some(view) = state.browser.focused() else {
        return Vec::new();
    };
    let Some(target) = view.entry.link_target.clone() else {
        info(state, "not a symlink");
        return Vec::new();
    };
    let base = view
        .entry
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let resolved = if target.is_absolute() {
        target
    } else {
        base.join(target)
    };
    if view.entry.link_dir {
        return navigate_with(state, resolved, true);
    }
    match resolved.parent().map(Path::to_path_buf) {
        Some(parent) => {
            let fx = navigate_with(state, parent, true);
            state.pending_focus = Some(resolved);
            fx
        }
        None => Vec::new(),
    }
}

// --- Tabs ----------------------------------------------------------------

fn stash_active(state: &mut AppState) {
    let idx = state.active_tab;
    state.tabs[idx] = Tab {
        browser: state.browser.clone(),
        history: state.history.clone(),
    };
}

fn activate(state: &mut AppState, idx: usize) -> Vec<Effect> {
    stash_active(state);
    load_tab(state, idx)
}

/// Makes `idx` the active tab without saving the current one first (the
/// caller already stashed it, or the slot it occupied is gone).
fn load_tab(state: &mut AppState, idx: usize) -> Vec<Effect> {
    state.active_tab = idx;
    let tab = state.tabs[idx].clone();
    state.browser = tab.browser;
    state.history = tab.history;
    state.hover = Default::default();
    state.anim.start_cascade(state.now);
    state.preview.key = None;
    state.preview.content = None;
    // Refresh: the folder may have changed while the tab was hidden.
    let mut fx = vec![Effect::LoadDirectory(state.browser.cwd.clone())];
    if state.view() == ViewMode::Columns
        && let Some(parent) = state.browser.cwd.parent()
        && !state.side_listings.contains_key(parent)
    {
        fx.push(Effect::LoadSideListing(parent.to_path_buf()));
    }
    fx
}

pub fn tab_new(state: &mut AppState) -> Vec<Effect> {
    stash_active(state);
    let tab = Tab {
        browser: {
            let mut b = state.browser.clone();
            b.clear_selection();
            b
        },
        history: Default::default(),
    };
    state.tabs.insert(state.active_tab + 1, tab);
    let fx = load_tab(state, state.active_tab + 1);
    info(state, format!("tab {} opened", state.active_tab + 1));
    fx
}

pub fn tab_select(state: &mut AppState, idx: usize) -> Vec<Effect> {
    if idx >= state.tabs.len() || idx == state.active_tab {
        return Vec::new();
    }
    activate(state, idx)
}

pub fn tab_step(state: &mut AppState, delta: isize) -> Vec<Effect> {
    let n = state.tabs.len();
    if n < 2 {
        info(state, "only one tab (gn opens another)");
        return Vec::new();
    }
    let idx = (state.active_tab as isize + delta).rem_euclid(n as isize) as usize;
    activate(state, idx)
}

pub fn tab_close(state: &mut AppState) -> Vec<Effect> {
    if state.tabs.len() < 2 {
        info(state, "last tab stays open (q quits)");
        return Vec::new();
    }
    stash_active(state);
    let closed = state.tabs.remove(state.active_tab);
    state.closed_tabs.push(closed);
    if state.closed_tabs.len() > 10 {
        state.closed_tabs.remove(0);
    }
    let idx = state.active_tab.min(state.tabs.len() - 1);
    // The removed slot was active; load the neighbor without stashing.
    let fx = load_tab(state, idx);
    info(state, "tab closed (uq restores)");
    fx
}

pub fn tab_restore(state: &mut AppState) -> Vec<Effect> {
    let Some(tab) = state.closed_tabs.pop() else {
        info(state, "no closed tab to restore");
        return Vec::new();
    };
    stash_active(state);
    state.tabs.insert(state.active_tab + 1, tab);
    let fx = load_tab(state, state.active_tab + 1);
    info(state, "tab restored");
    fx
}

// --- Selection & clipboard ---------------------------------------------

pub fn select_all(state: &mut AppState) {
    let paths: Vec<PathBuf> = state
        .browser
        .visible_entries()
        .map(|(_, e)| e.entry.path.clone())
        .collect();
    let n = paths.len();
    state.browser.set_selection(paths);
    info(state, format!("{} selected", plural(n, "item")));
}

pub fn invert_selection(state: &mut AppState) {
    let paths: Vec<PathBuf> = state
        .browser
        .visible_entries()
        .map(|(_, e)| e.entry.path.clone())
        .filter(|p| !state.browser.selection.contains(p))
        .collect();
    state.browser.set_selection(paths);
}

pub fn copy_selection(state: &mut AppState, mode: ClipMode) {
    let items = state.browser.action_targets();
    if items.is_empty() {
        info(state, "nothing to copy");
        return;
    }
    let n = items.len();
    state.clipboard = ClipboardState {
        mode: Some(mode),
        items,
    };
    let verb = match mode {
        ClipMode::Copy => "copied",
        ClipMode::Cut => "cut",
    };
    info(state, format!("{verb} {} · pp pastes", plural(n, "item")));
}

/// `pp` / `po` / Ctrl-V. Copies pasted into their own folder duplicate
/// under a fresh name instead of failing.
pub fn paste_here(state: &mut AppState, overwrite: bool) -> Vec<Effect> {
    let Some(mode) = state.clipboard.mode else {
        info(state, "clipboard is empty (yy copies, dd cuts)");
        return Vec::new();
    };
    if state.clipboard.items.is_empty() {
        info(state, "clipboard is empty (yy copies, dd cuts)");
        return Vec::new();
    }
    let cwd = state.browser.cwd.clone();
    let all_here = state
        .clipboard
        .items
        .iter()
        .all(|p| p.parent() == Some(cwd.as_path()));
    if mode == ClipMode::Cut && all_here {
        info(state, "already here");
        return Vec::new();
    }
    let policy = if overwrite {
        ConflictPolicy::Replace
    } else if mode == ClipMode::Copy && all_here {
        ConflictPolicy::KeepBoth
    } else {
        ConflictPolicy::Ask
    };
    let plan = OperationPlan {
        kind: match mode {
            ClipMode::Copy => OperationKind::Copy,
            ClipMode::Cut => OperationKind::Move,
        },
        sources: state.clipboard.items.clone(),
        dest_dir: Some(cwd),
        rename_to: None,
        policy,
    };
    match validate(&plan) {
        Ok(()) => {
            state.pending_paste_mode = Some(mode);
            start_operation(state, plan)
        }
        Err(e) => {
            state.set_error(e.to_string());
            Vec::new()
        }
    }
}

pub fn paste_symlinks(state: &mut AppState) -> Vec<Effect> {
    if state.clipboard.items.is_empty() {
        info(state, "clipboard is empty (yy copies)");
        return Vec::new();
    }
    let plan = OperationPlan {
        kind: OperationKind::Symlink,
        sources: state.clipboard.items.clone(),
        dest_dir: Some(state.browser.cwd.clone()),
        rename_to: None,
        policy: ConflictPolicy::KeepBoth,
    };
    match validate(&plan) {
        Ok(()) => start_operation(state, plan),
        Err(e) => {
            state.set_error(e.to_string());
            Vec::new()
        }
    }
}

pub fn yank(state: &mut AppState, kind: YankKind) -> Vec<Effect> {
    let targets = state.browser.action_targets();
    let text = match kind {
        YankKind::Dir => state.browser.cwd.display().to_string(),
        YankKind::Path => targets
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        YankKind::Name | YankKind::Stem => targets
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| {
                let name = n.to_string_lossy().into_owned();
                if kind == YankKind::Stem {
                    crate::filesystem::split_ext(&name).0
                } else {
                    name
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };
    if text.is_empty() {
        return Vec::new();
    }
    let preview = crate::ui::format::truncate(&text.replace('\n', " · "), 60);
    info(state, format!("yanked {preview}"));
    vec![Effect::CopyText(text)]
}

// --- Trash, delete, undo -------------------------------------------------

pub fn trash_selection(state: &mut AppState) -> Vec<Effect> {
    let targets = state.browser.action_targets();
    if targets.is_empty() {
        return Vec::new();
    }
    let plan = OperationPlan {
        kind: OperationKind::Trash,
        sources: targets,
        dest_dir: None,
        rename_to: None,
        policy: ConflictPolicy::Ask,
    };
    state.browser.clear_selection();
    start_operation(state, plan)
}

pub fn delete_selection(state: &mut AppState) -> Vec<Effect> {
    let targets = state.browser.action_targets();
    delete_confirm_targets(state, targets)
}

/// Records an undoable job from its report.
pub fn journal(state: &mut AppState, label: &str, report: &crate::operations::OperationReport) {
    let mut steps: Vec<UndoStep> = report
        .moves
        .iter()
        .map(|(from, to)| UndoStep::Move {
            from: from.clone(),
            to: to.clone(),
        })
        .collect();
    steps.extend(report.created.iter().cloned().map(UndoStep::Created));
    if steps.is_empty() {
        return;
    }
    state.undo.push(UndoEntry {
        label: label.to_string(),
        steps,
    });
    if state.undo.len() > 50 {
        state.undo.remove(0);
    }
}

pub fn undo(state: &mut AppState) -> Vec<Effect> {
    let Some(entry) = state.undo.pop() else {
        info(state, "nothing to undo");
        return Vec::new();
    };
    let mut moves = Vec::new();
    let mut trash = Vec::new();
    for step in entry.steps.into_iter().rev() {
        match step {
            UndoStep::Move { from, to } => moves.push((to, from)),
            UndoStep::Created(path) => trash.push(path),
        }
    }
    info(state, format!("undoing {}", entry.label));
    vec![Effect::RunUndo { moves, trash }]
}

// --- Inline prompts --------------------------------------------------------

pub fn rename_start(state: &mut AppState, cursor: RenameCursor) -> Vec<Effect> {
    let Some(view) = state.browser.focused() else {
        return Vec::new();
    };
    let name = view.entry.display_name();
    let target = view.entry.path.clone();
    let edit = match cursor {
        RenameCursor::Replace => LineEdit::default(),
        RenameCursor::End => LineEdit::new(name),
        RenameCursor::Start => LineEdit::with_cursor(name, 0),
        RenameCursor::BeforeExt => {
            let (stem, _) = crate::filesystem::split_ext(&name);
            let at = stem.chars().count();
            LineEdit::with_cursor(name, at)
        }
    };
    state.mode = Mode::Rename(Box::new(RenameState { target, edit }));
    Vec::new()
}

pub fn search_start(state: &mut AppState, kind: SearchKind) -> Vec<Effect> {
    let origin_filter = state.browser.filter.clone();
    let seed = match kind {
        SearchKind::Filter => origin_filter.clone().unwrap_or_default(),
        _ => String::new(),
    };
    state.mode = Mode::Search(Box::new(SearchState {
        kind,
        edit: LineEdit::new(seed),
        origin: state.browser.selected,
        origin_filter,
    }));
    Vec::new()
}

/// Visible positions whose names contain the active search query.
pub fn search_matches(state: &AppState) -> Vec<usize> {
    let Some(query) = state.browser.search.as_ref().filter(|q| !q.is_empty()) else {
        return Vec::new();
    };
    let query = query.to_lowercase();
    state
        .browser
        .visible_entries()
        .enumerate()
        .filter(|(_, (_, e))| e.entry.display_name().to_lowercase().contains(&query))
        .map(|(pos, _)| pos)
        .collect()
}

fn jump_to_match(state: &mut AppState, from: usize, forward: bool, inclusive: bool) -> bool {
    let matches = search_matches(state);
    if matches.is_empty() {
        return false;
    }
    let target = if forward {
        matches
            .iter()
            .copied()
            .find(|&p| if inclusive { p >= from } else { p > from })
            .unwrap_or(matches[0])
    } else {
        matches
            .iter()
            .rev()
            .copied()
            .find(|&p| if inclusive { p <= from } else { p < from })
            .unwrap_or(*matches.last().unwrap_or(&0))
    };
    goto_index(state, target);
    true
}

pub fn search_step(state: &mut AppState, forward: bool) -> Vec<Effect> {
    if state.browser.search.is_none() {
        info(state, "no search (/ starts one)");
        return Vec::new();
    }
    let from = state.browser.selected;
    if jump_to_match(state, from, forward, false) {
        let total = search_matches(state).len();
        let idx = search_matches(state)
            .iter()
            .position(|&p| p == state.browser.selected)
            .map(|i| i + 1)
            .unwrap_or(0);
        info(state, format!("match {idx}/{total}"));
    } else {
        info(state, "no matches");
    }
    Vec::new()
}

pub fn line_edit(state: &mut AppState, edit: Edit) -> Vec<Effect> {
    match &mut state.mode {
        Mode::Rename(r) => {
            r.edit.apply(edit);
            Vec::new()
        }
        Mode::Search(s) => {
            s.edit.apply(edit);
            let query = s.edit.text.clone();
            let kind = s.kind;
            let origin = s.origin;
            match kind {
                SearchKind::Filter => {
                    state.browser.set_filter(Some(query));
                }
                SearchKind::Search | SearchKind::Find => {
                    state.browser.search = (!query.is_empty()).then_some(query.clone());
                    jump_to_match(state, origin, true, true);
                    if kind == SearchKind::Find && !query.is_empty() {
                        let matches = search_matches(state);
                        if matches.len() == 1 {
                            // A unique match opens immediately (ranger's find).
                            state.mode = Mode::Browser;
                            state.browser.search = None;
                            goto_index(state, matches[0]);
                            return reduce_inner(state, Action::OpenFocused);
                        }
                    }
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub fn prompt_submit(state: &mut AppState) -> Vec<Effect> {
    match std::mem::replace(&mut state.mode, Mode::Browser) {
        Mode::Rename(r) => submit_rename(state, r.target, r.edit.text),
        Mode::Search(s) => match s.kind {
            SearchKind::Filter => {
                if let Some(q) = &state.browser.filter {
                    info(state, format!("filter applied: {q}"));
                }
                Vec::new()
            }
            SearchKind::Search => {
                let n = search_matches(state).len();
                if n == 0 {
                    state.browser.search = None;
                    info(state, "no matches");
                } else {
                    info(state, format!("{} · n / N to jump", plural(n, "match")));
                }
                Vec::new()
            }
            SearchKind::Find => {
                state.browser.search = None;
                if s.edit.text.is_empty() {
                    return Vec::new();
                }
                reduce_inner(state, Action::OpenFocused)
            }
        },
        other => {
            state.mode = other;
            Vec::new()
        }
    }
}

/// Esc in an inline prompt: restore what it changed.
pub fn prompt_cancel(state: &mut AppState) -> bool {
    match &state.mode {
        Mode::Rename(_) => {
            state.mode = Mode::Browser;
            true
        }
        Mode::Search(s) => {
            let s = s.clone();
            state.mode = Mode::Browser;
            match s.kind {
                SearchKind::Filter => state.browser.set_filter(s.origin_filter),
                _ => {
                    state.browser.search = None;
                    goto_index(state, s.origin);
                }
            }
            true
        }
        _ => false,
    }
}

fn submit_rename(state: &mut AppState, target: PathBuf, name: String) -> Vec<Effect> {
    let name = name.trim().to_string();
    let current = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() || name == current {
        return Vec::new();
    }
    let plan = OperationPlan {
        kind: OperationKind::Move,
        sources: vec![target],
        dest_dir: None,
        rename_to: Some(std::ffi::OsString::from(&name)),
        policy: ConflictPolicy::Ask,
    };
    match crate::operations::validate_rename(&plan) {
        Ok(dst) => {
            state.pending_focus = Some(dst);
            vec![Effect::RunRename(Box::new(plan))]
        }
        Err(e) => {
            state.set_error(e.to_string());
            Vec::new()
        }
    }
}

// --- Misc ------------------------------------------------------------------

pub fn quick_look(state: &mut AppState) -> Vec<Effect> {
    if state.browser.focused().is_none() {
        return Vec::new();
    }
    state.mode = Mode::QuickLook(Box::default());
    Vec::new()
}

pub fn edit_focused(state: &mut AppState) -> Vec<Effect> {
    let Some(view) = state.browser.focused() else {
        return Vec::new();
    };
    if view.entry.is_dir_like() {
        info(state, "E edits files; Enter opens folders");
        return Vec::new();
    }
    let editor = std::env::var("VISUAL")
        .ok()
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| "vi".to_string());
    let words = crate::input::command::split_words(&editor).unwrap_or_else(|_| vec![editor]);
    let Some((program, args)) = words.split_first() else {
        return Vec::new();
    };
    vec![Effect::OpenPathWith {
        path: view.entry.path.clone(),
        program: program.clone(),
        args: args.to_vec(),
    }]
}

pub fn disk_usage(state: &mut AppState) -> Vec<Effect> {
    let mut dirs: Vec<PathBuf> = state
        .browser
        .action_targets()
        .into_iter()
        .filter(|p| {
            state
                .browser
                .entries
                .iter()
                .any(|e| e.entry.path == *p && e.entry.kind.is_dir())
        })
        .collect();
    if dirs.is_empty() || state.browser.selection.is_empty() {
        // Without a selection, size every folder in view.
        dirs = state
            .browser
            .visible_entries()
            .filter(|(_, e)| e.entry.kind.is_dir())
            .map(|(_, e)| e.entry.path.clone())
            .collect();
    }
    if dirs.is_empty() {
        info(state, "no folders here");
        return Vec::new();
    }
    info(
        state,
        format!("measuring {}…", plural(dirs.len(), "folder")),
    );
    vec![Effect::DiskUsage(dirs)]
}

pub fn toggle_animations(state: &mut AppState) {
    let on = !state.settings.animations;
    state.settings.animations = on;
    state.settings_dirty = true;
    state.anim.set_enabled(on);
    info(
        state,
        if on {
            "animations on"
        } else {
            "animations off (reduced motion)"
        },
    );
}

/// Bracketed paste: paths and URLs open the address bar; prompts get the
/// text inserted.
pub fn paste_text(state: &mut AppState, text: String) -> Vec<Effect> {
    let text = text.trim_end_matches(['\n', '\r']).to_string();
    match &mut state.mode {
        Mode::Command => {
            state.command_input.push_str(&text.replace('\n', " "));
            Vec::new()
        }
        Mode::Rename(_) | Mode::Search(_) => {
            for c in text.chars().filter(|c| *c != '\n') {
                line_edit(state, Edit::Insert(c));
            }
            Vec::new()
        }
        Mode::Browser => {
            let first = text.lines().next().unwrap_or("").trim().to_string();
            if first.is_empty() {
                return Vec::new();
            }
            reduce_inner(state, Action::OpenAddressBar);
            if let Mode::Command = state.mode {
                state.command_input = format!("cd {first}");
            }
            Vec::new()
        }
        Mode::OpenWith(_)
        | Mode::Bookmarks(_)
        | Mode::Results(_)
        | Mode::Help
        | Mode::Password(_) => {
            let build: fn(char) -> Action = match &state.mode {
                Mode::OpenWith(_) => Action::OpenWithChar,
                Mode::Bookmarks(_) => Action::BookmarkChar,
                Mode::Results(_) => Action::ResultsChar,
                Mode::Help => Action::HelpChar,
                _ => Action::PasswordChar,
            };
            for c in text.chars().filter(|c| !c.is_control()) {
                reduce_inner(state, build(c));
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Sidebar-independent count requests after a listing arrives.
pub fn child_count_effect(state: &AppState) -> Option<Effect> {
    let dirs: Vec<PathBuf> = state
        .browser
        .entries
        .iter()
        .filter(|e| e.entry.is_dir_like())
        .map(|e| e.entry.path.clone())
        .take(2000)
        .collect();
    (!dirs.is_empty()).then_some(Effect::CountChildren(dirs))
}
