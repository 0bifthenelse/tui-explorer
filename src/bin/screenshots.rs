use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use tui_explorer::app::action::Action;
use tui_explorer::app::state::AppState;
use tui_explorer::testing::builders::{FIXED_TIME, demo_fs, demo_state};
use tui_explorer::testing::svg::buffer_to_svg;
use tui_explorer::testing::{SyncHandler, drive};
use tui_explorer::ui;

fn render_state(state: &mut AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    buffer_to_svg(terminal.backend().buffer())
}

fn base_state(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    handler
        .tags
        .tag_paths(
            &[
                PathBuf::from("/home/demo/src"),
                PathBuf::from("/home/demo/main.rs"),
            ],
            "src",
            FIXED_TIME,
        )
        .expect("tag src");
    handler
        .tags
        .tag_paths(&[PathBuf::from("/home/demo/main.rs")], "fav", FIXED_TIME)
        .expect("tag fav");
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

fn shot_main() -> String {
    let (mut state, mut handler) = base_state(120, 36);
    drive(
        &mut state,
        &mut handler,
        [
            Action::ToggleSelect,
            Action::ToggleSelect,
            Action::MoveDown,
            Action::MoveDown,
            Action::MoveDown,
        ],
    );
    render_state(&mut state, 120, 36)
}

fn shot_tags() -> String {
    let (mut state, mut handler) = base_state(120, 36);
    drive(&mut state, &mut handler, [Action::OpenTagPicker]);
    render_state(&mut state, 120, 36)
}

fn shot_command() -> String {
    let (mut state, mut handler) = base_state(120, 36);
    let mut actions = vec![Action::EnterCommand];
    for c in "copy \"/mnt/backup drive\"".chars() {
        actions.push(Action::CommandChar(c));
    }
    drive(&mut state, &mut handler, actions);
    render_state(&mut state, 120, 36)
}

fn shot_compact() -> String {
    let (mut state, _handler) = base_state(60, 16);
    render_state(&mut state, 60, 16)
}

fn shot_help() -> String {
    let (mut state, mut handler) = base_state(120, 36);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    render_state(&mut state, 120, 36)
}

type ShotFn = fn() -> String;

fn main() -> std::io::Result<()> {
    let out_dir = std::path::Path::new("docs/screenshots");
    std::fs::create_dir_all(out_dir)?;
    let shots: &[(&str, ShotFn)] = &[
        ("main.svg", shot_main),
        ("tags.svg", shot_tags),
        ("command-mode.svg", shot_command),
        ("compact.svg", shot_compact),
        ("help.svg", shot_help),
    ];
    for (name, build) in shots {
        let svg = build();
        std::fs::write(out_dir.join(name), svg)?;
        println!("wrote docs/screenshots/{name}");
    }
    Ok(())
}
