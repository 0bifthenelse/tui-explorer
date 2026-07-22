use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};

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
use tui_explorer::input::keymap::{ClickTracker, map_key};
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
    h/l, Enter       parent / enter or open
    g g, G           first / last entry
    Ctrl-u/Ctrl-d    half page up/down
    Space, v         select, visual mode
    .                toggle hidden files
    t, T             quick tag / tag picker
    :                command mode (:copy :move :rename :delete :tag :untag :tags :open :cd :quit :help)
    ?                help overlay
    q                quit

MOUSE:
    click select, double click open, right click menu, wheel scroll, breadcrumb navigates

DATA:
    tags database: $XDG_DATA_HOME/tui-explorer/tags.sqlite3
    fallback:      $HOME/.local/share/tui-explorer/tags.sqlite3
";

struct ProdHandler {
    fs: RealFileSystem,
    mutations: RealMutations,
    tags: Option<TagStore>,
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
            Effect::OpenPath(_) => Vec::new(),
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

fn open_external(session: &mut TerminalSession<CrosstermTty>, path: &Path) -> Option<Action> {
    let opener = std::env::var("TUI_EXPLORER_OPENER")
        .ok()
        .filter(|v| !v.is_empty());
    let editor = std::env::var("EDITOR").ok().filter(|v| !v.is_empty());
    if let Some(program) = opener.or(editor) {
        if session.suspend().is_err() {
            return Some(Action::OpenFailed(
                "could not suspend terminal for editor".to_string(),
            ));
        }
        let status = std::process::Command::new(&program).arg(path).status();
        let resume = session.resume();
        let result = match (status, resume) {
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
        };
        return result;
    }
    match std::process::Command::new("xdg-open").arg(path).spawn() {
        Ok(_) => None,
        Err(e) => Some(Action::OpenFailed(format!("could not start xdg-open: {e}"))),
    }
}

fn map_mouse(kind: MouseEventKind, x: u16, y: u16, tracker: &mut ClickTracker) -> Option<Action> {
    let kind = match kind {
        MouseEventKind::Down(MouseButton::Left) => tracker.register(Instant::now(), x, y),
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
    let (sender, receiver) = sync_channel::<Action>(64);
    let mut handler = ProdHandler {
        fs: RealFileSystem::new(),
        mutations: RealMutations::new(),
        tags,
        sender,
    };
    let mut state = AppState::new(start, home);
    if let Some(err) = startup_error {
        state.message = Some(tui_explorer::app::state::StatusMessage::error(err));
    }
    let mut tracker = ClickTracker::new();
    let mut pending: VecDeque<Action> = VecDeque::new();
    pending.push_back(Action::LoadInitial);
    loop {
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
                        if let Some(action) =
                            map_mouse(mouse.kind, mouse.column, mouse.row, &mut tracker)
                        {
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
        while let Some(action) = pending.pop_front() {
            let effects = reduce(&mut state, action);
            for effect in effects {
                match effect {
                    Effect::Quit => {
                        state.should_quit = true;
                    }
                    Effect::OpenPath(path) => {
                        if let Some(action) = open_external(&mut session, &path) {
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
