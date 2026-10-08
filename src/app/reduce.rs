use ratatui::layout::Rect;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::action::{Action, ConflictDecision, DirectorySnapshot, MouseKind};
use crate::app::effects::Effect;
use crate::app::ranger;
use crate::app::state::{
    AppState, ClipMode, ClipboardState, ConfirmAction, ConfirmState, ConflictState, ContextItem,
    ContextMenuState, ContextTarget, DragPhase, DragState, HoverState, HubSection, MarqueePhase,
    MarqueeState, MediaState, Mode, OperationState, Password, PasswordPurpose, PasswordState,
    PreviewContent, StatusMessage, TagPickerState,
};
use crate::browser::SortMode;
use crate::crypto::CryptoKind;
use crate::input::command::{self, Command};
use crate::media::{AfterStop, MediaCommand, MediaKind, MediaPhase, classify_path};
use crate::operations::{
    ConflictPolicy, OpOutcome, OperationKind, OperationPlan, OperationReport, validate,
    validate_rename,
};
use crate::settings::ViewMode;
use crate::sidebar::SidebarItem;
use crate::tags::validate_name;
use crate::ui::hit::{HitTarget, LegendAction};

pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    let mut effects = reduce_inner(state, action);
    if !matches!(state.mode, Mode::Command) {
        state.address_bar = false;
    }
    sync_visual(state);
    if let Some(effect) = preview_followup(state) {
        effects.push(effect);
    }
    effects
}

/// Visual mode: the selection is the base set plus every entry between the
/// anchor and the cursor.
fn sync_visual(state: &mut AppState) {
    if !state.browser.visual {
        state.browser.visual_anchor = None;
        return;
    }
    let Some(anchor) = state.browser.visual_anchor else {
        return;
    };
    let cursor = state.browser.selected;
    let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
    let mut selection = state.browser.visual_base.clone();
    for (pos, (_, e)) in state.browser.visible_entries().enumerate() {
        if pos >= lo && pos <= hi {
            selection.insert(e.entry.path.clone());
        }
    }
    state.browser.selection = selection;
}

/// When the preview panel is visible and the focused entry changed (or its
/// modification metadata changed), ask for fresh preview content.
fn preview_followup(state: &AppState) -> Option<Effect> {
    if !crate::ui::preview_needed(state) && !matches!(state.mode, Mode::QuickLook(_)) {
        return None;
    }
    if state.mode.is_overlay() && !matches!(state.mode, Mode::QuickLook(_)) {
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
        is_dir: view.entry.is_dir_like(),
    })
}

pub(crate) fn reduce_inner(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::LoadInitial => {
            let mut fx = vec![Effect::LoadDirectory(state.browser.cwd.clone())];
            fx.extend(side_listing_effect(state));
            fx
        }
        Action::SetView(view) => set_view(state, view),
        Action::CycleView => {
            let next = state.view().next();
            set_view(state, next)
        }
        Action::GridZoom(large) => {
            state.settings.grid_size = if large {
                crate::settings::GridSize::Large
            } else {
                crate::settings::GridSize::Small
            };
            state.settings_dirty = true;
            if state.view() != ViewMode::Grid {
                return set_view(state, ViewMode::Grid);
            }
            Vec::new()
        }
        Action::SortBy(key) => {
            let current = state.browser.sort_mode;
            let mode = if current.key == key {
                current.reversed()
            } else {
                SortMode::new(key, false)
            };
            apply_sort(state, mode);
            Vec::new()
        }
        Action::SideListingLoaded { path, entries } => {
            let wanted = state.browser.cwd.parent().map(Path::to_path_buf);
            state
                .side_listings
                .retain(|p, _| Some(p) == wanted.as_ref());
            if Some(&path) == wanted.as_ref() {
                state.side_listings.insert(path, entries);
            }
            Vec::new()
        }
        // Legacy `g` key: the first half of the `gg` chord.
        Action::KeyG => ranger::chord_key(state, "g".to_string()),
        Action::ChordKey(token) => ranger::chord_key(state, token),
        Action::Repeat(n, inner) => {
            let mut fx = Vec::new();
            for _ in 0..n {
                fx.extend(reduce_inner(state, (*inner).clone()));
            }
            fx
        }
        Action::GotoIndex(i) => browser_only(state, |s| ranger::goto_index(s, i)),
        Action::GoTo(spec) => browser_only_fx(state, |s| ranger::goto_spec(s, &spec)),
        Action::HistoryBack => browser_only_fx(state, ranger::history_back),
        Action::HistoryForward => browser_only_fx(state, ranger::history_forward),
        Action::PreviousDir => browser_only_fx(state, ranger::previous_dir),
        Action::SetMark(c) => browser_only_fx(state, |s| ranger::set_mark(s, c)),
        Action::JumpMark(c) => browser_only_fx(state, |s| ranger::jump_mark(s, c)),
        Action::DeleteMark(c) => browser_only_fx(state, |s| ranger::delete_mark(s, c)),
        Action::EnterSearch => browser_only_fx(state, |s| {
            ranger::search_start(s, crate::app::state::SearchKind::Search)
        }),
        Action::EnterFind => browser_only_fx(state, |s| {
            ranger::search_start(s, crate::app::state::SearchKind::Find)
        }),
        Action::SearchNext => browser_only_fx(state, |s| ranger::search_step(s, true)),
        Action::SearchPrev => browser_only_fx(state, |s| ranger::search_step(s, false)),
        Action::SelectAll => browser_only(state, ranger::select_all),
        Action::InvertSelection => browser_only(state, ranger::invert_selection),
        Action::ClearSelection => browser_only(state, |s| s.browser.clear_selection()),
        Action::CopySelection => browser_only(state, |s| ranger::copy_selection(s, ClipMode::Copy)),
        Action::CutSelection => browser_only(state, |s| ranger::copy_selection(s, ClipMode::Cut)),
        Action::PasteHere { overwrite } => {
            browser_only_fx(state, |s| ranger::paste_here(s, overwrite))
        }
        Action::PasteSymlinks => browser_only_fx(state, ranger::paste_symlinks),
        Action::ClearClipboard => {
            state.clipboard = ClipboardState::default();
            state.message = Some(StatusMessage::info("clipboard cleared"));
            Vec::new()
        }
        Action::Yank(kind) => browser_only_fx(state, |s| ranger::yank(s, kind)),
        Action::TrashSelection => browser_only_fx(state, ranger::trash_selection),
        Action::DeleteSelection => browser_only_fx(state, ranger::delete_selection),
        Action::Undo => browser_only_fx(state, ranger::undo),
        Action::RenameStart(cursor) => browser_only_fx(state, |s| ranger::rename_start(s, cursor)),
        Action::EnterCreate => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                state.command_input = "create ".to_string();
            }
            Vec::new()
        }
        Action::EnterShell => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                state.command_input = "shell ".to_string();
            }
            Vec::new()
        }
        Action::Subshell => browser_only_fx(state, |s| {
            vec![
                Effect::RunShell {
                    command: None,
                    cwd: s.browser.cwd.clone(),
                },
                Effect::LoadDirectory(s.browser.cwd.clone()),
            ]
        }),
        Action::EditFocused => browser_only_fx(state, ranger::edit_focused),
        Action::QuickLook => browser_only_fx(state, ranger::quick_look),
        Action::QuickLookScroll(delta) => {
            if let Mode::QuickLook(q) = &mut state.mode {
                q.scroll = (q.scroll as isize + delta).max(0) as usize;
            }
            Vec::new()
        }
        Action::DiskUsage => browser_only_fx(state, ranger::disk_usage),
        Action::DiskUsageReady(sizes) => {
            let n = sizes.len();
            state.dir_sizes.extend(sizes);
            state.message = Some(StatusMessage::info(format!(
                "measured {n} folder{}",
                if n == 1 { "" } else { "s" }
            )));
            Vec::new()
        }
        Action::ChildCountsReady(counts) => {
            state.browser.child_counts.extend(counts);
            Vec::new()
        }
        Action::ToggleAnimations => {
            ranger::toggle_animations(state);
            Vec::new()
        }
        Action::TabNew => browser_only_fx(state, ranger::tab_new),
        Action::TabNext => browser_only_fx(state, |s| ranger::tab_step(s, 1)),
        Action::TabPrev => browser_only_fx(state, |s| ranger::tab_step(s, -1)),
        Action::TabClose => browser_only_fx(state, ranger::tab_close),
        Action::TabRestore => browser_only_fx(state, ranger::tab_restore),
        Action::TabSelect(i) => browser_only_fx(state, |s| ranger::tab_select(s, i)),
        Action::OpenUrlFromFile => browser_only_fx(state, crate::app::links::open_url_from_file),
        Action::FollowLink => browser_only_fx(state, ranger::follow_link),
        Action::Paste(text) => ranger::paste_text(state, text),
        Action::LineEdit(edit) => ranger::line_edit(state, edit),
        Action::PromptSubmit => ranger::prompt_submit(state),
        Action::EntryCreated(path) => {
            // Nested creations (`a/b/c`) focus and undo from the first new
            // component inside the current folder.
            let top = path
                .strip_prefix(&state.browser.cwd)
                .ok()
                .and_then(|rel| rel.components().next())
                .map(|first| state.browser.cwd.join(first))
                .unwrap_or_else(|| path.clone());
            let shown = path
                .strip_prefix(&state.browser.cwd)
                .map(|rel| rel.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            state.message = Some(StatusMessage::info(format!("created {shown} · uu undoes")));
            let existed_before = state.browser.entries.iter().any(|e| e.entry.path == top);
            let undo_target = if existed_before {
                path.clone()
            } else {
                top.clone()
            };
            state.undo.push(crate::app::state::UndoEntry {
                label: format!("create {shown}"),
                steps: vec![crate::app::state::UndoStep::Created(undo_target)],
            });
            let nested = path.parent() != Some(state.browser.cwd.as_path());
            state.pending_focus = Some(top);
            if nested {
                // The handler reloads the new entry's own parent; the
                // current folder needs its own listing to show `top`.
                vec![Effect::LoadDirectory(state.browser.cwd.clone())]
            } else {
                Vec::new()
            }
        }
        Action::UndoFinished { report } => operation_finished_with(state, report, false),
        Action::SetSort(mode) => {
            apply_sort(state, mode);
            Vec::new()
        }
        Action::ReverseSort => {
            let mode = state.browser.sort_mode.reversed();
            apply_sort(state, mode);
            Vec::new()
        }
        Action::FindResults { title, root, hits } => {
            if hits.is_empty() {
                state.message = Some(StatusMessage::info(format!("{title}: nothing found")));
                return Vec::new();
            }
            let matches = (0..hits.len()).collect();
            state.message = Some(StatusMessage::info(format!(
                "{title}: {} result{}{}",
                hits.len(),
                if hits.len() == 1 { "" } else { "s" },
                if hits.len() >= crate::search::MAX_HITS {
                    " (limit reached)"
                } else {
                    ""
                }
            )));
            state.mode = Mode::Results(Box::new(crate::app::state::ResultsState {
                title,
                root,
                hits,
                query: String::new(),
                matches,
                selected: 0,
            }));
            Vec::new()
        }
        Action::ResultsChar(c) => {
            if let Mode::Results(r) = &mut state.mode {
                r.query.push(c);
                refilter_results(r);
            }
            Vec::new()
        }
        Action::ResultsBackspace => {
            if let Mode::Results(r) = &mut state.mode {
                r.query.pop();
                refilter_results(r);
            }
            Vec::new()
        }
        Action::ResultsMove(delta) => {
            if let Mode::Results(r) = &mut state.mode {
                let len = r.matches.len();
                if len > 0 {
                    r.selected = (r.selected as isize + delta).clamp(0, len as isize - 1) as usize;
                }
            }
            Vec::new()
        }
        Action::ResultsSubmit => {
            let hit = match &state.mode {
                Mode::Results(r) => r
                    .matches
                    .get(r.selected)
                    .and_then(|&i| r.hits.get(i))
                    .cloned(),
                _ => None,
            };
            let Some(hit) = hit else {
                return Vec::new();
            };
            state.mode = Mode::Browser;
            let Some(parent) = hit.path.parent().map(Path::to_path_buf) else {
                return Vec::new();
            };
            let fx = if parent == state.browser.cwd {
                focus_path(state, &hit.path);
                Vec::new()
            } else {
                let fx = navigate(state, parent);
                state.pending_focus = Some(hit.path.clone());
                fx
            };
            if let Some((line, _)) = hit.line {
                state.message = Some(StatusMessage::info(format!(
                    "match on line {line} · i quick look"
                )));
            }
            fx
        }
        Action::BulkRenamePlan(pairs) => {
            if pairs.is_empty() {
                state.message = Some(StatusMessage::info("bulk rename: nothing changed"));
                return Vec::new();
            }
            let detail = pairs
                .iter()
                .take(6)
                .map(|(a, b)| {
                    format!(
                        "{} → {}",
                        a.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        b.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let count = pairs.len();
            state.mode = Mode::Confirm(Box::new(ConfirmState {
                title: format!(
                    "Rename {count} entr{}?",
                    if count == 1 { "y" } else { "ies" }
                ),
                detail,
                stage: 1,
                recursive: false,
                action: ConfirmAction::BulkRename { pairs },
            }));
            Vec::new()
        }
        Action::OpenWithCycle(delta) => {
            if let Mode::OpenWith(o) = &mut state.mode
                && !o.suggestions.is_empty()
            {
                let n = o.suggestions.len() as isize;
                let next = match o.suggestion {
                    Some(i) => (i as isize + delta).rem_euclid(n),
                    None if delta < 0 => n - 1,
                    None => 0,
                } as usize;
                o.suggestion = Some(next);
                o.input = o.suggestions[next].clone();
            }
            Vec::new()
        }
        Action::HelpChar(c) => {
            if matches!(state.mode, Mode::Help) {
                state.help_query.push(c);
                state.help_scroll = 0;
            }
            Vec::new()
        }
        Action::HelpBackspace => {
            state.help_query.pop();
            state.help_scroll = 0;
            Vec::new()
        }
        Action::HelpScroll(delta) => {
            state.help_scroll = (state.help_scroll as isize + delta).max(0) as usize;
            Vec::new()
        }
        Action::OpenWithToggleRemember => {
            if let Mode::OpenWith(o) = &mut state.mode {
                o.remember = !o.remember;
            }
            Vec::new()
        }
        Action::MediaPrev => crate::app::media_ctl::prev(state),
        Action::MediaMinimize => crate::app::media_ctl::minimize(state),
        Action::MediaExpand => crate::app::media_ctl::expand(state),
        Action::MediaMute => crate::app::media_ctl::toggle_mute(state),
        Action::MediaShuffle => crate::app::media_ctl::toggle_shuffle(state),
        Action::MediaRepeat => crate::app::media_ctl::cycle_repeat(state),
        Action::MediaSpeed(step) => crate::app::media_ctl::speed(state, step),
        Action::MediaSeekPercent(n) => crate::app::media_ctl::seek_percent(state, n),
        Action::MediaCycleSub => crate::app::media_ctl::cycle_sub(state),
        Action::MediaToggleSubs => crate::app::media_ctl::toggle_subs(state),
        Action::MediaCycleAudio => crate::app::media_ctl::cycle_audio(state),
        Action::MediaSubDelay(n) => crate::app::media_ctl::sub_delay(state, n),
        Action::MediaOpenSubs => crate::app::media_ctl::open_subs(state),
        Action::TrackInfoLoaded { session, info } => {
            let cover = info
                .cover
                .as_ref()
                .and_then(|bytes| image::load_from_memory(bytes).ok());
            if let Some(media) = crate::app::media_ctl::media_mut(state)
                && media.session == session
            {
                media.tags = Some(info.tags);
                state.cover = cover.map(|img| crate::app::state::Cover {
                    session,
                    image: Box::new(state.picker.new_resize_protocol(img)),
                });
            }
            Vec::new()
        }
        Action::SubsFound { session, files } => {
            crate::app::media_ctl::subs_found(state, session, files)
        }
        Action::SubPickerChar(c) => crate::app::media_ctl::picker_char(state, c),
        Action::SubPickerBackspace => crate::app::media_ctl::picker_backspace(state),
        Action::SubPickerMove(d) => crate::app::media_ctl::picker_move(state, d),
        Action::SubPickerSubmit => crate::app::media_ctl::picker_submit(state),
        Action::SubPickerClose => crate::app::media_ctl::picker_close(state),
        Action::MediaEnqueue => crate::app::media_ctl::enqueue(state),
        Action::MediaAddSub(path) => crate::app::media_ctl::add_sub(state, path),
        Action::MediaSetVolume(v) => crate::app::media_ctl::set_volume(state, v),
        Action::CommandComplete => {
            crate::app::cmdline::complete(state);
            Vec::new()
        }
        Action::CommandHistory(delta) => {
            crate::app::cmdline::history_step(state, delta);
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
        Action::OpenParent => browser_only_fx(state, go_parent),
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
            crate::app::hub::open(s);
            Vec::new()
        }),
        Action::BookmarkChar(c) => {
            if let Mode::Bookmarks(nav) = &mut state.mode {
                nav.query.push(c);
                nav.selected = 0;
            }
            crate::app::hub::refresh(state);
            Vec::new()
        }
        Action::BookmarkBackspace => {
            if let Mode::Bookmarks(nav) = &mut state.mode {
                nav.query.pop();
            }
            crate::app::hub::refresh(state);
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
        Action::BookmarkSection(delta) => {
            crate::app::hub::section_step(state, delta);
            Vec::new()
        }
        Action::BookmarkDelete => crate::app::hub::delete(state),
        Action::BookmarkSubmit => crate::app::hub::submit(state),
        Action::BookmarksChanged { bookmarks, message } => {
            state.bookmarks = bookmarks;
            state.message = Some(StatusMessage::info(message));
            crate::app::hub::refresh(state);
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
            let Some(media) = crate::app::media_ctl::media_mut(state) else {
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
                backend: media.backend,
            }]
        }
        Action::MediaBackendReady { session } => {
            let Some(media) = crate::app::media_ctl::media_mut(state) else {
                return Vec::new();
            };
            if media.session != session {
                return Vec::new();
            }
            media.error = None;
            media.phase = MediaPhase::Starting;
            let mut fx = vec![Effect::MediaCommand {
                session,
                command: MediaCommand::Load,
            }];
            if media.kind == MediaKind::Audio && media.tags.is_none() {
                fx.push(Effect::LoadTrackInfo {
                    session,
                    path: media.path.clone(),
                });
            }
            // A fresh backend starts at defaults: re-apply volume, mute,
            // speed and subtitles.
            fx.extend(
                crate::app::media_ctl::restore_commands(media)
                    .into_iter()
                    .map(|command| Effect::MediaCommand { session, command }),
            );
            fx
        }
        Action::MediaStatus {
            session,
            phase,
            position,
            duration,
            volume,
        } => {
            if let Some(media) = crate::app::media_ctl::media_mut(state)
                && media.session == session
                && !matches!(media.phase, MediaPhase::Stopping | MediaPhase::Error)
            {
                media.phase = phase;
                media.position = position.max(0.0);
                media.duration = duration;
                // A muted backend reports 0; keep the level to restore.
                if !media.muted {
                    media.volume = volume.min(crate::app::media_ctl::MAX_VOLUME);
                }
                if matches!(phase, MediaPhase::Playing) {
                    media.error = None;
                }
            }
            Vec::new()
        }
        Action::MediaStopped { session } => {
            let in_mini = !matches!(state.mode, Mode::Media(_)) && state.mini.is_some();
            let Some(media) = crate::app::media_ctl::media_mut(state) else {
                return Vec::new();
            };
            // Only a Stopping media accepts the terminal handback; any
            // duplicate or out-of-phase MediaStopped is stale.
            if media.session != session || media.phase != MediaPhase::Stopping {
                return Vec::new();
            }
            let after_stop = media.after_stop.take().unwrap_or(AfterStop::Close);
            match after_stop {
                AfterStop::Close if in_mini => {
                    state.mini = None;
                }
                AfterStop::Close => {
                    state.mode = Mode::Browser;
                }
                AfterStop::Quit => {
                    if in_mini {
                        state.mini = None;
                    } else {
                        state.mode = Mode::Browser;
                    }
                    return vec![Effect::Quit];
                }
                AfterStop::RestartAfterResize { position, paused } => {
                    media.phase = MediaPhase::Preparing;
                    media.position = position;
                    media.surface = None;
                    media.awaiting_surface_ready = true;
                    media.resume_position = Some(position);
                    media.resume_paused = Some(paused);
                }
                AfterStop::ShowError(message) => {
                    media.phase = MediaPhase::Error;
                    media.error = Some(message);
                }
            }
            Vec::new()
        }
        Action::MediaSpectrum { session, spectrum } => {
            if let Some(media) = crate::app::media_ctl::media_mut(state)
                && media.session == session
            {
                media.spectrum = spectrum.map(|value| value.clamp(0.0, 1.0));
            }
            Vec::new()
        }
        Action::MediaEnded { session } => {
            let Some(media) = crate::app::media_ctl::media_mut(state) else {
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
            // Queue auto-advance (repeat / shuffle aware).
            if let Some(pos) = crate::app::media_ctl::after_end(media) {
                let fx = crate::app::media_ctl::play_index(state, pos);
                if let Some(next) = crate::app::media_ctl::media_ref(state) {
                    let title = next.title();
                    state.message = Some(StatusMessage::info(format!("now playing {title}")));
                }
                return fx;
            }
            media.phase = MediaPhase::Stopping;
            media.after_stop = Some(AfterStop::Close);
            state.message = Some(StatusMessage::info(format!("finished {name}")));
            vec![Effect::StopMedia { session }]
        }
        Action::MediaFailed { session, message } => {
            let Some(media) = crate::app::media_ctl::media_mut(state) else {
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
        Action::MediaTogglePause => media_command(state, MediaCommand::TogglePause),
        Action::MediaSeek(seconds) => media_command(state, MediaCommand::SeekRelative(seconds)),
        Action::MediaSeekAbsolute(seconds) => {
            media_command(state, MediaCommand::SeekAbsolute(seconds))
        }
        Action::MediaToggleFullscreen => toggle_media_fullscreen(state),
        Action::MediaNext => next_media(state),
        Action::MediaVolume(delta) => crate::app::media_ctl::change_volume(state, delta),
        Action::MediaStop => media_command(state, MediaCommand::Stop),
        Action::MediaClose => close_media(state, AfterStop::Close),
        Action::ClipboardCopy { paths } => {
            state.clipboard = ClipboardState {
                mode: Some(ClipMode::Copy),
                items: paths,
            };
            Vec::new()
        }
        Action::ClipboardCut { paths } => {
            state.clipboard = ClipboardState {
                mode: Some(ClipMode::Cut),
                items: paths,
            };
            Vec::new()
        }
        Action::ClipboardPaste => paste_from_clipboard(state),
        Action::QuickTag => quick_tag(state),
        Action::OpenTagPicker => open_picker(state),
        Action::EnterCommand => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                state.command_input.clear();
                state.history_cursor = None;
                state.completions.clear();
                state.completion_index = None;
            }
            Vec::new()
        }
        Action::OpenAddressBar => {
            if matches!(state.mode, Mode::Browser) {
                state.mode = Mode::Command;
                let mut cwd = state.browser.cwd.display().to_string();
                if !cwd.ends_with('/') {
                    cwd.push('/');
                }
                state.command_input = format!("cd {cwd}");
                state.address_bar = true;
                crate::app::cmdline::reset(state);
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
                state.history_cursor = None;
                crate::app::cmdline::reset(state);
            }
            Vec::new()
        }
        Action::CommandBackspace => {
            if matches!(state.mode, Mode::Command) {
                // The address bar never erases its hidden `cd ` prefix.
                if state.address_bar && state.command_input.len() <= 3 {
                    return Vec::new();
                }
                state.command_input.pop();
                state.history_cursor = None;
                crate::app::cmdline::reset(state);
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
            state.help_query.clear();
            state.help_scroll = 0;
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
                let item = menu.items[menu.selected].action;
                return context_apply(state, item);
            }
            Vec::new()
        }
        Action::Mouse { kind, x, y, ctrl } => mouse(state, kind, x, y, ctrl),
        Action::Resize { width, height } => {
            state.width = width;
            state.height = height;
            state.drag = None;
            // Seek-rail interaction geometry is stale at the new size.
            if let Mode::Media(media) = &mut state.mode {
                media.clear_slider_state();
            }
            if let Mode::Media(media) = &mut state.mode
                && media.kind == crate::media::MediaKind::Video
                && !matches!(
                    media.phase,
                    MediaPhase::Stopping | MediaPhase::Preparing | MediaPhase::Error
                )
            {
                if width < 70 || height < 18 {
                    media.phase = MediaPhase::Stopping;
                    let message = "video playback needs at least 70x18 cells".to_string();
                    media.after_stop = Some(AfterStop::ShowError(message));
                    return vec![Effect::StopMedia {
                        session: media.session,
                    }];
                }
                let was_paused = matches!(media.phase, MediaPhase::Paused);
                media.phase = MediaPhase::Stopping;
                media.after_stop = Some(AfterStop::RestartAfterResize {
                    position: media.position,
                    paused: was_paused,
                });
                return vec![Effect::StopMedia {
                    session: media.session,
                }];
            }
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
        Action::DragCancel => {
            cancel_drag(state);
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
pub(crate) fn grid_dims(state: &AppState) -> (usize, usize) {
    let cols = state.grid_cols.max(1);
    let rows = (state.list_viewport / cols).max(1);
    (cols, rows)
}

fn side_listing_effect(state: &AppState) -> Option<Effect> {
    if state.view() != ViewMode::Columns {
        return None;
    }
    let parent = state.browser.cwd.parent()?.to_path_buf();
    if state.side_listings.contains_key(&parent) {
        return None;
    }
    Some(Effect::LoadSideListing(parent))
}

fn set_view(state: &mut AppState, view: ViewMode) -> Vec<Effect> {
    if state.view() == view {
        return Vec::new();
    }
    state.settings.view = view;
    state.settings_dirty = true;
    state.hover = HoverState::default();
    cancel_drag(state);
    state.anim.start_cascade(state.now);
    state.message = Some(StatusMessage::info(format!("{} layout", view.label())));
    side_listing_effect(state).into_iter().collect()
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

pub(crate) fn browser_only(state: &mut AppState, f: impl FnOnce(&mut AppState)) -> Vec<Effect> {
    if matches!(state.mode, Mode::Browser) {
        f(state);
    }
    Vec::new()
}

pub(crate) fn browser_only_fx(
    state: &mut AppState,
    f: impl FnOnce(&mut AppState) -> Vec<Effect>,
) -> Vec<Effect> {
    if matches!(state.mode, Mode::Browser) {
        return f(state);
    }
    Vec::new()
}

fn navigate(state: &mut AppState, dir: PathBuf) -> Vec<Effect> {
    navigate_with(state, dir, true)
}

/// Changes directory. `record` pushes the current folder onto the back
/// history (history jumps themselves pass false).
pub(crate) fn navigate_with(state: &mut AppState, dir: PathBuf, record: bool) -> Vec<Effect> {
    let cwd = state.browser.cwd.clone();
    if record && cwd != dir && !cwd.as_os_str().is_empty() {
        state.history.back.push(cwd.clone());
        if state.history.back.len() > 200 {
            state.history.back.remove(0);
        }
        state.history.forward.clear();
    }
    if cwd != dir {
        state.previous_dir = Some(cwd);
    }
    state.settings.record_visit(&dir, state.wall_clock);
    state.settings_dirty = true;
    state.pending_nav = Some(state.browser.cwd.clone());
    cancel_drag(state);
    // A filename search is scoped to one directory, matching desktop file
    // managers: changing location should not hide unrelated entries.
    state.browser.set_filter(None);
    state.browser.search = None;
    state.browser.enter(&dir);
    state.hover = HoverState::default();
    state.anim.start_cascade(state.now);
    let mut fx = vec![Effect::LoadDirectory(dir)];
    fx.extend(side_listing_effect(state));
    fx
}

/// Goes to the parent directory and puts the cursor back on the child we
/// came from (ranger behavior).
fn go_parent(state: &mut AppState) -> Vec<Effect> {
    let cwd = state.browser.cwd.clone();
    match cwd.parent().map(Path::to_path_buf) {
        Some(parent) if parent != cwd => {
            let fx = navigate(state, parent);
            state.pending_focus = Some(cwd);
            fx
        }
        _ => Vec::new(),
    }
}

fn media_command(state: &mut AppState, command: MediaCommand) -> Vec<Effect> {
    let Some(media) = crate::app::media_ctl::media_ref(state) else {
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
    let Some(media) = crate::app::media_ctl::media_mut(state) else {
        return Vec::new();
    };
    if media.phase == MediaPhase::Stopping {
        return Vec::new();
    }
    media.phase = MediaPhase::Stopping;
    media.after_stop = Some(after_stop);
    // A closing session leaves no rail hover/drag residue behind.
    media.clear_slider_state();
    vec![Effect::StopMedia {
        session: media.session,
    }]
}

/// Flips video fullscreen through the supervised stop/restart cycle, reusing
/// the resize-resume machinery so position and pause state survive.
fn toggle_media_fullscreen(state: &mut AppState) -> Vec<Effect> {
    let Mode::Media(media) = &mut state.mode else {
        return Vec::new();
    };
    if media.kind == MediaKind::Video {
        match media.backend {
            // The GUI window toggles in place, no restart.
            crate::media::VideoBackend::Window => {
                media.fullscreen = !media.fullscreen;
                let on = media.fullscreen;
                return media_command(state, MediaCommand::SetFullscreen(on));
            }
            crate::media::VideoBackend::Tct => {
                state.message = Some(StatusMessage::info(
                    "text video always uses the whole terminal",
                ));
                return Vec::new();
            }
            _ => {}
        }
    }
    let Mode::Media(media) = &state.mode else {
        return Vec::new();
    };
    let allowed = media.kind == MediaKind::Video
        && matches!(
            media.phase,
            MediaPhase::Playing | MediaPhase::Paused | MediaPhase::Stopped
        );
    if !allowed {
        return Vec::new();
    }
    let (position, paused) = {
        let Mode::Media(media) = &mut state.mode else {
            return Vec::new();
        };
        media.fullscreen = !media.fullscreen;
        media.clear_slider_state();
        (media.position, media.phase == MediaPhase::Paused)
    };
    close_media(state, AfterStop::RestartAfterResize { position, paused })
}

/// Advances to the next queue entry (shuffle aware; wraps with repeat all).
fn next_media(state: &mut AppState) -> Vec<Effect> {
    crate::app::media_ctl::next(state)
}

fn open_focused(state: &mut AppState) -> Vec<Effect> {
    browser_only_fx(state, |s| {
        let Some(view) = s.browser.focused() else {
            return Vec::new();
        };
        let path = view.entry.path.clone();
        if view.entry.is_dir_like() {
            navigate(s, path)
        } else {
            crate::app::open::open_file(s, path)
        }
    })
}

fn prompt_open_with(state: &mut AppState, target: PathBuf) {
    crate::app::open::prompt(state, target);
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
    let remember = dialog.remember;
    state.mode = Mode::Browser;
    if input.trim().is_empty() {
        state.set_error("no command entered");
        return Vec::new();
    }
    match command::split_words(&input) {
        Ok(words) => {
            if words.is_empty() {
                state.set_error("no command entered");
                return Vec::new();
            }
            let assoc = crate::settings::Association {
                command: input.trim().to_string(),
                detach: crate::app::open::default_detach(&input),
            };
            if remember && let Some(key) = crate::settings::association_key(&target) {
                state
                    .settings
                    .associations
                    .insert(key.clone(), assoc.clone());
                state.settings_dirty = true;
                state.message = Some(StatusMessage::info(format!(
                    ".{key} will open with {} (r changes it)",
                    words[0]
                )));
            }
            crate::app::open::launch(&target, &assoc)
                .into_iter()
                .collect()
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
        let targets = s.browser.action_targets();
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
    let targets = state.browser.action_targets();
    open_picker_with(state, targets)
}

fn open_picker_with(state: &mut AppState, targets: Vec<PathBuf>) -> Vec<Effect> {
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

pub(crate) fn resolve_user_path(state: &AppState, input: &str) -> PathBuf {
    let decoded;
    let input = match input.trim().strip_prefix("file://") {
        Some(rest) => {
            decoded = crate::urls::percent_decode(rest);
            decoded.as_str()
        }
        None => input.trim(),
    };
    let expanded = if let Some(rest) = input.strip_prefix("~/") {
        state.home.join(rest)
    } else if input == "~" {
        state.home.clone()
    } else {
        PathBuf::from(input)
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        state.browser.cwd.join(expanded)
    };
    normalize_lexically(&joined)
}

/// Removes `.` and resolves `..` without touching the filesystem.
fn normalize_lexically(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component);
                }
                if out.as_os_str().is_empty() {
                    out.push("/");
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn delete_confirm(state: &mut AppState) -> Vec<Effect> {
    delete_confirm_targets(state, state.browser.action_targets())
}

/// Builds the recursive-delete confirmation for explicit captured targets
/// (keyboard selection/focus or a context menu's captured paths).
pub(crate) fn delete_confirm_targets(state: &mut AppState, targets: Vec<PathBuf>) -> Vec<Effect> {
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
    state.completions.clear();
    state.history_cursor = None;
    state.settings.push_history(&input);
    state.settings_dirty = true;
    let parsed = match command::parse(&input) {
        Ok(c) => c,
        Err(e) => {
            state.set_error(e.to_string());
            return Vec::new();
        }
    };
    let parsed = match crate::app::commands::run_or_return(state, parsed) {
        Ok(fx) => return fx,
        Err(cmd) => cmd,
    };
    match parsed {
        Command::Copy { dest } => start_copy_move(state, OperationKind::Copy, dest),
        Command::Move { dest } => start_copy_move(state, OperationKind::Move, dest),
        Command::Rename { name } => {
            let sources = state
                .browser
                .focused()
                .map(|v| vec![v.entry.path.clone()])
                .unwrap_or_default();
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
            let targets = state.browser.action_targets();
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
            let targets = state.browser.action_targets();
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
            if crate::urls::is_web_url(path.trim()) {
                return crate::app::links::open_url(state, path.trim());
            }
            let target = resolve_user_path(state, &path);
            // A file target opens its folder with the file focused.
            let is_file = state
                .browser
                .entries
                .iter()
                .find(|e| e.entry.path == target)
                .map(|e| !e.entry.is_dir_like())
                .unwrap_or_else(|| std::fs::metadata(&target).is_ok_and(|m| m.is_file()));
            match target.parent() {
                Some(parent) if is_file => {
                    let parent = parent.to_path_buf();
                    if parent == state.browser.cwd {
                        focus_path(state, &target);
                        Vec::new()
                    } else {
                        let fx = navigate(state, parent);
                        state.pending_focus = Some(target);
                        fx
                    }
                }
                _ => navigate(state, target),
            }
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
            if let Some(mode) = SortMode::parse(&field) {
                apply_sort(state, mode);
            } else {
                state.set_error(
                    "sort expects name, size, modified, type, extension, or a -desc variant",
                );
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
        // Extended commands were dispatched by `commands::run_or_return`.
        _ => Vec::new(),
    }
}

/// Applies a sort order, persists it, and reports it.
pub fn apply_sort(state: &mut AppState, mode: SortMode) {
    state.browser.set_sort_mode(mode);
    state.settings.sort = mode.token();
    state.settings_dirty = true;
    let arrow = if mode.desc { "descending" } else { "ascending" };
    state.message = Some(StatusMessage::info(format!(
        "sorted by {} ({arrow})",
        mode.label()
    )));
}

fn start_copy_move(state: &mut AppState, kind: OperationKind, dest: String) -> Vec<Effect> {
    let sources = state.browser.action_targets();
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

pub(crate) fn start_operation(state: &mut AppState, plan: OperationPlan) -> Vec<Effect> {
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
        ConfirmAction::BulkRename { pairs } => {
            state.message = Some(StatusMessage::info(format!(
                "renaming {} entr{}",
                pairs.len(),
                if pairs.len() == 1 { "y" } else { "ies" }
            )));
            vec![Effect::MovePairs(pairs)]
        }
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
        ConflictDecision::KeepBoth => {
            let mut plan = *conflict.plan;
            plan.policy = ConflictPolicy::KeepBoth;
            vec![Effect::RunOperation(Box::new(plan))]
        }
    }
}

fn context_apply(state: &mut AppState, item: ContextItem) -> Vec<Effect> {
    let Mode::ContextMenu(menu) = std::mem::replace(&mut state.mode, Mode::Browser) else {
        return Vec::new();
    };
    // Menus carry explicit captured targets; nothing routes back through
    // browser.targets() so multi-selections never collapse per-item.
    if !item_enabled(&menu, item) {
        return Vec::new();
    }
    match item {
        ContextItem::Open => match menu.target {
            ContextTarget::Single { path } => open_explicit(state, path),
            _ => Vec::new(),
        },
        ContextItem::OpenWith => match menu.target {
            ContextTarget::Single { path } => {
                prompt_open_with(state, path);
                Vec::new()
            }
            _ => Vec::new(),
        },
        ContextItem::Rename => {
            // Rename is single-item by nature; focus the captured row and
            // prefill command mode (Command::Rename acts on the cursor row).
            if let ContextTarget::Single { path } = &menu.target {
                focus_path(state, path);
                state.mode = Mode::Command;
                state.command_input = "rename ".to_string();
            }
            Vec::new()
        }
        ContextItem::Cut => {
            state.clipboard = ClipboardState {
                mode: Some(ClipMode::Cut),
                items: menu.target.paths(),
            };
            Vec::new()
        }
        ContextItem::ClipboardCopy => {
            state.clipboard = ClipboardState {
                mode: Some(ClipMode::Copy),
                items: menu.target.paths(),
            };
            Vec::new()
        }
        ContextItem::Paste => paste_from_clipboard(state),
        ContextItem::Delete => delete_confirm_targets(state, menu.target.paths()),
        ContextItem::Tags => match menu.target {
            ContextTarget::Single { path } => open_picker_with(state, vec![path]),
            _ => Vec::new(),
        },
    }
}

fn item_enabled(menu: &ContextMenuState, item: ContextItem) -> bool {
    menu.items.iter().any(|mi| mi.action == item && mi.enabled)
}

/// Opens an explicit path from a context menu without touching the
/// navigation cursor or the selection set.
fn open_explicit(state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    if !matches!(state.mode, Mode::Browser) {
        return Vec::new();
    }
    let is_dir = state
        .browser
        .entries
        .iter()
        .any(|e| e.entry.path == path && e.entry.is_dir_like());
    if is_dir {
        navigate(state, path)
    } else {
        focus_path(state, &path);
        crate::app::open::open_file(state, path)
    }
}

/// Moves the navigation cursor onto `path` when visible (no selection
/// mutation); used only where a flow is inherently focused-coupled.
pub(crate) fn focus_path(state: &mut AppState, path: &Path) {
    if let Some(pos) = state
        .browser
        .visible_indices()
        .iter()
        .position(|&i| state.browser.entries[i].entry.path == path)
    {
        state.browser.selected = pos;
        let (c, r) = grid_dims(state);
        state.browser.clamp_scroll_grid(c, r);
    }
}

/// Builds the playlist context for media sessions: every visible entry of
/// the same media kind in display order, with `current`'s position.
pub fn media_playlist(state: &AppState, current: &Path, kind: MediaKind) -> (Vec<PathBuf>, usize) {
    // Playing from a multi-selection makes the selection the queue.
    let selected: Vec<PathBuf> = state
        .browser
        .visible_entries()
        .map(|(_, e)| &e.entry.path)
        .filter(|p| state.browser.selection.contains(*p) && classify_path(p) == Some(kind))
        .cloned()
        .collect();
    if selected.len() > 1 && selected.iter().any(|p| p == current) {
        let pos = selected.iter().position(|p| p == current).unwrap_or(0);
        return (selected, pos);
    }
    let playlist: Vec<PathBuf> = state
        .browser
        .visible_entries()
        .map(|(_, e)| &e.entry.path)
        .filter(|p| classify_path(p) == Some(kind))
        .cloned()
        .collect();
    let pos = playlist.iter().position(|p| p == current).unwrap_or(0);
    (playlist, pos)
}

/// Mints a fresh media session carrying playlist context.
pub fn start_media_session(state: &mut AppState, path: PathBuf, kind: MediaKind) -> Vec<Effect> {
    let session = state.next_media_session;
    state.next_media_session = state.next_media_session.wrapping_add(1).max(1);
    let (playlist, pos) = media_playlist(state, &path, kind);
    let mut media = MediaState::preparing_with_playlist(session, path, kind, playlist, pos);
    media.volume = state.settings.volume.min(crate::app::media_ctl::MAX_VOLUME);
    crate::app::media_ctl::choose_backend(state, &mut media);
    // A new track replaces whatever plays in the background.
    let mut fx = Vec::new();
    if let Some(mini) = state.mini.take() {
        fx.push(Effect::StopMedia {
            session: mini.session,
        });
    }
    state.mode = Mode::Media(Box::new(media));
    fx
}

fn refilter_results(r: &mut crate::app::state::ResultsState) {
    let query = r.query.to_lowercase();
    let mut scored: Vec<(i32, usize)> = r
        .hits
        .iter()
        .enumerate()
        .filter_map(|(i, hit)| {
            let rel = hit
                .path
                .strip_prefix(&r.root)
                .unwrap_or(&hit.path)
                .to_string_lossy()
                .to_lowercase();
            crate::app::fuzzy::fuzzy_score(&query, &rel).map(|s| (s, i))
        })
        .collect();
    if !query.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    r.matches = scored.into_iter().map(|(_, i)| i).collect();
    r.selected = r.selected.min(r.matches.len().saturating_sub(1));
}

/// Streams a URL through the media player (mpv + yt-dlp).
pub fn start_url_media(state: &mut AppState, url: String) -> Vec<Effect> {
    let kind = classify_path(Path::new(url.split(['?', '#']).next().unwrap_or(&url)))
        .unwrap_or(MediaKind::Video);
    let session = state.next_media_session;
    state.next_media_session = state.next_media_session.wrapping_add(1).max(1);
    let mut media =
        MediaState::preparing_with_playlist(session, PathBuf::from(url), kind, Vec::new(), 0);
    media.volume = state.settings.volume.min(crate::app::media_ctl::MAX_VOLUME);
    crate::app::media_ctl::choose_backend(state, &mut media);
    state.mode = Mode::Media(Box::new(media));
    Vec::new()
}

/// Shows a one-off picker of links (URLs found in a file).
pub fn open_link_picker(state: &mut AppState, links: Vec<crate::urls::Link>) {
    crate::app::hub::open_link_picker(state, links);
}

/// Starts a copy/move of every clipboard item into the current directory.
fn paste_from_clipboard(state: &mut AppState) -> Vec<Effect> {
    let Some(mode) = state.clipboard.mode else {
        return Vec::new();
    };
    if state.clipboard.items.is_empty() || matches!(state.mode, Mode::Media(_)) {
        return Vec::new();
    }
    let op_kind = match mode {
        ClipMode::Copy => OperationKind::Copy,
        ClipMode::Cut => OperationKind::Move,
    };
    let plan = OperationPlan {
        kind: op_kind,
        sources: state.clipboard.items.clone(),
        dest_dir: Some(state.browser.cwd.clone()),
        rename_to: None,
        policy: ConflictPolicy::Ask,
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

fn cancel(state: &mut AppState) -> Vec<Effect> {
    cancel_drag(state);
    let had_chord = !state.pending_keys.is_empty() || state.pending_count.is_some();
    ranger::clear_pending(state);
    if had_chord {
        return Vec::new();
    }
    if ranger::prompt_cancel(state) {
        return Vec::new();
    }
    if matches!(state.mode, Mode::Help) && !state.help_query.is_empty() {
        state.help_query.clear();
        state.help_scroll = 0;
        return Vec::new();
    }
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
        | Mode::QuickLook(_)
        | Mode::Results(_)
        | Mode::Rename(_)
        | Mode::Search(_)
        | Mode::Help => {
            state.mode = Mode::Browser;
            state.command_input.clear();
        }
        Mode::Browser => {
            if state.browser.search.is_some() {
                state.browser.search = None;
            } else if state.browser.filter.is_some() {
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
            // A fresh listing invalidates any in-flight band geometry.
            cancel_drag(state);
            let mut fx = Vec::new();
            if snapshot.path == state.browser.cwd {
                state.browser.set_entries(snapshot.entries);
                if let Some(target) = state.pending_focus.take() {
                    focus_path(state, &target);
                }
                state.browser.child_counts.clear();
                fx.extend(ranger::child_count_effect(state));
            }
            state.tag_defs = snapshot.defs;
            state.pending_nav = None;
            if let Mode::TagPicker(picker) = &mut state.mode {
                picker.defs = state.tag_defs.clone();
                if picker.selected >= picker.defs.len() {
                    picker.selected = picker.defs.len().saturating_sub(1);
                }
            }
            return fx;
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
    operation_finished_with(state, report, true)
}

/// Friendly summary for a job that fully succeeded.
fn success_message(report: &OperationReport, record_undo: bool) -> String {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string())
    };
    let done = report.done_count();
    let what = match report.results.first() {
        Some(first) if done == 1 => name(&first.source),
        _ => format!("{done} items"),
    };
    if !record_undo {
        return format!("undone · {what}");
    }
    match report.kind {
        Some(OperationKind::Copy) => format!("copied {what} · uu undoes"),
        Some(OperationKind::Move) => format!("moved {what} · uu undoes"),
        Some(OperationKind::Trash) => format!("moved {what} to the trash · uu undoes"),
        Some(OperationKind::Symlink) => format!("linked {what} · uu undoes"),
        Some(OperationKind::Delete) => format!("deleted {what}"),
        Some(OperationKind::Encrypt | OperationKind::Decrypt) => format!("{done}/{done} done"),
        None => match report.moves.as_slice() {
            [(from, to)] => format!("renamed {} → {} · uu undoes", name(from), name(to)),
            _ => format!("renamed {what} · uu undoes"),
        },
    }
}

fn operation_finished_with(
    state: &mut AppState,
    report: OperationReport,
    record_undo: bool,
) -> Vec<Effect> {
    state.operation = None;
    if record_undo {
        let label = match report.kind {
            Some(OperationKind::Copy) => "copy",
            Some(OperationKind::Move) => "move",
            Some(OperationKind::Trash) => "trash",
            Some(OperationKind::Symlink) => "link",
            _ => "rename",
        };
        ranger::journal(state, label, &report);
    }
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
        if skipped == 0 && done > 0 {
            text = success_message(&report, record_undo);
        }
        state.message = Some(StatusMessage::info(text));
    } else {
        state.set_error(text);
    }
    // Paste lifecycle: a completed Cut removes the successfully moved
    // sources from the clipboard (failed/skipped stay so the paste can be
    // retried); Copy keeps everything for repeat pastes. Either way the
    // pending flag is consumed.
    let pending = state.pending_paste_mode.take();
    if pending == Some(ClipMode::Cut) {
        let moved: Vec<PathBuf> = report.moves.iter().map(|(from, _)| from.clone()).collect();
        state.clipboard.items.retain(|p| !moved.contains(p));
        if state.clipboard.items.is_empty() {
            state.clipboard.mode = None;
        }
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

fn mouse(state: &mut AppState, kind: MouseKind, x: u16, y: u16, ctrl: bool) -> Vec<Effect> {
    // Drag-and-drop intercepts left-button motion and release before the
    // ordinary click handling; the click path stays untouched below.
    if let Some(drag) = state.drag.clone() {
        if kind != MouseKind::Left {
            return handle_drag_motion(state, drag, kind, x, y);
        }
        // A fresh left-down means no release was ever seen (test harnesses
        // synthesize clicks directly): drop the stale armed press and fall
        // through to the normal click path.
        state.drag = None;
    }
    // An armed marquee owns pointer motion and release until it finishes;
    // what lies under the pointer is irrelevant while the band is live.
    if let Some(mut marquee) = state.marquee.take() {
        match kind {
            MouseKind::LeftDrag | MouseKind::Moved => {
                let dx = x.abs_diff(marquee.origin.0);
                let dy = y.abs_diff(marquee.origin.1);
                if marquee.phase == MarqueePhase::Armed
                    && dx <= DRAG_THRESHOLD_CELLS
                    && dy <= DRAG_THRESHOLD_CELLS
                {
                    marquee.current = (x, y);
                    state.marquee = Some(marquee);
                    return Vec::new();
                }
                marquee.phase = MarqueePhase::Selecting;
                marquee.current = (x, y);
                apply_marquee(state, &marquee);
                state.marquee = Some(marquee);
                return Vec::new();
            }
            MouseKind::LeftUp => {
                let selecting = marquee.phase == MarqueePhase::Selecting
                    || x.abs_diff(marquee.origin.0) > DRAG_THRESHOLD_CELLS
                    || y.abs_diff(marquee.origin.1) > DRAG_THRESHOLD_CELLS;
                if selecting {
                    marquee.phase = MarqueePhase::Selecting;
                    marquee.current = (x, y);
                    apply_marquee(state, &marquee);
                } else if matches!(state.mode, Mode::Browser) {
                    // Simple background click without a real drag: clear the
                    // selection, like any desktop file explorer.
                    state.browser.clear_selection();
                }
                return Vec::new();
            }
            // A fresh press cancels the gesture and falls through to normal
            // click handling (which may arm a new marquee at the new origin).
            MouseKind::Left => {}
            // Right click and scrolling during a live band are ignored.
            _ => return Vec::new(),
        }
    }
    // A fresh left press supersedes an in-flight rail gesture without
    // committing (mirroring the stale-press drop above).
    if kind == MouseKind::Left
        && media_slider_drag_active(state)
        && let Mode::Media(media) = &mut state.mode
    {
        media.clear_slider_state();
    }
    // While a rail drag is live it owns the pointer until release, even
    // when the pointer slips off the registered track: updates clamp to
    // the nearest endpoint, and the release commits exactly once.
    if media_slider_drag_active(state) && matches!(kind, MouseKind::LeftDrag | MouseKind::LeftUp) {
        return match kind {
            MouseKind::LeftUp => commit_slider_drag(state),
            _ => {
                let rect = state.hit_map.rect_for(HitTarget::MediaSeekRail);
                if let Mode::Media(media) = &mut state.mode
                    && let Some(seconds) = rect.and_then(|r| rail_seconds(r, x, media.duration))
                {
                    media.slider_drag_pos = Some(seconds);
                }
                Vec::new()
            }
        };
    }
    // While a context menu is open, a left press anywhere off its items
    // dismisses it (the clipboard stays untouched); a right press dismisses
    // and falls through so the normal dispatch can reopen a menu at the
    // new point.
    if matches!(state.mode, Mode::ContextMenu(_)) {
        let on_item = matches!(state.hit_map.hit(x, y), Some(HitTarget::ContextItem(_)));
        match kind {
            MouseKind::Right => state.mode = Mode::Browser,
            MouseKind::Left if !on_item => {
                state.mode = Mode::Browser;
                return Vec::new();
            }
            _ => {}
        }
    }
    let Some(target) = state.hit_map.hit(x, y) else {
        // Off every registered region: nothing is hovered.
        if kind == MouseKind::Moved {
            state.hover = HoverState::default();
            if let Mode::Media(media) = &mut state.mode {
                media.slider_hover = None;
            }
        }
        return Vec::new();
    };
    // True pointer-hover bookkeeping: the hovered grid row or control, and
    // the seek-rail preview while a media session is up. Runs before the
    // dispatch so dedicated arms (menu-item selection) still apply.
    if kind == MouseKind::Moved {
        update_hover(state, target, x);
    }
    match target {
        HitTarget::GridBackground => match kind {
            MouseKind::Left => {
                // Only browser mode arms gestures; overlays register their
                // own blockers above this region anyway.
                if matches!(state.mode, Mode::Browser) {
                    let base = if ctrl {
                        state.browser.selected_paths_set().clone()
                    } else {
                        std::collections::BTreeSet::new()
                    };
                    state.marquee = Some(MarqueeState::armed((x, y), base));
                }
                Vec::new()
            }
            MouseKind::Right => {
                // Background right-click: paste-oriented menu; the current
                // selection is left untouched.
                if matches!(state.mode, Mode::Browser) {
                    let target = ContextTarget::Background;
                    state.mode = Mode::ContextMenu(Box::new(ContextMenuState {
                        items: ContextItem::menu_for(&target, !state.clipboard.is_empty()),
                        target,
                        selected: 0,
                        x,
                        y,
                    }));
                }
                Vec::new()
            }
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
                // Arm a potential drag: snapshot the source set now because
                // sorting/filtering/refresh may shift rows during a drag.
                state.drag = Some(DragState {
                    phase: DragPhase::Armed,
                    origin: (x, y),
                    sources: drag_sources(state),
                    cursor: (x, y),
                });
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
                    if let Some(path) = state
                        .browser
                        .visible_indices()
                        .get(pos)
                        .and_then(|&i| state.browser.entries.get(i))
                        .map(|v| v.entry.path.clone())
                    {
                        // Right-clicking a selection member acts on the
                        // whole selection; anything else targets just the
                        // clicked row. The selection set itself is left
                        // untouched in both cases.
                        let selection = state.browser.selected_paths_set();
                        let target = if selection.contains(&path) && selection.len() > 1 {
                            // A BTreeSet iterates sorted and deduped.
                            ContextTarget::Bulk {
                                paths: selection.iter().cloned().collect(),
                            }
                        } else {
                            ContextTarget::Single { path }
                        };
                        state.mode = Mode::ContextMenu(Box::new(ContextMenuState {
                            items: ContextItem::menu_for(&target, !state.clipboard.is_empty()),
                            target,
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
            MouseKind::LeftUp | MouseKind::LeftDrag | MouseKind::Moved => Vec::new(),
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
                    Some(SidebarItem::Link { url, .. }) => crate::app::links::open_url(state, &url),
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
        HitTarget::ContextItem(idx) => match kind {
            MouseKind::Moved => {
                // Pointer hover previews the entry: selection follows the
                // pointer, but a hover never executes the command.
                if let Mode::ContextMenu(menu) = &mut state.mode {
                    menu.selected = idx;
                }
                Vec::new()
            }
            MouseKind::Left => {
                if let Mode::ContextMenu(menu) = &mut state.mode {
                    menu.selected = idx;
                }
                reduce(state, Action::ContextChoose)
            }
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
        HitTarget::ConflictKeepBoth => match kind {
            MouseKind::Left => conflict_choice(state, ConflictDecision::KeepBoth),
            _ => Vec::new(),
        },
        HitTarget::HubTab(idx) => match kind {
            MouseKind::Left => {
                let current = match &state.mode {
                    Mode::Bookmarks(nav) => HubSection::ALL.iter().position(|s| *s == nav.section),
                    _ => None,
                };
                match current {
                    Some(cur) => {
                        crate::app::hub::section_step(state, idx as isize - cur as isize);
                        Vec::new()
                    }
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        },
        HitTarget::HubRow(idx) => match kind {
            MouseKind::Moved => {
                if let Mode::Bookmarks(nav) = &mut state.mode {
                    nav.selected = idx;
                }
                Vec::new()
            }
            MouseKind::Left => {
                if let Mode::Bookmarks(nav) = &mut state.mode {
                    nav.selected = idx;
                }
                crate::app::hub::submit(state)
            }
            MouseKind::ScrollUp => reduce(state, Action::BookmarkMove(-1)),
            MouseKind::ScrollDown => reduce(state, Action::BookmarkMove(1)),
            _ => Vec::new(),
        },
        HitTarget::ResultRow(idx) => match kind {
            MouseKind::Moved => {
                if let Mode::Results(r) = &mut state.mode {
                    r.selected = idx;
                }
                Vec::new()
            }
            MouseKind::Left => {
                if let Mode::Results(r) = &mut state.mode {
                    r.selected = idx;
                }
                reduce(state, Action::ResultsSubmit)
            }
            MouseKind::ScrollUp => reduce(state, Action::ResultsMove(-1)),
            MouseKind::ScrollDown => reduce(state, Action::ResultsMove(1)),
            _ => Vec::new(),
        },
        HitTarget::OpenWithChip(idx) => match kind {
            MouseKind::Left => {
                if let Mode::OpenWith(o) = &mut state.mode
                    && let Some(cmd) = o.suggestions.get(idx).cloned()
                {
                    o.suggestion = Some(idx);
                    o.input = cmd;
                }
                Vec::new()
            }
            _ => Vec::new(),
        },
        HitTarget::OpenWithRemember => match kind {
            MouseKind::Left => reduce(state, Action::OpenWithToggleRemember),
            _ => Vec::new(),
        },
        HitTarget::Tab(idx) => match kind {
            MouseKind::Left => reduce(state, Action::TabSelect(idx)),
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
        HitTarget::MediaTogglePause => match kind {
            MouseKind::Left => reduce(state, Action::MediaTogglePause),
            _ => Vec::new(),
        },
        HitTarget::MediaSeekBack => match kind {
            MouseKind::Left => reduce(state, Action::MediaSeek(-15)),
            _ => Vec::new(),
        },
        HitTarget::MediaSeekForward => match kind {
            MouseKind::Left => reduce(state, Action::MediaSeek(15)),
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
        HitTarget::MediaSeekRail => match kind {
            // Press: record the preview position; no seek is emitted yet.
            MouseKind::Left => {
                let rect = state.hit_map.rect_for(HitTarget::MediaSeekRail);
                if let Mode::Media(media) = &mut state.mode {
                    media.slider_drag_active = true;
                    media.slider_drag_pos = rect.and_then(|r| rail_seconds(r, x, media.duration));
                }
                Vec::new()
            }
            // Drag: keep the preview glued to the pointer while a press is
            // live; without a known duration nothing is recorded.
            MouseKind::LeftDrag => {
                let rect = state.hit_map.rect_for(HitTarget::MediaSeekRail);
                if let Mode::Media(media) = &mut state.mode
                    && media.slider_drag_active
                    && let Some(seconds) = rect.and_then(|r| rail_seconds(r, x, media.duration))
                {
                    media.slider_drag_pos = Some(seconds);
                }
                Vec::new()
            }
            // Release: exactly one absolute seek when the gesture captured
            // a position; the interaction state always resets.
            MouseKind::LeftUp => commit_slider_drag(state),
            _ => Vec::new(),
        },

        HitTarget::MediaFullscreen => match kind {
            MouseKind::Left => reduce(state, Action::MediaToggleFullscreen),
            _ => Vec::new(),
        },
        HitTarget::MediaNext => match kind {
            MouseKind::Left => reduce(state, Action::MediaNext),
            _ => Vec::new(),
        },
        HitTarget::MediaPrev => match kind {
            MouseKind::Left => reduce(state, Action::MediaPrev),
            _ => Vec::new(),
        },
        HitTarget::MediaMute => match kind {
            MouseKind::Left => reduce(state, Action::MediaMute),
            MouseKind::ScrollUp => reduce(state, Action::MediaVolume(5)),
            MouseKind::ScrollDown => reduce(state, Action::MediaVolume(-5)),
            _ => Vec::new(),
        },
        HitTarget::MediaShuffle => match kind {
            MouseKind::Left => reduce(state, Action::MediaShuffle),
            _ => Vec::new(),
        },
        HitTarget::MediaRepeat => match kind {
            MouseKind::Left => reduce(state, Action::MediaRepeat),
            _ => Vec::new(),
        },
        HitTarget::MiniPlayer => match kind {
            MouseKind::Left => reduce(state, Action::MediaExpand),
            MouseKind::ScrollUp => reduce(state, Action::MediaVolume(5)),
            MouseKind::ScrollDown => reduce(state, Action::MediaVolume(-5)),
            _ => Vec::new(),
        },
        HitTarget::SubRow(idx) => match kind {
            MouseKind::Left => {
                if let Mode::Media(media) = &mut state.mode
                    && let Some(picker) = &mut media.sub_picker
                {
                    picker.selected = idx;
                }
                reduce(state, Action::SubPickerSubmit)
            }
            MouseKind::Moved => {
                if let Mode::Media(media) = &mut state.mode
                    && let Some(picker) = &mut media.sub_picker
                {
                    picker.selected = idx;
                }
                Vec::new()
            }
            MouseKind::ScrollUp => reduce(state, Action::SubPickerMove(-1)),
            MouseKind::ScrollDown => reduce(state, Action::SubPickerMove(1)),
            _ => Vec::new(),
        },
        HitTarget::QueueRow(idx) => match kind {
            MouseKind::Left => crate::app::media_ctl::play_index(state, idx),
            _ => Vec::new(),
        },
        HitTarget::ParentRow(idx) => match kind {
            MouseKind::Left => browser_only_fx(state, |s| {
                let Some(path) = s.parent_rows.get(idx).cloned() else {
                    return Vec::new();
                };
                if path == s.browser.cwd {
                    return Vec::new();
                }
                let is_dir = s
                    .browser
                    .cwd
                    .parent()
                    .and_then(|p| s.side_listings.get(p))
                    .and_then(|list| list.iter().find(|e| e.entry.path == path))
                    .is_some_and(|e| e.entry.is_dir_like());
                if is_dir {
                    navigate(s, path)
                } else if let Some(parent) = path.parent().map(Path::to_path_buf) {
                    let fx = navigate(s, parent);
                    s.pending_focus = Some(path);
                    fx
                } else {
                    Vec::new()
                }
            }),
            MouseKind::ScrollUp => reduce(state, Action::MoveUp),
            MouseKind::ScrollDown => reduce(state, Action::MoveDown),
            _ => Vec::new(),
        },
        HitTarget::SortBy(key) => match kind {
            MouseKind::Left => browser_only_fx(state, |s| reduce(s, Action::SortBy(key))),
            _ => Vec::new(),
        },
        HitTarget::ViewSwitch(view) => match kind {
            MouseKind::Left => browser_only_fx(state, |s| set_view(s, view)),
            _ => Vec::new(),
        },
        HitTarget::PathBar => match kind {
            MouseKind::Left => browser_only_fx(state, |s| reduce(s, Action::OpenAddressBar)),
            _ => Vec::new(),
        },
        HitTarget::HelpChip => match kind {
            MouseKind::Left => reduce(state, Action::ToggleHelp),
            _ => Vec::new(),
        },
        HitTarget::Details => Vec::new(),
        HitTarget::Blocker => match kind {
            MouseKind::Left => cancel(state),
            _ => Vec::new(),
        },
    }
}

/// Whether a hit target is an interactive control worth surfacing as
/// hovered (buttons, legend entries, breadcrumbs, ...). Rows, blank grid
/// space, and inert regions are excluded.
fn control_target(target: HitTarget) -> bool {
    !matches!(
        target,
        HitTarget::Row(_) | HitTarget::GridBackground | HitTarget::Blocker | HitTarget::Details
    )
}

/// Resolves pointer motion into hover state: the grid row under the cursor
/// in browser mode, any interactive control wherever it lives, and the
/// seek-rail preview timestamp while a media session is up. A miss clears
/// everything.
fn update_hover(state: &mut AppState, target: HitTarget, x: u16) {
    let rail_rect = state.hit_map.rect_for(HitTarget::MediaSeekRail);
    match target {
        HitTarget::MediaSeekRail => {
            if let Mode::Media(media) = &mut state.mode {
                media.slider_hover = rail_rect.and_then(|r| rail_seconds(r, x, media.duration));
            }
        }
        HitTarget::Row(pos) if matches!(state.mode, Mode::Browser) => {
            state.hover.row = Some(pos);
            state.hover.control = None;
        }
        t if control_target(t) => {
            state.hover.control = Some(t);
            state.hover.row = None;
        }
        _ => {
            state.hover = HoverState::default();
            // Leaving the rail clears its preview.
            if let Mode::Media(media) = &mut state.mode {
                media.slider_hover = None;
            }
        }
    }
}

/// Duration clamped to something usable for rail math: only finite,
/// strictly positive values count.
fn usable_duration(duration: Option<f64>) -> Option<f64> {
    duration.filter(|d| d.is_finite() && *d > 0.0)
}

/// True while a seek-rail drag gesture is in flight.
fn media_slider_drag_active(state: &AppState) -> bool {
    matches!(&state.mode, Mode::Media(media) if media.slider_drag_active)
}

/// Maps a pointer column onto seek seconds using the registered rail
/// rectangle. The track spans the rect's inner area (`x+1 .. x+width-1`),
/// matching the widget layer's `rail_geometry` so draw and mouse math agree.
fn rail_seconds(rect: Rect, x: u16, duration: Option<f64>) -> Option<f64> {
    let duration = usable_duration(duration)?;
    let inner_left = i32::from(rect.x + 1);
    let inner_width = i32::from(rect.width.saturating_sub(2)).max(1);
    let ratio = (i32::from(x) - inner_left) as f64 / inner_width as f64;
    Some((ratio.clamp(0.0, 1.0) * duration).clamp(0.0, duration))
}

/// Release after a seek-rail drag: exactly one absolute seek when the
/// gesture captured a position (a known duration); otherwise the gesture
/// ends silently. Either way the interaction state resets.
fn commit_slider_drag(state: &mut AppState) -> Vec<Effect> {
    let committed = match &mut state.mode {
        Mode::Media(media) => {
            let value = if media.slider_drag_active {
                media.slider_drag_pos
            } else {
                None
            };
            media.clear_slider_state();
            value
        }
        _ => None,
    };
    match committed {
        Some(seconds) => reduce(state, Action::MediaSeekAbsolute(seconds)),
        None => Vec::new(),
    }
}

/// Bottom-footer focus text, by priority: hovered row's basename, single
/// explicit selection's basename, selection count, or nothing.
pub fn footer_focus_text(state: &AppState) -> Option<String> {
    if let Some(row) = state.hover.row
        && let Some((_, view)) = state.browser.visible_entries().nth(row)
    {
        return Some(basename_of(&view.entry.path));
    }
    let selection = state.browser.selected_paths_set();
    match selection.len() {
        1 => selection.iter().next().map(|p| basename_of(p)),
        n if n > 1 => Some(format!("{n} items selected")),
        _ => None,
    }
}

fn basename_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Chebyshev distance threshold before an armed press becomes a drag.
const DRAG_THRESHOLD_CELLS: u16 = 2;

fn drag_sources(state: &AppState) -> Vec<PathBuf> {
    let selected = state.browser.selected_paths_set();
    if selected.is_empty() {
        state
            .browser
            .focused()
            .map(|view| vec![view.entry.path.clone()])
            .unwrap_or_default()
    } else {
        selected.iter().cloned().collect()
    }
}

/// Resolves the hovered hit target to a drop destination directory, or None
/// when the relationship is invalid (files, labels without a directory,
/// sources themselves, descendants of a moved source).
fn drag_drop_target_with_sources(
    state: &mut AppState,
    sources: &[PathBuf],
    x: u16,
    y: u16,
) -> Option<PathBuf> {
    if !matches!(state.mode, Mode::Browser) {
        return None;
    }
    match state.hit_map.hit(x, y)? {
        HitTarget::Row(pos) => {
            let (_, view) = state.browser.visible_entries().nth(pos)?;
            if !view.entry.kind.is_dir() {
                return None;
            }
            Some(view.entry.path.clone())
        }
        HitTarget::Breadcrumb(idx) => {
            let segments = breadcrumb_segments(&state.browser.cwd);
            segments.get(idx).map(|(path, _)| path.clone())
        }
        HitTarget::Sidebar(idx) => match state.sidebar_items.get(idx)? {
            SidebarItem::Place { path, .. }
            | SidebarItem::Mount { path, .. }
            | SidebarItem::Bookmark { path } => Some(path.clone()),
            _ => None,
        },
        _ => None,
    }
    .filter(|dest| {
        !sources
            .iter()
            .any(|source| dest.starts_with(source) || dest == source)
    })
}

fn drag_drop_target(state: &mut AppState, x: u16, y: u16) -> Option<PathBuf> {
    let sources = state.drag.as_ref()?.sources.clone();
    drag_drop_target_with_sources(state, &sources, x, y)
}

fn handle_drag_motion(
    state: &mut AppState,
    mut drag: DragState,
    kind: MouseKind,
    x: u16,
    y: u16,
) -> Vec<Effect> {
    match kind {
        MouseKind::LeftUp | MouseKind::Right => {
            let was_dragging = drag.phase == DragPhase::Dragging;
            state.drag = None;
            if !was_dragging || !matches!(state.mode, Mode::Browser) {
                return Vec::new();
            }
            let Some(dest) = drag_drop_target_with_sources(state, &drag.sources, x, y) else {
                return Vec::new();
            };
            start_drag_move(state, drag.sources, dest)
        }
        MouseKind::LeftDrag => {
            let dx = x.abs_diff(drag.origin.0);
            let dy = y.abs_diff(drag.origin.1);
            // One-row-tall list rows: any vertical move onto another row is
            // a deliberate drag; tall grid tiles keep the jitter allowance.
            let dy_limit = if state.view() == ViewMode::Grid {
                DRAG_THRESHOLD_CELLS
            } else {
                0
            };
            if drag.phase == DragPhase::Armed && dx <= DRAG_THRESHOLD_CELLS && dy <= dy_limit {
                return Vec::new();
            }
            drag.phase = DragPhase::Dragging;
            drag.cursor = (x, y);
            state.drag = Some(drag);
            Vec::new()
        }
        // Any other mouse event during a drag is ignored.
        _ => Vec::new(),
    }
}

/// Clears any in-flight pointer gestures (file drag or background marquee)
/// on mode changes, resize, refresh, navigation, or Esc.
pub fn cancel_drag(state: &mut AppState) {
    state.drag = None;
    state.marquee = None;
}

/// Replaces the selection with every visible tile intersecting the marquee
/// band, unioned with the additive base captured at press time.
fn apply_marquee(state: &mut AppState, marquee: &MarqueeState) {
    if !matches!(state.mode, Mode::Browser) {
        return;
    }
    let rect = marquee.rect();
    let indices = state.browser.visible_indices();
    let mut selected = marquee.base.clone();
    for pos in state.hit_map.rows_intersecting(rect) {
        if let Some(&entry_index) = indices.get(pos) {
            selected.insert(state.browser.entries[entry_index].entry.path.clone());
        }
    }
    state.browser.set_selection(selected);
}

fn start_drag_move(state: &mut AppState, mut sources: Vec<PathBuf>, dest: PathBuf) -> Vec<Effect> {
    sources.sort();
    sources.dedup();
    let plan = OperationPlan {
        kind: OperationKind::Move,
        sources,
        dest_dir: Some(dest),
        rename_to: None,
        policy: ConflictPolicy::Ask,
    };
    if let Err(err) = validate(&plan) {
        state.set_error(err.to_string());
        return Vec::new();
    }
    start_operation(state, plan)
}

/// UI-only validity probe used by the drag renderer: same rules as
/// `drag_drop_target`, exposed without touching the reducer's private state.
pub fn drag_drop_target_for_ui(state: &mut AppState, x: u16, y: u16) -> Option<PathBuf> {
    drag_drop_target(state, x, y)
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
        LegendAction::Search => reduce(state, Action::EnterFilter),
        LegendAction::Paste => reduce(state, Action::ClipboardPaste),
        LegendAction::View => reduce(state, Action::CycleView),
        LegendAction::Player => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::EntryKind;
    use crate::testing::builders::{FIXED_TIME, demo_state, entry};
    use std::collections::BTreeSet;

    fn state_with_entries() -> AppState {
        let root = crate::testing::builders::demo_root();
        let mut state = demo_state(100, 30);
        state.browser.set_entries(vec![
            crate::browser::EntryView {
                entry: entry(&root, "alpha", EntryKind::File, 1, 0o644, FIXED_TIME),
                tags: Vec::new(),
            },
            crate::browser::EntryView {
                entry: entry(&root, "beta", EntryKind::File, 2, 0o644, FIXED_TIME),
                tags: Vec::new(),
            },
            crate::browser::EntryView {
                entry: entry(&root, "gamma", EntryKind::File, 3, 0o644, FIXED_TIME),
                tags: Vec::new(),
            },
        ]);
        state
    }

    fn paths(state: &AppState, names: &[&str]) -> BTreeSet<PathBuf> {
        names.iter().map(|n| state.browser.cwd.join(n)).collect()
    }

    #[test]
    fn footer_hover_row_wins_over_selection() {
        let mut state = state_with_entries();
        state.browser.selection = paths(&state, &["alpha", "gamma"]);
        // Display order is alpha, beta, gamma: row 1 is beta.
        state.hover.row = Some(1);
        assert_eq!(footer_focus_text(&state).as_deref(), Some("beta"));
    }

    #[test]
    fn footer_single_selection_basename() {
        let mut state = state_with_entries();
        state.browser.selection = paths(&state, &["beta"]);
        assert_eq!(footer_focus_text(&state).as_deref(), Some("beta"));
    }

    #[test]
    fn footer_multi_selection_reports_count() {
        let mut state = state_with_entries();
        state.browser.selection = paths(&state, &["alpha", "beta", "gamma"]);
        assert_eq!(
            footer_focus_text(&state).as_deref(),
            Some("3 items selected")
        );
    }

    #[test]
    fn footer_no_hover_or_selection_is_none() {
        let state = state_with_entries();
        assert_eq!(footer_focus_text(&state), None);
    }

    #[test]
    fn footer_stale_hover_row_falls_back_to_selection() {
        let mut state = state_with_entries();
        state.browser.selection = paths(&state, &["gamma"]);
        state.hover.row = Some(99);
        assert_eq!(footer_focus_text(&state).as_deref(), Some("gamma"));
    }

    #[test]
    fn rail_seconds_maps_inner_track_and_clamps() {
        let rect = Rect::new(10, 5, 22, 1); // inner track 11..=30, width 20
        assert_eq!(rail_seconds(rect, 11, Some(60.0)), Some(0.0));
        assert_eq!(rail_seconds(rect, 31, Some(60.0)), Some(60.0));
        assert_eq!(rail_seconds(rect, 0, Some(60.0)), Some(0.0));
        assert_eq!(rail_seconds(rect, 200, Some(60.0)), Some(60.0));
        assert!((rail_seconds(rect, 21, Some(60.0)).unwrap() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn rail_seconds_needs_usable_duration() {
        let rect = Rect::new(0, 0, 20, 1);
        assert_eq!(rail_seconds(rect, 5, None), None);
        assert_eq!(rail_seconds(rect, 5, Some(0.0)), None);
        assert_eq!(rail_seconds(rect, 5, Some(-3.0)), None);
        assert_eq!(rail_seconds(rect, 5, Some(f64::NAN)), None);
    }
    // --- Seek-rail slider machine (hand-built media state; no backend) ---

    fn playing_state(duration: Option<f64>) -> AppState {
        let root = crate::testing::builders::demo_root();
        let mut state = demo_state(100, 30);
        let mut media =
            MediaState::preparing(1, root.join("song.mp3"), crate::media::MediaKind::Audio);
        media.phase = MediaPhase::Playing;
        media.duration = duration;
        state.mode = Mode::Media(Box::new(media));
        state.hit_map.push(
            Rect::new(10, 5, 22, 3), // inner track x 11..=30, width 20
            HitTarget::MediaSeekRail,
        );
        state
    }

    fn click(kind: MouseKind, x: u16) -> Action {
        Action::Mouse {
            kind,
            x,
            y: 6,
            ctrl: false,
        }
    }

    fn committed_seek(effects: &[Effect]) -> Option<f64> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::MediaCommand {
                    command: MediaCommand::SeekAbsolute(v),
                    ..
                } => Some(*v),
                _ => None,
            })
            .next()
    }

    #[test]
    fn slider_press_records_preview_and_never_seeks() {
        let mut state = playing_state(Some(60.0));
        let effects = reduce(&mut state, click(MouseKind::Left, 21));
        assert!(effects.is_empty());
        let Mode::Media(media) = &state.mode else {
            panic!("media mode");
        };
        assert!(media.slider_drag_active);
        assert!((media.slider_drag_pos.unwrap() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn slider_drag_then_release_commits_exactly_once() {
        let mut state = playing_state(Some(60.0));
        reduce(&mut state, click(MouseKind::Left, 11));
        reduce(&mut state, click(MouseKind::LeftDrag, 31));
        let effects = reduce(&mut state, click(MouseKind::LeftUp, 31));
        let value = committed_seek(&effects).expect("one seek committed");
        assert!((value - 60.0).abs() < 1e-9);
        assert_eq!(effects.len(), 1);
        let Mode::Media(media) = &state.mode else {
            panic!("media mode");
        };
        assert!(!media.slider_drag_active);
        assert_eq!(media.slider_drag_pos, None);
        assert_eq!(media.slider_hover, None);
    }

    #[test]
    fn slider_release_off_track_still_commits_clamped() {
        let mut state = playing_state(Some(60.0));
        reduce(&mut state, click(MouseKind::Left, 11));
        // Pointer slips far past the right edge: clamps to the duration.
        reduce(&mut state, click(MouseKind::LeftDrag, 90));
        let effects = reduce(&mut state, click(MouseKind::LeftUp, 90));
        let value = committed_seek(&effects).expect("off-track release commits");
        assert!((value - 60.0).abs() < 1e-9);
    }

    #[test]
    fn fresh_press_supersedes_active_gesture_without_commit() {
        let mut state = playing_state(Some(60.0));
        reduce(&mut state, click(MouseKind::Left, 21));
        // A brand-new press OFF the rail drops the gesture silently (a
        // fresh on-rail press legitimately starts a new gesture).
        let effects = reduce(&mut state, click(MouseKind::Left, 2));
        assert!(committed_seek(&effects).is_none());
        let Mode::Media(media) = &state.mode else {
            panic!("media mode");
        };
        assert!(!media.slider_drag_active);
        // The subsequent release commits nothing.
        let effects = reduce(&mut state, click(MouseKind::LeftUp, 12));
        assert!(committed_seek(&effects).is_none());
    }

    #[test]
    fn unknown_duration_gesture_is_completely_inert() {
        let mut state = playing_state(None);
        assert!(reduce(&mut state, click(MouseKind::Left, 21)).is_empty());
        {
            let Mode::Media(media) = &state.mode else {
                panic!("media mode");
            };
            assert!(media.slider_drag_active);
            assert_eq!(media.slider_drag_pos, None);
        }
        assert!(reduce(&mut state, click(MouseKind::LeftDrag, 25)).is_empty());
        let effects = reduce(&mut state, click(MouseKind::LeftUp, 25));
        assert!(committed_seek(&effects).is_none());
        let Mode::Media(media) = &state.mode else {
            panic!("media mode");
        };
        assert!(!media.slider_drag_active);
    }
}
