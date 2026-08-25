//! Replay-style coverage for the internal clipboard, context-menu
//! targeting (single/bulk/background) and the paste operation lifecycle.
//!
//! Every test drives the REAL `reduce()` through `testing::drive` with the
//! `MemoryFileSystem` + `RecordingMutations` fakes, then asserts on state
//! fields and recorded effects/mutations.
//!
//! Note on filesystem fidelity: `RecordingMutations` records mutation intent
//! (copy/move/delete) but never rewrites `MemoryFileSystem.dirs`, so
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;

use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};
use tui_explorer::app::action::{Action, ConflictDecision, MouseKind};
use tui_explorer::app::state::{
    AppState, ClipMode, ContextItem, ContextMenuState, ContextTarget, Mode,
};
use tui_explorer::filesystem::RecordedMutation;
use tui_explorer::operations::{OpEntryResult, OpOutcome, OperationReport};
use tui_explorer::testing::builders::{demo_fs, demo_state};
use tui_explorer::testing::{SyncHandler, drive};
use tui_explorer::ui;

const ROOT: &str = "/home/demo";

// ---------------------------------------------------------------------------
// Helpers (mirroring tests/replay.rs conventions)
// ---------------------------------------------------------------------------

fn loaded(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

fn rendered(width: u16, height: u16) -> (AppState, SyncHandler) {
    let (mut state, handler) = loaded(width, height);
    rerender(&mut state);
    (state, handler)
}

/// Re-renders in place so modal hit regions (e.g. context items) exist.
fn rerender(state: &mut AppState) {
    let backend = TestBackend::new(state.width, state.height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
}

fn row_rect(state: &AppState, pos: usize) -> Rect {
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

fn context_item_rect(state: &AppState, idx: usize) -> Rect {
    state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            tui_explorer::ui::hit::HitTarget::ContextItem(i) if *i == idx => Some(*rect),
            _ => None,
        })
        .expect("context item hit region")
}

fn context_item_hits(state: &AppState) -> Vec<(Rect, usize)> {
    state
        .hit_map
        .regions
        .iter()
        .filter_map(|(rect, target)| match target {
            tui_explorer::ui::hit::HitTarget::ContextItem(i) => Some((*rect, *i)),
            _ => None,
        })
        .collect()
}

fn grid_background_rect(state: &AppState) -> Rect {
    state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, target)| match target {
            tui_explorer::ui::hit::HitTarget::GridBackground => Some(*rect),
            _ => None,
        })
        .expect("grid background hit region")
}

/// Finds a cell that resolves to the grid background, not to any tile.
fn background_point(state: &AppState) -> (u16, u16) {
    let bg = grid_background_rect(state);
    for y in bg.y..bg.y + bg.height {
        for x in bg.x..bg.x + bg.width {
            if state.hit_map.hit(x, y) == Some(tui_explorer::ui::hit::HitTarget::GridBackground) {
                return (x, y);
            }
        }
    }
    panic!("no blank grid background cell in this layout");
}

fn path_at(state: &AppState, pos: usize) -> PathBuf {
    let indices = state.browser.visible_indices();
    state.browser.entries[indices[pos]].entry.path.clone()
}

/// Visible-grid position of the entry called `name`.
fn pos_of_name(state: &AppState, name: &str) -> usize {
    let indices = state.browser.visible_indices();
    indices
        .iter()
        .position(|&i| state.browser.entries[i].entry.name == name)
        .unwrap_or_else(|| panic!("entry {name} not visible"))
}

/// Moves the navigation cursor onto the entry called `name`.
fn focus_entry(state: &mut AppState, handler: &mut SyncHandler, name: &str) {
    let pos = pos_of_name(state, name);
    let mut actions = vec![Action::GotoFirst];
    actions.extend(std::iter::repeat_n(Action::MoveDown, pos));
    drive(state, handler, actions);
}

/// Double-clicks a row; on a directory this navigates into it.
fn enter_by_double_click(state: &mut AppState, handler: &mut SyncHandler, name: &str) {
    let rect = row_rect(state, pos_of_name(state, name));
    let press = || Action::Mouse {
        kind: MouseKind::Left,
        x: rect.x + 1,
        y: rect.y + rect.height / 2,
        ctrl: false,
    };
    drive(state, handler, [press(), press()]);
}

fn mouse(kind: MouseKind, x: u16, y: u16) -> Action {
    Action::Mouse {
        kind,
        x,
        y,
        ctrl: false,
    }
}

fn right_click(state: &mut AppState, handler: &mut SyncHandler, x: u16, y: u16) {
    drive(state, handler, [mouse(MouseKind::Right, x, y)]);
}

fn left_click(state: &mut AppState, handler: &mut SyncHandler, x: u16, y: u16) {
    drive(state, handler, [mouse(MouseKind::Left, x, y)]);
}

/// Right-clicks row `pos` and re-renders so the menu registers hit regions.
fn open_menu_on_row(state: &mut AppState, handler: &mut SyncHandler, pos: usize) {
    let rect = row_rect(state, pos);
    right_click(state, handler, rect.x + 1, rect.y + rect.height / 2);
    rerender(state);
}

/// Right-clicks a blank grid cell and re-renders.
fn open_menu_on_background(state: &mut AppState, handler: &mut SyncHandler) -> (u16, u16) {
    let point = background_point(state);
    right_click(state, handler, point.0, point.1);
    rerender(state);
    point
}

fn menu(state: &AppState) -> &ContextMenuState {
    let Mode::ContextMenu(menu) = &state.mode else {
        panic!("expected context menu mode, got {:?}", state.mode.name());
    };
    menu
}

fn command_actions(input: &str) -> Vec<Action> {
    let mut actions = vec![Action::EnterCommand];
    for c in input.chars() {
        actions.push(Action::CommandChar(c));
    }
    actions.push(Action::CommandSubmit);
    actions
}
fn menu_actions(state: &AppState) -> Vec<ContextItem> {
    menu(state).items.iter().map(|i| i.action).collect()
}

/// Selects the first two visible entries via real toggle-select actions.
fn select_first_two(state: &mut AppState, handler: &mut SyncHandler) -> BTreeSet<PathBuf> {
    drive(
        state,
        handler,
        [
            Action::GotoFirst,
            Action::ToggleSelect,
            Action::ToggleSelect,
        ],
    );
    state.browser.selection.clone()
}

fn assert_rect_inside(rect: Rect, width: u16, height: u16, what: &str) {
    assert!(
        rect.width > 0 && rect.height > 0,
        "{what} collapsed: {rect:?}"
    );
    assert!(
        rect.x + rect.width <= width,
        "{what} overflows right edge: {rect:?} (w={width})"
    );
    assert!(
        rect.y + rect.height <= height,
        "{what} overflows bottom edge: {rect:?} (h={height})"
    );
}

fn copies(recorded: &[RecordedMutation]) -> Vec<(&PathBuf, &PathBuf, bool)> {
    recorded
        .iter()
        .filter_map(|m| match m {
            RecordedMutation::Copy { src, dst, replace } => Some((src, dst, *replace)),
            _ => None,
        })
        .collect()
}

fn moves(recorded: &[RecordedMutation]) -> Vec<(&PathBuf, &PathBuf)> {
    recorded
        .iter()
        .filter_map(|m| match m {
            RecordedMutation::Move { src, dst, .. } => Some((src, dst)),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Bulk right-click on a selected member of a multi-selection
// ---------------------------------------------------------------------------

#[test]
fn bulk_right_click_on_selected_member_preserves_selection_and_targets_bulk() {
    let (mut state, mut handler) = rendered(120, 36);
    let selection = select_first_two(&mut state, &mut handler);
    assert_eq!(selection.len(), 2);

    // Right-click the FIRST selected member (docs).
    open_menu_on_row(&mut state, &mut handler, 0);

    let m = menu(&state);
    assert_eq!(
        m.target,
        ContextTarget::Bulk {
            paths: vec![
                PathBuf::from(format!("{ROOT}/docs")),
                PathBuf::from(format!("{ROOT}/src")),
            ]
        },
        "menu must capture the whole sorted selection"
    );
    assert_eq!(state.browser.selection, selection, "selection preserved");
    assert_eq!(
        menu_actions(&state),
        vec![
            ContextItem::Cut,
            ContextItem::ClipboardCopy,
            ContextItem::Delete
        ],
        "bulk menu offers exactly Cut/Copy/Delete"
    );
    assert!(m.items.iter().all(|i| i.enabled));

    // Choosing Copy captures every path without touching the filesystem.
    drive(
        &mut state,
        &mut handler,
        [Action::ContextMove(1), Action::ContextChoose],
    );
    assert_eq!(state.clipboard.mode, Some(ClipMode::Copy));
    assert_eq!(
        state.clipboard.items,
        vec![
            PathBuf::from(format!("{ROOT}/docs")),
            PathBuf::from(format!("{ROOT}/src")),
        ]
    );
    assert!(handler.mutations.recorded().is_empty());
    assert!(matches!(state.mode, Mode::Browser));
}

// ---------------------------------------------------------------------------
// 2. ClipboardCopy/Cut store ALL captured paths; zero FS ops during capture
// ---------------------------------------------------------------------------

#[test]
fn clipboard_capture_stores_all_paths_and_never_touches_filesystem() {
    // Bulk copy through the menu keeps both entries.
    let (mut state, mut handler) = rendered(120, 36);
    select_first_two(&mut state, &mut handler);
    open_menu_on_row(&mut state, &mut handler, 1);
    drive(
        &mut state,
        &mut handler,
        [Action::ContextMove(1), Action::ContextChoose],
    );
    assert_eq!(state.clipboard.mode, Some(ClipMode::Copy));
    assert_eq!(state.clipboard.items.len(), 2);
    assert!(handler.mutations.recorded().is_empty());

    // Single cut through the menu captures exactly the clicked path.
    let (mut state, mut handler) = rendered(120, 36);
    let cargo_pos = pos_of_name(&state, "Cargo.toml");
    open_menu_on_row(&mut state, &mut handler, cargo_pos);
    assert_eq!(
        menu_actions(&state),
        vec![
            ContextItem::Open,
            ContextItem::OpenWith,
            ContextItem::Rename,
            ContextItem::Cut,
            ContextItem::ClipboardCopy,
            ContextItem::Delete,
            ContextItem::Tags,
        ]
    );
    drive(
        &mut state,
        &mut handler,
        [Action::ContextMove(3), Action::ContextChoose],
    );
    assert_eq!(state.clipboard.mode, Some(ClipMode::Cut));
    assert_eq!(
        state.clipboard.items,
        vec![PathBuf::from(format!("{ROOT}/Cargo.toml"))]
    );
    assert!(handler.mutations.recorded().is_empty());

    // The direct action arms accept arbitrary multi-path captures verbatim.
    let (mut state, mut handler) = rendered(120, 36);
    let paths = vec![
        PathBuf::from(format!("{ROOT}/notes.md")),
        PathBuf::from(format!("{ROOT}/src/lib.rs")),
    ];
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCut {
            paths: paths.clone(),
        }],
    );
    assert_eq!(state.clipboard.mode, Some(ClipMode::Cut));
    assert_eq!(state.clipboard.items, paths);
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCopy {
            paths: paths.clone(),
        }],
    );
    assert_eq!(state.clipboard.mode, Some(ClipMode::Copy));
    assert_eq!(state.clipboard.items, paths);
    assert!(handler.mutations.recorded().is_empty());
}

// ---------------------------------------------------------------------------
// 3. Clipboard survives deselection, directory change and Esc
// ---------------------------------------------------------------------------

#[test]
fn clipboard_survives_selection_clear_navigation_and_escape() {
    let (mut state, mut handler) = rendered(120, 36);
    let clicked_pos = pos_of_name(&state, "Cargo.toml");
    open_menu_on_row(&mut state, &mut handler, clicked_pos);
    drive(
        &mut state,
        &mut handler,
        [Action::ContextMove(4), Action::ContextChoose],
    );
    assert_eq!(state.clipboard.chip().as_deref(), Some("COPY: 1 item"));

    // Esc in browser mode clears selection/filters but must keep the board.
    drive(&mut state, &mut handler, [Action::Cancel]);
    // Re-render so the closed menu stops shadowing row hit regions.
    rerender(&mut state);
    assert_eq!(state.clipboard.mode, Some(ClipMode::Copy));

    // Navigate into the empty docs/ directory, then Esc again.
    enter_by_double_click(&mut state, &mut handler, "docs");
    assert_eq!(state.browser.cwd, PathBuf::from(format!("{ROOT}/docs")));
    drive(&mut state, &mut handler, [Action::Cancel]);

    assert_eq!(state.clipboard.mode, Some(ClipMode::Copy));
    assert_eq!(
        state.clipboard.items,
        vec![PathBuf::from(format!("{ROOT}/Cargo.toml"))]
    );
    assert_eq!(state.clipboard.chip().as_deref(), Some("COPY: 1 item"));
}

// ---------------------------------------------------------------------------
// 4. Right-click an UNSELECTED row while a multi-selection exists
// ---------------------------------------------------------------------------

#[test]
fn right_click_unselected_row_targets_single_and_keeps_multi_selection() {
    let (mut state, mut handler) = rendered(120, 36);
    let selection = select_first_two(&mut state, &mut handler); // docs + src

    // Third visible entry is unselected.
    let clicked = path_at(&state, 2);
    assert!(!selection.contains(&clicked));
    open_menu_on_row(&mut state, &mut handler, 2);

    let m = menu(&state);
    assert_eq!(
        m.target,
        ContextTarget::Single {
            path: clicked.clone()
        },
        "unselected row gets a single-item menu for THAT row only"
    );
    assert_eq!(state.browser.selection, selection, "selection untouched");
    assert!(!state.browser.selection.contains(&clicked));
    assert_eq!(menu_actions(&state).len(), 7, "full single-item menu");
}

// ---------------------------------------------------------------------------
// 5. Background right-click: Paste enabled iff clipboard non-empty;
//    disabled Paste fires nothing
// ---------------------------------------------------------------------------

#[test]
fn background_right_click_without_clipboard_shows_disabled_paste_that_fires_nothing() {
    let (mut state, mut handler) = rendered(120, 36);
    assert!(state.clipboard.is_empty());

    open_menu_on_background(&mut state, &mut handler);
    let m = menu(&state);
    assert_eq!(m.target, ContextTarget::Background);
    assert_eq!(menu_actions(&state), vec![ContextItem::Paste]);
    assert!(!m.items[0].enabled, "Paste disabled without clipboard");

    // Disabled entries register no hit region at all.
    let raw_point = (m.x, m.y);
    assert!(
        context_item_hits(&state).is_empty(),
        "disabled Paste must not be clickable"
    );

    // Clicking where the disabled entry sits executes nothing at all.
    left_click(&mut state, &mut handler, raw_point.0, raw_point.1);
    assert!(handler.mutations.recorded().is_empty(), "no FS ops fired");
    assert!(state.clipboard.is_empty());
    assert!(state.operation.is_none(), "no operation started");
}

#[test]
fn background_right_click_with_clipboard_enables_paste_that_pastes_here() {
    let (mut state, mut handler) = rendered(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCopy {
            paths: vec![PathBuf::from(format!("{ROOT}/Cargo.toml"))],
        }],
    );

    // Move somewhere else so the paste has a distinct destination…
    enter_by_double_click(&mut state, &mut handler, "docs");
    rerender(&mut state);
    open_menu_on_background(&mut state, &mut handler);

    let m = menu(&state);
    assert_eq!(m.target, ContextTarget::Background);
    assert!(m.items[0].enabled, "Paste enabled with clipboard");

    let item = context_item_rect(&state, 0);
    drive(
        &mut state,
        &mut handler,
        [
            mouse(MouseKind::Moved, item.x + 1, item.y),
            mouse(MouseKind::Left, item.x + 1, item.y),
        ],
    );
    assert_eq!(
        copies(&handler.mutations.recorded()),
        vec![(
            &PathBuf::from(format!("{ROOT}/Cargo.toml")),
            &PathBuf::from(format!("{ROOT}/docs/Cargo.toml")),
            false
        )],
        "Paste targets the current directory"
    );
    // Copy mode survives pasting: repeat paste stays possible.
    assert_eq!(state.clipboard.mode, Some(ClipMode::Copy));
    assert!(!state.clipboard.is_empty());
}

// ---------------------------------------------------------------------------
// 6. Copy paste duplicates into destination; originals stay put
// ---------------------------------------------------------------------------

#[test]
fn paste_in_copy_mode_duplicates_files_into_destination() {
    let (mut state, mut handler) = rendered(120, 36);
    let sources = vec![
        PathBuf::from(format!("{ROOT}/Cargo.toml")),
        PathBuf::from(format!("{ROOT}/notes.md")),
    ];
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCopy {
            paths: sources.clone(),
        }],
    );

    enter_by_double_click(&mut state, &mut handler, "docs");
    drive(&mut state, &mut handler, [Action::ClipboardPaste]);

    let recorded = handler.mutations.recorded();
    assert_eq!(
        copies(&recorded),
        vec![
            (
                &sources[0],
                &PathBuf::from(format!("{ROOT}/docs/Cargo.toml")),
                false
            ),
            (
                &sources[1],
                &PathBuf::from(format!("{ROOT}/docs/notes.md")),
                false
            ),
        ],
        "copies land under the destination directory"
    );
    assert!(moves(&recorded).is_empty());

    // Originals remain present in the source listing (fake FS is never
    // rewritten by mutations, so the listing itself proves non-destruction).
    let root_names: Vec<OsString> = handler.fs.dirs[&PathBuf::from(ROOT)]
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert!(root_names.contains(&OsString::from("Cargo.toml")));
    assert!(root_names.contains(&OsString::from("notes.md")));

    let message = state.message.as_ref().expect("completion status");
    assert!(!message.is_error);
    assert!(message.text.starts_with("2/2"), "got {:?}", message.text);
}

// ---------------------------------------------------------------------------
// 7. Cut paste moves sources out; clipboard prunes after success
// ---------------------------------------------------------------------------

#[test]
fn paste_in_cut_mode_moves_sources_and_prunes_clipboard() {
    let (mut state, mut handler) = rendered(120, 36);
    let src = PathBuf::from(format!("{ROOT}/notes.md"));
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCut {
            paths: vec![src.clone()],
        }],
    );

    enter_by_double_click(&mut state, &mut handler, "docs");
    drive(&mut state, &mut handler, [Action::ClipboardPaste]);

    let recorded = handler.mutations.recorded();
    assert_eq!(
        moves(&recorded),
        vec![(&src, &PathBuf::from(format!("{ROOT}/docs/notes.md")))],
        "the move out of the origin directory was executed"
    );
    assert!(copies(&recorded).is_empty());

    // A fully successful Cut empties the clipboard (moved sources leave it)
    // and clears the pending-paste flag.
    assert!(
        state.clipboard.is_empty(),
        "clipboard after cut-paste: {:?}",
        state.clipboard
    );
    assert_eq!(state.clipboard.mode, None);
    assert!(state.pending_paste_mode.is_none());

    let message = state.message.as_ref().expect("completion status");
    assert!(!message.is_error);
    assert!(message.text.starts_with("1/1"), "got {:?}", message.text);
}

// ---------------------------------------------------------------------------
// 8. Pre-existing destination opens the conflict dialog; Replace completes
// ---------------------------------------------------------------------------

#[test]
fn conflicting_paste_opens_conflict_dialog_and_replace_resolves_it() {
    let (mut state, mut handler) = rendered(120, 36);
    let src = PathBuf::from(format!("{ROOT}/main.rs"));
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCopy {
            paths: vec![src.clone()],
        }],
    );

    // /home/demo/src/main.rs already exists => conflict.
    enter_by_double_click(&mut state, &mut handler, "src");
    drive(&mut state, &mut handler, [Action::ClipboardPaste]);

    assert!(
        matches!(state.mode, Mode::Conflict(_)),
        "conflict modal opened"
    );
    assert!(
        handler.mutations.recorded().is_empty(),
        "nothing written yet"
    );

    drive(
        &mut state,
        &mut handler,
        [Action::ConflictChoice(ConflictDecision::Replace)],
    );

    let recorded = handler.mutations.recorded();
    assert_eq!(
        copies(&recorded),
        vec![(&src, &PathBuf::from(format!("{ROOT}/src/main.rs")), true)],
        "replace overwrote the existing destination"
    );
    assert!(matches!(state.mode, Mode::Browser));
}

// ---------------------------------------------------------------------------
// 9. Missing source at paste => per-item Failed outcome, no panic
// ---------------------------------------------------------------------------

#[test]
fn failing_sources_report_per_item_failure_without_panicking() {
    let (mut state, mut handler) = rendered(120, 36);
    let sources = vec![
        PathBuf::from(format!("{ROOT}/Cargo.toml")),
        PathBuf::from(format!("{ROOT}/notes.md")),
    ];
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCopy {
            paths: sources.clone(),
        }],
    );

    enter_by_double_click(&mut state, &mut handler, "docs");

    // Every mutation now fails as if each source were unreadable.
    handler.mutations.fail_with = Some("input/output error".to_string());
    drive(&mut state, &mut handler, [Action::ClipboardPaste]);

    let report = &state.operation;
    assert!(report.is_none(), "operation finished and cleared");
    let message = state.message.as_ref().expect("failure status");
    assert!(message.is_error, "got {:?}", message.text);
    assert!(message.text.contains("failed"), "got {:?}", message.text);

    // Both items failed individually (per-item outcomes, not a global abort).
    let recorded = handler.mutations.recorded();
    assert_eq!(copies(&recorded).len(), 2, "both items were attempted");
    assert!(
        message.text.contains("0/2 done"),
        "no item succeeded: {:?}",
        message.text
    );

    // The reducer composes per-item outcomes into the status line: feed it a
    // mixed report shaped like a paste where one source vanished mid-flight.
    let (mut state, mut _handler) = rendered(120, 36);
    drive(
        &mut state,
        &mut _handler,
        [Action::OperationFinished {
            report: OperationReport {
                results: vec![
                    OpEntryResult {
                        source: PathBuf::from(format!("{ROOT}/Cargo.toml")),
                        outcome: OpOutcome::Done,
                    },
                    OpEntryResult {
                        source: PathBuf::from(format!("{ROOT}/ghost.md")),
                        outcome: OpOutcome::Failed(
                            "No such file or directory (os error 2)".to_string(),
                        ),
                    },
                ],
                moves: Vec::new(),
            },
        }],
    );
    let message = state.message.as_ref().expect("mixed outcome status");
    assert!(message.is_error);
    assert!(
        message.text.contains("1/2 done") && message.text.contains("1 failed"),
        "got {:?}",
        message.text
    );
    assert!(state.operation.is_none());
}

// ---------------------------------------------------------------------------
// 10. Into-itself pastes are rejected by validate before any operation
// ---------------------------------------------------------------------------

#[test]
fn into_itself_paste_errors_before_starting_anything() {
    let (mut state, mut handler) = rendered(120, 36);
    let src = PathBuf::from(format!("{ROOT}/src"));
    drive(
        &mut state,
        &mut handler,
        [Action::ClipboardCopy {
            paths: vec![src.clone()],
        }],
    );

    // Paste src while INSIDE src: destination_for joins the name, producing
    // /home/demo/src/src — strictly under the source, so validate rejects it
    // as IntoItself. (SamePath is unreachable through paste: dst always
    // appends the entry name, so an identical path collapses into the
    // starts_with branch.)
    enter_by_double_click(&mut state, &mut handler, "src");
    drive(&mut state, &mut handler, [Action::ClipboardPaste]);

    let message = state.message.as_ref().expect("validation error");
    assert!(message.is_error);
    assert!(
        message.text.contains("into itself"),
        "got {:?}",
        message.text
    );
    assert!(handler.mutations.recorded().is_empty(), "no FS ops");
    assert!(state.operation.is_none(), "no operation started");
    assert!(state.pending_paste_mode.is_none());

    // Build /home/demo/src/deeper and paste INSIDE it: dest under source.
    drive(&mut state, &mut handler, command_actions("mkdir deeper"));
    focus_entry(&mut state, &mut handler, "deeper");
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    assert_eq!(
        state.browser.cwd,
        PathBuf::from(format!("{ROOT}/src/deeper"))
    );

    drive(&mut state, &mut handler, [Action::ClipboardPaste]);
    let message = state.message.as_ref().expect("validation error");
    assert!(message.is_error);
    assert!(
        message.text.contains("into itself"),
        "got {:?}",
        message.text
    );
    let recorded = handler.mutations.recorded();
    assert!(copies(&recorded).is_empty() && moves(&recorded).is_empty());
    assert!(state.operation.is_none());
    assert!(state.pending_paste_mode.is_none());
}

// ---------------------------------------------------------------------------
// 11. Menu bounds-clamped near the terminal edges
// ---------------------------------------------------------------------------

#[test]
fn menu_opened_near_bottom_row_clamps_inside_the_terminal() {
    let (width, height) = (48u16, 16u16);
    let (mut state, mut handler) = rendered(width, height);

    // Bottom-most RENDERED tile (rows below the viewport have no hit rect).
    let bottom = state
        .hit_map
        .regions
        .iter()
        .filter_map(|(rect, target)| match target {
            tui_explorer::ui::hit::HitTarget::Row(pos) => Some((rect.y, *pos)),
            _ => None,
        })
        .max()
        .expect("rows are rendered")
        .1;
    open_menu_on_row(&mut state, &mut handler, bottom);

    let hits = context_item_hits(&state);
    assert_eq!(
        hits.len(),
        7,
        "every enabled single-menu entry is clickable"
    );
    for (rect, idx) in hits {
        assert_rect_inside(rect, width, height, &format!("context item {idx}"));
    }
}

#[test]
fn menu_anchored_beyond_edges_is_clamped_fully_visible() {
    let (width, height) = (40u16, 14u16);
    let (mut state, mut _handler) = rendered(width, height);

    // Place the menu anchor outside the clamping margin directly.
    state.mode = Mode::ContextMenu(Box::new(ContextMenuState {
        items: ContextItem::menu_for(
            &ContextTarget::Single {
                path: PathBuf::from(format!("{ROOT}/Cargo.toml")),
            },
            false,
        ),
        target: ContextTarget::Single {
            path: PathBuf::from(format!("{ROOT}/Cargo.toml")),
        },
        selected: 0,
        x: width - 2,
        y: height - 2,
    }));
    rerender(&mut state);

    let hits = context_item_hits(&state);
    assert_eq!(hits.len(), 7, "whole menu stays reachable near the corner");
    for (rect, idx) in hits {
        assert_rect_inside(rect, width, height, &format!("clamped item {idx}"));
    }
}

// ---------------------------------------------------------------------------
// 12. Hover selects-without-execute until ContextChoose
// ---------------------------------------------------------------------------

#[test]
fn hover_moves_menu_highlight_without_executing_until_choose() {
    let (mut state, mut handler) = rendered(120, 36);
    let cargo_pos = pos_of_name(&state, "Cargo.toml");
    open_menu_on_row(&mut state, &mut handler, cargo_pos);

    // Hover straight to the Cut row.
    let item = context_item_rect(&state, 3);
    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::Moved, item.x + 1, item.y)],
    );
    assert_eq!(menu(&state).selected, 3, "highlight follows pointer");
    assert!(matches!(state.mode, Mode::ContextMenu(_)));
    assert!(
        handler.mutations.recorded().is_empty(),
        "hover never executes"
    );
    assert!(state.clipboard.is_empty(), "hover never executes");

    // Only the explicit choose runs the highlighted row (Cut).
    drive(&mut state, &mut handler, [Action::ContextChoose]);
    assert_eq!(state.clipboard.mode, Some(ClipMode::Cut));
    assert_eq!(
        state.clipboard.items,
        vec![PathBuf::from(format!("{ROOT}/Cargo.toml"))]
    );
    assert!(handler.mutations.recorded().is_empty());
}

// ---------------------------------------------------------------------------
// 13. Drag-drop commit regression (minimal port from replay.rs)
// ---------------------------------------------------------------------------

#[test]
fn drag_drop_onto_directory_still_commits_a_move() {
    let (mut state, mut handler) = rendered(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);

    let file_pos = pos_of_name(&state, "Cargo.toml");
    let src = row_rect(&state, file_pos);
    let dst = row_rect(&state, 0); // docs directory row

    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::Left, src.x + 1, src.y + 1)],
    );
    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::LeftDrag, dst.x + 1, dst.y + 1)],
    );
    assert!(state.drag.is_some(), "drag activates past threshold");
    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::LeftUp, dst.x + 1, dst.y + 1)],
    );

    assert!(matches!(state.mode, Mode::Browser));
    let recorded = handler.mutations.recorded();
    let dropped = moves(&recorded);
    assert_eq!(dropped.len(), 1, "drop performs exactly one move");
    assert_eq!(
        dropped[0].1,
        &PathBuf::from(format!("{ROOT}/docs/Cargo.toml")),
        "drop lands inside the directory row"
    );
}
