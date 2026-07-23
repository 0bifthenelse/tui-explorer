// quick visual dump
fn main() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tui_explorer::app::action::Action;
    use tui_explorer::testing::builders::{demo_fs, demo_state};
    use tui_explorer::testing::{SyncHandler, drive};
    let (w, h): (u16, u16) = (
        std::env::args().nth(1).unwrap().parse().unwrap(),
        std::env::args().nth(2).unwrap().parse().unwrap(),
    );
    let mut state = demo_state(w, h);
    let mut handler = SyncHandler::new(demo_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| tui_explorer::ui::render(frame, &mut state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push_str(buffer[(x, y)].symbol());
        }
        println!("{}", line.trim_end());
    }
}
