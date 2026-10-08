//! Renders one named UI scenario to an SVG frame for visual review.
//!
//! cargo run --example shot -- <scenario> <width> <height> <out.svg>
//!
//! Scenarios: list grid grid-small columns selected help confirm context
//! command audio video bookmarks picker empty hub results grep quicklook
//! openwith whichkey suggest rename search conflict

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tui_explorer::app::action::{Action, MouseKind};
use tui_explorer::app::state::Mode;
use tui_explorer::settings::ViewMode;
use tui_explorer::testing::builders::{demo_fs_showcase, demo_fs_with_video, demo_state};
use tui_explorer::testing::svg::buffer_to_svg;
use tui_explorer::testing::{SyncHandler, drive};
use tui_explorer::ui::hit::HitTarget;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("list");
    let w: u16 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(160);
    let h: u16 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(48);
    let out = args.get(4).cloned().unwrap_or_else(|| "shot.svg".into());

    let fs = if matches!(scenario, "audio" | "video" | "mini" | "subs" | "queue") {
        demo_fs_with_video()
    } else {
        demo_fs_showcase()
    };
    let mut state = demo_state(w, h);
    let mut fs = fs;
    if scenario == "columns" {
        // A believable parent folder for the Miller parent pane.
        let home = std::path::PathBuf::from("/home");
        for name in ["alex", "demo", "guest", "shared"] {
            fs.add_entry(
                &home,
                tui_explorer::filesystem::DirEntry::synthetic(&home, name, true),
            );
        }
    }
    let mut handler = SyncHandler::new(fs);
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    let render = |state: &mut tui_explorer::app::state::AppState| {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| tui_explorer::ui::render(f, state))
            .unwrap();
        terminal.backend().buffer().clone()
    };
    match scenario {
        "grid" => drive(&mut state, &mut handler, [Action::SetView(ViewMode::Grid)]),
        "grid-small" => drive(&mut state, &mut handler, [Action::GridZoom(false)]),
        "columns" => {
            drive(
                &mut state,
                &mut handler,
                [Action::SetView(ViewMode::Columns)],
            );
            for _ in 0..6 {
                drive(&mut state, &mut handler, [Action::MoveDown]);
            }
            if let Some(key) = state.focused_preview_key() {
                drive(
                    &mut state,
                    &mut handler,
                    [Action::PreviewLoaded {
                        key,
                        result: tui_explorer::preview::PreviewLoaded::Directory(
                            [
                                "components/",
                                "hooks/",
                                "app.rs",
                                "main.rs",
                                "lib.rs",
                                "config.toml",
                                "README.md",
                                "build.rs",
                            ]
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                        ),
                    }],
                );
            }
        }
        "selected" => {
            drive(
                &mut state,
                &mut handler,
                [
                    Action::MoveDown,
                    Action::ToggleSelect,
                    Action::ToggleSelect,
                    Action::MoveDown,
                    Action::MoveDown,
                ],
            );
            render(&mut state);
            if let Some(rect) = state.hit_map.rect_for(HitTarget::Row(7)) {
                drive(
                    &mut state,
                    &mut handler,
                    [Action::Mouse {
                        kind: MouseKind::Moved,
                        x: rect.x + 4,
                        y: rect.y,
                        ctrl: false,
                    }],
                );
            }
        }
        "help" => drive(&mut state, &mut handler, [Action::ToggleHelp]),
        "confirm" => {
            let mut actions = vec![Action::EnterCommand];
            actions.extend("delete".chars().map(Action::CommandChar));
            actions.push(Action::CommandSubmit);
            drive(&mut state, &mut handler, actions);
        }
        "context" => {
            render(&mut state);
            if let Some(rect) = state.hit_map.rect_for(HitTarget::Row(4)) {
                drive(
                    &mut state,
                    &mut handler,
                    [Action::Mouse {
                        kind: MouseKind::Right,
                        x: rect.x + 10,
                        y: rect.y,
                        ctrl: false,
                    }],
                );
            }
        }
        "command" => {
            let mut actions = vec![Action::EnterCommand];
            actions.extend("sort size-desc".chars().map(Action::CommandChar));
            drive(&mut state, &mut handler, actions);
        }
        "bookmarks" => {
            state.bookmarks = vec![
                "/home/demo/src".into(),
                "/home/demo/docs".into(),
                "/var/log".into(),
            ];
            drive(&mut state, &mut handler, [Action::OpenBookmarks]);
        }
        "hub" => {
            state.bookmarks = vec!["/home/demo/src".into(), "/home/demo/docs".into()];
            state.links = vec![
                tui_explorer::urls::Link {
                    title: "Rust docs".into(),
                    url: "https://doc.rust-lang.org/std/".into(),
                },
                tui_explorer::urls::Link {
                    title: "ratatui".into(),
                    url: "https://ratatui.rs".into(),
                },
            ];
            state
                .settings
                .marks
                .insert("a".into(), "/home/demo/src".into());
            drive(&mut state, &mut handler, [Action::OpenBookmarks]);
            drive(
                &mut state,
                &mut handler,
                "ru".chars().map(Action::BookmarkChar),
            );
        }
        "results" => {
            let mut actions = vec![Action::EnterCommand];
            actions.extend("find *.rs".chars().map(Action::CommandChar));
            actions.push(Action::CommandSubmit);
            drive(&mut state, &mut handler, actions);
        }
        "grep" => {
            for path in ["/home/demo/main.rs", "/home/demo/notes.md"] {
                handler.contents.insert(
                    path.into(),
                    "fn main() {\n    // TODO: wire the needle\n    let needle = 42;\n}".into(),
                );
            }
            let mut actions = vec![Action::EnterCommand];
            actions.extend("grep needle".chars().map(Action::CommandChar));
            actions.push(Action::CommandSubmit);
            drive(&mut state, &mut handler, actions);
        }
        "quicklook" => {
            let pos = state
                .browser
                .visible_entries()
                .position(|(_, e)| e.entry.display_name() == "main.rs")
                .unwrap_or(0);
            state.browser.selected = pos;
            drive(&mut state, &mut handler, [Action::QuickLook]);
        }
        "openwith" => {
            let pos = state
                .browser
                .visible_entries()
                .position(|(_, e)| e.entry.display_name().ends_with(".pdf"))
                .unwrap_or(0);
            state.browser.selected = pos;
            drive(&mut state, &mut handler, [Action::OpenWithPrompt]);
            if let Mode::OpenWith(o) = &mut state.mode {
                o.suggestions = vec!["zathura".into(), "evince".into(), "xdg-open".into()];
                o.suggestion = Some(0);
                o.input = "zathura".into();
            }
        }
        "address" => {
            drive(&mut state, &mut handler, [Action::OpenAddressBar]);
            drive(
                &mut state,
                &mut handler,
                "s".chars().map(Action::CommandChar),
            );
        }
        "mini" | "queue" => {
            let pos = state
                .browser
                .visible_entries()
                .position(|(_, e)| e.entry.display_name() == "track02.mp3")
                .unwrap_or(0);
            state.browser.selected = pos;
            drive(&mut state, &mut handler, [Action::OpenFocused]);
            render(&mut state);
            if let Mode::Media(media) = &mut state.mode {
                media.phase = tui_explorer::media::MediaPhase::Playing;
                media.position = 71.0;
                media.duration = Some(214.0);
                media.shuffle = true;
                media.repeat = tui_explorer::app::state::Repeat::All;
                for (i, band) in media.spectrum.iter_mut().enumerate() {
                    *band = ((i as f32 * 0.55).sin().abs() * 0.85 + 0.08).min(1.0);
                }
            }
            if let Mode::Media(media) = &mut state.mode {
                media.tags = Some(tui_explorer::media::tags::TrackTags {
                    title: Some("Neon Horizon".into()),
                    artist: Some("The Synthwave Collective".into()),
                    album: Some("Night Drive".into()),
                    year: Some("2024".into()),
                });
                let session = media.session;
                let cover =
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 64, |x, y| {
                        image::Rgb([255, (125 + x) as u8, (39 + y * 2) as u8])
                    }));
                state.cover = Some(tui_explorer::app::state::Cover {
                    session,
                    image: Box::new(state.picker.new_resize_protocol(cover)),
                });
            }
            if scenario == "mini" {
                drive(&mut state, &mut handler, [Action::MediaMinimize]);
            }
        }
        "subs" => {
            let pos = state
                .browser
                .visible_entries()
                .position(|(_, e)| e.entry.display_name() == "clip.mkv")
                .unwrap_or(0);
            state.browser.selected = pos;
            drive(&mut state, &mut handler, [Action::OpenFocused]);
            render(&mut state);
            if let Mode::Media(media) = &mut state.mode {
                media.phase = tui_explorer::media::MediaPhase::Paused;
                media.position = 12.0;
                media.duration = Some(95.0);
            }
            drive(&mut state, &mut handler, [Action::MediaOpenSubs]);
        }
        "whichkey" => drive(&mut state, &mut handler, [Action::ChordKey("g".into())]),
        "suggest" => {
            let mut actions = vec![Action::EnterCommand];
            actions.extend("s".chars().map(Action::CommandChar));
            drive(&mut state, &mut handler, actions);
        }
        "rename" => {
            drive(
                &mut state,
                &mut handler,
                [Action::MoveDown, Action::MoveDown, Action::MoveDown],
            );
            drive(
                &mut state,
                &mut handler,
                [Action::RenameStart(
                    tui_explorer::app::action::RenameCursor::BeforeExt,
                )],
            );
        }
        "search" => {
            drive(&mut state, &mut handler, [Action::EnterSearch]);
            drive(
                &mut state,
                &mut handler,
                "ar".chars()
                    .map(|c| Action::LineEdit(tui_explorer::input::line::Edit::Insert(c))),
            );
        }
        "conflict" => {
            state.browser.selected = 4;
            drive(&mut state, &mut handler, [Action::CopySelection]);
            drive(
                &mut state,
                &mut handler,
                [Action::PasteHere { overwrite: false }],
            );
        }
        "picker" => {
            let mut actions = vec![Action::EnterCommand];
            actions.extend("tag fav".chars().map(Action::CommandChar));
            actions.push(Action::CommandSubmit);
            actions.push(Action::OpenTagPicker);
            drive(&mut state, &mut handler, actions);
        }
        "audio" | "video" => {
            let name = if scenario == "audio" {
                "song.mp3"
            } else {
                "clip.mkv"
            };
            let pos = state
                .browser
                .visible_entries()
                .position(|(_, e)| e.entry.display_name() == name)
                .unwrap_or(0);
            state.browser.selected = pos;
            drive(&mut state, &mut handler, [Action::OpenFocused]);
            render(&mut state);
            if let Mode::Media(media) = &mut state.mode {
                media.phase = tui_explorer::media::MediaPhase::Playing;
                media.position = 42.0;
                media.duration = Some(184.0);
                media.volume = 65;
                for (i, band) in media.spectrum.iter_mut().enumerate() {
                    *band = ((i as f32 * 0.7).sin().abs() * 0.8 + 0.1).min(1.0);
                }
            }
        }
        _ => {}
    }
    let buffer = render(&mut state);
    std::fs::write(&out, buffer_to_svg(&buffer)).unwrap();
    // Also print the text so layout can be eyeballed in a terminal.
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push_str(buffer[(x, y)].symbol());
        }
        println!("{}", line.trim_end());
    }
}
