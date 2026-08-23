use std::path::{Path, PathBuf};

use tui_explorer::app::action::{Action, ConflictDecision};
use tui_explorer::app::state::{AppState, Mode};
use tui_explorer::filesystem::RecordedMutation;
use tui_explorer::testing::builders::{demo_fs, demo_state};
use tui_explorer::testing::{SyncHandler, drive};

fn open_with_actions(text: &str) -> Vec<Action> {
    let mut actions = vec![Action::OpenWithPrompt];
    for c in text.chars() {
        actions.push(Action::OpenWithChar(c));
    }
    actions.push(Action::OpenWithSubmit);
    actions
}

fn command_actions(input: &str) -> Vec<Action> {
    let mut actions = vec![Action::EnterCommand];
    for c in input.chars() {
        actions.push(Action::CommandChar(c));
    }
    actions.push(Action::CommandSubmit);
    actions
}

fn loaded(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

#[test]
fn initial_load_sorts_dirs_first() {
    let (state, _) = loaded(120, 36);
    let names: Vec<String> = state
        .browser
        .visible_entries()
        .map(|(_, e)| e.entry.display_name())
        .collect();
    assert_eq!(
        &names[..3],
        [
            "docs",
            "src",
            "a very long file name that keeps going and going.txt"
        ]
    );
    assert!(names.len() > 10);
}

#[test]
fn enter_directory_and_back() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::OpenFocused],
    );
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo/docs"));
    drive(&mut state, &mut handler, [Action::OpenParent]);
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo"));
}

#[test]
fn vim_navigation_sequence() {
    let (mut state, mut handler) = loaded(120, 36);
    let len = state.browser.visible_len();
    drive(&mut state, &mut handler, [Action::KeyG, Action::KeyG]);
    assert_eq!(state.browser.selected, 0);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    assert_eq!(state.browser.selected, len - 1);
    drive(&mut state, &mut handler, [Action::HalfPageUp]);
    assert!(state.browser.selected < len - 1);
    drive(
        &mut state,
        &mut handler,
        [Action::KeyG, Action::KeyG, Action::MoveDown],
    );
    assert_eq!(state.browser.selected, 1);
}

#[test]
fn cd_command_and_bad_cd_restores() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, command_actions("cd src"));
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo/src"));
    drive(
        &mut state,
        &mut handler,
        command_actions("cd /nonexistent-dir"),
    );
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo/src"));
    let msg = state.message.as_ref().expect("error message");
    assert!(msg.is_error);
}

#[test]
fn delete_flow_requires_double_confirm_for_dirs() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    drive(&mut state, &mut handler, command_actions("delete"));
    let Mode::Confirm(confirm) = &state.mode else {
        panic!("expected confirm modal, got {:?}", state.mode.name())
    };
    assert!(confirm.recursive);
    drive(&mut state, &mut handler, [Action::Confirm]);
    let Mode::Confirm(confirm) = &state.mode else {
        panic!("expected second stage confirm")
    };
    assert_eq!(confirm.stage, 2);
    assert!(handler.mutations.recorded().is_empty());
    drive(&mut state, &mut handler, [Action::Confirm]);
    let recorded = handler.mutations.recorded();
    assert_eq!(
        recorded,
        vec![RecordedMutation::Delete {
            path: PathBuf::from("/home/demo/docs"),
            recursive: true,
        }]
    );
}

#[test]
fn delete_cancel_records_nothing() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    drive(&mut state, &mut handler, command_actions("delete"));
    drive(&mut state, &mut handler, [Action::Reject]);
    assert!(handler.mutations.recorded().is_empty());
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn multi_select_delete_reports_targets() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::ToggleSelect, Action::ToggleSelect],
    );
    assert_eq!(state.browser.selection.len(), 2);
    drive(&mut state, &mut handler, command_actions("delete"));
    drive(&mut state, &mut handler, [Action::Confirm, Action::Confirm]);
    assert_eq!(handler.mutations.recorded().len(), 2);
}

#[test]
fn copy_conflict_skip_and_replace() {
    let (mut state, mut handler) = loaded(120, 36);
    let downs = vec![Action::MoveDown; 11];
    drive(&mut state, &mut handler, downs);
    assert_eq!(
        state.browser.focused().unwrap().entry.path,
        PathBuf::from("/home/demo/main.rs")
    );
    drive(&mut state, &mut handler, command_actions("copy src"));
    let Mode::Conflict(_) = &state.mode else {
        panic!("expected conflict modal, got {:?}", state.mode.name())
    };
    drive(
        &mut state,
        &mut handler,
        [Action::ConflictChoice(ConflictDecision::Skip)],
    );
    assert!(handler.mutations.recorded().is_empty());
    drive(&mut state, &mut handler, command_actions("copy src"));
    drive(
        &mut state,
        &mut handler,
        [Action::ConflictChoice(ConflictDecision::Replace)],
    );
    let recorded = handler.mutations.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(matches!(
        &recorded[0],
        RecordedMutation::Copy { replace: true, .. }
    ));
}

#[test]
fn copy_same_path_rejected_with_message() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    drive(&mut state, &mut handler, command_actions("copy ."));
    let msg = state.message.as_ref().expect("validation message");
    assert!(msg.is_error);
    assert!(msg.text.contains("same"));
    assert!(handler.mutations.recorded().is_empty());
}

#[test]
fn move_dir_into_itself_rejected() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    drive(&mut state, &mut handler, command_actions("move docs/sub"));
    let msg = state.message.as_ref().expect("validation message");
    assert!(msg.is_error);
    assert!(handler.mutations.recorded().is_empty());
}

#[test]
fn rename_follows_tags() {
    let (mut state, mut handler) = loaded(120, 36);
    let target = PathBuf::from("/home/demo/main.rs");
    let renamed = PathBuf::from("/home/demo/renamed.rs");
    let downs = vec![Action::MoveDown; 11];
    drive(&mut state, &mut handler, downs);
    drive(&mut state, &mut handler, command_actions("tag fav"));
    assert_eq!(handler.tags.tags_for_path(&target).unwrap(), vec!["fav"]);
    drive(
        &mut state,
        &mut handler,
        command_actions("rename renamed.rs"),
    );
    assert_eq!(handler.tags.tags_for_path(&renamed).unwrap(), vec!["fav"]);
    assert!(handler.tags.tags_for_path(&target).unwrap().is_empty());
    assert!(matches!(
        handler.mutations.recorded().as_slice(),
        [RecordedMutation::Move { .. }]
    ));
}

#[test]
fn tag_and_untag_via_command() {
    let (mut state, mut handler) = loaded(120, 36);
    let target = PathBuf::from("/home/demo/Cargo.lock");
    drive(&mut state, &mut handler, vec![Action::MoveDown; 6]);
    drive(&mut state, &mut handler, command_actions("tag lock"));
    assert_eq!(handler.tags.tags_for_path(&target).unwrap(), vec!["lock"]);
    let view = state
        .browser
        .entries
        .iter()
        .find(|e| e.entry.path == target)
        .expect("entry");
    assert_eq!(view.tags, vec!["lock".to_string()]);
    drive(&mut state, &mut handler, command_actions("untag lock"));
    assert!(handler.tags.tags_for_path(&target).unwrap().is_empty());
}

#[test]
fn quick_tag_toggles_last_used() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::QuickTag]);
    let msg = state.message.as_ref().expect("no tags message");
    assert!(msg.text.contains("no tags"));
    drive(&mut state, &mut handler, command_actions("tag fav"));
    drive(
        &mut state,
        &mut handler,
        [Action::MoveDown, Action::QuickTag],
    );
    let target = PathBuf::from("/home/demo/src");
    assert_eq!(handler.tags.tags_for_path(&target).unwrap(), vec!["fav"]);
    drive(&mut state, &mut handler, [Action::QuickTag]);
    assert!(handler.tags.tags_for_path(&target).unwrap().is_empty());
}

#[test]
fn picker_create_assign_delete() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::OpenTagPicker]);
    let Mode::TagPicker(_) = &state.mode else {
        panic!("expected picker")
    };
    drive(&mut state, &mut handler, [Action::PickerNew]);
    for c in "work".chars() {
        drive(&mut state, &mut handler, [Action::PickerChar(c)]);
    }
    drive(&mut state, &mut handler, [Action::PickerSubmitNew]);
    let target = PathBuf::from("/home/demo/docs");
    assert_eq!(handler.tags.tags_for_path(&target).unwrap(), vec!["work"]);
    drive(&mut state, &mut handler, [Action::PickerDelete]);
    assert!(handler.tags.list_tags().unwrap().is_empty());
    drive(&mut state, &mut handler, [Action::Cancel]);
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn picker_rejects_invalid_names() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::OpenTagPicker, Action::PickerNew],
    );
    drive(&mut state, &mut handler, [Action::PickerChar(' ')]);
    drive(&mut state, &mut handler, [Action::PickerSubmitNew]);
    let msg = state.message.as_ref().expect("validation message");
    assert!(msg.is_error);
    assert!(handler.tags.list_tags().unwrap().is_empty());
}

#[test]
fn current_directory_filter_reduces_and_restores_entries() {
    let (mut state, mut handler) = loaded(120, 36);
    let before = state.browser.visible_len();
    drive(
        &mut state,
        &mut handler,
        command_actions("filter rs")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    assert!(state.browser.visible_len() < before);
    assert!(
        state
            .message
            .as_ref()
            .is_some_and(|message| message.text.contains("filter applied"))
    );
    assert!(
        state
            .browser
            .visible_entries()
            .all(|(_, entry)| { entry.entry.display_name().to_lowercase().contains("rs") })
    );
    drive(&mut state, &mut handler, [Action::Cancel]);
    assert!(state.browser.filter.is_none());
    drive(
        &mut state,
        &mut handler,
        command_actions("filter rs")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    drive(
        &mut state,
        &mut handler,
        command_actions("clearfilter")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    assert_eq!(state.browser.visible_len(), before);

    drive(
        &mut state,
        &mut handler,
        command_actions("filter rs")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    drive(
        &mut state,
        &mut handler,
        command_actions("cd src")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    assert!(state.browser.filter.is_none());
}

#[test]
fn refresh_reloads_the_current_directory_without_resetting_view_options() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("filter rs")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    drive(&mut state, &mut handler, [Action::Refresh]);
    assert!(
        state
            .message
            .as_ref()
            .is_some_and(|message| message.text == "refreshing directory")
    );
    assert_eq!(state.browser.filter.as_deref(), Some("rs"));
    assert_eq!(state.browser.visible_len(), 1);
}

#[test]
fn invalid_sort_reports_an_error_without_changing_the_listing() {
    let (mut state, mut handler) = loaded(120, 36);
    let before = state.browser.visible_len();
    drive(
        &mut state,
        &mut handler,
        command_actions("sort nonsense")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    assert_eq!(state.browser.visible_len(), before);
    assert!(
        state
            .message
            .as_ref()
            .is_some_and(|message| message.is_error)
    );
}

#[test]
fn command_parse_error_stays_in_tui() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, command_actions("bogus stuff"));
    let msg = state.message.as_ref().expect("parse error message");
    assert!(msg.is_error);
    assert!(msg.text.contains("unknown command"));
    assert!(!state.should_quit);
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn quit_only_from_browser_mode() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::EnterCommand, Action::Quit],
    );
    assert!(!handler.quit);
    drive(&mut state, &mut handler, [Action::Cancel, Action::Quit]);
    assert!(handler.quit);
}

#[test]
fn mkdir_creates_directory_and_refreshes_listing() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, command_actions("mkdir fresh-dir"));
    assert!(matches!(
        handler.mutations.recorded().as_slice(),
        [RecordedMutation::CreateDir { path }] if path == &PathBuf::from("/home/demo/fresh-dir")
    ));
    assert!(
        state
            .browser
            .entries
            .iter()
            .any(|e| e.entry.path == Path::new("/home/demo/fresh-dir"))
    );
}

#[test]
fn touch_creates_file_and_refreshes_listing() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, command_actions("touch fresh.txt"));
    assert!(matches!(
        handler.mutations.recorded().as_slice(),
        [RecordedMutation::CreateFile { path }] if path == &PathBuf::from("/home/demo/fresh.txt")
    ));
    assert!(
        state
            .browser
            .entries
            .iter()
            .any(|e| e.entry.path == Path::new("/home/demo/fresh.txt"))
    );
}

#[test]
fn mkdir_rejects_invalid_names() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, command_actions("mkdir a/b"));
    let msg = state.message.as_ref().expect("error message");
    assert!(msg.is_error);
    assert!(handler.mutations.recorded().is_empty());
}

#[test]
fn selection_utilities_via_command() {
    let (mut state, mut handler) = loaded(120, 36);
    let total = state.browser.entries.len();
    drive(&mut state, &mut handler, command_actions("selectall"));
    assert_eq!(state.browser.selection.len(), total);
    drive(&mut state, &mut handler, command_actions("invert"));
    assert!(state.browser.selection.is_empty());
    drive(&mut state, &mut handler, command_actions("selectall"));
    drive(&mut state, &mut handler, command_actions("deselect"));
    assert!(state.browser.selection.is_empty());
}

#[test]
fn open_with_prompt_runs_explicit_program() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    let target = state.browser.focused().unwrap().entry.path.clone();
    drive(&mut state, &mut handler, open_with_actions("mupdf -r 150"));
    assert_eq!(
        handler.opened_with,
        vec![(
            target,
            "mupdf".to_string(),
            vec!["-r".to_string(), "150".to_string()]
        )]
    );
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn open_with_command_runs_without_prompt() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    let target = state.browser.focused().unwrap().entry.path.clone();
    drive(&mut state, &mut handler, command_actions("open-with mupdf"));
    assert_eq!(
        handler.opened_with,
        vec![(target, "mupdf".to_string(), vec![])]
    );
}

#[test]
fn open_with_empty_input_reports_error() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    drive(&mut state, &mut handler, open_with_actions(""));
    let msg = state.message.as_ref().expect("error message");
    assert!(msg.is_error);
    assert!(handler.opened_with.is_empty());
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn open_file_prompts_for_command() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoLast, Action::OpenFocused],
    );
    let Mode::OpenWith(dialog) = &state.mode else {
        panic!("expected open-with modal, got {:?}", state.mode.name())
    };
    let target = state.browser.focused().unwrap().entry.path.clone();
    assert_eq!(dialog.target, target, "modal targets the focused file");
    assert!(handler.opened_with.is_empty());
    assert!(handler.mutations.recorded().is_empty());
}

#[test]
fn open_command_prompts_for_command() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    drive(&mut state, &mut handler, command_actions("open"));
    let Mode::OpenWith(dialog) = &state.mode else {
        panic!("expected open-with modal, got {:?}", state.mode.name())
    };
    let target = state.browser.focused().unwrap().entry.path.clone();
    assert_eq!(dialog.target, target);
    assert!(handler.opened_with.is_empty());
}

#[test]
fn open_with_command_skips_prompt() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoLast]);
    let target = state.browser.focused().unwrap().entry.path.clone();
    drive(&mut state, &mut handler, command_actions("open-with cat"));
    assert_eq!(
        handler.opened_with,
        vec![(target, "cat".to_string(), vec![])]
    );
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn visual_mode_and_cancel_clears() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::ToggleVisual, Action::MoveDown, Action::ToggleSelect],
    );
    assert!(state.browser.visual);
    assert!(!state.browser.selection.is_empty());
    drive(&mut state, &mut handler, [Action::Cancel]);
    assert!(state.browser.selection.is_empty());
    assert!(!state.browser.visual);
}

#[test]
fn memory_fs_untouched_by_operations() {
    let (mut state, mut handler) = loaded(120, 36);
    let before = handler.fs.known_paths();
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    drive(&mut state, &mut handler, command_actions("delete"));
    drive(&mut state, &mut handler, [Action::Confirm, Action::Confirm]);
    drive(
        &mut state,
        &mut handler,
        command_actions("copy /home/demo/src"),
    );
    assert_eq!(handler.fs.known_paths(), before);
}

// ---- Unified open interaction (Phase 2) ----

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tui_explorer::app::action::MouseKind;
use tui_explorer::ui;

fn rendered(width: u16, height: u16) -> (AppState, SyncHandler) {
    let (mut state, handler) = loaded(width, height);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .expect("render");
    (state, handler)
}

fn row_rect(state: &AppState, pos: usize) -> ratatui::layout::Rect {
    state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            tui_explorer::ui::hit::HitTarget::Row(p) if *p == pos => Some(*rect),
            _ => None,
        })
        .expect("row hit region")
}

fn click(state: &mut AppState, handler: &mut SyncHandler, pos: usize) {
    let rect = row_rect(state, pos);
    drive(
        state,
        handler,
        [Action::Mouse {
            kind: MouseKind::Left,
            x: rect.x + 1,
            y: rect.y + 1,
        }],
    );
}

#[test]
fn single_click_selects_but_never_opens() {
    let (mut state, mut handler) = rendered(120, 36);
    let cwd = state.browser.cwd.clone();
    click(&mut state, &mut handler, 1);
    assert_eq!(state.browser.selected, 1);
    assert_eq!(state.browser.cwd, cwd, "single click must not navigate");
    assert!(
        handler.opened_with.is_empty(),
        "single click must not open files"
    );
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn double_click_same_entry_opens_exactly_once() {
    let (mut state, mut handler) = rendered(120, 36);
    // First entry is the "docs" directory.
    click(&mut state, &mut handler, 0);
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo"));
    click(&mut state, &mut handler, 0);
    assert_eq!(
        state.browser.cwd,
        PathBuf::from("/home/demo/docs"),
        "double click enters the folder"
    );
    assert!(
        handler.opened_with.is_empty(),
        "folder opens navigate, not spawn"
    );
}

#[test]
fn double_click_different_entries_does_not_open() {
    let (mut state, mut handler) = rendered(120, 36);
    let cwd = state.browser.cwd.clone();
    click(&mut state, &mut handler, 0);
    click(&mut state, &mut handler, 1);
    assert_eq!(state.browser.selected, 1);
    assert_eq!(state.browser.cwd, cwd, "clicks on different entries select");
    assert!(handler.opened_with.is_empty());
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn e_and_enter_both_open_focused() {
    // `e`
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    let key_e = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('e'),
        crossterm::event::KeyModifiers::NONE,
    );
    let action = tui_explorer::input::keymap::map_key(key_e, &state);
    assert!(matches!(action, Some(Action::OpenFocused)));
    drive(&mut state, &mut handler, [action.unwrap()]);
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo/docs"));
    // Enter
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    let key_enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );
    let action = tui_explorer::input::keymap::map_key(key_enter, &state);
    assert!(matches!(action, Some(Action::OpenFocused)));
    drive(&mut state, &mut handler, [action.unwrap()]);
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo/docs"));
}

#[test]
fn l_and_arrow_do_not_open() {
    let (mut state, _) = loaded(120, 36);
    for code in [
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyCode::Right,
    ] {
        let key = crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
        let action = tui_explorer::input::keymap::map_key(key, &state);
        assert!(
            !matches!(action, Some(Action::OpenFocused)),
            "{code:?} must not open"
        );
    }
    state.browser.selected = 0;
}

#[test]
fn open_on_empty_directory_is_safe() {
    let mut fs = tui_explorer::testing::MemoryFileSystem::new();
    fs.add_dir(std::path::Path::new("/home/demo"));
    let mut state = demo_state(120, 36);
    let mut handler = SyncHandler::new(fs);
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    assert!(handler.opened_with.is_empty());
    assert!(matches!(state.mode, Mode::Browser));
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo"));
}

fn first_file_pos(state: &AppState) -> usize {
    state
        .browser
        .visible_indices()
        .iter()
        .position(|&i| !state.browser.entries[i].entry.kind.is_dir())
        .expect("demo fs has files")
}

#[test]
fn double_click_file_prompts_for_command() {
    let (mut state, mut handler) = rendered(120, 36);
    let pos = first_file_pos(&state);
    click(&mut state, &mut handler, pos);
    click(&mut state, &mut handler, pos);
    let Mode::OpenWith(dialog) = &state.mode else {
        panic!("expected open-with modal, got {:?}", state.mode.name())
    };
    let target = state.browser.focused().unwrap().entry.path.clone();
    assert_eq!(dialog.target, target);
    assert!(handler.opened_with.is_empty());
}

#[test]
fn context_menu_open_prompts_for_command() {
    let (mut state, mut handler) = rendered(120, 36);
    let pos = first_file_pos(&state);
    let rect = row_rect(&state, pos);
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: MouseKind::Right,
            x: rect.x + 1,
            y: rect.y + 1,
        }],
    );
    assert!(matches!(state.mode, Mode::ContextMenu(_)));
    let target = state.browser.focused().unwrap().entry.path.clone();
    // ContextItem::all() starts with Open, and the menu opens on it.
    drive(&mut state, &mut handler, [Action::ContextChoose]);
    let Mode::OpenWith(dialog) = &state.mode else {
        panic!("expected open-with modal, got {:?}", state.mode.name())
    };
    assert_eq!(dialog.target, target);
    assert!(handler.opened_with.is_empty());
}

#[test]
fn bookmark_navigator_filters_submits_and_cancels() {
    let (mut state, mut handler) = loaded(120, 36);
    state.bookmarks = vec![
        PathBuf::from("/home/demo/docs"),
        PathBuf::from("/home/demo/src"),
    ];
    drive(&mut state, &mut handler, [Action::OpenBookmarks]);
    let Mode::Bookmarks(nav) = &state.mode else {
        panic!("expected bookmarks modal")
    };
    assert_eq!(nav.matches.len(), 2);
    assert_eq!(nav.selected, 0);
    // Moving then typing a query that shrinks the list keeps selection valid.
    drive(&mut state, &mut handler, [Action::BookmarkMove(1)]);
    for c in "src".chars() {
        drive(&mut state, &mut handler, [Action::BookmarkChar(c)]);
    }
    let Mode::Bookmarks(nav) = &state.mode else {
        panic!("still in bookmarks modal")
    };
    assert_eq!(nav.matches, vec![PathBuf::from("/home/demo/src")]);
    assert_eq!(nav.selected, 0, "selection clamps to the shrunk list");
    drive(&mut state, &mut handler, [Action::BookmarkSubmit]);
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo/src"));
    assert!(matches!(state.mode, Mode::Browser));
    // Reopen; Esc closes without navigating.
    let cwd = state.browser.cwd.clone();
    drive(&mut state, &mut handler, [Action::OpenBookmarks]);
    drive(&mut state, &mut handler, [Action::Cancel]);
    assert!(matches!(state.mode, Mode::Browser));
    assert_eq!(state.browser.cwd, cwd);
}

#[test]
fn bookmark_navigator_opens_empty() {
    let (mut state, mut handler) = loaded(120, 36);
    state.bookmarks.clear();
    drive(&mut state, &mut handler, [Action::OpenBookmarks]);
    let Mode::Bookmarks(nav) = &state.mode else {
        panic!("expected bookmarks modal")
    };
    assert!(nav.matches.is_empty());
    drive(&mut state, &mut handler, [Action::Cancel]);
    assert!(matches!(state.mode, Mode::Browser));
}

#[test]
fn error_epoch_bumps_only_on_errors() {
    let (mut state, mut handler) = loaded(120, 36);
    let before = state.error_epoch;
    drive(
        &mut state,
        &mut handler,
        [Action::TagsApplied {
            message: "tagged 1 entry with [x]".to_string(),
            last_tag: Some("x".to_string()),
        }],
    );
    assert_eq!(
        state.error_epoch, before,
        "info messages do not bump the epoch"
    );
    drive(
        &mut state,
        &mut handler,
        [Action::ErrorMessage("boom".to_string())],
    );
    assert_eq!(state.error_epoch, before + 1);
    assert!(state.message.as_ref().is_some_and(|m| m.is_error));
    drive(
        &mut state,
        &mut handler,
        [Action::DirectoryLoaded {
            result: Err("cannot read /nope: no such directory".to_string()),
        }],
    );
    assert_eq!(state.error_epoch, before + 2);
    assert!(state.message.as_ref().is_some_and(|m| m.is_error));
}

// ---- Encryption flow through the UI (real temp fixture) ----

fn crypto_fixture() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "tui-explorer-replay-crypto-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("secret.txt");
    std::fs::write(&file, b"top secret bytes\n").unwrap();
    (dir, file)
}

fn crypto_state(dir: &std::path::Path, names: &[&str]) -> (AppState, SyncHandler) {
    use tui_explorer::filesystem::{DirEntry, EntryKind};
    use tui_explorer::testing::MemoryFileSystem;
    let mut fs = MemoryFileSystem::new();
    fs.add_dir(dir);
    for name in names {
        let path = dir.join(name);
        fs.add_entry(
            dir,
            DirEntry {
                name: std::ffi::OsString::from(name),
                path: path.clone(),
                kind: if path.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                mode: 0o644,
                modified: 1_700_000_000,
                executable: false,
                hidden: name.starts_with('.'),
                device: None,
                inode: None,
            },
        );
    }
    let mut state = AppState::new(dir.to_path_buf(), dir.to_path_buf());
    state.width = 120;
    state.height = 36;
    let mut handler = SyncHandler::new(fs);
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

fn type_password(state: &mut AppState, handler: &mut SyncHandler, pass: &str) {
    let actions: Vec<Action> = pass
        .chars()
        .map(Action::PasswordChar)
        .chain([Action::PasswordSubmit])
        .collect();
    drive(state, handler, actions);
}

#[test]
fn encrypt_decrypt_roundtrip_through_ui() {
    let (dir, file) = crypto_fixture();
    let original = std::fs::read(&file).unwrap();
    let (mut state, mut handler) = crypto_state(&dir, &["secret.txt"]);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    assert!(
        matches!(state.mode, Mode::Password(_)),
        "X opens the password dialog"
    );
    type_password(&mut state, &mut handler, "pw");
    assert!(
        matches!(state.mode, Mode::Password(_)),
        "encryption asks for confirmation"
    );
    type_password(&mut state, &mut handler, "pw");
    assert!(matches!(state.mode, Mode::Browser));
    let enc = dir.join("secret.txt.age");
    assert!(enc.exists(), "encrypted output written");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        original,
        "source never deleted or modified"
    );
    // Wrong password fails recoverably without touching data.
    std::fs::remove_file(&file).unwrap();
    let (mut state, mut handler) = crypto_state(&dir, &["secret.txt.age"]);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    type_password(&mut state, &mut handler, "zxqwv");
    assert!(!file.exists(), "wrong password produces no output");
    assert!(
        state.message.as_ref().map(|m| m.is_error).unwrap_or(false),
        "wrong password shows a recoverable error"
    );
    let msg = state.message.as_ref().unwrap().text.clone();
    assert!(!msg.contains("zxqwv"), "password never appears in messages");
    assert!(!msg.contains("pw"), "password never appears in messages");
    // Correct password restores the bytes.
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    type_password(&mut state, &mut handler, "pw");
    assert_eq!(std::fs::read(&file).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mismatched_confirmation_blocks_encryption() {
    let (dir, file) = crypto_fixture();
    let (mut state, mut handler) = crypto_state(&dir, &["secret.txt"]);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    type_password(&mut state, &mut handler, "one");
    type_password(&mut state, &mut handler, "two");
    assert!(
        matches!(state.mode, Mode::Password(_)),
        "mismatch keeps the dialog open"
    );
    assert!(!dir.join("secret.txt.age").exists());
    assert!(file.exists());
    // Escape cancels without filesystem changes.
    drive(&mut state, &mut handler, [Action::Cancel]);
    assert!(matches!(state.mode, Mode::Browser));
    assert!(!dir.join("secret.txt.age").exists());
    let leftovers = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".part-"))
        .count();
    assert_eq!(leftovers, 0, "no temporary artifacts after cancel");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn directory_encrypts_and_decrypts_through_ui() {
    let dir = std::env::temp_dir().join(format!(
        "tui-explorer-replay-dircrypto-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let tree = dir.join("proj");
    std::fs::create_dir_all(tree.join("sub/empty")).unwrap();
    std::fs::write(tree.join("sub/data.bin"), vec![9u8; 2048]).unwrap();
    let (mut state, mut handler) = crypto_state(&dir, &["proj"]);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    type_password(&mut state, &mut handler, "pw");
    type_password(&mut state, &mut handler, "pw");
    let enc = dir.join("proj.tar.age");
    assert!(enc.exists(), "folder archive uses the .tar.age convention");
    std::fs::remove_dir_all(&tree).unwrap();
    let (mut state, mut handler) = crypto_state(&dir, &["proj.tar.age"]);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    type_password(&mut state, &mut handler, "pw");
    assert_eq!(
        std::fs::read(tree.join("sub/data.bin")).unwrap(),
        vec![9u8; 2048]
    );
    assert!(tree.join("sub/empty").is_dir(), "empty dirs preserved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_media_enters_preparing_mode_and_starts_after_surface() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    let downs = vec![Action::MoveDown; 16];
    drive(&mut state, &mut handler, downs);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let Mode::Media(media) = &state.mode else {
        panic!("expected media mode");
    };
    assert_eq!(media.phase, tui_explorer::media::MediaPhase::Preparing);
    assert!(media.awaiting_surface_ready);
    assert!(media.surface.is_none());
    let session = media.session;

    let surface = tui_explorer::app::state::MediaSurface {
        rect: ratatui::layout::Rect::new(0, 0, 40, 8),
        terminal_cells: (120, 36),
        cell_pixels: (8, 16),
    };
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady { session, surface }],
    );
    assert_eq!(
        handler.started_media.len(),
        1,
        "one StartMedia after the surface"
    );
    // The audio path starts playback directly after StartMedia; the Load
    // command is only meaningful for the later mpv video backend.
    assert_eq!(
        handler
            .media_commands
            .iter()
            .filter(|(_, command)| *command == tui_explorer::media::MediaCommand::Load)
            .count(),
        1,
        "exactly one Load follows MediaBackendReady"
    );
}

/// Focuses the demo `song.mp3` entry (visible position 16).
fn focus_song(state: &mut AppState, handler: &mut SyncHandler) {
    drive(state, handler, [Action::GotoFirst]);
    drive(state, handler, vec![Action::MoveDown; 16]);
}

#[test]
fn stale_media_results_are_rejected_by_session() {
    let (mut state, mut handler) = loaded(120, 36);
    focus_song(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = match &state.mode {
        Mode::Media(media) => media.session,
        _ => panic!("expected media mode"),
    };
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSpectrum {
            session: session + 1,
            spectrum: [0.9; 24],
        }],
    );
    let Mode::Media(media) = &state.mode else {
        panic!("expected media mode");
    };
    assert!(
        media.spectrum.iter().all(|value| *value == 0.0),
        "stale spectrum must not apply"
    );
    assert!(handler.started_media.is_empty());
}

#[test]
fn media_close_stops_then_returns_to_browser_exactly_once() {
    let (mut state, mut handler) = loaded(120, 36);
    focus_song(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = match &state.mode {
        Mode::Media(media) => media.session,
        _ => panic!("expected media mode"),
    };
    drive(&mut state, &mut handler, [Action::MediaClose]);
    assert_eq!(
        handler.stopped_media,
        vec![session],
        "close emits exactly one StopMedia"
    );
    drive(&mut state, &mut handler, [Action::MediaStopped { session }]);
    assert!(matches!(state.mode, Mode::Browser));
    drive(&mut state, &mut handler, [Action::MediaStopped { session }]);
    assert!(matches!(state.mode, Mode::Browser), "stale stop ignored");
    assert_eq!(handler.stopped_media.len(), 1);
}

fn video_surface() -> tui_explorer::app::state::MediaSurface {
    tui_explorer::app::state::MediaSurface {
        rect: ratatui::layout::Rect::new(4, 4, 60, 12),
        terminal_cells: (120, 36),
        cell_pixels: (8, 16),
    }
}

fn loaded_with_video(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(tui_explorer::testing::builders::demo_fs_with_video());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

#[test]
fn video_ready_then_load_ordering() {
    let (mut state, mut handler) = loaded_with_video(120, 36);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    // No StartMedia may occur before the surface is ready.
    assert!(handler.started_media.is_empty());
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session,
            surface: video_surface(),
        }],
    );
    assert_eq!(handler.started_media, vec![(session, video_path())]);
    // The sync harness answers StartMedia with MediaBackendReady, so the
    // single Load must have been emitted right after the backend was ready.
    let loads = handler
        .media_commands
        .iter()
        .filter(|(s, c)| *s == session && *c == tui_explorer::media::MediaCommand::Load)
        .count();
    assert_eq!(loads, 1, "exactly one Load follows MediaBackendReady");
}

#[test]
fn video_resize_snapshots_and_reenters_preparing() {
    let (mut state, mut handler) = loaded_with_video(120, 36);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session,
            surface: video_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session,
            phase: tui_explorer::media::MediaPhase::Playing,
            position: 12.5,
            duration: Some(90.0),
            volume: 80,
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::Resize {
            width: 160,
            height: 48,
        }],
    );
    assert_eq!(
        handler.stopped_media,
        vec![session],
        "resize stops the running video"
    );
    drive(&mut state, &mut handler, [Action::MediaStopped { session }]);
    let Mode::Media(media) = &state.mode else {
        panic!("expected media mode");
    };
    assert_eq!(media.phase, tui_explorer::media::MediaPhase::Preparing);
    assert_eq!(media.resume_position, Some(12.5));
    assert_eq!(media.resume_paused, Some(false));
    assert!(media.awaiting_surface_ready);
    assert!(media.surface.is_none());
}

#[test]
fn video_resize_below_minimum_fails_with_exact_message() {
    let (mut state, mut handler) = loaded_with_video(120, 36);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session,
            surface: video_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::Resize {
            width: 69,
            height: 18,
        }],
    );
    drive(&mut state, &mut handler, [Action::MediaStopped { session }]);
    let Mode::Media(media) = &state.mode else {
        panic!("expected media mode");
    };
    assert_eq!(media.phase, tui_explorer::media::MediaPhase::Error);
    assert_eq!(
        media.error.as_deref(),
        Some("video playback needs at least 70x18 cells")
    );
}
#[test]
fn quit_from_video_stops_before_quitting() {
    let (mut state, mut handler) = loaded_with_video(120, 36);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    drive(&mut state, &mut handler, [Action::Quit]);
    assert_eq!(handler.stopped_media, vec![session]);
    // The sync harness answers StopMedia with MediaStopped immediately, so
    // by the time drive() returns, quit has already been committed: the
    // stop strictly precedes the quit effect.
    assert!(handler.quit, "quit proceeds only after the media stopped");
}

fn focus_video(state: &mut AppState, handler: &mut SyncHandler) {
    drive(state, handler, [Action::GotoFirst]);
    let steps = state
        .browser
        .visible_entries()
        .position(|(_, view)| view.entry.display_name() == "clip.mkv")
        .expect("demo video file");
    drive(state, handler, vec![Action::MoveDown; steps]);
}

fn current_session(state: &AppState) -> u64 {
    match &state.mode {
        Mode::Media(media) => media.session,
        _ => panic!("expected media mode"),
    }
}

fn drag_rect(state: &AppState, pos: usize) -> ratatui::layout::Rect {
    state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            tui_explorer::ui::hit::HitTarget::Row(p) if *p == pos => Some(*rect),
            _ => None,
        })
        .expect("row hit region")
}

#[test]
fn drag_moves_file_to_directory_target() {
    let (mut state, mut handler) = rendered(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    // docs is a directory row (position 0); pick a file row as source.
    let file_pos = first_file_pos(&state);
    let src = drag_rect(&state, file_pos);
    let dst = drag_rect(&state, 0);
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: MouseKind::Left,
            x: src.x + 1,
            y: src.y + 1,
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: MouseKind::LeftDrag,
            x: dst.x + 1,
            y: dst.y + 1,
        }],
    );
    assert!(state.drag.is_some(), "drag activates past threshold");
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: MouseKind::LeftUp,
            x: dst.x + 1,
            y: dst.y + 1,
        }],
    );
    assert!(matches!(state.mode, Mode::Browser));
    // The move ran through the real operation path and was recorded.
    let moves: Vec<_> = handler
        .mutations
        .recorded()
        .into_iter()
        .filter(|m| matches!(m, tui_explorer::filesystem::RecordedMutation::Move { .. }))
        .collect();
    assert_eq!(moves.len(), 1, "drop performs exactly one move");
}

#[test]
fn drag_below_threshold_is_click_not_move() {
    let (mut state, mut handler) = rendered(120, 36);
    let cwd = state.browser.cwd.clone();
    let rect = drag_rect(&state, 0);
    drive(
        &mut state,
        &mut handler,
        [
            Action::Mouse {
                kind: MouseKind::Left,
                x: rect.x + 1,
                y: rect.y + 1,
            },
            Action::Mouse {
                kind: MouseKind::LeftDrag,
                x: rect.x + 2,
                y: rect.y + 1,
            },
            Action::Mouse {
                kind: MouseKind::LeftUp,
                x: rect.x + 2,
                y: rect.y + 1,
            },
        ],
    );
    assert!(state.drag.is_none());
    assert_eq!(state.browser.cwd, cwd);
    assert!(
        !handler
            .mutations
            .recorded()
            .iter()
            .any(|m| matches!(m, tui_explorer::filesystem::RecordedMutation::Move { .. })),
        "sub-threshold motion must not mutate"
    );
}

#[test]
fn esc_cancels_active_drag() {
    let (mut state, mut handler) = rendered(120, 36);
    let src = drag_rect(&state, 0);
    let dst = drag_rect(&state, 1);
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: MouseKind::Left,
            x: src.x + 1,
            y: src.y + 1,
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: MouseKind::LeftDrag,
            x: dst.x + 1,
            y: dst.y + 1,
        }],
    );
    assert!(state.drag.is_some());
    drive(&mut state, &mut handler, [Action::DragCancel]);
    assert!(state.drag.is_none());
}
#[test]
fn stale_media_stop_after_close_is_ignored() {
    let (mut state, mut handler) = loaded(120, 36);
    focus_song(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = match &state.mode {
        Mode::Media(media) => media.session,
        _ => panic!("expected media mode"),
    };
    drive(&mut state, &mut handler, [Action::MediaClose]);
    drive(&mut state, &mut handler, [Action::MediaStopped { session }]);
    assert!(matches!(state.mode, Mode::Browser));
}

fn video_path() -> std::path::PathBuf {
    tui_explorer::testing::builders::demo_root().join("clip.mkv")
}
