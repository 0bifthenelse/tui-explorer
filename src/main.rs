use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

use crossterm::event::{self, Event, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use tui_explorer::app::action::{Action, DirectorySnapshot, MouseKind};
use tui_explorer::app::effects::{Effect, EffectHandler};
use tui_explorer::app::reduce::reduce;
use tui_explorer::app::state::AppState;
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
    F5               refresh current directory
    e, Enter         enter folder, or choose a command to open a file
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
            Effect::Quit => Vec::new(),
        }
    }
}

/// Choose a graphics protocol without emitting terminal capability queries.
/// Native protocols are deliberately opt-in: incorrect terminal detection or
/// pixel geometry can overdraw the surrounding TUI, while half-blocks stay
/// inside Ratatui's cell buffer on every terminal.
fn image_protocol(override_: Option<&str>) -> ratatui_image::picker::ProtocolType {
    use ratatui_image::picker::ProtocolType;
    match override_
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("kitty") => ProtocolType::Kitty,
        Some("sixel") => ProtocolType::Sixel,
        Some("iterm2") => ProtocolType::Iterm2,
        Some("halfblocks") => ProtocolType::Halfblocks,
        _ => ProtocolType::Halfblocks,
    }
}

fn detect_picker() -> ratatui_image::picker::Picker {
    use ratatui_image::picker::Picker;
    let mut picker = Picker::from_fontsize((8, 16));
    let override_ = std::env::var("TUI_EXPLORER_IMAGE_PROTOCOL").ok();
    picker.set_protocol_type(image_protocol(override_.as_deref()));
    picker
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

fn map_mouse(kind: MouseEventKind, x: u16, y: u16) -> Option<Action> {
    // Double-click detection lives in the reducer so it can require the same
    // entry (not just the same cell) and use the configured threshold.
    let kind = match kind {
        MouseEventKind::Down(MouseButton::Left) => MouseKind::Left,
        MouseEventKind::Down(MouseButton::Right) => MouseKind::Right,
        MouseEventKind::ScrollUp => MouseKind::ScrollUp,
        MouseEventKind::ScrollDown => MouseKind::ScrollDown,
        _ => return None,
    };
    Some(Action::Mouse { kind, x, y })
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
                return Err(ExitCode::SUCCESS);
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
    let mut session = TerminalSession::enter(CrosstermTty::new())?;
    terminal::install_panic_hook();
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut term = Terminal::new(backend)?;
    // Use the cell-based renderer unless a native graphics protocol is
    // explicitly selected. This is deterministic and never emits protocol
    // data into terminals that did not opt in.
    let picker = detect_picker();
    let (sender, receiver) = sync_channel::<Action>(64);
    let bookmark_store = tui_explorer::sidebar::BookmarkStore::new(config::bookmarks_path(&dirs));
    let bookmarks = bookmark_store.load();
    let mut handler = ProdHandler {
        fs: RealFileSystem::new(),
        mutations: RealMutations::new(),
        tags,
        bookmarks: bookmark_store,
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
    loop {
        if redraw.take_full() {
            term.clear()?;
        }
        term.draw(|frame| ui::render(frame, &mut state))?;
        drain_channel(&receiver, &mut pending);
        if pending.is_empty() {
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) => {
                        if let Some(action) = map_key(key, &state) {
                            pending.push_back(action);
                        }
                    }
                    Event::Mouse(mouse) => {
                        if let Some(action) = map_mouse(mouse.kind, mouse.column, mouse.row) {
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
    use super::image_protocol;
    use ratatui_image::picker::ProtocolType;

    #[test]
    fn image_protocol_defaults_to_halfblocks() {
        assert_eq!(image_protocol(None), ProtocolType::Halfblocks);
        assert_eq!(image_protocol(Some("")), ProtocolType::Halfblocks);
        assert_eq!(image_protocol(Some("unknown")), ProtocolType::Halfblocks);
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
            assert_eq!(image_protocol(Some(value)), expected, "override {value}");
        }
    }
}
