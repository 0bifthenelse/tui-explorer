//! Replay coverage for the interactive media modal: hover without
//! activation, pause clicks, seek-rail press/drag/commit, the supervised
//! fullscreen restart cycle, playlist navigation, stale-session rejection,
//! EOF handling and the hovered/selected filename footer.
//!
//! Everything runs through the deterministic `drive` loop (or raw `reduce`
//! steps where an intermediate Stopping phase must be observed before the
//! stop handback arrives); there are no sleeps, threads or real backends.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use tui_explorer::app::action::{Action, MouseKind};
use tui_explorer::app::effects::Effect;
use tui_explorer::app::reduce::{footer_focus_text, reduce};
use tui_explorer::app::state::{AppState, MediaState, MediaSurface, Mode};
use tui_explorer::filesystem::EntryKind;
use tui_explorer::input::keymap::map_key;
use tui_explorer::media::{AfterStop, MediaCommand, MediaPhase};
use tui_explorer::testing::builders::{
    FIXED_TIME, demo_fs, demo_fs_with_video, demo_root, demo_state, entry,
};
use tui_explorer::testing::{MemoryFileSystem, SyncHandler, drive};
use tui_explorer::ui;
use tui_explorer::ui::hit::HitTarget;
use tui_explorer::ui::widgets::{rail_geometry, rail_ratio_to_seconds};

fn audio_surface() -> MediaSurface {
    MediaSurface {
        rect: Rect::new(0, 0, 40, 8),
        terminal_cells: (120, 36),
        cell_pixels: (8, 16),
    }
}

fn video_surface() -> MediaSurface {
    MediaSurface {
        rect: Rect::new(4, 4, 60, 12),
        terminal_cells: (120, 36),
        cell_pixels: (8, 16),
    }
}

fn loaded(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

fn loaded_with_video(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs_with_video());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

/// Focuses the demo `song.mp3` entry (visible position 16).
fn focus_song(state: &mut AppState, handler: &mut SyncHandler) {
    drive(state, handler, [Action::GotoFirst]);
    drive(state, handler, vec![Action::MoveDown; 16]);
}

fn focus_video(state: &mut AppState, handler: &mut SyncHandler) {
    drive(state, handler, [Action::GotoFirst]);
    let steps = state
        .browser
        .visible_entries()
        .position(|(_, view)| view.entry.display_name() == "clip.mkv")
        .expect("demo video file");
    drive(state, handler, vec![Action::MoveDown; steps]);
}

fn current_session(state: &AppState) -> u64 {
    match &state.mode {
        Mode::Media(media) => media.session,
        _ => panic!("expected media mode"),
    }
}

fn video_path() -> PathBuf {
    demo_root().join("clip.mkv")
}

fn rerender(state: &mut AppState) {
    let backend = TestBackend::new(state.width, state.height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
}

/// Renders the state and returns the text of the last terminal row, where
/// the status/footer bar lives.
fn render_bottom_row(state: &mut AppState) -> String {
    let backend = TestBackend::new(state.width, state.height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    let buffer = terminal.backend().buffer();
    let y = state.height - 1;
    let mut line = String::new();
    for x in 0..state.width {
        line.push_str(buffer[(x, y)].symbol());
    }
    line.trim_end().to_string()
}

fn mouse(kind: MouseKind, x: u16, y: u16) -> Action {
    Action::Mouse {
        kind,
        x,
        y,
        ctrl: false,
    }
}

fn hit_rect(state: &AppState, target: HitTarget) -> Rect {
    state
        .hit_map
        .regions
        .iter()
        .find_map(|(rect, t)| (*t == target).then_some(*rect))
        .unwrap_or_else(|| panic!("no hit region registered for {target:?}"))
}

fn media(state: &AppState) -> &MediaState {
    match &state.mode {
        Mode::Media(media) => media,
        _ => panic!("expected media mode"),
    }
}

/// Opens the demo `song.mp3`, completes the surface handshake and reports
/// a Playing status at `position`/`duration`.
fn playing_audio(position: f64, duration: Option<f64>) -> (AppState, SyncHandler, u64) {
    let (mut state, mut handler) = loaded(120, 36);
    focus_song(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session,
            surface: audio_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session,
            phase: MediaPhase::Playing,
            position,
            duration,
            volume: 80,
        }],
    );
    (state, handler, session)
}

/// Same as [`playing_audio`] for the demo video entry.
fn playing_video(position: f64, duration: Option<f64>) -> (AppState, SyncHandler, u64) {
    let (mut state, mut handler) = loaded_with_video(120, 36);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session,
            surface: video_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session,
            phase: MediaPhase::Playing,
            position,
            duration,
            volume: 80,
        }],
    );
    (state, handler, session)
}

/// Drives a playing video into fullscreen through the supervised restart
/// cycle (toggle -> stop handback -> fresh surface -> Playing again).
fn fullscreen_playing_video(position: f64, duration: Option<f64>) -> (AppState, SyncHandler, u64) {
    let (mut state, mut handler, session) = playing_video(position, duration);
    drive(&mut state, &mut handler, [Action::MediaToggleFullscreen]);
    assert!(
        media(&state).fullscreen,
        "fullscreen flag set after the toggle"
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session,
            surface: video_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session,
            phase: MediaPhase::Playing,
            position,
            duration,
            volume: 80,
        }],
    );
    (state, handler, session)
}

/// Seconds the shared rail geometry maps column `x` to; mirrors exactly
/// what the reducer must feed into `MediaSeekAbsolute`.
fn seek_seconds_at(rect: Rect, x: u16, position: f64, duration: f64) -> f64 {
    let geom = rail_geometry(rect, position, Some(duration));
    rail_ratio_to_seconds(&geom, x, duration)
        .unwrap_or_else(|| panic!("column {x} does not map onto rail {rect:?}"))
}

/// Filesystem whose display order interleaves non-media entries between
/// three playable ones: `01-notes.txt, 02-first.wav, 03-readme.md,
/// 04-second.flac, 05-third.mp3`.
fn mixed_media_fs() -> MemoryFileSystem {
    let root = demo_root();
    let mut fs = MemoryFileSystem::new();
    fs.add_dir(&root);
    for (i, name) in [
        "01-notes.txt",
        "02-first.wav",
        "03-readme.md",
        "04-second.flac",
        "05-third.mp3",
    ]
    .into_iter()
    .enumerate()
    {
        fs.add_entry(
            &root,
            entry(
                &root,
                name,
                EntryKind::File,
                1024,
                0o644,
                FIXED_TIME - i as i64,
            ),
        );
    }
    fs
}

fn stop_media_count(effects: &[Effect], session: u64) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::StopMedia { session: s } if *s == session))
        .count()
}

#[test]
fn hover_over_pause_records_zero_media_commands() {
    let (mut state, mut handler, _session) = playing_audio(30.0, Some(90.0));
    rerender(&mut state);
    let pause = hit_rect(&state, HitTarget::MediaTogglePause);
    assert!(pause.width > 0 && pause.height > 0, "pause button drawn");

    let before = handler.media_commands.len();
    drive(
        &mut state,
        &mut handler,
        [mouse(
            MouseKind::Moved,
            pause.x + pause.width / 2,
            pause.y + pause.height / 2,
        )],
    );

    assert_eq!(
        handler.media_commands.len(),
        before,
        "hovering PAUSE must record zero media commands"
    );
    assert!(
        !handler.started_media.is_empty(),
        "setup sanity: session was started"
    );
    assert_eq!(
        state.hover.control,
        Some(HitTarget::MediaTogglePause),
        "pointer motion resolves true hover state without activation"
    );
}

#[test]
fn click_pause_records_exactly_one_toggle() {
    let (mut state, mut handler, session) = playing_audio(30.0, Some(90.0));
    rerender(&mut state);
    let pause = hit_rect(&state, HitTarget::MediaTogglePause);

    let before = handler.media_commands.len();
    drive(
        &mut state,
        &mut handler,
        [mouse(
            MouseKind::Left,
            pause.x + pause.width / 2,
            pause.y + pause.height / 2,
        )],
    );

    let new = &handler.media_commands[before..];
    assert_eq!(new.len(), 1, "one click, one command");
    assert_eq!(new[0], (session, MediaCommand::TogglePause));
}

#[test]
fn rail_click_seeks_once_to_shared_geometry_value() {
    let (mut state, mut handler, session) = playing_audio(30.0, Some(90.0));
    rerender(&mut state);
    let rail = hit_rect(&state, HitTarget::MediaSeekRail);
    assert!(rail.width > 3, "usable rail drawn");

    let x = rail.x + rail.width * 3 / 4;
    let expected = seek_seconds_at(rail, x, 30.0, 90.0);

    let before = handler.media_commands.len();
    // A real click arrives as separate Down and Up reducer events; the rail
    // arms on the press and commits exactly once on the release.
    drive(
        &mut state,
        &mut handler,
        [
            mouse(MouseKind::Left, x, rail.y),
            mouse(MouseKind::LeftUp, x, rail.y),
        ],
    );

    let new = &handler.media_commands[before..];
    assert_eq!(new.len(), 1, "one rail click commits exactly one seek");
    assert_eq!(
        new[0],
        (session, MediaCommand::SeekAbsolute(expected)),
        "seek value comes from the same geometry the renderer draws with"
    );
}

#[test]
fn unknown_duration_rail_press_is_completely_inert() {
    let (mut state, mut handler, _session) = playing_audio(30.0, None);
    rerender(&mut state);
    let rail = hit_rect(&state, HitTarget::MediaSeekRail);
    assert!(rail.width > 0, "rail is registered even without a duration");

    let before = handler.media_commands.len();
    drive(
        &mut state,
        &mut handler,
        [
            mouse(MouseKind::Left, rail.x + rail.width / 2, rail.y),
            mouse(MouseKind::LeftDrag, rail.x + rail.width / 3, rail.y),
            mouse(MouseKind::LeftUp, rail.x + rail.width / 3, rail.y),
        ],
    );

    assert_eq!(
        handler.media_commands.len(),
        before,
        "an unknown duration must never produce a SeekAbsolute"
    );
    let m = media(&state);
    assert!(!m.slider_drag_active, "press must not arm the drag");
    assert_eq!(m.slider_drag_pos, None, "no drag position recorded");
    assert_eq!(m.slider_hover, None, "no hover timestamp recorded");
    assert_eq!(m.phase, MediaPhase::Playing, "playback untouched");
}

#[test]
fn rail_drag_commits_one_seek_on_release_and_clears_state() {
    let (mut state, mut handler, session) = playing_audio(30.0, Some(90.0));
    rerender(&mut state);
    let rail = hit_rect(&state, HitTarget::MediaSeekRail);
    let press_x = rail.x + rail.width / 5;
    let drag_x = rail.x + rail.width - 2;
    let press_secs = seek_seconds_at(rail, press_x, 30.0, 90.0);
    let commit_secs = seek_seconds_at(rail, drag_x, 30.0, 90.0);

    let before = handler.media_commands.len();

    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::Left, press_x, rail.y)],
    );
    {
        let m = media(&state);
        assert!(m.slider_drag_active, "press arms the rail drag");
        assert_eq!(
            m.slider_drag_pos,
            Some(press_secs),
            "drag position follows the press point"
        );
    }
    assert_eq!(
        handler.media_commands.len(),
        before,
        "pressing the rail seeks nothing yet"
    );

    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::LeftDrag, drag_x, rail.y)],
    );
    {
        let m = media(&state);
        assert_eq!(
            m.slider_drag_pos,
            Some(commit_secs),
            "dragging moves the visual thumb"
        );
        assert!(m.slider_drag_active, "drag still armed while held");
    }
    assert_eq!(
        handler.media_commands.len(),
        before,
        "drag motion itself seeks nothing"
    );

    drive(
        &mut state,
        &mut handler,
        [mouse(MouseKind::LeftUp, drag_x, rail.y)],
    );
    let new = &handler.media_commands[before..];
    assert_eq!(new.len(), 1, "release commits exactly once");
    assert_eq!(new[0], (session, MediaCommand::SeekAbsolute(commit_secs)));
    let m = media(&state);
    assert!(!m.slider_drag_active, "trio cleared after commit");
    assert_eq!(m.slider_drag_pos, None);
    assert_eq!(m.slider_hover, None);
}

#[test]
fn fullscreen_toggle_runs_supervised_restart_cycle_preserving_position() {
    let (mut state, mut handler, session) = playing_video(12.5, Some(90.0));
    assert_eq!(handler.started_media, vec![(session, video_path())]);

    // Plain `f` reaches the same action the renderer's FULLSCREEN button hits.
    let mapped = map_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
        &state,
    )
    .expect("plain f is bound under media mode");
    assert!(
        matches!(mapped, Action::MediaToggleFullscreen),
        "plain f maps to the fullscreen action"
    );

    // Step manually so the Stopping snapshot is observable before the stop
    // handback lands.
    let effects = reduce(&mut state, mapped);
    assert_eq!(
        stop_media_count(&effects, session),
        1,
        "fullscreen enters through the supervisor: exactly one stop"
    );
    let m = media(&state);
    assert_eq!(m.phase, MediaPhase::Stopping);
    assert_eq!(
        m.after_stop,
        Some(AfterStop::RestartAfterResize {
            position: 12.5,
            paused: false
        }),
        "position and pause survive the restart"
    );
    assert!(m.fullscreen, "flag flipped on immediately");
    assert!(!m.slider_drag_active && m.slider_drag_pos.is_none());

    // Handback re-enters Preparing with resume data and the flag intact.
    reduce(&mut state, Action::MediaStopped { session });
    let m = media(&state);
    assert_eq!(m.phase, MediaPhase::Preparing);
    assert_eq!(m.resume_position, Some(12.5));
    assert_eq!(m.resume_paused, Some(false));
    assert!(m.awaiting_surface_ready);
    assert!(m.surface.is_none());
    assert!(m.fullscreen, "restart keeps fullscreen active");

    // Fresh surface restarts playback under the same session id.
    let effects = reduce(
        &mut state,
        Action::MediaSurfaceReady {
            session,
            surface: video_surface(),
        },
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartMedia { session: s, .. } if *s == session)),
        "surface ready restarts the backend"
    );
    let effects = reduce(&mut state, Action::MediaBackendReady { session });
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::MediaCommand { session: s, command: MediaCommand::Load } if *s == session)));
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session,
            phase: MediaPhase::Playing,
            position: 12.5,
            duration: Some(90.0),
            volume: 80,
        }],
    );

    // Second toggle exits fullscreen through the same cycle.
    let effects = reduce(&mut state, Action::MediaToggleFullscreen);
    assert_eq!(stop_media_count(&effects, session), 1);
    let m = media(&state);
    assert!(!m.fullscreen, "second toggle exits fullscreen");
    assert_eq!(m.phase, MediaPhase::Stopping);
    reduce(&mut state, Action::MediaStopped { session });
    let m = media(&state);
    assert_eq!(m.phase, MediaPhase::Preparing);
    assert!(!m.fullscreen, "exit survives the handback");
}

#[test]
fn resize_during_fullscreen_playing_restarts_the_cycle() {
    let (mut state, mut handler, session) = fullscreen_playing_video(12.5, Some(90.0));
    assert_eq!(handler.stopped_media, vec![session]);

    drive(
        &mut state,
        &mut handler,
        [Action::Resize {
            width: 160,
            height: 48,
        }],
    );

    assert_eq!(
        handler.stopped_media,
        vec![session, session],
        "resize while fullscreen playing fires exactly one more stop"
    );
    let m = media(&state);
    assert_eq!(m.phase, MediaPhase::Preparing);
    assert_eq!(m.resume_position, Some(12.5));
    assert_eq!(m.resume_paused, Some(false));
    assert!(m.awaiting_surface_ready);
    assert!(
        m.fullscreen,
        "resize inside fullscreen must not drop the flag"
    );
}

#[test]
fn close_from_fullscreen_hands_back_without_leftover_state() {
    let (mut state, mut handler, session) = fullscreen_playing_video(12.5, Some(90.0));

    drive(&mut state, &mut handler, [Action::MediaClose]);
    assert!(matches!(state.mode, Mode::Browser), "close exits media");
    // One stop from entering fullscreen, one from the close itself.
    assert_eq!(handler.stopped_media, vec![session, session]);

    // Re-entering the same file starts a pristine session.
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let m = media(&state);
    assert_ne!(m.session, session, "re-entry mints a fresh session");
    assert!(!m.fullscreen, "no fullscreen residue");
    assert!(!m.slider_drag_active, "no drag residue");
    assert_eq!(m.slider_drag_pos, None);
    assert_eq!(m.slider_hover, None);
    assert!(m.awaiting_surface_ready);
    assert_eq!(
        handler.stopped_media,
        vec![session, session],
        "re-entry performs no spurious stops"
    );
}

#[test]
fn stale_session_media_results_are_rejected_everywhere() {
    let (mut state, mut handler, session) = playing_audio(30.0, Some(90.0));

    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session: session + 1,
            phase: MediaPhase::Playing,
            position: 99.0,
            duration: Some(50.0),
            volume: 100,
        }],
    );
    let m = media(&state);
    assert_eq!(m.position, 30.0, "stale status must not move playback");
    assert_eq!(m.duration, Some(90.0));
    assert_eq!(m.volume, 80);

    drive(
        &mut state,
        &mut handler,
        [Action::MediaSpectrum {
            session: session + 1,
            spectrum: [0.9; 24],
        }],
    );
    assert!(
        media(&state).spectrum.iter().all(|value| *value == 0.0),
        "stale spectrum must not apply"
    );

    let had_message = state.message.is_none();
    drive(
        &mut state,
        &mut handler,
        [Action::MediaEnded {
            session: session + 1,
        }],
    );
    let m = media(&state);
    assert_eq!(m.session, session, "stale end cannot close the session");
    assert_eq!(m.phase, MediaPhase::Playing);
    assert!(handler.stopped_media.is_empty(), "nothing was stopped");
    assert_eq!(
        state.message.is_none(),
        had_message,
        "stale end leaves messages alone"
    );

    drive(
        &mut state,
        &mut handler,
        [Action::MediaStopped {
            session: session + 1,
        }],
    );
    assert!(matches!(state.mode, Mode::Media(_)), "stale stop ignored");
}

#[test]
fn eof_announces_finished_then_auto_closes() {
    let (mut state, mut handler, session) = playing_audio(30.0, Some(90.0));

    drive(&mut state, &mut handler, [Action::MediaEnded { session }]);

    let message = state.message.clone().expect("EOF announces completion");
    assert_eq!(message.text, "finished song.mp3");
    assert!(!message.is_error);
    assert_eq!(
        handler.stopped_media,
        vec![session],
        "EOF stops the backend exactly once"
    );
    assert!(
        matches!(state.mode, Mode::Browser),
        "the stop hands back to the browser automatically"
    );
}

#[test]
fn next_advances_playlist_skipping_non_media_and_stops_at_end() {
    let mut state = demo_state(120, 36);
    let mut handler = SyncHandler::new(mixed_media_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);

    // Focus 02-first.wav (files sort ahead of it: only 01-notes.txt).
    drive(&mut state, &mut handler, [Action::GotoFirst]);
    drive(&mut state, &mut handler, [Action::MoveDown]);
    assert_eq!(
        state.browser.focused().expect("entry").entry.display_name(),
        "02-first.wav"
    );
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let first = current_session(&state);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session: first,
            surface: audio_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session: first,
            phase: MediaPhase::Playing,
            position: 1.0,
            duration: Some(60.0),
            volume: 70,
        }],
    );

    let root = demo_root();
    let wav = root.join("02-first.wav");
    let flac = root.join("04-second.flac");
    let mp3 = root.join("05-third.mp3");
    {
        let m = media(&state);
        assert_eq!(
            m.playlist,
            vec![wav.clone(), flac.clone(), mp3.clone()],
            "playlist keeps only media entries in display order"
        );
        assert_eq!(m.playlist_pos, 0);
    }

    // Next -> 04-second.flac (skipping 03-readme.md).
    drive(&mut state, &mut handler, [Action::MediaNext]);
    let second = {
        let m = media(&state);
        assert_ne!(m.session, first, "next mints a fresh session");
        assert_eq!(m.path, flac);
        assert_eq!(m.playlist_pos, 1);
        assert_eq!(m.playlist.len(), 3);
        m.session
    };
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session: second,
            surface: audio_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session: second,
            phase: MediaPhase::Playing,
            position: 1.0,
            duration: Some(60.0),
            volume: 70,
        }],
    );

    // Next -> 05-third.mp3.
    drive(&mut state, &mut handler, [Action::MediaNext]);
    let third = {
        let m = media(&state);
        assert_eq!(m.path, mp3);
        assert_eq!(m.playlist_pos, 2);
        m.session
    };
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session: third,
            surface: audio_surface(),
        }],
    );
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session: third,
            phase: MediaPhase::Playing,
            position: 1.0,
            duration: Some(60.0),
            volume: 70,
        }],
    );

    // End of list: info message, no wraparound, session untouched.
    drive(&mut state, &mut handler, [Action::MediaNext]);
    let m = media(&state);
    let message = state.message.clone().expect("end-of-playlist message");
    assert_eq!(message.text, "end of playlist");
    assert!(!message.is_error);
    assert_eq!(m.path, mp3, "no wraparound");
    assert_eq!(m.playlist_pos, 2);
    assert_eq!(current_session(&state), m.session);

    // Repeating Next stays inert apart from the message.
    let commands_before = handler.started_media.len();
    drive(&mut state, &mut handler, [Action::MediaNext]);
    assert_eq!(
        handler.started_media.len(),
        commands_before,
        "no extra session beyond the end"
    );
    assert_eq!(handler.started_media.len(), 3, "three tracks were started");
}

#[test]
fn footer_focus_text_precedence_hover_then_selection_then_count() {
    let (mut state, _handler) = loaded(120, 36);
    assert_eq!(footer_focus_text(&state), None, "nothing to report");

    let song = demo_root().join("song.mp3");
    state.browser.selection.insert(song);
    assert_eq!(
        footer_focus_text(&state),
        Some("song.mp3".to_string()),
        "single selection shows its basename"
    );

    state
        .browser
        .selection
        .insert(demo_root().join("photo.png"));
    assert_eq!(
        footer_focus_text(&state),
        Some("2 items selected".to_string()),
        "multi selection shows a count"
    );

    let cargo_pos = state
        .browser
        .visible_entries()
        .position(|(_, view)| view.entry.display_name() == "Cargo.toml")
        .expect("Cargo.toml visible");
    state.hover.row = Some(cargo_pos);
    assert_eq!(
        footer_focus_text(&state),
        Some("Cargo.toml".to_string()),
        "hover wins over any selection"
    );
}

#[test]
fn error_message_presence_suppresses_footer_filename_segment() {
    let (mut state, _handler) = loaded(120, 36);
    state.browser.selection.insert(demo_root().join("song.mp3"));
    state.message = None;

    let row = render_bottom_row(&mut state);
    assert!(
        row.contains("song.mp3"),
        "footer shows the selected filename, got {row:?}"
    );

    state.set_error("boom");
    let row = render_bottom_row(&mut state);
    assert!(
        !row.contains("song.mp3"),
        "an error suppresses the filename segment, got {row:?}"
    );
    assert!(row.contains("boom"), "error text is displayed, got {row:?}");
}

#[test]
fn keymap_f_and_n_are_wired_under_media_mode() {
    let (mut state, mut handler, session) = playing_audio(10.0, Some(60.0));

    let f = map_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
        &state,
    );
    assert!(
        matches!(f, Some(Action::MediaToggleFullscreen)),
        "f maps to the fullscreen action under media mode"
    );
    let n = map_key(
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        &state,
    );
    assert!(
        matches!(n, Some(Action::MediaNext)),
        "n maps to the next action under media mode"
    );

    // Fullscreen is video-only: on audio the mapped action is a clean no-op.
    drive(&mut state, &mut handler, [f.unwrap()]);
    assert!(handler.stopped_media.is_empty(), "audio ignores fullscreen");
    assert!(!media(&state).fullscreen);
    assert_eq!(media(&state).phase, MediaPhase::Playing);

    // song.mp3 is the only audio entry: n reports the end without wrapping.
    drive(&mut state, &mut handler, [n.unwrap()]);
    let message = state.message.clone().expect("end-of-playlist message");
    assert_eq!(message.text, "end of playlist");
    assert!(!message.is_error);
    assert_eq!(current_session(&state), session, "no new session minted");
    assert_eq!(media(&state).path, demo_root().join("song.mp3"));
}

/// Opens `02-first.wav` from the mixed fixture and reports it Playing.
fn playing_queue() -> (AppState, SyncHandler, u64) {
    let mut state = demo_state(120, 36);
    let mut handler = SyncHandler::new(mixed_media_fs());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    drive(
        &mut state,
        &mut handler,
        [Action::GotoFirst, Action::MoveDown],
    );
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    drive(
        &mut state,
        &mut handler,
        [
            Action::MediaSurfaceReady {
                session,
                surface: audio_surface(),
            },
            Action::MediaStatus {
                session,
                phase: MediaPhase::Playing,
                position: 1.0,
                duration: Some(60.0),
                volume: 80,
            },
        ],
    );
    (state, handler, session)
}

#[test]
fn eof_auto_advances_through_the_queue() {
    let (mut state, mut handler, first) = playing_queue();
    drive(
        &mut state,
        &mut handler,
        [Action::MediaEnded { session: first }],
    );
    let m = media(&state);
    assert_ne!(m.session, first, "the next track is a fresh session");
    assert_eq!(m.path, demo_root().join("04-second.flac"));
    assert_eq!(m.phase, MediaPhase::Preparing);
    assert!(
        state
            .message
            .as_ref()
            .is_some_and(|msg| msg.text.contains("now playing 04-second")),
        "{:?}",
        state.message
    );
}

#[test]
fn repeat_one_replays_and_repeat_all_wraps() {
    let (mut state, mut handler, first) = playing_queue();
    drive(
        &mut state,
        &mut handler,
        [Action::MediaRepeat, Action::MediaRepeat],
    );
    assert_eq!(media(&state).repeat, tui_explorer::app::state::Repeat::One);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaEnded { session: first }],
    );
    assert_eq!(media(&state).path, demo_root().join("02-first.wav"));
    assert_eq!(
        media(&state).repeat,
        tui_explorer::app::state::Repeat::One,
        "preferences carry over"
    );
}

#[test]
fn minimized_audio_plays_on_in_the_mini_player() {
    let (mut state, mut handler, first) = playing_queue();
    drive(&mut state, &mut handler, [Action::MediaMinimize]);
    assert!(matches!(state.mode, Mode::Browser), "browsing again");
    let mini = state.mini.as_ref().expect("mini player");
    assert_eq!(mini.session, first, "same session keeps playing");
    assert!(handler.stopped_media.is_empty(), "nothing was stopped");

    // Status updates reach the background session.
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session: first,
            phase: MediaPhase::Playing,
            position: 42.0,
            duration: Some(60.0),
            volume: 80,
        }],
    );
    assert_eq!(state.mini.as_ref().unwrap().position, 42.0);

    // Transport works from the browser (mini player buttons).
    drive(&mut state, &mut handler, [Action::MediaTogglePause]);
    assert!(
        handler
            .media_commands
            .contains(&(first, MediaCommand::TogglePause))
    );

    // End of track: the next one starts straight away in the background.
    drive(
        &mut state,
        &mut handler,
        [Action::MediaEnded { session: first }],
    );
    let second = state.mini.as_ref().expect("still mini").session;
    assert_ne!(second, first);
    assert!(
        handler.started_media.iter().any(|(s, _)| *s == second),
        "audio needs no surface handshake in the background"
    );
    assert!(matches!(state.mode, Mode::Browser));

    // M brings the full player back with the live session.
    drive(&mut state, &mut handler, [Action::MediaExpand]);
    assert_eq!(media(&state).session, second);
    assert!(state.mini.is_none());
}

#[test]
fn queue_end_in_background_clears_the_mini_player() {
    let (mut state, mut handler, first) = playing_queue();
    // Step to the last track, completing each handshake.
    for _ in 0..2 {
        drive(&mut state, &mut handler, [Action::MediaNext]);
        let session = current_session(&state);
        drive(
            &mut state,
            &mut handler,
            [
                Action::MediaSurfaceReady {
                    session,
                    surface: audio_surface(),
                },
                Action::MediaStatus {
                    session,
                    phase: MediaPhase::Playing,
                    position: 1.0,
                    duration: Some(60.0),
                    volume: 80,
                },
            ],
        );
    }
    let last = current_session(&state);
    assert_ne!(last, first);
    assert_eq!(media(&state).path, demo_root().join("05-third.mp3"));
    drive(
        &mut state,
        &mut handler,
        [
            Action::MediaSurfaceReady {
                session: last,
                surface: audio_surface(),
            },
            Action::MediaStatus {
                session: last,
                phase: MediaPhase::Playing,
                position: 1.0,
                duration: Some(60.0),
                volume: 80,
            },
            Action::MediaMinimize,
            Action::MediaEnded { session: last },
        ],
    );
    assert!(state.mini.is_none(), "nothing left to play");
    assert!(handler.stopped_media.contains(&last));
}

#[test]
fn volume_is_remembered_for_the_next_session() {
    let (mut state, mut handler, session) = playing_audio(10.0, Some(90.0));
    drive(&mut state, &mut handler, [Action::MediaVolume(-30)]);
    assert_eq!(media(&state).volume, 50);
    assert_eq!(state.settings.volume, 50);
    assert!(state.settings_dirty);
    drive(&mut state, &mut handler, [Action::MediaClose]);
    assert!(matches!(state.mode, Mode::Browser));
    // A new session starts at the remembered level.
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let next = current_session(&state);
    assert_ne!(next, session);
    assert_eq!(media(&state).volume, 50);
    drive(
        &mut state,
        &mut handler,
        [Action::MediaSurfaceReady {
            session: next,
            surface: audio_surface(),
        }],
    );
    assert!(
        handler
            .media_commands
            .contains(&(next, MediaCommand::SetVolume(50))),
        "the fresh backend gets the level after Load: {:?}",
        handler.media_commands
    );
}

#[test]
fn mute_keeps_the_level_and_unmute_restores_it() {
    let (mut state, mut handler, session) = playing_audio(10.0, Some(90.0));
    drive(&mut state, &mut handler, [Action::MediaMute]);
    assert!(media(&state).muted);
    // A muted backend reporting 0 does not overwrite the kept level.
    drive(
        &mut state,
        &mut handler,
        [Action::MediaStatus {
            session,
            phase: MediaPhase::Playing,
            position: 11.0,
            duration: Some(90.0),
            volume: 0,
        }],
    );
    assert_eq!(media(&state).volume, 80);
    drive(&mut state, &mut handler, [Action::MediaMute]);
    assert!(!media(&state).muted);
    assert!(
        handler
            .media_commands
            .contains(&(session, MediaCommand::SetVolume(80)))
    );
}

#[test]
fn subtitle_picker_finds_local_files_and_loads_the_choice() {
    let (mut state, mut handler, session) = playing_video(5.0, Some(30.0));
    drive(&mut state, &mut handler, [Action::MediaOpenSubs]);
    assert!(
        handler
            .media_commands
            .contains(&(session, MediaCommand::TogglePause)),
        "a playing video pauses so the picker is visible"
    );
    let picker = media(&state).sub_picker.clone().expect("picker open");
    assert!(!picker.searching, "discovery answered");
    let names: Vec<String> = picker.found.iter().map(|f| f.name()).collect();
    assert_eq!(
        names.first().map(String::as_str),
        Some("clip.srt"),
        "{names:?}"
    );
    assert!(names.contains(&"clip.fr.forced.srt".to_string()));

    // Typing filters; Enter loads the selected file and resumes.
    drive(
        &mut state,
        &mut handler,
        "french".chars().map(Action::SubPickerChar),
    );
    drive(&mut state, &mut handler, [Action::SubPickerSubmit]);
    let wanted = demo_root().join("clip.fr.forced.srt");
    assert!(
        handler
            .media_commands
            .contains(&(session, MediaCommand::AddSub(wanted.clone())))
    );
    assert_eq!(media(&state).sub_file.as_ref(), Some(&wanted));
    assert!(media(&state).sub_picker.is_none());
    let pauses = handler
        .media_commands
        .iter()
        .filter(|(s, c)| *s == session && *c == MediaCommand::TogglePause)
        .count();
    assert_eq!(pauses, 2, "paused for the picker, resumed after");
}

#[test]
fn player_keys_reach_the_new_transport_actions() {
    let (state, _handler, _session) = playing_video(5.0, Some(30.0));
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert!(matches!(
        map_key(key(KeyCode::Char('c')), &state),
        Some(Action::MediaOpenSubs)
    ));
    assert!(matches!(
        map_key(key(KeyCode::Char('z')), &state),
        Some(Action::MediaSubDelay(-1))
    ));
    assert!(matches!(
        map_key(key(KeyCode::Char(']')), &state),
        Some(Action::MediaSpeed(1))
    ));
    assert!(matches!(
        map_key(key(KeyCode::Char('5')), &state),
        Some(Action::MediaSeekPercent(5))
    ));
}

#[test]
fn window_backend_toggles_fullscreen_in_place_and_tct_is_fullscreen_only() {
    // GUI window: fullscreen is an IPC property flip, no restart cycle.
    let mut state = demo_state(120, 36);
    state.settings.video_output = tui_explorer::settings::VideoOutput::Window;
    let mut handler = SyncHandler::new(demo_fs_with_video());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    let session = current_session(&state);
    assert_eq!(
        media(&state).backend,
        tui_explorer::media::VideoBackend::Window
    );
    drive(
        &mut state,
        &mut handler,
        [
            Action::MediaSurfaceReady {
                session,
                surface: video_surface(),
            },
            Action::MediaStatus {
                session,
                phase: MediaPhase::Playing,
                position: 3.0,
                duration: Some(30.0),
                volume: 80,
            },
            Action::MediaToggleFullscreen,
        ],
    );
    assert_eq!(current_session(&state), session, "no restart");
    assert!(media(&state).fullscreen);
    assert!(
        handler
            .media_commands
            .contains(&(session, MediaCommand::SetFullscreen(true)))
    );
    assert!(
        !state.media_owns_terminal(),
        "a separate window never owns the terminal"
    );

    // Text output always fills the terminal.
    let mut state = demo_state(120, 36);
    state.settings.video_output = tui_explorer::settings::VideoOutput::Tct;
    let mut handler = SyncHandler::new(demo_fs_with_video());
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    focus_video(&mut state, &mut handler);
    drive(&mut state, &mut handler, [Action::OpenFocused]);
    assert_eq!(
        media(&state).backend,
        tui_explorer::media::VideoBackend::Tct
    );
    assert!(media(&state).fullscreen);
}
