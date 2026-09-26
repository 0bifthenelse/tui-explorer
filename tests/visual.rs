use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use tui_explorer::app::action::Action;
use tui_explorer::app::reduce::reduce;
use tui_explorer::app::state::{AppState, Mode, OperationState, PreviewContent};
use tui_explorer::filesystem::EntryKind;
use tui_explorer::operations::OperationKind;
use tui_explorer::preview::PreviewLoaded;
use tui_explorer::testing::builders::{
    FIXED_TIME, demo_fs, demo_fs_with_non_utf8, demo_state, entry,
};
use tui_explorer::testing::{MemoryFileSystem, SyncHandler, drive};
use tui_explorer::ui;
use tui_explorer::ui::hit::HitTarget;
use tui_explorer::ui::palette::{
    ACCENT, ACCENT_SOFT, BORDER_STRONG, BORDER_SUBTLE, DANGER, FOCUS_BG, SELECTED_BG, SURFACE_2,
    SURFACE_3, TEXT_PRIMARY,
};
use tui_explorer::ui::theme;

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
fn rendered_terminal(state: &mut AppState, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    terminal
}

fn cells_matching<F>(buffer: &Buffer, mut predicate: F) -> usize
where
    F: FnMut(&ratatui::buffer::Cell) -> bool,
{
    let area = buffer.area;
    let mut count = 0;
    for y in 0..area.height {
        for x in 0..area.width {
            if predicate(&buffer[(x, y)]) {
                count += 1;
            }
        }
    }
    count
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
fn sort_mode_is_visible_in_the_grid_header() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("sort size")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 120, 36);
    assert!(
        text.contains("Sort: size (asc)"),
        "sort indicator missing:\n{text}"
    );

    drive(
        &mut state,
        &mut handler,
        command_actions("sort size-desc")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let descending = render(&mut state, 120, 36);
    assert!(
        descending.contains("Sort: size desc (desc)"),
        "descending sort indicator missing:\n{descending}"
    );
}

#[test]
fn empty_filter_result_explains_why_the_grid_is_blank() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("filter definitely-no-such-file")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 120, 36);
    assert!(
        text.contains("No matching files"),
        "empty result unclear:\n{text}"
    );
}

#[test]
fn current_directory_filter_is_visible_and_limits_tiles() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("filter rs")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let text = render(&mut state, 120, 36);
    assert!(
        text.contains("Filter: rs"),
        "filter indicator missing:\n{text}"
    );
    assert!(
        text.contains("1/"),
        "filtered result count missing:\n{text}"
    );
    assert!(text.contains("main.rs"), "matching tile missing:\n{text}");
    assert!(
        !text.contains("README.md"),
        "non-matching tile shown:\n{text}"
    );
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
fn palette_header_cell_is_primary_on_surface() {
    let (mut state, _) = loaded(120, 36);
    let terminal = rendered_terminal(&mut state, 120, 36);
    let buffer = terminal.backend().buffer();
    assert!(
        cells_matching(buffer, |cell| {
            cell.fg == TEXT_PRIMARY && cell.bg == SURFACE_2 && !cell.symbol().trim().is_empty()
        }) > 0
    );
}

#[test]
fn palette_constants_are_pinned_to_the_default_theme() {
    // The palette constants are frozen copies of theme 0, so every color
    // assertion in this file is only meaningful while theme 0 is active.
    pin_default_theme();
    assert_eq!(theme::current().accent, ACCENT);
    assert_eq!(theme::current().surface_2, SURFACE_2);
    assert_eq!(theme::current().surface_3, SURFACE_3);
    assert_eq!(theme::current().text_primary, TEXT_PRIMARY);
    assert_eq!(theme::current().border_subtle, BORDER_SUBTLE);
    assert_eq!(theme::current().border_strong, BORDER_STRONG);
    assert_eq!(theme::current().accent_soft, ACCENT_SOFT);
    assert_eq!(theme::current().danger, DANGER);
    assert_eq!(theme::current().selected_bg, SELECTED_BG);
    assert_eq!(theme::current().focus_bg, FOCUS_BG);
}

#[test]
fn palette_tiles_expose_focus_and_selection_fills() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::ToggleSelect, Action::MoveDown],
    );
    let terminal = rendered_terminal(&mut state, 120, 36);
    let buffer = terminal.backend().buffer();
    // Selected tiles keep the selection fill; the bare navigation cursor
    // is a strong border only — it must NOT paint a focus fill (the old
    // cursor-looks-selected bug), so FOCUS_BG never appears here.
    assert!(cells_matching(buffer, |cell| cell.bg == SELECTED_BG) > 0);
    assert!(
        cells_matching(buffer, |cell| cell.fg == BORDER_STRONG) > 0,
        "cursor-only tile must draw its BORDER_STRONG frame"
    );
    assert_eq!(
        cells_matching(buffer, |cell| cell.bg == FOCUS_BG),
        0,
        "cursor-as-selection fill must be gone"
    );
    assert!(cells_matching(buffer, |cell| cell.fg == ACCENT) > 0);
}

#[test]
fn palette_tag_text_uses_soft_accent() {
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        command_actions("tag fav")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let terminal = rendered_terminal(&mut state, 120, 36);
    assert!(cells_matching(terminal.backend().buffer(), |cell| cell.fg == ACCENT_SOFT) > 0);
}

#[test]
fn palette_error_has_literal_marker_and_danger_bold_style() {
    let (mut state, mut handler) = loaded(80, 24);
    drive(
        &mut state,
        &mut handler,
        command_actions("bogus")
            .into_iter()
            .chain([Action::CommandSubmit])
            .collect::<Vec<_>>(),
    );
    let terminal = rendered_terminal(&mut state, 80, 24);
    let buffer = terminal.backend().buffer();
    let text = buffer_text(&terminal);
    assert!(text.contains("[!]"));
    assert!(
        cells_matching(buffer, |cell| {
            cell.fg == DANGER && cell.modifier.contains(Modifier::BOLD)
        }) > 0
    );
}

#[test]
fn palette_overlay_uses_strong_frame_and_surface_interior() {
    let (mut state, mut handler) = loaded(80, 24);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    let terminal = rendered_terminal(&mut state, 80, 24);
    let buffer = terminal.backend().buffer();
    assert!(cells_matching(buffer, |cell| cell.fg == BORDER_STRONG) > 0);
    assert!(cells_matching(buffer, |cell| cell.bg == SURFACE_3) > 0);
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
            ctrl: false,
        }],
    );
    assert!(matches!(state.mode, Mode::ContextMenu(_)));
    let text = render(&mut state, 120, 36);
    assert_snapshot("context-120x36", &text);
}

#[test]
fn bookmark_modal() {
    let (mut state, mut handler) = loaded(120, 36);
    state.bookmarks = vec![
        PathBuf::from("/home/demo/docs"),
        PathBuf::from("/home/demo/src"),
        PathBuf::from("/var/log"),
    ];
    drive(&mut state, &mut handler, [Action::OpenBookmarks]);
    drive(&mut state, &mut handler, [Action::BookmarkChar('d')]);
    let text = render(&mut state, 120, 36);
    assert_snapshot("bookmarks-120x36", &text);
}

#[test]
fn bookmark_modal_empty() {
    let (mut state, mut handler) = loaded(80, 24);
    state.bookmarks.clear();
    drive(&mut state, &mut handler, [Action::OpenBookmarks]);
    let text = render(&mut state, 80, 24);
    assert_snapshot("bookmarks-empty-80x24", &text);
}

#[test]
fn bookmark_modal_renders_at_every_size() {
    for (w, h) in SIZES {
        let (mut state, mut handler) = loaded(*w, *h);
        state.bookmarks = vec![
            PathBuf::from("/home/demo/docs"),
            PathBuf::from("/home/demo/src"),
            PathBuf::from("/var/log"),
            PathBuf::from("/etc"),
        ];
        drive(&mut state, &mut handler, [Action::OpenBookmarks]);
        for c in "do".chars() {
            drive(&mut state, &mut handler, [Action::BookmarkChar(c)]);
        }
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
    }
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
        let expected_breadcrumbs = if ui::tier_for(*w, *h) == ui::Tier::Narrow {
            0
        } else {
            tui_explorer::app::reduce::breadcrumb_segments(&state.browser.cwd).len()
        };
        assert_eq!(
            breadcrumb_hits, expected_breadcrumbs,
            "breadcrumb hits diverge from rendered segments at {w}x{h}"
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
            ctrl: false,
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
            ctrl: false,
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
            ctrl: false,
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
            ctrl: false,
        }],
    );
    // Narrow mode is a one-column list, so one scroll tick advances one row.
    assert_eq!(state.browser.selected, 1);
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
fn image_preview_reports_protocol_and_keeps_content_inside_frame() {
    let (mut state, _) = loaded(160, 48);
    state.preview.content = Some(PreviewContent::Image(Box::new(
        state
            .picker
            .new_resize_protocol(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                8,
                6,
                image::Rgb([200, 30, 30]),
            ))),
    )));
    let terminal = rendered_terminal(&mut state, 160, 48);
    let text = buffer_text(&terminal);
    assert!(text.contains("Preview (Halfblocks)"));
    let buffer = terminal.backend().buffer();
    let frame_origin = (0..buffer.area.height)
        .flat_map(|y| (124..buffer.area.width).map(move |x| (x, y)))
        .find(|&(x, y)| {
            let cell = &buffer[(x, y)];
            cell.symbol() == "+" && cell.fg == BORDER_SUBTLE
        })
        .expect("preview content frame");
    let image_cell = (0..buffer.area.height)
        .flat_map(|y| (124..buffer.area.width).map(move |x| (x, y)))
        .find(|&(x, y)| buffer[(x, y)].symbol() == "▀")
        .expect("half-block image cell");
    assert!(image_cell.0 > frame_origin.0);
    assert!(image_cell.1 > frame_origin.1);
}

#[test]
fn image_preview_rejects_stale_result_and_reuses_protocol_across_resize() {
    let (mut state, _) = loaded(160, 48);
    let key = state.focused_preview_key().expect("focused preview key");
    reduce(
        &mut state,
        Action::PreviewLoaded {
            key: key.clone(),
            result: PreviewLoaded::Image(image::DynamicImage::ImageRgb8(
                image::RgbImage::from_pixel(8, 6, image::Rgb([30, 200, 30])),
            )),
        },
    );
    let pointer_before = match &state.preview.content {
        Some(PreviewContent::Image(protocol)) => protocol.as_ref() as *const _,
        other => panic!("expected image preview, got {other:?}"),
    };
    let _ = render(&mut state, 160, 48);
    let _ = render(&mut state, 200, 60);
    let pointer_after = match &state.preview.content {
        Some(PreviewContent::Image(protocol)) => protocol.as_ref() as *const _,
        other => panic!("expected image preview after resize, got {other:?}"),
    };
    assert_eq!(pointer_before, pointer_after);

    reduce(
        &mut state,
        Action::PreviewLoaded {
            key: (PathBuf::from("/stale"), 0, 0),
            result: PreviewLoaded::Text {
                lines: vec!["stale".to_string()],
                truncated: false,
            },
        },
    );
    assert!(matches!(
        &state.preview.content,
        Some(PreviewContent::Image(_))
    ));

    reduce(
        &mut state,
        Action::PreviewLoaded {
            key,
            result: PreviewLoaded::Text {
                lines: vec!["replacement text".to_string()],
                truncated: false,
            },
        },
    );
    let text = render(&mut state, 160, 48);
    assert!(text.contains("replacement text"));
    assert!(!text.contains('▀'));
}

#[test]
fn hiding_preview_discards_protocol_state() {
    let (mut state, _) = loaded(160, 48);
    state.preview.key = state.focused_preview_key();
    state.preview.content = Some(PreviewContent::Image(Box::new(
        state
            .picker
            .new_resize_protocol(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                8,
                6,
                image::Rgb([30, 30, 200]),
            ))),
    )));
    state.show_preview = Some(false);
    let text = render(&mut state, 160, 48);
    assert!(!text.contains("Preview ("));
    assert!(state.preview.key.is_none());
    assert!(state.preview.content.is_none());
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

// --- media modal feedback driven by backend-confirmed state ---

fn audio_media_state(
    phase: tui_explorer::media::MediaPhase,
) -> tui_explorer::app::state::MediaState {
    let mut media = tui_explorer::app::state::MediaState::preparing(
        1,
        PathBuf::from("/home/demo/song.mp3"),
        tui_explorer::media::MediaKind::Audio,
    );
    media.phase = phase;
    media.position = 12.0;
    media.duration = Some(180.0);
    media.volume = 70;
    media.spectrum = [0.5; 24];
    media
}

fn video_media_state(
    phase: tui_explorer::media::MediaPhase,
) -> tui_explorer::app::state::MediaState {
    let mut media = tui_explorer::app::state::MediaState::preparing(
        1,
        PathBuf::from("/home/demo/clip.mkv"),
        tui_explorer::media::MediaKind::Video,
    );
    media.phase = phase;
    media.position = 4.0;
    media.duration = Some(30.0);
    media
}

#[test]
fn media_toggle_control_tracks_real_phase() {
    // While the backend reports Playing the control offers PAUSE.
    // Buttons render as bordered widgets, so the label appears surrounded
    // by border glyphs rather than bracketed text; note PLAY is a
    // substring of PAUSE, hence the exact-token comparison over
    // non-alphanumeric boundaries.
    let has_token = |text: &str, token: &str| {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == token)
    };
    let (mut state, _) = loaded(120, 36);
    state.mode = Mode::Media(Box::new(audio_media_state(
        tui_explorer::media::MediaPhase::Playing,
    )));
    let text = render(&mut state, 120, 36);
    assert!(
        has_token(&text, "PAUSE"),
        "playing modal must offer PAUSE: {text}"
    );
    assert!(
        !has_token(&text, "PLAY"),
        "stale PLAY label must not remain: {text}"
    );

    // While paused it offers PLAY again.
    let (mut state, _) = loaded(120, 36);
    state.mode = Mode::Media(Box::new(audio_media_state(
        tui_explorer::media::MediaPhase::Paused,
    )));
    let text = render(&mut state, 120, 36);
    assert!(has_token(&text, "PLAY"), "paused modal must offer PLAY");
}

#[test]
fn video_modal_never_draws_spectrum_over_the_surface() {
    // Live video: no spectrum bars anywhere in the modal.
    let (mut state, _) = loaded(160, 48);
    state.mode = Mode::Media(Box::new(video_media_state(
        tui_explorer::media::MediaPhase::Playing,
    )));
    let terminal = rendered_terminal(&mut state, 160, 48);
    let text = buffer_text(&terminal);
    assert!(text.contains("VIDEO"));
    let bars = cells_matching(terminal.backend().buffer(), |cell| {
        cell.symbol() == "#" && cell.fg == tui_explorer::ui::palette::ACCENT_SOFT
    });
    assert_eq!(bars, 0, "spectrum bars would fight live video frames");

    // Startup shows placeholder chrome instead of a black void.
    let (mut state, _) = loaded(160, 48);
    state.mode = Mode::Media(Box::new(video_media_state(
        tui_explorer::media::MediaPhase::Starting,
    )));
    let text = render(&mut state, 160, 48);
    assert!(
        text.contains("loading video"),
        "startup needs a placeholder"
    );
}

#[test]
fn marquee_band_renders_accent_outline_without_fill() {
    use tui_explorer::app::state::{MarqueePhase, MarqueeState};

    let (mut state, _) = loaded(120, 36);
    state.marquee = Some(MarqueeState {
        phase: MarqueePhase::Selecting,
        origin: (30, 6),
        current: (60, 20),
        base: std::collections::BTreeSet::new(),
    });
    let terminal = rendered_terminal(&mut state, 120, 36);
    let buffer = terminal.backend().buffer();
    // Outline corners land on band edges; interior stays untouched.
    let accent_cells = cells_matching(buffer, |cell| {
        cell.fg == tui_explorer::ui::palette::ACCENT_SOFT
    });
    assert!(
        accent_cells >= 2 * (31 + 15),
        "outline missing: {accent_cells}"
    );
}

// --- runtime theme selection ---

/// Palette constants are frozen copies of theme 0; pin the active theme so
/// these assertions can never drift onto a different color scheme.
fn pin_default_theme() {
    theme::set_current(0);
    assert_eq!(theme::current_index(), 0, "default theme must be index 0");
}

/// Distinct explicitly-colored foregrounds in `buffer`, ignoring unset cells.
fn distinct_foregrounds(buffer: &Buffer) -> HashSet<Color> {
    let area = buffer.area;
    let mut out = HashSet::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let fg = buffer[(x, y)].fg;
            if fg != Color::default() {
                out.insert(fg);
            }
        }
    }
    out
}

/// Draws one frame, turning a backend error into a message naming the theme.
fn themed_render(
    state: &mut AppState,
    width: u16,
    height: u16,
    label: &str,
) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .unwrap_or_else(|e| panic!("{label} failed to render at {width}x{height}: {e}"));
    terminal
}

/// Fails if any cell needs truecolor, which would break 256-color terminals.
fn assert_no_truecolor(terminal: &Terminal<TestBackend>, label: &str, width: u16, height: u16) {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            assert!(
                !matches!(cell.fg, Color::Rgb(..)),
                "{label} foreground needs truecolor at ({x},{y}) of {width}x{height}: {:?}",
                cell.fg
            );
            assert!(
                !matches!(cell.bg, Color::Rgb(..)),
                "{label} background needs truecolor at ({x},{y}) of {width}x{height}: {:?}",
                cell.bg
            );
        }
    }
}

#[test]
fn all_themes_render_every_surface() {
    assert!(theme::count() >= 16, "themes were expected to be added");
    for index in 0..theme::count() {
        theme::set_current(index);
        assert_eq!(
            theme::current_index(),
            index,
            "set_current must accept every advertised index"
        );
        let names = theme::names();
        let label = format!("theme {index} ({})", names[index % names.len()]);

        for (w, h) in [(120, 36), (70, 22)] {
            let (mut state, _) = loaded(w, h);
            let terminal = themed_render(&mut state, w, h, &label);
            let text = buffer_text(&terminal);
            assert!(
                !text.trim().is_empty(),
                "{label} rendered an empty frame at {w}x{h}"
            );
        }

        // The help overlay is the modal surface: it fills a centered block
        // with its own background and frame colors.
        let (mut state, mut handler) = loaded(120, 36);
        drive(&mut state, &mut handler, [Action::ToggleHelp]);
        assert!(
            matches!(state.mode, Mode::Help),
            "{label} help overlay did not open"
        );
        let terminal = themed_render(&mut state, 120, 36, &label);
        let text = buffer_text(&terminal);
        assert!(
            text.contains("HELP"),
            "{label} help overlay missing its title:\n{text}"
        );
    }
    pin_default_theme();
}

#[test]
fn themed_output_is_256_color_only() {
    let index = theme::count() - 1;
    theme::set_current(index);
    assert_eq!(theme::current_index(), index);
    let label = format!("theme {index}");

    // Every theme surface, including the preview panel and the help modal,
    // must stay inside the xterm-256 palette.
    for (w, h) in [(120, 36), (160, 48), (70, 22)] {
        let (mut state, _) = loaded(w, h);
        let terminal = themed_render(&mut state, w, h, &label);
        assert_no_truecolor(&terminal, &label, w, h);
    }

    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    let terminal = themed_render(&mut state, 120, 36, &label);
    assert_no_truecolor(&terminal, &label, 120, 36);

    // The 16-slot palette of the active theme is itself fully indexed.
    let colors = theme::current();
    for color in [
        colors.surface_0,
        colors.surface_1,
        colors.surface_2,
        colors.surface_3,
        colors.border_subtle,
        colors.border_strong,
        colors.text_primary,
        colors.text_secondary,
        colors.text_muted,
        colors.accent,
        colors.accent_hover,
        colors.accent_soft,
        colors.danger,
        colors.selected_bg,
        colors.focus_bg,
        colors.ink,
    ] {
        assert!(
            matches!(color, Color::Indexed(_)),
            "{color:?} is not indexed"
        );
    }
    pin_default_theme();
}

#[test]
fn theme_changes_actually_change_rendering() {
    let render_fg = |index: usize| {
        theme::set_current(index);
        assert_eq!(theme::current_index(), index);
        let (mut state, _) = loaded(120, 36);
        let terminal = themed_render(&mut state, 120, 36, "theme comparison");
        distinct_foregrounds(terminal.backend().buffer())
    };

    let base = render_fg(0);
    let other = render_fg(7);
    assert!(
        !base.is_empty() && !other.is_empty(),
        "expected colored foregrounds in both frames"
    );
    assert_ne!(
        base, other,
        "theme 0 and theme 7 rendered identical foregrounds; colors are frozen constants"
    );
    let shared: Vec<Color> = base.intersection(&other).copied().collect();
    assert!(
        !shared.is_empty(),
        "the two themes share no foreground at all, so the comparison proves nothing: {shared:?}"
    );
    pin_default_theme();
}
