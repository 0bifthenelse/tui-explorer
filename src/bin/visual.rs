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

    // Marquee band mid-drag over the icon grid.
    let (mut state, _) = loaded(120, 36);
    state.marquee = Some(tui_explorer::app::state::MarqueeState {
        phase: tui_explorer::app::state::MarqueePhase::Selecting,
        origin: (34, 8),
        current: (72, 24),
        base: std::collections::BTreeSet::new(),
    });
    let (text, svg) = render(&mut state, 120, 36);
    emit(&out, "marquee-120x36", &text, &svg);

    // Context menu with the pointer hovering the Delete row.
    let (mut state, mut handler) = loaded(120, 36);
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    {
        let backend = ratatui::backend::TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| ui::render(frame, &mut state))
            .expect("render");
    }
    let rect = state
        .hit_map
        .rect_for(tui_explorer::ui::hit::HitTarget::Row(0))
        .expect("row region");
    drive(
        &mut state,
        &mut handler,
        [Action::Mouse {
            kind: tui_explorer::app::action::MouseKind::Right,
            x: rect.x + 2,
            y: rect.y + 1,
            ctrl: false,
        }],
    );
    // Re-render to build the menu hit regions, then hover item 4.
    {
        let backend = ratatui::backend::TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| ui::render(frame, &mut state))
            .expect("render");
    }
    if let Some(item) = state
        .hit_map
        .rect_for(tui_explorer::ui::hit::HitTarget::ContextItem(4))
    {
        drive(
            &mut state,
            &mut handler,
            [Action::Mouse {
                kind: tui_explorer::app::action::MouseKind::Moved,
                x: item.x + 1,
                y: item.y,
                ctrl: false,
            }],
        );
    }
    let (text, svg) = render(&mut state, 120, 36);
    emit(&out, "context-hover-120x36", &text, &svg);

    // Media modals: audio playing with live FFT, video paused on a frame.
    let audio_scene = |phase| {
        let mut media = tui_explorer::app::state::MediaState::preparing(
            1,
            PathBuf::from("/home/demo/song.mp3"),
            tui_explorer::media::MediaKind::Audio,
        );
        media.phase = phase;
        media.position = 42.0;
        media.duration = Some(184.0);
        media.volume = 65;
        media.spectrum = [
            0.15, 0.5, 0.9, 0.7, 0.35, 0.6, 0.85, 0.45, 0.25, 0.55, 0.75, 0.3, 0.4, 0.65, 0.2, 0.5,
            0.8, 0.6, 0.35, 0.45, 0.7, 0.25, 0.55, 0.3,
        ];
        media
    };
    let (mut state, _) = loaded(120, 36);
    state.mode = tui_explorer::app::state::Mode::Media(Box::new(audio_scene(
        tui_explorer::media::MediaPhase::Playing,
    )));
    let (text, svg) = render(&mut state, 120, 36);
    emit(&out, "media-audio-playing-120x36", &text, &svg);

    let video_scene = |phase| {
        let mut media = tui_explorer::app::state::MediaState::preparing(
            1,
            PathBuf::from("/home/demo/clip.mkv"),
            tui_explorer::media::MediaKind::Video,
        );
        media.phase = phase;
        media.position = 17.5;
        media.duration = Some(96.0);
        media.volume = 80;
        media
    };
    let (mut state, _) = loaded(120, 36);
    state.mode = tui_explorer::app::state::Mode::Media(Box::new(video_scene(
        tui_explorer::media::MediaPhase::Paused,
    )));
    let (text, svg) = render(&mut state, 120, 36);
    emit(&out, "media-video-paused-120x36", &text, &svg);

    println!("done -> {}", out.display());
}
