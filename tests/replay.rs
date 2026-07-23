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
        handler.opened.is_empty(),
        "single click must not open files"
    );
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
    assert_eq!(handler.opened.len(), 0, "folder opens navigate, not spawn");
}

#[test]
fn double_click_different_entries_does_not_open() {
    let (mut state, mut handler) = rendered(120, 36);
    let cwd = state.browser.cwd.clone();
    click(&mut state, &mut handler, 0);
    click(&mut state, &mut handler, 1);
    assert_eq!(state.browser.selected, 1);
    assert_eq!(state.browser.cwd, cwd, "clicks on different entries select");
    assert!(handler.opened.is_empty());
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
    assert!(handler.opened.is_empty());
    assert_eq!(state.browser.cwd, PathBuf::from("/home/demo"));
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
