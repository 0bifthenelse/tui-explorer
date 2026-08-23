use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::action::{Action, ConflictDecision, DirectorySnapshot, MouseKind};
use crate::app::effects::Effect;
use crate::app::state::{
    AppState, BookmarkNavState, ConfirmAction, ConfirmState, ConflictState, ContextItem,
    ContextMenuState, MediaState, Mode, OpenWithState, OperationState, Password, PasswordPurpose,
    PasswordState, PreviewContent, StatusMessage, TagPickerState,
};
use crate::browser::SortMode;
use crate::crypto::CryptoKind;
use crate::filesystem::EntryKind;
use crate::input::command::{self, Command};
use crate::media::{AfterStop, MediaCommand, MediaPhase, classify_path};
use crate::operations::{
    ConflictPolicy, OpOutcome, OperationKind, OperationPlan, OperationReport, validate,
    validate_rename,
};
use crate::sidebar::SidebarItem;
use crate::tags::validate_name;
use crate::ui::hit::{HitTarget, LegendAction};

pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    let mut effects = reduce_inner(state, action);
    if let Some(effect) = preview_followup(state) {
        effects.push(effect);
    }
    effects
}

/// When the preview panel is visible and the focused entry changed (or its
/// modification metadata changed), ask for fresh preview content.
fn preview_followup(state: &AppState) -> Option<Effect> {
    if !crate::ui::preview_visible(state.width, state.height, state.show_preview) {
        return None;
    }
    if state.mode.is_overlay() {
        return None;
    }
    let key = state.focused_preview_key()?;
    if state.preview.key.as_ref() == Some(&key) {
        return None;
    }
    let view = state.browser.focused()?;
    Some(Effect::LoadPreview {
        key,
        name: view.entry.display_name(),
        is_dir: view.entry.kind.is_dir(),
    })
}

fn reduce_inner(state: &mut AppState, action: Action) -> Vec<Effect> {
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
            let (c, r) = grid_dims(s);
            s.browser.grid_move(c as isize, c, r);
        }),
        Action::MoveUp => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move(-(c as isize), c, r);
        }),
        Action::MoveLeft => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move(-1, c, r);
        }),
        Action::MoveRight => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move(1, c, r);
        }),
        Action::PageDown => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move((c * r) as isize, c, r);
        }),
        Action::PageUp => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move(-((c * r) as isize), c, r);
        }),
        Action::HalfPageDown => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move((c * r / 2).max(1) as isize, c, r);
        }),
        Action::HalfPageUp => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.grid_move(-((c * r / 2).max(1) as isize), c, r);
        }),
        Action::GotoFirst => browser_only(state, |s| {
            s.browser.goto_first();
        }),
        Action::GotoLast => browser_only(state, |s| {
            let (c, r) = grid_dims(s);
            s.browser.goto_last_grid(c, r);
        }),
        Action::OpenFocused => open_focused(state),
        Action::OpenParent => browser_only_fx(state, |s| {
            let parent = s.browser.cwd.parent().map(Path::to_path_buf);
            match parent {
                Some(p) if p != s.browser.cwd => navigate(s, p),
                _ => Vec::new(),
            }
        }),
        Action::Refresh => {
            if matches!(state.mode, Mode::Browser) {
                state.message = Some(StatusMessage::info("refreshing directory"));
                vec![Effect::LoadDirectory(state.browser.cwd.clone())]
            } else {
                Vec::new()
            }
        }
        Action::OpenWithPrompt => open_with_prompt(state),
        Action::OpenWithChar(c) => {
            if let Mode::OpenWith(o) = &mut state.mode {
                o.input.push(c);
            }
            Vec::new()
        }
        Action::OpenWithBackspace => {
            if let Mode::OpenWith(o) = &mut state.mode {
                o.input.pop();
            }
            Vec::new()
        }
        Action::OpenWithSubmit => open_with_submit(state),
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
        Action::SetFilter(query) => browser_only(state, |s| s.browser.set_filter(query)),
        Action::ToggleSidebar => {
            if matches!(state.mode, Mode::Browser) {
                let now = crate::ui::sidebar_visible(state.width, state.height, state.show_sidebar);
                state.show_sidebar = Some(!now);
            }
            Vec::new()
        }
        Action::TogglePreview => {
            if matches!(state.mode, Mode::Browser) {
                let now = crate::ui::preview_visible(state.width, state.height, state.show_preview);
                state.show_preview = Some(!now);
                if now {
                    state.preview.key = None;
                    state.preview.content = None;
                }
            }
            Vec::new()
        }
        Action::ToggleBookmark => {
            if matches!(state.mode, Mode::Browser) {
                let cwd = state.browser.cwd.clone();
                return vec![Effect::ToggleBookmark(cwd)];
            }
            Vec::new()
        }
        Action::OpenBookmarks => browser_only_fx(state, |s| {
            s.mode = Mode::Bookmarks(Box::new(BookmarkNavState {
                query: String::new(),
                matches: Vec::new(),
                selected: 0,
            }));
            refresh_bookmark_matches(s);
            Vec::new()
        }),
        Action::BookmarkChar(c) => {
            if let Mode::Bookmarks(nav) = &mut state.mode {
                nav.query.push(c);
            }
            refresh_bookmark_matches(state);
            Vec::new()
        }
        Action::BookmarkBackspace => {
            if let Mode::Bookmarks(nav) = &mut state.mode {
                nav.query.pop();
            }
            refresh_bookmark_matches(state);
            Vec::new()
        }
        Action::BookmarkMove(delta) => {
            if let Mode::Bookmarks(nav) = &mut state.mode {
                let len = nav.matches.len();
                if len > 0 {
                    let next = (nav.selected as isize + delta).clamp(0, len as isize - 1);
                    nav.selected = next as usize;
                }
            }
            Vec::new()
        }
        Action::BookmarkSubmit => {
            let path = match &state.mode {
                Mode::Bookmarks(nav) => nav.matches.get(nav.selected).cloned(),
                _ => None,
            };
            let Some(path) = path else {
                return Vec::new();
            };
            state.mode = Mode::Browser;
            navigate(state, path)
        }
        Action::BookmarksChanged { bookmarks, message } => {
            state.bookmarks = bookmarks;
            state.message = Some(StatusMessage::info(message));
            Vec::new()
        }
        Action::EncryptToggle => encrypt_toggle(state),
        Action::PasswordChar(c) => {
            if let Mode::Password(p) = &mut state.mode {
                p.input.push(c);
            }
            Vec::new()
        }
        Action::PasswordBackspace => {
            if let Mode::Password(p) = &mut state.mode {
                p.input.pop();
            }
            Vec::new()
        }
        Action::PasswordSubmit => password_submit(state),
        Action::CryptoFinished { done, failed } => {
            state.operation = None;
            let text = if failed.is_empty() {
                format!(
                    "{} entr{} processed",
                    done.len(),
                    if done.len() == 1 { "y" } else { "ies" }
                )
            } else {
                let (path, err) = &failed[0];
                format!(
                    "{}/{} failed: {}: {}",
                    failed.len(),
                    done.len() + failed.len(),
                    path.display(),
                    err
                )
            };
            if failed.is_empty() {
                state.message = Some(StatusMessage::info(text));
            } else {
                state.set_error(text);
            }
            vec![Effect::LoadDirectory(state.browser.cwd.clone())]
        }
        Action::PreviewLoaded { key, result } => {
            if state.focused_preview_key().as_ref() == Some(&key) {
                state.preview.key = Some(key);
                state.preview.content = Some(match result {
                    crate::preview::PreviewLoaded::Text { lines, truncated } => {
                        PreviewContent::Text { lines, truncated }
                    }
                    crate::preview::PreviewLoaded::Image(img) => {
                        PreviewContent::Image(Box::new(state.picker.new_resize_protocol(img)))
                    }
                    crate::preview::PreviewLoaded::Directory(names) => {
                        PreviewContent::Directory(names)
                    }
                    crate::preview::PreviewLoaded::Unavailable(msg) => {
                        PreviewContent::Unavailable(msg)
                    }
                });
            }
            Vec::new()
        }
        Action::MediaSurfaceReady { session, surface } => {
            let Mode::Media(media) = &mut state.mode else {
                return Vec::new();
            };
            if media.session != session
                || media.phase != MediaPhase::Preparing
                || !media.awaiting_surface_ready
            {
                return Vec::new();
            }
            media.surface = Some(surface);
            media.awaiting_surface_ready = false;
            vec![Effect::StartMedia {
                session,
                path: media.path.clone(),
                kind: media.kind,
                surface,
                resume_position: media.resume_position,
                resume_paused: media.resume_paused,
            }]
        }
        Action::MediaBackendReady { session } => {
            let Mode::Media(media) = &mut state.mode else {
                return Vec::new();
            };
            if media.session != session {
                return Vec::new();
            }
            media.error = None;
            media.phase = MediaPhase::Starting;
            vec![Effect::MediaCommand {
                session,
                command: MediaCommand::Load,
            }]
        }
        Action::MediaStatus {
            session,
            phase,
            position,
            duration,
            volume,
        } => {
            if let Mode::Media(media) = &mut state.mode
                && media.session == session
            {
                media.phase = phase;
                media.position = position.max(0.0);
                media.duration = duration;
                media.volume = volume.min(100);
                if matches!(phase, MediaPhase::Playing) {
                    media.error = None;
                }
            }
            Vec::new()
        }
        Action::MediaSpectrum { session, spectrum } => {
            if let Mode::Media(media) = &mut state.mode
                && media.session == session
            {
                media.spectrum = spectrum.map(|value| value.clamp(0.0, 1.0));
            }
            Vec::new()
        }
        Action::MediaEnded { session } => {
            let Mode::Media(media) = &mut state.mode else {
                return Vec::new();
            };
            if media.session != session {
                return Vec::new();
            }
            let name = media
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| media.path.display().to_string());
            state.message = Some(StatusMessage::info(format!("finished {name}")));
            media.phase = MediaPhase::Stopping;
            media.after_stop = Some(AfterStop::Close);
            vec![Effect::StopMedia { session }]
        }
        Action::MediaFailed { session, message } => {
            let Mode::Media(media) = &mut state.mode else {
                return Vec::new();
            };
            if media.session != session {
                return Vec::new();
            }
            media.error = Some(message.clone());
            media.phase = MediaPhase::Stopping;
            media.after_stop = Some(AfterStop::ShowError(message));
            vec![Effect::StopMedia { session }]
        }
        Action::MediaStopped { session } => media_stopped(state, session),
        Action::MediaTogglePause => media_command(state, MediaCommand::TogglePause),
        Action::MediaSeek(seconds) => media_command(state, MediaCommand::SeekRelative(seconds)),
        Action::MediaVolume(delta) => {
            let Mode::Media(media) = &mut state.mode else {
                return Vec::new();
            };
            let volume = (media.volume as i16 + delta as i16).clamp(0, 100) as u8;
            media.volume = volume;
            vec![Effect::MediaCommand {
                session: media.session,
                command: MediaCommand::SetVolume(volume),
            }]
        }
        Action::MediaStop => media_command(state, MediaCommand::Stop),
        Action::MediaClose => close_media(state, AfterStop::Close),
        Action::QuickTag => quick_tag(state),
        Action::OpenTagPicker => open_picker(state),
        Action::EnterCommand => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                state.command_input.clear();
            }
            Vec::new()
        }
        Action::EnterFilter => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                state.command_input = "filter ".to_string();
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
            } else if matches!(state.mode, Mode::Media(_)) {
                close_media(state, AfterStop::Quit)
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
            state.set_error(err);
            Vec::new()
        }
        Action::ErrorMessage(err) => {
            state.set_error(err);
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

/// (columns, rows) of the current grid layout, as recorded by the renderer.
fn grid_dims(state: &AppState) -> (usize, usize) {
    let cols = state.grid_cols.max(1);
    let rows = (state.list_viewport / cols).max(1);
    (cols, rows)
}

/// `X` on the focused entry: encrypted outputs decrypt, everything else
/// encrypts. Opens the masked password dialog.
fn encrypt_toggle(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.mode, Mode::Browser) {
        return Vec::new();
    }
    let Some(view) = state.browser.focused() else {
        state.message = Some(StatusMessage::info("no entry focused"));
        return Vec::new();
    };
    let target = view.entry.path.clone();
    let name = view.entry.display_name();
    let purpose = if crate::crypto::is_encrypted_name(&name) {
        PasswordPurpose::Decrypt
    } else {
        PasswordPurpose::Encrypt
    };
    state.mode = Mode::Password(Box::new(PasswordState {
        purpose,
        target,
        input: String::new(),
        first: None,
    }));
    Vec::new()
}

fn password_submit(state: &mut AppState) -> Vec<Effect> {
    let Mode::Password(dialog) = &mut state.mode else {
        return Vec::new();
    };
    match dialog.purpose {
        PasswordPurpose::Encrypt => {
            if dialog.input.is_empty() {
                state.set_error("password cannot be empty");
                return Vec::new();
            }
            if let Some(first) = &dialog.first {
                if *first != dialog.input {
                    // Mismatched confirmation blocks encryption; start over.
                    dialog.first = None;
                    dialog.input.clear();
                    state.set_error("passwords do not match, try again");
                    return Vec::new();
                }
                start_crypto(state, CryptoKind::Encrypt)
            } else {
                dialog.first = Some(std::mem::take(&mut dialog.input));
                Vec::new()
            }
        }
        PasswordPurpose::Decrypt => {
            if dialog.input.is_empty() {
                state.set_error("password cannot be empty");
                return Vec::new();
            }
            start_crypto(state, CryptoKind::Decrypt)
        }
    }
}

fn start_crypto(state: &mut AppState, kind: CryptoKind) -> Vec<Effect> {
    let Mode::Password(dialog) = std::mem::replace(&mut state.mode, Mode::Browser) else {
        return Vec::new();
    };
    state.operation = Some(OperationState {
        kind: match kind {
            CryptoKind::Encrypt => crate::operations::OperationKind::Encrypt,
            CryptoKind::Decrypt => crate::operations::OperationKind::Decrypt,
        },
        current: dialog.target.clone(),
        done: 0,
        total: 1,
    });
    vec![Effect::Crypto {
        kind,
        target: dialog.target,
        password: Password(dialog.input),
    }]
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

fn bookmark_matches(bookmarks: &[PathBuf], query: &str) -> Vec<PathBuf> {
    let mut scored: Vec<(i32, usize, &PathBuf)> = bookmarks
        .iter()
        .enumerate()
        .filter_map(|(idx, path)| {
            crate::app::fuzzy::score_bookmark(query, path).map(|score| (score, idx, path))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, p)| p.clone()).collect()
}

fn refresh_bookmark_matches(state: &mut AppState) {
    let bookmarks = state.bookmarks.clone();
    if let Mode::Bookmarks(nav) = &mut state.mode {
        nav.matches = bookmark_matches(&bookmarks, &nav.query);
        nav.selected = nav.selected.min(nav.matches.len().saturating_sub(1));
    }
}

fn navigate(state: &mut AppState, dir: PathBuf) -> Vec<Effect> {
    state.pending_nav = Some(state.browser.cwd.clone());
    // A filename search is scoped to one directory, matching desktop file
    // managers: changing location should not hide unrelated entries.
    state.browser.set_filter(None);
    state.browser.enter(&dir);
    vec![Effect::LoadDirectory(dir)]
}

fn media_command(state: &mut AppState, command: MediaCommand) -> Vec<Effect> {
    let Mode::Media(media) = &state.mode else {
        return Vec::new();
    };
    if matches!(
        media.phase,
        MediaPhase::Preparing | MediaPhase::Stopping | MediaPhase::Error
    ) {
        return Vec::new();
    }
    vec![Effect::MediaCommand {
        session: media.session,
        command,
    }]
}

fn close_media(state: &mut AppState, after_stop: AfterStop) -> Vec<Effect> {
    let Mode::Media(media) = &mut state.mode else {
        return Vec::new();
    };
    if media.phase == MediaPhase::Stopping {
        return Vec::new();
    }
    media.phase = MediaPhase::Stopping;
    media.after_stop = Some(after_stop);
    vec![Effect::StopMedia {
        session: media.session,
    }]
}

fn media_stopped(state: &mut AppState, session: u64) -> Vec<Effect> {
    let Mode::Media(media) = &mut state.mode else {
        return Vec::new();
    };
    if media.session != session {
        return Vec::new();
    }
    let after_stop = media.after_stop.take().unwrap_or(AfterStop::Close);
    match after_stop {
        AfterStop::Close => {
            state.mode = Mode::Browser;
            Vec::new()
        }
        AfterStop::Quit => {
            state.mode = Mode::Browser;
            vec![Effect::Quit]
        }
        AfterStop::RestartAfterResize { position, paused } => {
            media.phase = MediaPhase::Preparing;
            media.position = position;
            media.surface = None;
            media.awaiting_surface_ready = true;
            media.resume_position = Some(position);
            media.resume_paused = Some(paused);
            Vec::new()
        }
        AfterStop::ShowError(message) => {
            media.phase = MediaPhase::Error;
            media.error = Some(message);
            Vec::new()
        }
    }
}

fn open_focused(state: &mut AppState) -> Vec<Effect> {
    browser_only_fx(state, |s| {
        let Some(view) = s.browser.focused() else {
            return Vec::new();
        };
        let path = view.entry.path.clone();
        match &view.entry.kind {
            EntryKind::Directory => navigate(s, path),
            _ => {
                if let Some(kind) = classify_path(&path) {
                    let session = s.next_media_session;
                    s.next_media_session = s.next_media_session.wrapping_add(1).max(1);
                    s.mode = Mode::Media(Box::new(MediaState::preparing(session, path, kind)));
                } else {
                    prompt_open_with(s, path);
                }
                Vec::new()
            }
        }
    })
}

fn prompt_open_with(state: &mut AppState, target: PathBuf) {
    state.mode = Mode::OpenWith(Box::new(OpenWithState {
        target,
        input: String::new(),
    }));
}

fn open_with_prompt(state: &mut AppState) -> Vec<Effect> {
    browser_only_fx(state, |s| {
        let Some(view) = s.browser.focused() else {
            s.message = Some(StatusMessage::info("nothing focused"));
            return Vec::new();
        };
        let target = view.entry.path.clone();
        prompt_open_with(s, target);
        Vec::new()
    })
}

fn open_with_submit(state: &mut AppState) -> Vec<Effect> {
    let Mode::OpenWith(dialog) = &state.mode else {
        return Vec::new();
    };
    let target = dialog.target.clone();
    let input = dialog.input.clone();
    state.mode = Mode::Browser;
    if input.trim().is_empty() {
        state.set_error("no command entered");
        return Vec::new();
    }
    match command::split_words(&input) {
        Ok(words) => {
            let Some((program, args)) = words.split_first() else {
                state.set_error("no command entered");
                return Vec::new();
            };
            vec![Effect::OpenPathWith {
                path: target,
                program: program.clone(),
                args: args.to_vec(),
            }]
        }
        Err(e) => {
            state.set_error(e.to_string());
            Vec::new()
        }
    }
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
        state.set_error(e.to_string());
        return Vec::new();
    }
    if picker.defs.iter().any(|d| d.name == name) {
        state.set_error(format!("tag exists: {name}"));
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

/// Rejects names unsafe to create directly in the current directory:
/// empty, containing a path separator, `.`/`..`, or a NUL byte.
fn validate_entry_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("empty name");
    }
    if name.contains('/') {
        return Err("name cannot contain '/'");
    }
    if name == "." || name == ".." {
        return Err("invalid name");
    }
    if name.contains('\0') {
        return Err("name cannot contain a NUL byte");
    }
    Ok(())
}

fn create_entry(state: &mut AppState, name: String, is_dir: bool) -> Vec<Effect> {
    if let Err(e) = validate_entry_name(&name) {
        state.set_error(e.to_string());
        return Vec::new();
    }
    let path = state.browser.cwd.join(&name);
    vec![Effect::CreateEntry { path, is_dir }]
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
            state.set_error(e.to_string());
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
                    state.set_error(e.to_string());
                    Vec::new()
                }
            }
        }
        Command::Delete => delete_confirm(state),
        Command::Tag { name } => {
            if let Err(e) = validate_name(&name) {
                state.set_error(e.to_string());
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
        Command::OpenWith { program, args } => {
            let Some(view) = state.browser.focused() else {
                state.message = Some(StatusMessage::info("nothing focused"));
                return Vec::new();
            };
            vec![Effect::OpenPathWith {
                path: view.entry.path.clone(),
                program,
                args,
            }]
        }
        Command::Cd { path } => {
            let dir = resolve_user_path(state, &path);
            navigate(state, dir)
        }
        Command::Mkdir { name } => create_entry(state, name, true),
        Command::Touch { name } => create_entry(state, name, false),
        Command::SelectAll => {
            if state.browser.entries.is_empty() {
                state.message = Some(StatusMessage::info("nothing to select"));
                return Vec::new();
            }
            state.browser.select_all();
            Vec::new()
        }
        Command::InvertSelection => {
            state.browser.invert_selection();
            Vec::new()
        }
        Command::Deselect => {
            state.browser.clear_selection();
            Vec::new()
        }
        Command::Filter { query } => {
            state.browser.set_filter(Some(query.clone()));
            state.message = Some(StatusMessage::info(format!("filter applied: {query}")));
            Vec::new()
        }
        Command::ClearFilter => {
            state.browser.set_filter(None);
            state.message = Some(StatusMessage::info("filter cleared"));
            Vec::new()
        }
        Command::Sort { field } => {
            let mode = match field.to_ascii_lowercase().as_str() {
                "name" => Some(SortMode::NameDirsFirst),
                "name-desc" | "name-descending" => Some(SortMode::NameDesc),
                "size" => Some(SortMode::Size),
                "size-desc" | "size-descending" => Some(SortMode::SizeDesc),
                "modified" | "time" | "date" => Some(SortMode::Modified),
                "modified-desc" | "time-desc" | "date-desc" => Some(SortMode::ModifiedDesc),
                _ => None,
            };
            if let Some(mode) = mode {
                let label = mode.label();
                state.browser.set_sort_mode(mode);
                state.message = Some(StatusMessage::info(format!("sorted by {label}")));
            } else {
                state.set_error("sort expects name, size, modified, or a -desc variant");
            }
            Vec::new()
        }
        Command::Refresh => {
            if matches!(state.mode, Mode::Browser) {
                state.message = Some(StatusMessage::info("refreshing directory"));
                vec![Effect::LoadDirectory(state.browser.cwd.clone())]
            } else {
                Vec::new()
            }
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
            state.set_error(e.to_string());
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
        ContextItem::OpenWith => open_with_prompt(state),
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
        Mode::Media(_) => return close_media(state, AfterStop::Close),
        Mode::Command
        | Mode::Confirm(_)
        | Mode::Conflict(_)
        | Mode::TagPicker(_)
        | Mode::ContextMenu(_)
        | Mode::Password(_)
        | Mode::OpenWith(_)
        | Mode::Bookmarks(_)
        | Mode::Help => {
            state.mode = Mode::Browser;
            state.command_input.clear();
        }
        Mode::Browser => {
            if state.browser.filter.is_some() {
                state.browser.set_filter(None);
            } else if !state.browser.selection.is_empty() || state.browser.visual {
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
                state.set_error(err);
                return vec![Effect::LoadDirectory(prev)];
            }
            state.set_error(err);
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
    if failed.is_empty() {
        state.message = Some(StatusMessage::info(text));
    } else {
        state.set_error(text);
    }
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
                if !matches!(state.mode, Mode::Browser) {
                    return Vec::new();
                }
                state.browser.selected = pos;
                let (c, r) = grid_dims(state);
                state.browser.clamp_scroll_grid(c, r);
                // Double-click requires the same entry, left button, within
                // the configured threshold; it is consumed once so one
                // double click can never trigger duplicate opens.
                let now = Instant::now();
                let is_double = matches!(
                    state.last_click,
                    Some((when, prev)) if prev == pos && now.duration_since(when) <= state.double_click
                );
                if is_double {
                    state.last_click = None;
                    return open_focused(state);
                }
                state.last_click = Some((now, pos));
                Vec::new()
            }
            MouseKind::Right => {
                if matches!(state.mode, Mode::Browser) {
                    state.browser.selected = pos;
                    let (c, r) = grid_dims(state);
                    state.browser.clamp_scroll_grid(c, r);
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
                let (c, r) = grid_dims(s);
                s.browser.grid_move(-(c as isize), c, r);
            }),
            MouseKind::ScrollDown => browser_only(state, |s| {
                let (c, r) = grid_dims(s);
                s.browser.grid_move(c as isize, c, r);
            }),
        },
        HitTarget::Sidebar(idx) => match kind {
            MouseKind::Left => {
                if !matches!(state.mode, Mode::Browser) {
                    return Vec::new();
                }
                match state.sidebar_items.get(idx).cloned() {
                    Some(SidebarItem::Place { path, .. })
                    | Some(SidebarItem::Mount { path, .. })
                    | Some(SidebarItem::Bookmark { path }) => navigate(state, path),
                    Some(SidebarItem::Tag { .. }) => open_picker(state),
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
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
            MouseKind::Left => {
                if matches!(state.mode, Mode::Password(_)) {
                    password_submit(state)
                } else if matches!(state.mode, Mode::OpenWith(_)) {
                    open_with_submit(state)
                } else {
                    confirm(state)
                }
            }
            _ => Vec::new(),
        },
        HitTarget::ModalCancel => match kind {
            MouseKind::Left => {
                if matches!(state.mode, Mode::OpenWith(_)) {
                    cancel(state)
                } else {
                    reduce(state, Action::Reject)
                }
            }
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
        HitTarget::MediaTogglePause => match kind {
            MouseKind::Left => reduce(state, Action::MediaTogglePause),
            _ => Vec::new(),
        },
        HitTarget::MediaSeekBack => match kind {
            MouseKind::Left => reduce(state, Action::MediaSeek(-5)),
            _ => Vec::new(),
        },
        HitTarget::MediaSeekForward => match kind {
            MouseKind::Left => reduce(state, Action::MediaSeek(5)),
            _ => Vec::new(),
        },
        HitTarget::MediaVolumeDown => match kind {
            MouseKind::Left => reduce(state, Action::MediaVolume(-5)),
            _ => Vec::new(),
        },
        HitTarget::MediaVolumeUp => match kind {
            MouseKind::Left => reduce(state, Action::MediaVolume(5)),
            _ => Vec::new(),
        },
        HitTarget::MediaStop => match kind {
            MouseKind::Left => reduce(state, Action::MediaStop),
            _ => Vec::new(),
        },
        HitTarget::MediaClose => match kind {
            MouseKind::Left => reduce(state, Action::MediaClose),
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
        LegendAction::OpenWith => reduce(state, Action::OpenWithPrompt),
        LegendAction::Parent => reduce(state, Action::OpenParent),
        LegendAction::Cancel => reduce(state, Action::Cancel),
        LegendAction::Encrypt => reduce(state, Action::EncryptToggle),
        LegendAction::Sidebar => reduce(state, Action::ToggleSidebar),
        LegendAction::Preview => reduce(state, Action::TogglePreview),
        LegendAction::Bookmarks => reduce(state, Action::OpenBookmarks),
    }
}
