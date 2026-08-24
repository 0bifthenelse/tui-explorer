use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tui_explorer::app::action::{Action, DirectorySnapshot, MouseKind};
use tui_explorer::media::mpv::{IpcMessage, PropertyChange};

use tui_explorer::app::effects::{Effect, EffectHandler};
use tui_explorer::app::reduce::reduce;
use tui_explorer::app::state::{AppState, Mode};
use tui_explorer::browser::EntryView;
use tui_explorer::config;
use tui_explorer::filesystem::real::{RealFileSystem, RealMutations};
use tui_explorer::filesystem::{FileSystem, MutationBackend};
use tui_explorer::input::keymap::map_key;
use tui_explorer::operations::{ConflictPolicy, find_conflicts, run_operation, run_rename};
use tui_explorer::tags::TagStore;
use tui_explorer::terminal::{self, TerminalSession, crossterm_driver::CrosstermTty};
use tui_explorer::ui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "tui-explorer 0.1.0
fast terminal file explorer for Linux

USAGE:
    tui-explorer [PATH]

ARGS:
    PATH    starting directory (default: current directory)

OPTIONS:
    -h, --help       show this help
    -V, --version    show version

KEYS:
    j/k or arrows    move selection
    h/l              move between tiles
    Backspace        parent directory
    e, Enter         enter folder, or open a file (audio and video play in
                     the built-in media modal; other files ask for a command)
    r                open with: prompt for a command to run on the focused entry
    X                encrypt / decrypt focused entry
    b, p             toggle sidebar / preview panel
    B                search bookmarks (fuzzy navigator)
    Ctrl-b           bookmark / unbookmark current directory
    g g, G           first / last entry
    Ctrl-u/Ctrl-d    half page up/down
    Space, v         select, visual mode
    .                toggle hidden files
    t, T             quick tag / tag picker
    :                command mode (:copy :move :rename :delete :tag :untag :tags :open
                     :open-with :mkdir :touch :selectall :invert :deselect :filter :sort :refresh :cd :quit :help)
    /, Ctrl-f        quick current-directory filename filter
    ?                help overlay
    q                quit

MOUSE:
    click selects, double click (or e/Enter) opens, right click menu, wheel scrolls,
    breadcrumb and sidebar navigate

MEDIA (audio):
    Space or Enter   play / pause
    Left, h          seek back 5 seconds
    Right, l         seek forward 5 seconds
    Up, Down         volume up / down (5% steps)
    s                stop and restart from the beginning
    Esc, q           close the media modal
    Supported audio: wav flac ogg oga mp3 m4a (Symphonia decoders; rodio
    playback with a real FFT spectrum). Video requires mpv and a Kitty
    graphics terminal.

DATA:
    tags database: $XDG_DATA_HOME/tui-explorer/tags.sqlite3
    fallback:      $HOME/.local/share/tui-explorer/tags.sqlite3
";

struct ProdHandler {
    fs: RealFileSystem,
    mutations: RealMutations,
    tags: Option<TagStore>,
    bookmarks: tui_explorer::sidebar::BookmarkStore,
    sender: SyncSender<Action>,
    media: MediaSupervisor,
}

/// Owns the active media runtime on one dedicated thread. Audio keeps a real
/// rodio output stream; every asynchronous result is reported through the
/// bounded action sender with its session generation.
struct MediaSupervisor {
    sender: SyncSender<Action>,
    commands: Option<std::sync::mpsc::Sender<MediaRequest>>,
}

enum MediaRequest {
    Start {
        session: u64,
        path: PathBuf,
        kind: tui_explorer::media::MediaKind,
        surface: tui_explorer::app::state::MediaSurface,
        resume_position: Option<f64>,
        resume_paused: Option<bool>,
    },
    Command {
        session: u64,
        command: tui_explorer::media::MediaCommand,
    },
    Stop {
        session: u64,
    },
    Shutdown,
}

impl MediaSupervisor {
    fn new(
        sender: SyncSender<Action>,
        picker_protocol: ratatui_image::picker::ProtocolType,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<MediaRequest>();
        let action_sender = sender.clone();
        std::thread::Builder::new()
            .name("media-supervisor".into())
            .spawn(move || media_supervisor_loop(rx, action_sender, picker_protocol))
            .expect("spawn media supervisor");
        MediaSupervisor {
            sender,
            commands: Some(tx),
        }
    }

    fn send(&self, request: MediaRequest) {
        if let Some(commands) = &self.commands
            && commands.send(request).is_err()
        {
            let _ = self.sender.send(Action::MediaFailed {
                session: 0,
                message: "audio backend stopped".to_string(),
            });
        }
    }

    fn start(
        &self,
        session: u64,
        path: PathBuf,
        kind: tui_explorer::media::MediaKind,
        surface: tui_explorer::app::state::MediaSurface,
        resume_position: Option<f64>,
        resume_paused: Option<bool>,
    ) {
        self.send(MediaRequest::Start {
            session,
            path,
            kind,
            resume_position,
            surface,
            resume_paused,
        });
    }

    fn command(&self, session: u64, command: tui_explorer::media::MediaCommand) {
        self.send(MediaRequest::Command { session, command });
    }

    fn stop(&self, session: u64) {
        self.send(MediaRequest::Stop { session });
    }
}

impl Drop for MediaSupervisor {
    fn drop(&mut self) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(MediaRequest::Shutdown);
        }
    }
}

/// How often the supervisor samples the active backend and publishes
/// coherent status. Small enough for smooth progress feedback, large enough
/// to never busy-spin.
const MEDIA_TICK: Duration = Duration::from_millis(100);

/// Cached view of one live mpv instance between ticks. Values change only
/// through observed property changes, so published status is always
/// backend-confirmed.
struct VideoPlayback {
    process: Box<tui_explorer::media::mpv::MpvProcess>,
    position: f64,
    duration: Option<f64>,
    volume: u8,
    paused: bool,
    /// Set once EOF/end-file was seen; status stops being published so a
    /// Stopping session can never be overwritten by stale ticks.
    finished: bool,
    /// Guards the one-shot MediaEnded emission.
    end_reported: bool,
}

/// One live backend on the supervisor thread.
enum ActiveMedia {
    Audio {
        #[allow(dead_code)] // kept alive so the device stream stays open
        stream: rodio::OutputStream,
        sink: rodio::Sink,
        /// Real decoded duration captured from the symphonia track.
        duration: Option<f64>,
        /// Set once the sink ran dry and EOF was reported.
        finished: bool,
        /// Tells the spectrum publisher thread to exit with the session.
        spectrum_stop: Arc<AtomicBool>,
    },
    Video(VideoPlayback),
}

fn media_supervisor_loop(
    receiver: std::sync::mpsc::Receiver<MediaRequest>,
    sender: SyncSender<Action>,
    picker_protocol: ratatui_image::picker::ProtocolType,
) {
    let mut active: Option<(u64, ActiveMedia)> = None;
    let mut running = true;
    while running {
        // Block briefly per tick so an idle supervisor never spins, yet the
        // active backend is sampled continuously even with no commands.
        match receiver.recv_timeout(MEDIA_TICK) {
            Ok(request) => {
                running &= handle_media_request(&mut active, &sender, picker_protocol, request);
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        // Drain queued requests so ticks stay regular under bursts.
        while running {
            match receiver.try_recv() {
                Ok(request) => {
                    running &= handle_media_request(&mut active, &sender, picker_protocol, request);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        if running {
            tick_status(&mut active, &sender);
        }
    }
    // Dropping `active` tears the backend down gracefully (mpv quit + kill +
    // socket unlink; rodio stream/sink drop; spectrum threads observe their
    // stop flag or a closed action channel).
}

/// Applies one supervisor request. Returns false when shutdown requested.
fn handle_media_request(
    active: &mut Option<(u64, ActiveMedia)>,
    sender: &SyncSender<Action>,
    picker_protocol: ratatui_image::picker::ProtocolType,
    request: MediaRequest,
) -> bool {
    match request {
        MediaRequest::Shutdown => false,
        MediaRequest::Start {
            session,
            path,
            kind,
            surface,
            resume_position,
            resume_paused,
        } => {
            // Replacing any previous backend drops it first.
            stop_active(active);
            if kind != tui_explorer::media::MediaKind::Video {
                match start_audio(sender, session, &path) {
                    Ok((stream, sink, duration, spectrum_stop)) => {
                        if let Some(position) = resume_position {
                            let _ = sink.try_seek(Duration::from_secs_f64(position.max(0.0)));
                        }
                        if resume_paused.unwrap_or(false) {
                            sink.pause();
                        } else {
                            sink.play();
                        }
                        let _ = sender.send(Action::MediaBackendReady { session });
                        *active = Some((
                            session,
                            ActiveMedia::Audio {
                                stream,
                                sink,
                                duration,
                                finished: false,
                                spectrum_stop,
                            },
                        ));
                    }
                    Err(message) => {
                        let _ = sender.send(Action::MediaFailed { session, message });
                    }
                }
                return true;
            }
            // Video requires a Kitty-protocol picker.
            if picker_protocol != ratatui_image::picker::ProtocolType::Kitty {
                let _ = sender.send(Action::MediaFailed {
                    session,
                    message: "video playback requires a Kitty-compatible terminal".to_string(),
                });
                return true;
            }
            match tui_explorer::media::mpv::MpvProcess::spawn(
                &path,
                (surface.rect.x, surface.rect.y),
                (surface.rect.width, surface.rect.height),
                surface.cell_pixels,
                session,
            ) {
                Ok(process) => {
                    let mut playback = VideoPlayback {
                        process: Box::new(process),
                        position: resume_position.unwrap_or(0.0),
                        duration: None,
                        volume: 100,
                        paused: resume_paused.unwrap_or(false),
                        finished: false,
                        end_reported: false,
                    };
                    // Distinct observer ids per property; dispatch keys off
                    // the echoed property name, ids keep the mapping explicit.
                    const OBSERVED: [(u64, &str); 5] = [
                        (1, "time-pos"),
                        (2, "duration"),
                        (3, "pause"),
                        (4, "volume"),
                        (5, "eof-reached"),
                    ];
                    let mut observe_error = None;
                    for (id, property) in OBSERVED {
                        if let Err(error) = playback.process.send_command(
                            &[
                                serde_json::Value::from("observe_property"),
                                serde_json::Value::from(id),
                                serde_json::Value::from(property),
                            ],
                            None,
                        ) {
                            observe_error = Some(format!("cannot observe mpv {property}: {error}"));
                            break;
                        }
                    }
                    if let Some(message) = observe_error {
                        let _ = sender.send(Action::MediaFailed { session, message });
                        return true; // playback dropped -> graceful shutdown
                    }
                    let _ = sender.send(Action::MediaBackendReady { session });
                    *active = Some((session, ActiveMedia::Video(playback)));
                }
                Err(message) => {
                    let _ = sender.send(Action::MediaFailed { session, message });
                }
            }
            true
        }
        MediaRequest::Command { session, command } => {
            let Some((active_session, media)) = active.as_mut() else {
                return true;
            };
            if *active_session != session {
                return true;
            }
            match media {
                ActiveMedia::Audio { sink, duration, .. } => {
                    apply_audio_command(sender, session, sink, *duration, command);
                }
                ActiveMedia::Video(playback) => {
                    if let Err(error) = apply_video_command(&mut playback.process, command) {
                        let _ = sender.send(Action::MediaFailed {
                            session,
                            message: format!("mpv command failed: {error}"),
                        });
                    }
                }
            }
            true
        }
        MediaRequest::Stop { session } => {
            tracing::info!(session, "supervisor stop received");
            if matches!(&active, Some((active_session, _)) if *active_session == session) {
                stop_active(active);
            }
            let _ = sender.send(Action::MediaStopped { session });
            true
        }
    }
}

/// Stops and drops the active backend, signalling the spectrum publisher to
/// exit. Dropping MpvProcess runs its graceful shutdown; dropping the audio
/// stack closes the device.
fn stop_active(active: &mut Option<(u64, ActiveMedia)>) {
    if let Some((_, ActiveMedia::Audio { spectrum_stop, .. })) = active {
        spectrum_stop.store(true, Ordering::Relaxed);
    }
    *active = None;
}

/// Samples the active backend once per tick and publishes one coherent
/// status action derived from real backend state.
fn tick_status(active: &mut Option<(u64, ActiveMedia)>, sender: &SyncSender<Action>) {
    use tui_explorer::media::MediaPhase;
    let Some((session, media)) = active.as_mut() else {
        return;
    };
    match media {
        ActiveMedia::Audio {
            sink,
            duration,
            finished,
            ..
        } => {
            if *finished {
                return;
            }
            if sink.empty() {
                // Every source sample played out: real EOF, not a guess.
                *finished = true;
                let _ = sender.send(Action::MediaEnded { session: *session });
                return;
            }
            let phase = if sink.is_paused() {
                MediaPhase::Paused
            } else {
                MediaPhase::Playing
            };
            let _ = sender.send(Action::MediaStatus {
                session: *session,
                phase,
                position: sink.get_pos().as_secs_f64(),
                duration: *duration,
                volume: volume_percent(sink.volume()),
            });
        }
        ActiveMedia::Video(playback) => {
            // Drain every pending IPC event before publishing one coherent
            // snapshot; later observations overwrite earlier ones in-batch.
            loop {
                match playback.process.poll_message() {
                    Ok(None) => break,
                    Ok(Some(message)) => match message {
                        IpcMessage::Event { name, property } => {
                            if let Some(change) = property {
                                apply_property(playback, change);
                            } else if name == "end-file" {
                                playback.finished = true;
                            }
                        }
                        IpcMessage::Reply { error, .. }
                            if !error.is_empty() && error != "success" =>
                        {
                            tracing::warn!(%error, "mpv command failed");
                        }
                        _ => {}
                    },
                    Err(error) => {
                        // The IPC stream died: mpv exited or is unusable.
                        // Surface bounded stderr diagnostics when available.
                        tracing::warn!(session, %error, "mpv ipc stream closed");
                        let detail = if playback.process.has_exited() {
                            playback
                                .process
                                .stderr_tail()
                                .map(|tail| format!(": {tail}"))
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let _ = sender.send(Action::MediaFailed {
                            session: *session,
                            message: format!("mpv terminated unexpectedly{detail}"),
                        });
                        return;
                    }
                }
            }
            if playback.finished && !playback.end_reported {
                // One-shot EOF handoff; further ticks stay silent until the
                // reducer-driven Stop tears the backend down.
                playback.end_reported = true;
                tracing::info!(session, "mpv reached end of file");
                let _ = sender.send(Action::MediaEnded { session: *session });
                return;
            }
            let phase = if playback.paused {
                MediaPhase::Paused
            } else {
                MediaPhase::Playing
            };
            let _ = sender.send(Action::MediaStatus {
                session: *session,
                phase,
                position: playback.position,
                duration: playback.duration,
                volume: playback.volume,
            });
        }
    }
}

/// Folds one observed mpv property into the cached playback state. Missing,
/// null, or mistyped values are ignored; the next observation corrects them.
fn apply_property(playback: &mut VideoPlayback, change: PropertyChange) {
    match change.name.as_str() {
        "time-pos" => {
            if let Some(value) = change.value.as_f64() {
                playback.position = value.max(0.0);
            }
        }
        "duration" => {
            playback.duration = change.value.as_f64().filter(|value| *value > 0.0);
        }
        "pause" => {
            if let Some(value) = change.value.as_bool() {
                playback.paused = value;
            }
        }
        "volume" => {
            if let Some(value) = change.value.as_f64() {
                // mpv reports percent; values above 100 exist via volume-max.
                playback.volume = value.round().clamp(0.0, 100.0) as u8;
            }
        }
        "eof-reached" => {
            if change.value.as_bool() == Some(true) {
                playback.finished = true;
            }
        }
        _ => {}
    }
}

fn apply_video_command(
    process: &mut tui_explorer::media::mpv::MpvProcess,
    command: tui_explorer::media::MediaCommand,
) -> Result<(), String> {
    use tui_explorer::media::MediaCommand;
    match command {
        MediaCommand::Load | MediaCommand::Quit => Ok(()),
        MediaCommand::TogglePause => process
            .send_command(
                &[
                    serde_json::Value::from("cycle"),
                    serde_json::Value::from("pause"),
                ],
                Some(serde_json::json!({ "osd_message": "play/pause" })),
            )
            .map(|_| ()),
        MediaCommand::SeekRelative(seconds) => process
            .send_command(
                &[
                    serde_json::Value::from("seek"),
                    serde_json::Value::from(seconds),
                    serde_json::Value::from("relative"),
                ],
                Some(serde_json::json!({ "osd_message": format!("seek {seconds:+}s") })),
            )
            .map(|_| ()),
        MediaCommand::SetVolume(volume) => process
            .send_command(
                // mpv's volume property is percent-based (default 100); the
                // UI scale maps directly, never normalized to 0..1.
                &[
                    serde_json::Value::from("set_property"),
                    serde_json::Value::from("volume"),
                    serde_json::Value::from(volume),
                ],
                Some(serde_json::json!({ "osd_message": format!("volume {volume}%") })),
            )
            .map(|_| ()),
        MediaCommand::Stop => process
            .send_command(
                &[
                    serde_json::Value::from("seek"),
                    serde_json::Value::from(0),
                    serde_json::Value::from("absolute"),
                ],
                Some(serde_json::json!({ "osd_message": "restart" })),
            )
            .map(|_| ()),
    }
}
fn start_audio(
    sender: &SyncSender<Action>,
    session: u64,
    path: &Path,
) -> Result<
    (
        rodio::OutputStream,
        rodio::Sink,
        Option<f64>,
        Arc<AtomicBool>,
    ),
    String,
> {
    let source = tui_explorer::media::audio::SymphoniaSource::new(path)?;
    let duration = source.total_duration().map(|d| d.as_secs_f64());
    let (spectrum, snapshot) = tui_explorer::media::audio::SpectrumSource::new(source);
    let spectrum_stop = Arc::new(AtomicBool::new(false));
    let stopped = spectrum_stop.clone();
    let status_sender = sender.clone();
    // One publisher thread reads coherent snapshots at ~10 Hz and forwards
    // them through the bounded action channel. Dropped UI frames are fine;
    // the audio pull path is never blocked by the TUI. Position, pause, and
    // duration come from the sink itself via the supervisor tick: exactly
    // one authoritative playback clock exists.
    std::thread::Builder::new()
        .name("media-spectrum".into())
        .spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (bands, _position_ms) = snapshot.read();
                if status_sender
                    .send(Action::MediaSpectrum {
                        session,
                        spectrum: bands,
                    })
                    .is_err()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .map_err(|error| format!("cannot start spectrum thread: {error}"))?;
    let stream = rodio::OutputStreamBuilder::open_default_stream()
        .map_err(|error| format!("cannot open audio device: {error}"))?;
    let sink = rodio::Sink::connect_new(stream.mixer());
    sink.append(spectrum);
    sink.pause();
    Ok((stream, sink, duration, spectrum_stop))
}

fn apply_audio_command(
    sender: &SyncSender<Action>,
    session: u64,
    sink: &rodio::Sink,
    duration: Option<f64>,
    command: tui_explorer::media::MediaCommand,
) {
    use tui_explorer::media::MediaCommand;
    match command {
        MediaCommand::Load | MediaCommand::Quit => {}
        MediaCommand::TogglePause => {
            // A real toggle against the sink's own pause state.
            if sink.is_paused() {
                sink.play();
            } else {
                sink.pause();
            }
            report_status(sender, session, sink, duration);
        }
        MediaCommand::SeekRelative(seconds) => {
            let target = (sink.get_pos().as_secs_f64() + seconds as f64).max(0.0);
            let _ = sink.try_seek(Duration::from_secs_f64(target));
            report_status(sender, session, sink, duration);
        }
        MediaCommand::SetVolume(volume) => {
            // rodio's mixer volume IS normalized to 0..1.
            sink.set_volume(f32::from(volume) / 100.0);
            report_status(sender, session, sink, duration);
        }
        MediaCommand::Stop => {
            let _ = sink.try_seek(Duration::ZERO);
            sink.play();
            report_status(sender, session, sink, duration);
        }
    }
}

/// Publishes the sink's real state; the phase always derives from the sink,
/// never from what the UI last believed.
fn report_status(
    sender: &SyncSender<Action>,
    session: u64,
    sink: &rodio::Sink,
    duration: Option<f64>,
) {
    let phase = if sink.is_paused() {
        tui_explorer::media::MediaPhase::Paused
    } else {
        tui_explorer::media::MediaPhase::Playing
    };
    let _ = sender.send(Action::MediaStatus {
        session,
        phase,
        position: sink.get_pos().as_secs_f64(),
        duration,
        volume: volume_percent(sink.volume()),
    });
}

fn volume_percent(volume: f32) -> u8 {
    (volume * 100.0).round().clamp(0.0, 100.0) as u8
}

impl ProdHandler {
    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn snapshot(&self, path: &Path) -> Result<DirectorySnapshot, String> {
        let raw = self
            .fs
            .read_dir(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let paths: Vec<PathBuf> = raw.iter().map(|e| e.path.clone()).collect();
        let (tag_map, defs) = match &self.tags {
            Some(store) => (
                store.tags_for_paths(&paths).map_err(|e| e.to_string())?,
                store.list_tags().map_err(|e| e.to_string())?,
            ),
            None => (std::collections::HashMap::new(), Vec::new()),
        };
        let entries = raw
            .into_iter()
            .map(|entry| {
                let tags = tag_map.get(&entry.path).cloned().unwrap_or_default();
                EntryView { entry, tags }
            })
            .collect();
        Ok(DirectorySnapshot {
            path: path.to_path_buf(),
            entries,
            defs,
        })
    }

    fn tag_store_error(&self) -> Vec<Action> {
        vec![Action::ErrorMessage(
            "tag database unavailable in this session".to_string(),
        )]
    }
}

impl EffectHandler for ProdHandler {
    fn handle(&mut self, effect: Effect) -> Vec<Action> {
        match effect {
            Effect::LoadDirectory(path) => vec![Action::DirectoryLoaded {
                result: self.snapshot(&path),
            }],
            Effect::RunOperation(plan) => {
                let exists = |p: &Path| self.mutations.exists(p);
                let conflicts = find_conflicts(&plan, &exists);
                if !conflicts.is_empty() && plan.policy == ConflictPolicy::Ask {
                    return vec![Action::ConflictsFound { plan, conflicts }];
                }
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let report =
                        run_operation(&plan, &RealMutations::new(), |current, done, total| {
                            let _ = sender.try_send(Action::OperationProgress {
                                current,
                                done,
                                total,
                            });
                        });
                    let _ = sender.send(Action::OperationFinished { report });
                });
                Vec::new()
            }
            Effect::RunRename(plan) => {
                let sender = self.sender.clone();
                std::thread::spawn(move || match run_rename(&plan, &RealMutations::new()) {
                    Ok((from, to)) => {
                        let report = tui_explorer::operations::OperationReport {
                            results: vec![tui_explorer::operations::OpEntryResult {
                                source: from.clone(),
                                outcome: tui_explorer::operations::OpOutcome::Done,
                            }],
                            moves: vec![(from, to)],
                        };
                        let _ = sender.send(Action::OperationFinished { report });
                    }
                    Err(err) => {
                        let _ = sender.send(Action::ErrorMessage(err));
                    }
                });
                Vec::new()
            }
            Effect::LoadPreview { key, name, is_dir } => {
                // Decode/resize off the render loop; the reducer drops stale results.
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let result = tui_explorer::preview::load(&key.0, is_dir, &name);
                    let _ = sender.send(Action::PreviewLoaded { key, result });
                });
                Vec::new()
            }
            Effect::Crypto {
                kind,
                target,
                password,
            } => {
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let secret = age::secrecy::SecretString::from(password.0.clone());
                    let (done, failed) = tui_explorer::crypto::run_job(
                        kind,
                        std::slice::from_ref(&target),
                        &secret,
                        &cancel,
                        &mut |_, _, _| {},
                    );
                    let _ = sender.send(Action::CryptoFinished {
                        done,
                        failed: failed
                            .into_iter()
                            .map(|(p, e)| (p, e.to_string()))
                            .collect(),
                    });
                });
                Vec::new()
            }
            Effect::ToggleBookmark(path) => {
                let mut bookmarks = self.bookmarks.load();
                match self.bookmarks.toggle(&mut bookmarks, &path) {
                    Ok(added) => vec![Action::BookmarksChanged {
                        bookmarks,
                        message: if added {
                            format!("bookmarked {}", path.display())
                        } else {
                            format!("removed bookmark {}", path.display())
                        },
                    }],
                    Err(e) => vec![Action::ErrorMessage(format!(
                        "could not save bookmarks: {e}"
                    ))],
                }
            }
            Effect::OpenPathWith { .. } => Vec::new(),
            Effect::CreateEntry { path, is_dir } => {
                let result = if is_dir {
                    self.mutations.create_dir(&path)
                } else {
                    self.mutations.create_file(&path)
                };
                match result {
                    Ok(()) => {
                        let parent = path.parent().map(Path::to_path_buf).unwrap_or(path);
                        vec![Action::DirectoryLoaded {
                            result: self.snapshot(&parent),
                        }]
                    }
                    Err(e) => vec![Action::ErrorMessage(format!(
                        "could not create {}: {e}",
                        path.display()
                    ))],
                }
            }
            Effect::TagAssign {
                name,
                paths,
                create,
            } => {
                let Some(store) = &mut self.tags else {
                    return self.tag_store_error();
                };
                let now = Self::now();
                let result = if create {
                    store.tag_paths(&paths, &name, now)
                } else {
                    match store.find_tag(&name) {
                        Ok(Some(_)) => store.tag_paths(&paths, &name, now),
                        Ok(None) => {
                            return vec![Action::ErrorMessage(format!("tag not found: {name}"))];
                        }
                        Err(e) => return vec![Action::ErrorMessage(e.to_string())],
                    }
                };
                match result {
                    Ok(count) => vec![Action::TagsApplied {
                        message: format!(
                            "tagged {count} entr{} with [{name}]",
                            if count == 1 { "y" } else { "ies" }
                        ),
                        last_tag: Some(name),
                    }],
                    Err(e) => vec![Action::ErrorMessage(e.to_string())],
                }
            }
            Effect::TagUnassign { name, paths } => {
                let Some(store) = &mut self.tags else {
                    return self.tag_store_error();
                };
                match store.untag_paths(&paths, &name) {
                    Ok(count) => vec![Action::TagsApplied {
                        message: format!(
                            "untagged {count} entr{} from [{name}]",
                            if count == 1 { "y" } else { "ies" }
                        ),
                        last_tag: Some(name),
                    }],
                    Err(e) => vec![Action::ErrorMessage(e.to_string())],
                }
            }
            Effect::TagCreate(name) => {
                let Some(store) = &mut self.tags else {
                    return self.tag_store_error();
                };
                match store.create_tag(&name, Self::now()) {
                    Ok(_) => vec![Action::TagsApplied {
                        message: format!("created tag [{name}]"),
                        last_tag: Some(name),
                    }],
                    Err(e) => vec![Action::ErrorMessage(e.to_string())],
                }
            }
            Effect::TagDelete(name) => {
                let Some(store) = &mut self.tags else {
                    return self.tag_store_error();
                };
                match store.delete_tag(&name) {
                    Ok(()) => vec![Action::TagsApplied {
                        message: format!("deleted tag [{name}]"),
                        last_tag: None,
                    }],
                    Err(e) => vec![Action::ErrorMessage(e.to_string())],
                }
            }
            Effect::TagMove { from, to } => {
                let Some(store) = &mut self.tags else {
                    return self.tag_store_error();
                };
                match store.move_path(&from, &to, Self::now()) {
                    Ok(_) => Vec::new(),
                    Err(e) => vec![Action::ErrorMessage(e.to_string())],
                }
            }
            Effect::StartMedia {
                session,
                path,
                kind,
                surface,
                resume_position,
                resume_paused,
            } => {
                self.media
                    .start(session, path, kind, surface, resume_position, resume_paused);
                Vec::new()
            }
            Effect::MediaCommand { session, command } => {
                self.media.command(session, command);
                Vec::new()
            }
            Effect::StopMedia { session } => {
                self.media.stop(session);
                Vec::new()
            }
            Effect::Quit => Vec::new(),
        }
    }
}

fn image_protocol(value: &str) -> ratatui_image::picker::ProtocolType {
    use ratatui_image::picker::ProtocolType;
    match value.trim().to_ascii_lowercase().as_str() {
        "kitty" => ProtocolType::Kitty,
        "sixel" => ProtocolType::Sixel,
        "iterm2" => ProtocolType::Iterm2,
        "halfblocks" => ProtocolType::Halfblocks,
        _ => ProtocolType::Halfblocks,
    }
}

fn detect_picker_with<E, F>(override_: Option<&str>, query: F) -> ratatui_image::picker::Picker
where
    F: FnOnce() -> Result<ratatui_image::picker::Picker, E>,
{
    use ratatui_image::picker::{Picker, ProtocolType};
    let mut picker = match query() {
        Ok(picker) => picker,
        Err(_) => {
            let mut fallback = Picker::from_fontsize((8, 16));
            fallback.set_protocol_type(ProtocolType::Halfblocks);
            fallback
        }
    };
    if let Some(value) = override_ {
        picker.set_protocol_type(image_protocol(value));
    }
    picker
}

fn detect_picker() -> ratatui_image::picker::Picker {
    let override_ = std::env::var("TUI_EXPLORER_IMAGE_PROTOCOL").ok();
    detect_picker_with(
        override_.as_deref(),
        ratatui_image::picker::Picker::from_query_stdio,
    )
}

/// Runs the user-supplied program from the interactive "open with" prompt
/// (or the `:open-with`/`:ow` command) against `path`.
fn open_external_with(
    session: &mut TerminalSession<CrosstermTty>,
    path: &Path,
    program: &str,
    args: &[String],
) -> Option<Action> {
    if session.suspend().is_err() {
        return Some(Action::OpenFailed(
            "could not suspend terminal for editor".to_string(),
        ));
    }
    let status = std::process::Command::new(program)
        .args(args)
        .arg(path)
        .status();
    let resume = session.resume();
    match (status, resume) {
        (Err(e), _) => Some(Action::OpenFailed(format!(
            "could not start {program}: {e}"
        ))),
        (Ok(s), _) if !s.success() => {
            Some(Action::OpenFailed(format!("{program} exited with {s}")))
        }
        (Ok(_), Err(e)) => Some(Action::OpenFailed(format!(
            "could not restore terminal: {e}"
        ))),
        (Ok(_), Ok(())) => None,
    }
}

/// Maps crossterm mouse events onto reducer actions. Movement is mapped so
/// context-menu hover and marquee dragging can track the pointer; plain
/// movement carries the ctrl modifier for additive marquee selection.
fn map_mouse(mouse: crossterm::event::MouseEvent) -> Option<Action> {
    // Double-click detection lives in the reducer so it can require the same
    // entry (not just the same cell) and use the configured threshold.
    let kind = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => MouseKind::Left,
        MouseEventKind::Down(MouseButton::Right) => MouseKind::Right,
        MouseEventKind::ScrollUp => MouseKind::ScrollUp,
        MouseEventKind::ScrollDown => MouseKind::ScrollDown,
        MouseEventKind::Drag(MouseButton::Left) => MouseKind::LeftDrag,
        MouseEventKind::Up(MouseButton::Left) => MouseKind::LeftUp,
        MouseEventKind::Moved => MouseKind::Moved,
        _ => return None,
    };
    Some(Action::Mouse {
        kind,
        x: mouse.column,
        y: mouse.row,
        ctrl: mouse.modifiers.contains(KeyModifiers::CONTROL),
    })
}

struct Args {
    path: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Args>, ExitCode> {
    let mut path = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
            }
            "-V" | "--version" => {
                println!("tui-explorer {VERSION}");
                return Err(ExitCode::SUCCESS);
            }
            _ if arg.starts_with('-') => {
                eprintln!("unknown option: {arg}");
                eprintln!("try --help");
                return Err(ExitCode::FAILURE);
            }
            _ => {
                if path.is_some() {
                    eprintln!("too many arguments");
                    return Err(ExitCode::FAILURE);
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    Ok(Some(Args { path }))
}

fn init_logging(dirs: &config::XdgDirs) {
    let log = config::log_path(dirs);
    if config::ensure_private_parent(&log).is_err() {
        return;
    }
    let Ok(file) = std::fs::File::create(&log) else {
        return;
    };
    // Library noise (e.g. rodio's stream-drop notice, ALSA chatter) writes
    // to stderr. This app renders straight onto that same terminal without
    // an alternate screen, so any stderr write would corrupt the grid and
    // survive Ratatui's diff-based redraws. Point stderr at the private log
    // instead; panics still land there through the panic hook.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::io::AsRawFd;
        if libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) < 0 {
            // Best effort only; logging continues below regardless.
        }
    }
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .ok();
}

fn drain_channel(rx: &Receiver<Action>, pending: &mut VecDeque<Action>) {
    while let Ok(action) = rx.try_recv() {
        pending.push_back(action);
    }
}

fn run(start: PathBuf) -> std::io::Result<()> {
    let dirs = config::resolve(&|key| std::env::var(key).ok());
    init_logging(&dirs);
    let home = std::env::var("HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let db_path = config::database_path(&dirs);
    let mut startup_error: Option<String> = None;
    let tags = match config::ensure_private_parent(&db_path)
        .map_err(|e| e.to_string())
        .and_then(|()| TagStore::open(&db_path).map_err(|e| e.to_string()))
    {
        Ok(store) => Some(store),
        Err(err) => {
            startup_error = Some(format!(
                "tag database unavailable ({err}), tags will not persist this session"
            ));
            TagStore::open_in_memory().ok()
        }
    };
    let picker = detect_picker();
    let mut session = TerminalSession::enter(CrosstermTty::new())?;
    terminal::install_panic_hook();
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut term = Terminal::new(backend)?;
    let (sender, receiver) = sync_channel::<Action>(64);
    let bookmark_store = tui_explorer::sidebar::BookmarkStore::new(config::bookmarks_path(&dirs));
    let bookmarks = bookmark_store.load();
    let mut handler = ProdHandler {
        fs: RealFileSystem::new(),
        mutations: RealMutations::new(),
        tags,
        bookmarks: bookmark_store,
        media: MediaSupervisor::new(sender.clone(), picker.protocol_type()),
        sender,
    };
    let mut state = AppState::new(start, home);
    state.picker = picker;
    state.bookmarks = bookmarks;
    state.mounts = tui_explorer::sidebar::read_mounts();
    if let Some(ms) = std::env::var("TUI_EXPLORER_DOUBLE_CLICK_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        state.double_click = Duration::from_millis(ms);
    }
    if let Some(err) = startup_error {
        state.set_error(err);
    }
    let mut pending: VecDeque<Action> = VecDeque::new();
    pending.push_back(Action::LoadInitial);
    let mut redraw = terminal::RedrawGate::new();
    let mut had_media = false;
    loop {
        // Video no longer freezes the TUI: Ratatui keeps drawing modal chrome
        // while mpv paints frames into the reserved rectangle. Diffing leaves
        // stable surface cells untouched, so video survives every redraw; a
        // full clear only happens when ownership changes (see handback below).
        let video_owns = state.media_owns_terminal();
        if redraw.take_full() {
            term.clear()?;
        }
        term.draw(|frame| ui::render(frame, &mut state))?;
        if let Mode::Media(media) = &state.mode
            && media.awaiting_surface_ready
            && let Some(surface) = media.surface
        {
            pending.push_back(Action::MediaSurfaceReady {
                session: media.session,
                surface,
            });
        }
        drain_channel(&receiver, &mut pending);
        if pending.is_empty() {
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.code == KeyCode::Char('l')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            redraw.request_full();
                        } else if let Some(action) = map_key(key, &state) {
                            pending.push_back(action);
                        }
                    }
                    Event::Mouse(mouse) => {
                        if let Some(action) = map_mouse(mouse) {
                            pending.push_back(action);
                        }
                    }
                    Event::Resize(width, height) => {
                        pending.push_back(Action::Resize { width, height });
                    }
                    _ => {}
                }
            }
            drain_channel(&receiver, &mut pending);
        }
        let epoch_before = state.error_epoch;
        while let Some(action) = pending.pop_front() {
            let preview_loaded = matches!(&action, Action::PreviewLoaded { .. });
            let effects = reduce(&mut state, action);
            for effect in effects {
                match effect {
                    Effect::Quit => {
                        state.should_quit = true;
                    }
                    Effect::OpenPathWith {
                        path,
                        program,
                        args,
                    } => {
                        let follow = open_external_with(&mut session, &path, &program, &args);
                        // The child ran regardless of success: ratatui's
                        // cell buffer is stale now, force a full repaint.
                        redraw.request_full();
                        if let Some(action) = follow {
                            pending.push_back(action);
                        }
                    }
                    other => {
                        for follow in handler.handle(other) {
                            pending.push_back(follow);
                        }
                    }
                }
            }
            if preview_loaded {
                redraw.request_full();
            }
        }
        if state.error_epoch != epoch_before {
            redraw.request_full();
        }
        // When any media session releases the terminal (stop, error, EOF,
        // close), force a full redraw: mpv pixels must be overwritten by
        // Ratatui, and library teardown output may have dirtied the grid.
        let media_open_now = matches!(state.mode, Mode::Media(_));
        if (video_owns && !state.media_owns_terminal()) || (had_media && !media_open_now) {
            // Kitty images live on a graphics layer above the text grid:
            // `clear` cannot remove them. Broadcast a protocol-level delete
            // of all placements so no frame lingers under the redrawn UI.
            use std::io::Write;
            let _ = std::io::stdout()
                .write_all(b"\x1b_Ga=d,q=1\x1b\\")
                .and_then(|_| std::io::stdout().flush());
            redraw.request_full();
        }
        had_media = media_open_now;
        if state.should_quit {
            break;
        }
    }
    session.restore();
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::FAILURE,
        Err(code) => return code,
    };
    let start = match &args.path {
        Some(path) => path.clone(),
        None => match std::env::current_dir() {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("cannot read current directory: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    match run(start) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tui-explorer failed: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_picker_with, image_protocol};
    use ratatui_image::picker::{Picker, ProtocolType};

    fn picker(protocol: ProtocolType, font_size: (u16, u16)) -> Picker {
        let mut picker = Picker::from_fontsize(font_size);
        picker.set_protocol_type(protocol);
        picker
    }

    #[test]
    fn picker_query_success_is_preserved_without_override() {
        let picker =
            detect_picker_with(None, || Ok::<_, &str>(picker(ProtocolType::Kitty, (9, 18))));
        assert_eq!(picker.protocol_type(), ProtocolType::Kitty);
        assert_eq!(picker.font_size(), (9, 18));
    }

    #[test]
    fn picker_query_error_uses_halfblock_fallback_geometry() {
        let picker = detect_picker_with(None, || Err::<Picker, _>("query failed"));
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);
        assert_eq!(picker.font_size(), (8, 16));
    }

    #[test]
    fn image_protocol_honors_supported_overrides() {
        for (value, expected) in [
            ("halfblocks", ProtocolType::Halfblocks),
            ("kitty", ProtocolType::Kitty),
            ("sixel", ProtocolType::Sixel),
            ("iterm2", ProtocolType::Iterm2),
            (" KITTY ", ProtocolType::Kitty),
        ] {
            let picker = detect_picker_with(Some(value), || {
                Ok::<_, &str>(picker(ProtocolType::Sixel, (7, 14)))
            });
            assert_eq!(picker.protocol_type(), expected, "override {value}");
        }
    }

    #[test]
    fn invalid_override_forces_halfblocks_after_successful_query() {
        for value in ["", "unknown"] {
            let picker = detect_picker_with(Some(value), || {
                Ok::<_, &str>(picker(ProtocolType::Kitty, (9, 18)))
            });
            assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);
        }
        assert_eq!(image_protocol("invalid"), ProtocolType::Halfblocks);
    }
}
