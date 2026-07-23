use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use tui_explorer::app::action::Action;
use tui_explorer::app::state::{AppState, Mode, OperationState};
use tui_explorer::filesystem::EntryKind;
use tui_explorer::operations::OperationKind;
use tui_explorer::testing::builders::{
    FIXED_TIME, demo_fs, demo_fs_with_non_utf8, demo_state, entry,
};
use tui_explorer::testing::{MemoryFileSystem, SyncHandler, drive};
use tui_explorer::ui;
use tui_explorer::ui::hit::HitTarget;

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let mut lines = Vec::new();
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    while lines.last().map(|l| l.is_empty()) == Some(true) {
        lines.pop();
    }
    lines.join("\n")
}

fn render(state: &mut AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    buffer_text(&terminal)
}

fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join(format!("{name}.txt"))
}

fn assert_snapshot(name: &str, actual: &str) {
    let path = snapshot_path(name);
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot {}, run with UPDATE_SNAPSHOTS=1",
            path.display()
        )
    });
    if expected != actual {
        let exp_lines: Vec<&str> = expected.lines().collect();
        let act_lines: Vec<&str> = actual.lines().collect();
        for (idx, (e, a)) in exp_lines.iter().zip(act_lines.iter()).enumerate() {
            assert_eq!(e, a, "snapshot {name} diverges at line {}", idx + 1);
        }
        assert_eq!(
            exp_lines.len(),
            act_lines.len(),
            "snapshot {name} line count diverges"
        );
    }
}

fn loaded(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

fn command_actions(input: &str) -> Vec<Action> {
    let mut actions = vec![Action::EnterCommand];
    for c in input.chars() {
        actions.push(Action::CommandChar(c));
    }
    actions
}

// Includes the four target release sizes: 160x48, 120x36, 90x28, 70x22.
const SIZES: &[(u16, u16)] = &[
    (20, 8),
    (40, 12),
    (60, 16),
    (70, 22),
    (80, 24),
    (90, 28),
    (120, 36),
    (160, 48),
    (200, 60),
];

#[test]
fn main_state_all_sizes() {
    for (w, h) in SIZES {
        let (mut state, _) = loaded(*w, *h);
        let text = render(&mut state, *w, *h);
        assert_snapshot(&format!("main-{w}x{h}"), &text);
    }
}

#[test]
fn empty_directory() {
    let mut fs = MemoryFileSystem::new();
    fs.add_dir(Path::new("/home/demo"));
    let mut state = demo_state(80, 24);
    let mut handler = SyncHandler::new(fs);
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    let text = render(&mut state, 80, 24);
    assert_snapshot("empty-80x24", &text);
}

#[test]
fn tagged_and_selected() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::ToggleSelect, Action::ToggleSelect, Action::MoveDown],
    );
    drive(
        &mut state,
        &mut handler,
        command_actions("tag fav")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 120, 36);
    assert_snapshot("tagged-selected-120x36", &text);
}

#[test]
fn hidden_files_shown() {
    let (mut state, mut handler) = loaded(80, 24);
    drive(&mut state, &mut handler, [Action::ToggleHidden]);
    let text = render(&mut state, 80, 24);
    assert_snapshot("hidden-80x24", &text);
}

#[test]
fn non_utf8_filename() {
    let mut state = demo_state(80, 24);
    let mut handler = SyncHandler::new(demo_fs_with_non_utf8());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    let text = render(&mut state, 80, 24);
    assert_snapshot("non-utf8-80x24", &text);
    assert!(text.contains("bad-"));
}

#[test]
fn command_mode() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("copy \"/mnt/backup drive\""),
    );
    let text = render(&mut state, 120, 36);
    assert_snapshot("command-120x36", &text);
}

#[test]
fn confirm_modal() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    drive(
        &mut state,
        &mut handler,
        command_actions("delete")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 120, 36);
    assert_snapshot("confirm-120x36", &text);
}

#[test]
fn tag_picker() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("tag fav")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    drive(&mut state, &mut handler, [Action::OpenTagPicker]);
    let text = render(&mut state, 120, 36);
    assert_snapshot("picker-120x36", &text);
}

#[test]
fn error_notification() {
    let (mut state, mut handler) = loaded(80, 24);
    drive(
        &mut state,
        &mut handler,
        command_actions("bogus")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 80, 24);
    assert_snapshot("error-80x24", &text);
}

#[test]
fn operation_progress() {
    let (mut state, _) = loaded(120, 36);
    state.operation = Some(OperationState {
        kind: OperationKind::Copy,
        current: PathBuf::from("/home/demo/photo.png"),
        done: 3,
        total: 10,
    });
    let text = render(&mut state, 120, 36);
    assert_snapshot("operation-120x36", &text);
}

#[test]
fn help_overlay() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    let text = render(&mut state, 120, 36);
    assert_snapshot("help-120x36", &text);
}

#[test]
fn context_menu() {
    let (mut state, mut handler) = loaded(120, 36);
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .expect("render");
    let hit = state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            HitTarget::Row(3) => Some(*rect),
            _ => None,
        })
        .expect("row hit");
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: tui_explorer::app::action::MouseKind::Right,
            x: hit.x + 1,
            y: hit.y,
        }],
    );
    assert!(matches!(state.mode, Mode::ContextMenu(_)));
    let text = render(&mut state, 120, 36);
    assert_snapshot("context-120x36", &text);
}

#[test]
fn symlinks_and_executables_render() {
    let (state, _) = loaded(160, 48);
    let mut state = state;
    let text = render(&mut state, 160, 48);
    assert!(text.contains("LNK>"), "symlink tile badge present");
    assert!(text.contains("EXE>"), "executable tile badge present");
    assert!(text.contains("build.sh"));
    assert!(text.contains("README link"));
}

#[test]
fn invariants_all_sizes() {
    for (w, h) in SIZES {
        let (mut state, _) = loaded(*w, *h);
        let backend = TestBackend::new(*w, *h);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| ui::render(frame, &mut state))
            .unwrap_or_else(|e| panic!("render panicked at {w}x{h}: {e}"));
        let area = terminal.backend().buffer().area;
        for (rect, target) in &state.hit_map.regions {
            assert!(
                rect.x + rect.width <= area.x + area.width
                    && rect.y + rect.height <= area.y + area.height,
                "hit region {target:?} out of bounds at {w}x{h}"
            );
        }
        let visible = state.browser.visible_len();
        if visible > 0 {
            let selected = state.browser.selected;
            let scroll = state.browser.scroll;
            let viewport = state.list_viewport;
            assert!(
                selected >= scroll && selected < scroll + viewport,
                "selected row not visible at {w}x{h}"
            );
        }
        if ui::tier_for(*w, *h) == ui::Tier::TooSmall {
            continue;
        }
        let row_hits = state
            .hit_map
            .regions
            .iter()
            .filter(|(_, t)| matches!(t, HitTarget::Row(_)))
            .count();
        let expected_rows = visible.min(state.list_viewport);
        assert_eq!(
            row_hits, expected_rows,
            "row hit regions diverge from rendered rows at {w}x{h}"
        );
        let breadcrumb_hits = state
            .hit_map
            .regions
            .iter()
            .filter(|(_, t)| matches!(t, HitTarget::Breadcrumb(_)))
            .count();
        let segments = tui_explorer::app::reduce::breadcrumb_segments(&state.browser.cwd).len();
        assert_eq!(
            breadcrumb_hits, segments,
            "breadcrumb hits diverge from segments at {w}x{h}"
        );
    }
}

#[test]
fn tiny_layout_stays_operational() {
    let (mut state, mut handler) = loaded(20, 8);
    let text = render(&mut state, 20, 8);
    assert_snapshot("toosmall-20x8", &text);
    drive(&mut state, &mut handler, [Action::Quit]);
    assert!(handler.quit);
    let (mut state2, mut handler2) = loaded(24, 6);
    drive(&mut state2, &mut handler2, [Action::MoveDown]);
    let _ = render(&mut state2, 24, 6);
    drive(&mut state2, &mut handler2, [Action::Quit]);
    assert!(handler2.quit);
}

#[test]
fn tags_identifiable_without_color() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("tag fav")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 120, 36);
    assert!(text.contains("[fav]"), "tag badge text visible in buffer");
}

#[test]
fn wide_layout_has_details_panel() {
    let (mut state, _) = loaded(160, 48);
    let text = render(&mut state, 160, 48);
    assert!(text.contains("Type:"), "details panel visible");
    assert!(text.contains("Tags:"), "details tags visible");
    let tag_hits = state
        .hit_map
        .regions
        .iter()
        .filter(|(_, t)| matches!(t, HitTarget::TagBadge))
        .count();
    assert!(tag_hits > 0, "tag badge hit region exists");
}

#[test]
fn overlay_blocks_row_clicks() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .expect("render");
    let row_rect = state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            HitTarget::Row(0) => Some(*rect),
            _ => None,
        })
        .expect("row region under overlay");
    let hit = state.hit_map.hit(row_rect.x + 1, row_rect.y);
    assert_eq!(hit, Some(HitTarget::Blocker), "overlay blocks row clicks");
    let before = state.browser.selected;
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: tui_explorer::app::action::MouseKind::Left,
            x: row_rect.x + 1,
            y: row_rect.y,
        }],
    );
    assert_eq!(state.browser.selected, before);
    assert!(
        matches!(state.mode, Mode::Browser),
        "safe dismiss closes help"
    );
}

#[test]
fn mouse_row_click_and_breadcrumb() {
    let (mut state, mut handler) = loaded(120, 36);
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .expect("render");
    let row_rect = state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            HitTarget::Row(2) => Some(*rect),
            _ => None,
        })
        .expect("row 2 region");
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: tui_explorer::app::action::MouseKind::Left,
            x: row_rect.x + 2,
            y: row_rect.y,
        }],
    );
    assert_eq!(state.browser.selected, 2);
    let crumb = state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            HitTarget::Breadcrumb(0) => Some(*rect),
            _ => None,
        })
        .expect("breadcrumb root region");
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: tui_explorer::app::action::MouseKind::Left,
            x: crumb.x,
            y: crumb.y,
        }],
    );
    assert_eq!(state.browser.cwd, PathBuf::from("/"));
}

#[test]
fn mouse_scroll_moves_list() {
    let (mut state, mut handler) = loaded(40, 12);
    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .expect("render");
    let row_rect = state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            HitTarget::Row(0) => Some(*rect),
            _ => None,
        })
        .expect("row region");
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: tui_explorer::app::action::MouseKind::ScrollDown,
            x: row_rect.x + 1,
            y: row_rect.y,
        }],
    );
    // Grid at 40x12 has 2 columns; one scroll tick moves one row down.
    assert_eq!(state.browser.selected, 2);
}

#[test]
fn overlays_at_standard_size() {
    let (mut state, mut handler) = loaded(80, 24);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    let text = render(&mut state, 80, 24);
    assert_snapshot("help-80x24", &text);
    drive(
        &mut state,
        &mut handler,
        [Action::Cancel, Action::GotoFirst],
    );
    drive(
        &mut state,
        &mut handler,
        command_actions("delete")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 80, 24);
    assert_snapshot("confirm-80x24", &text);
}

#[test]
fn long_names_never_panic() {
    let mut fs = MemoryFileSystem::new();
    let root = PathBuf::from("/home/demo");
    fs.add_dir(&root);
    let long_name = "x".repeat(300);
    fs.add_entry(
        &root,
        entry(&root, &long_name, EntryKind::File, 1, 0o644, FIXED_TIME),
    );
    for (w, h) in SIZES {
        let mut state = demo_state(*w, *h);
        let mut handler = SyncHandler::new(fs.clone());
        drive(&mut state, &mut handler, [Action::LoadInitial]);
        let _ = render(&mut state, *w, *h);
    }
}
