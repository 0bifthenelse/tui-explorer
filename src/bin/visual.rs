//! Headless visual harness: renders the redesigned interface at the four
//! release terminal sizes plus interactive scenarios into text and SVG
//! frames under docs/screenshots/visual/. No display server needed.
//!
//! Usage: cargo run --bin visual-dump

use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use tui_explorer::app::action::Action;
use tui_explorer::app::state::AppState;
use tui_explorer::testing::builders::{demo_fs, demo_state};
use tui_explorer::testing::svg::buffer_to_svg;
use tui_explorer::testing::{SyncHandler, drive};
use tui_explorer::ui;

const SIZES: &[(u16, u16)] = &[(160, 48), (120, 36), (90, 28), (70, 22)];

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

fn render(state: &mut AppState, width: u16, height: u16) -> (String, String) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    let svg = buffer_to_svg(terminal.backend().buffer());
    (buffer_text(&terminal), svg)
}

fn loaded(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

fn emit(out: &Path, name: &str, text: &str, svg: &str) {
    std::fs::write(out.join(format!("{name}.txt")), text).expect("write txt");
    std::fs::write(out.join(format!("{name}.svg")), svg).expect("write svg");
    println!("wrote {name}");
}

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("screenshots")
        .join("visual");
    std::fs::create_dir_all(&out).expect("create output dir");

    // Main grid at all four release sizes.
    for (w, h) in SIZES {
        let (mut state, _) = loaded(*w, *h);
        let (text, svg) = render(&mut state, *w, *h);
        emit(&out, &format!("main-{w}x{h}"), &text, &svg);
    }

    // Selection + tags.
    let (mut state, mut handler) = loaded(160, 48);
    drive(
        &mut state,
        &mut handler,
        [Action::ToggleSelect, Action::ToggleSelect, Action::MoveDown],
    );
    let mut actions = vec![Action::EnterCommand];
    for c in "tag fav".chars() {
        actions.push(Action::CommandChar(c));
    }
    actions.push(Action::CommandSubmit);
    drive(&mut state, &mut handler, actions);
    let (text, svg) = render(&mut state, 160, 48);
    emit(&out, "tagged-160x48", &text, &svg);

    // Password dialog (masked).
    let (mut state, mut handler) = loaded(120, 36);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::EncryptToggle],
    );
    drive(
        &mut state,
        &mut handler,
        [
            Action::PasswordChar('h'),
            Action::PasswordChar('u'),
            Action::PasswordChar('n'),
            Action::PasswordChar('t'),
        ],
    );
    let (text, svg) = render(&mut state, 120, 36);
    emit(&out, "password-120x36", &text, &svg);

    // Help overlay.
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    let (text, svg) = render(&mut state, 120, 36);
    emit(&out, "help-120x36", &text, &svg);

    println!("done -> {}", out.display());
}
