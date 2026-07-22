use std::path::PathBuf;

use tui_explorer::app::action::{Action, ConflictDecision};
use tui_explorer::app::state::{AppState, Mode};
use tui_explorer::filesystem::RecordedMutation;
use tui_explorer::testing::builders::{demo_fs, demo_state};
use tui_explorer::testing::{SyncHandler, drive};

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
fn open_file_goes_to_opener_not_fs() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoLast, Action::OpenFocused],
    );
    assert_eq!(handler.opened.len(), 1);
    assert!(handler.mutations.recorded().is_empty());
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
