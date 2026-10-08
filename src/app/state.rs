use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::browser::{Browser, EntryView};
use crate::media::{AfterStop, MediaKind, MediaPhase};
use crate::operations::{OperationKind, OperationPlan};
use crate::settings::Settings;
use crate::sidebar::{MountInfo, SidebarItem};
use crate::tags::TagDef;
use crate::ui::anim::Animator;
use crate::ui::hit::HitMap;

/// Default double-click threshold in milliseconds. Can be overridden with the
/// `TUI_EXPLORER_DOUBLE_CLICK_MS` environment variable (see config module).
pub const DEFAULT_DOUBLE_CLICK_MS: u64 = 500;

/// A secret string that never leaks through Debug output.
#[derive(Clone)]
pub struct Password(pub String);

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "***")
    }
}

impl Password {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// What a password dialog is collecting the password for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordPurpose {
    /// Encrypt the target; password is entered twice for confirmation.
    Encrypt,
    /// Decrypt the target; password is entered once.
    Decrypt,
}

#[derive(Debug)]
pub struct PasswordState {
    pub purpose: PasswordPurpose,
    pub target: PathBuf,
    /// Current masked input buffer.
    pub input: String,
    /// First entry when confirming (encrypt only).
    pub first: Option<String>,
}

impl PasswordState {
    pub fn confirming(&self) -> bool {
        self.first.is_some()
    }
}

/// State for the interactive "open with" prompt (`r` key): asks which
/// program to run against the focused entry.
#[derive(Clone, Debug)]
pub struct OpenWithState {
    pub target: PathBuf,
    pub input: String,
    /// Openers detected on PATH for this file type (Tab cycles).
    pub suggestions: Vec<String>,
    pub suggestion: Option<usize>,
    /// Remember the command for this extension (Ctrl-R toggles).
    pub remember: bool,
}

/// One `:find` / `:grep` hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindHit {
    pub path: PathBuf,
    pub is_dir: bool,
    /// (1-based line number, line text) for content matches.
    pub line: Option<(usize, String)>,
}

/// Results of a recursive search, browsable with a fuzzy filter.
#[derive(Clone, Debug)]
pub struct ResultsState {
    pub title: String,
    pub root: PathBuf,
    pub hits: Vec<FindHit>,
    pub query: String,
    /// Indices into `hits` matching `query`.
    pub matches: Vec<usize>,
    pub selected: usize,
}

/// One destination in the bookmarks hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HubItem {
    /// Folder bookmark (`Ctrl-b`).
    Folder(PathBuf),
    /// Web link bookmark.
    Link(crate::urls::Link),
    /// Single-key mark (`m<key>`).
    Mark(char, PathBuf),
    /// Frequently / recently visited folder.
    Recent(PathBuf),
}

impl HubItem {
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            HubItem::Folder(p) | HubItem::Mark(_, p) | HubItem::Recent(p) => Some(p),
            HubItem::Link(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubSection {
    All,
    Folders,
    Links,
    Marks,
    Recent,
}

impl HubSection {
    pub const ALL: [HubSection; 5] = [
        HubSection::All,
        HubSection::Folders,
        HubSection::Links,
        HubSection::Marks,
        HubSection::Recent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HubSection::All => "All",
            HubSection::Folders => "Folders",
            HubSection::Links => "Links",
            HubSection::Marks => "Marks",
            HubSection::Recent => "Recent",
        }
    }
}

/// The bookmarks hub (`B`): folders, web links, marks and recent folders
/// behind one fuzzy search. `picker` restricts it to a one-off list of
/// links (URLs found in a file).
#[derive(Clone, Debug)]
pub struct BookmarkNavState {
    pub query: String,
    pub section: HubSection,
    /// Ranked items matching the query in the active section.
    pub matches: Vec<HubItem>,
    pub selected: usize,
    pub picker: Option<Vec<crate::urls::Link>>,
}

/// Decoded preview content for the focused entry.
pub enum PreviewContent {
    Text { lines: Vec<String>, truncated: bool },
    Image(Box<ratatui_image::protocol::StatefulProtocol>),
    Directory(Vec<String>),
    Unavailable(String),
}

#[derive(Default)]
pub struct PreviewState {
    /// Path the current content belongs to, with mtime+size for invalidation.
    pub key: Option<(PathBuf, i64, u64)>,
    pub content: Option<PreviewContent>,
}

#[derive(Clone, Debug)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

impl StatusMessage {
    pub fn info(text: impl Into<String>) -> Self {
        StatusMessage {
            text: text.into(),
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        StatusMessage {
            text: text.into(),
            is_error: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OperationState {
    pub kind: OperationKind,
    pub current: PathBuf,
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    Delete {
        plan: Box<OperationPlan>,
    },
    /// Apply a reviewed `:bulkrename`.
    BulkRename {
        pairs: Vec<(PathBuf, PathBuf)>,
    },
}

#[derive(Clone, Debug)]
pub struct ConfirmState {
    pub title: String,
    pub detail: String,
    pub stage: u8,
    pub recursive: bool,
    pub action: ConfirmAction,
}

#[derive(Clone, Debug)]
pub struct ConflictState {
    pub plan: Box<OperationPlan>,
    pub conflicts: Vec<(PathBuf, PathBuf)>,
}

#[derive(Clone, Debug)]
pub struct TagPickerState {
    pub defs: Vec<TagDef>,
    pub selected: usize,
    pub input: Option<String>,
    pub targets: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextItem {
    Open,
    OpenWith,
    Rename,
    Cut,
    ClipboardCopy,
    Paste,
    Delete,
    Tags,
}

impl ContextItem {
    pub fn label(&self) -> &'static str {
        match self {
            ContextItem::Open => "Open",
            ContextItem::OpenWith => "Open with",
            ContextItem::Rename => "Rename",
            ContextItem::Cut => "Cut",
            ContextItem::ClipboardCopy => "Copy",
            ContextItem::Paste => "Paste",
            ContextItem::Delete => "Delete",
            ContextItem::Tags => "Tags",
        }
    }

    /// The menu shown for a context target. `clipboard_has_items` enables
    /// the background Paste entry.
    pub fn menu_for(target: &ContextTarget, clipboard_has_items: bool) -> Vec<MenuItem> {
        let item = |action: ContextItem, enabled: bool| MenuItem { action, enabled };
        match target {
            ContextTarget::Single { .. } => vec![
                item(ContextItem::Open, true),
                item(ContextItem::OpenWith, true),
                item(ContextItem::Rename, true),
                item(ContextItem::Cut, true),
                item(ContextItem::ClipboardCopy, true),
                item(ContextItem::Delete, true),
                item(ContextItem::Tags, true),
            ],
            ContextTarget::Bulk { .. } => vec![
                item(ContextItem::Cut, true),
                item(ContextItem::ClipboardCopy, true),
                item(ContextItem::Delete, true),
            ],
            ContextTarget::Background => vec![item(ContextItem::Paste, clipboard_has_items)],
        }
    }
}

/// What a context menu was opened on; carries the explicit paths the menu
/// acts on so multi-selections never collapse into per-item semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextTarget {
    Single {
        path: PathBuf,
    },
    /// Sorted, deduped at open time.
    Bulk {
        paths: Vec<PathBuf>,
    },
    Background,
}

impl ContextTarget {
    /// Every path this target's menu would act on (empty for Background).
    pub fn paths(&self) -> Vec<PathBuf> {
        match self {
            ContextTarget::Single { path } => vec![path.clone()],
            ContextTarget::Bulk { paths } => paths.clone(),
            ContextTarget::Background => Vec::new(),
        }
    }

    /// Short title describing the target.
    pub fn title(&self) -> String {
        match self {
            ContextTarget::Single { path } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            ContextTarget::Bulk { paths } => format!("{} items selected", paths.len()),
            ContextTarget::Background => "here".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub action: ContextItem,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct ContextMenuState {
    pub target: ContextTarget,
    pub items: Vec<MenuItem>,
    pub selected: usize,
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// Left button pressed, not yet moved past the threshold.
    Armed,
    /// Past the threshold: a real drag in flight.
    Dragging,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarqueePhase {
    /// Left button pressed on empty background, not yet past the threshold.
    Armed,
    /// Past the threshold: the band selects everything it touches.
    Selecting,
}

/// Traditional explorer rectangle selection: a left press on empty grid
/// background arms a marquee; moving past the drag threshold selects every
/// visible tile whose rectangle intersects the rubber band. Kept separate
/// from `DragState`, which is exclusively file movement.
#[derive(Clone, Debug)]
pub struct MarqueeState {
    pub phase: MarqueePhase,
    /// Cell where the left button was pressed.
    pub origin: (u16, u16),
    /// Current pointer cell; the band is origin..current normalized.
    pub current: (u16, u16),
    /// Ctrl-additive mode: the selection snapshot taken at press time stays
    /// selected and the band adds to it.
    pub base: std::collections::BTreeSet<PathBuf>,
}

impl MarqueeState {
    pub fn armed(origin: (u16, u16), base: std::collections::BTreeSet<PathBuf>) -> Self {
        MarqueeState {
            phase: MarqueePhase::Armed,
            origin,
            current: origin,
            base,
        }
    }

    /// The normalized rubber-band rectangle in terminal cells.
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.origin.0.min(self.current.0),
            self.origin.1.min(self.current.1),
            self.origin.0.abs_diff(self.current.0) + 1,
            self.origin.1.abs_diff(self.current.1) + 1,
        )
    }
}

#[derive(Clone, Debug)]
pub struct DragState {
    pub phase: DragPhase,
    /// Cell where the left button was pressed.
    pub origin: (u16, u16),
    /// Snapshot of source paths taken at press time (stable against
    /// sorting/filtering changes during the drag).
    pub sources: Vec<PathBuf>,
    pub cursor: (u16, u16),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaState {
    pub session: u64,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub phase: MediaPhase,
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: u8,
    pub spectrum: [f32; 24],
    pub surface: Option<MediaSurface>,
    pub awaiting_surface_ready: bool,
    pub resume_position: Option<f64>,
    pub resume_paused: Option<bool>,
    pub after_stop: Option<AfterStop>,
    pub error: Option<String>,
    /// Video fullscreen flag; flips via the supervised stop/restart cycle.
    pub fullscreen: bool,
    /// Remaining playable tracks in display order (playlist navigation).
    pub playlist: Vec<PathBuf>,
    pub playlist_pos: usize,
    /// Hover position on the seek rail in seconds (None = not hovering).
    pub slider_hover: Option<f64>,
    /// Visual drag position in seconds while a rail drag is in flight.
    pub slider_drag_pos: Option<f64>,
    /// True between rail Left press and LeftUp; no seek until commit.
    pub slider_drag_active: bool,
    /// Muted (volume is kept so unmuting restores it).
    pub muted: bool,
    /// Queue playback order and repetition.
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Playback speed (1.0 = normal).
    pub speed: f64,
    /// External subtitle loaded into this session (re-applied after a
    /// fullscreen/resize restart).
    pub sub_file: Option<PathBuf>,
    /// Subtitles disabled for this session.
    pub subs_off: bool,
    /// Subtitle delay in seconds.
    pub sub_delay: f64,
    /// Subtitle picker overlay (`c` in the video player).
    pub sub_picker: Option<SubPickerState>,
    /// How video frames are painted (resolved when the session starts).
    pub backend: crate::media::VideoBackend,
    /// Title / artist / album from the file's tags (audio).
    pub tags: Option<crate::media::tags::TrackTags>,
}

/// Encoded cover art for one media session.
pub struct Cover {
    pub session: u64,
    pub image: Box<ratatui_image::protocol::StatefulProtocol>,
}

impl std::fmt::Debug for Cover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cover")
            .field("session", &self.session)
            .finish()
    }
}

/// Queue repetition mode (`r` in the player cycles it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn next(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Off => "repeat off",
            Repeat::All => "repeat all",
            Repeat::One => "repeat one",
        }
    }
}

/// One choice in the subtitle picker.
#[derive(Clone, Debug, PartialEq)]
pub enum SubChoice {
    Off,
    /// The video's own (embedded or auto-loaded) tracks: cycle through them.
    Embedded,
    File(crate::media::subs::SubtitleFile),
}

/// Subtitle picker: discovered files filtered by a fuzzy query.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubPickerState {
    pub query: String,
    /// Every candidate found near the video (ranked).
    pub found: Vec<crate::media::subs::SubtitleFile>,
    /// Visible choices for the current query.
    pub choices: Vec<SubChoice>,
    pub selected: usize,
    /// Discovery still running.
    pub searching: bool,
    /// Playback was paused to show the picker over the video.
    pub resume: bool,
}

impl MediaState {
    pub fn preparing(session: u64, path: PathBuf, kind: MediaKind) -> Self {
        Self::preparing_with_playlist(session, path, kind, Vec::new(), 0)
    }

    /// Fresh Preparing state carrying the playlist context used by
    /// next-track navigation. Resets all transient slider state.
    pub fn preparing_with_playlist(
        session: u64,
        path: PathBuf,
        kind: MediaKind,
        playlist: Vec<PathBuf>,
        playlist_pos: usize,
    ) -> Self {
        MediaState {
            session,
            path,
            kind,
            phase: MediaPhase::Preparing,
            position: 0.0,
            duration: None,
            volume: 100,
            spectrum: [0.0; 24],
            surface: None,
            awaiting_surface_ready: true,
            resume_position: None,
            resume_paused: None,
            after_stop: None,
            error: None,
            fullscreen: false,
            playlist,
            playlist_pos,
            slider_hover: None,
            slider_drag_pos: None,
            slider_drag_active: false,
            muted: false,
            shuffle: false,
            repeat: Repeat::Off,
            speed: 1.0,
            sub_file: None,
            subs_off: false,
            sub_delay: 0.0,
            sub_picker: None,
            backend: crate::media::VideoBackend::Kitty,
            tags: None,
        }
    }

    /// Carries the listener's preferences (volume, queue modes, speed)
    /// over to the next track's fresh state.
    pub fn inherit_preferences(&mut self, previous: &MediaState) {
        self.volume = previous.volume;
        self.muted = previous.muted;
        self.shuffle = previous.shuffle;
        self.repeat = previous.repeat;
        self.speed = previous.speed;
        self.backend = previous.backend;
        if self.backend.fullscreen_only() {
            self.fullscreen = true;
        }
    }

    pub fn title(&self) -> String {
        if let Some(title) = self.tags.as_ref().and_then(|t| t.title.clone()) {
            return title;
        }
        self.path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// Clears every transient seek-rail interaction state.
    pub fn clear_slider_state(&mut self) {
        self.slider_hover = None;
        self.slider_drag_pos = None;
        self.slider_drag_active = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipMode {
    Copy,
    Cut,
}

/// Internal clipboard for Copy/Cut/Paste semantics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipboardState {
    pub mode: Option<ClipMode>,
    pub items: Vec<PathBuf>,
}

impl ClipboardState {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Status-chip text ("COPY: 3 items"); None when the clipboard is empty.
    pub fn chip(&self) -> Option<String> {
        match (self.mode, self.items.len()) {
            (None, _) | (_, 0) => None,
            (Some(ClipMode::Copy), n) => {
                Some(format!("COPY: {n} item{}", if n == 1 { "" } else { "s" }))
            }
            (Some(ClipMode::Cut), n) => {
                Some(format!("CUT: {n} item{}", if n == 1 { "" } else { "s" }))
            }
        }
    }
}

/// Per-tab navigation history (`H` / `L`).
#[derive(Clone, Debug, Default)]
pub struct History {
    pub back: Vec<PathBuf>,
    pub forward: Vec<PathBuf>,
}

/// A browser tab. The active tab lives in `AppState::browser` and
/// `AppState::history`; its slot in `AppState::tabs` is a stale shell.
#[derive(Clone, Debug)]
pub struct Tab {
    pub browser: Browser,
    pub history: History,
}

/// One reversible step of an undoable job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoStep {
    /// The entry moved `from` → `to` (move, rename, trash): undo moves it
    /// back.
    Move { from: PathBuf, to: PathBuf },
    /// The job created this path (copy, symlink, mkdir, touch): undo
    /// trashes it.
    Created(PathBuf),
}

#[derive(Clone, Debug)]
pub struct UndoEntry {
    pub label: String,
    pub steps: Vec<UndoStep>,
}

/// Inline rename field.
#[derive(Clone, Debug)]
pub struct RenameState {
    pub target: PathBuf,
    pub edit: crate::input::line::LineEdit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKind {
    /// `/`: highlight matches and jump between them.
    Search,
    /// `f`: jump while typing; a unique match opens.
    Find,
    /// Ctrl-F / `zf`: hide non-matching names.
    Filter,
}

/// Live prompt for search / find / filter.
#[derive(Clone, Debug)]
pub struct SearchState {
    pub kind: SearchKind,
    pub edit: crate::input::line::LineEdit,
    /// Cursor position when the prompt opened (restored on Esc).
    pub origin: usize,
    /// Filter active when the prompt opened (restored on Esc).
    pub origin_filter: Option<String>,
}

/// Fullscreen quick look.
#[derive(Clone, Debug, Default)]
pub struct QuickLookState {
    pub scroll: usize,
}

#[derive(Default, Debug)]
pub struct HoverState {
    pub row: Option<usize>,
    pub control: Option<crate::ui::hit::HitTarget>,
}

#[derive(Debug)]
pub enum Mode {
    Browser,
    Command,
    Confirm(Box<ConfirmState>),
    Conflict(Box<ConflictState>),
    TagPicker(Box<TagPickerState>),
    ContextMenu(Box<ContextMenuState>),
    Password(Box<PasswordState>),
    OpenWith(Box<OpenWithState>),
    Bookmarks(Box<BookmarkNavState>),
    Help,
    Media(Box<MediaState>),
    Rename(Box<RenameState>),
    Search(Box<SearchState>),
    QuickLook(Box<QuickLookState>),
    Results(Box<ResultsState>),
}

impl Clone for Mode {
    fn clone(&self) -> Self {
        // Cloning a mode never carries over secret buffers.
        match self {
            Mode::Browser => Mode::Browser,
            Mode::Command => Mode::Command,
            Mode::Confirm(c) => Mode::Confirm(c.clone()),
            Mode::Conflict(c) => Mode::Conflict(c.clone()),
            Mode::TagPicker(p) => Mode::TagPicker(p.clone()),
            Mode::ContextMenu(m) => Mode::ContextMenu(m.clone()),
            Mode::Password(p) => Mode::Password(Box::new(PasswordState {
                purpose: p.purpose,
                target: p.target.clone(),
                input: String::new(),
                first: None,
            })),
            Mode::OpenWith(o) => Mode::OpenWith(o.clone()),
            Mode::Bookmarks(b) => Mode::Bookmarks(b.clone()),
            Mode::Help => Mode::Help,
            Mode::Media(media) => Mode::Media(media.clone()),
            Mode::Rename(r) => Mode::Rename(r.clone()),
            Mode::Search(s) => Mode::Search(s.clone()),
            Mode::QuickLook(q) => Mode::QuickLook(q.clone()),
            Mode::Results(r) => Mode::Results(r.clone()),
        }
    }
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Browser => "BROWSER",
            Mode::Command => "COMMAND",
            Mode::Confirm(_) => "CONFIRM",
            Mode::Conflict(_) => "CONFLICT",
            Mode::TagPicker(_) => "TAGS",
            Mode::ContextMenu(_) => "MENU",
            Mode::Password(_) => "CRYPTO",
            Mode::OpenWith(_) => "OPEN WITH",
            Mode::Bookmarks(_) => "BOOKMARKS",
            Mode::Help => "HELP",
            Mode::Media(_) => "MEDIA",
            Mode::Rename(_) => "RENAME",
            Mode::Search(s) => match s.kind {
                SearchKind::Search => "SEARCH",
                SearchKind::Find => "FIND",
                SearchKind::Filter => "FILTER",
            },
            Mode::QuickLook(_) => "QUICK LOOK",
            Mode::Results(_) => "RESULTS",
        }
    }

    /// Modal overlays (scrim, input captured, cursor hidden). Inline
    /// prompts (command line, rename, search) are not overlays: the
    /// listing stays live underneath them.
    pub fn is_overlay(&self) -> bool {
        !matches!(
            self,
            Mode::Browser | Mode::Command | Mode::Rename(_) | Mode::Search(_)
        )
    }
}

#[derive(Debug)]
pub struct AppState {
    pub browser: Browser,
    pub mode: Mode,
    pub command_input: String,
    pub message: Option<StatusMessage>,
    /// Bumped on every committed user-visible error; the event loop watches
    /// it to force a full redraw so the error is never lost to stale cells.
    pub error_epoch: u64,
    pub operation: Option<OperationState>,
    pub tag_defs: Vec<TagDef>,
    pub last_tag: Option<String>,
    pub should_quit: bool,
    /// Keys of an unfinished chord (`g`, `y`, `m`, ...).
    pub pending_keys: Vec<String>,
    /// Count prefix typed before a command (`5` in `5j`).
    pub pending_count: Option<usize>,
    /// Active tab's navigation history.
    pub history: History,
    /// The folder visited before the current one (`''`).
    pub previous_dir: Option<PathBuf>,
    /// All tabs; the active slot's contents are stale (see [`Tab`]).
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    /// Recently closed tabs (`uq` restores).
    pub closed_tabs: Vec<Tab>,
    /// Undo journal, newest last.
    pub undo: Vec<UndoEntry>,
    /// Cached sizes from `du`.
    pub dir_sizes: std::collections::HashMap<PathBuf, u64>,
    /// Web link bookmarks (`links.tsv`).
    pub links: Vec<crate::urls::Link>,
    /// mpv is installed (video, streams, extra audio codecs).
    pub mpv_available: bool,
    /// Path candidates for the command line's current argument.
    pub completions: Vec<String>,
    /// Index into the candidates while Tab cycles.
    pub completion_index: Option<usize>,
    /// The last input change came from Tab cycling (keeps the index).
    pub completion_cycle: bool,
    /// Position while walking command history with Up/Down.
    pub history_cursor: Option<usize>,
    /// What was typed before walking history.
    pub command_draft: String,
    /// A GUI display is available (`$DISPLAY` / `$WAYLAND_DISPLAY`).
    pub has_display: bool,
    /// Cover art of the playing track, encoded for the terminal, keyed by
    /// media session.
    pub cover: Option<Cover>,
    /// Background player (audio keeps playing while browsing); the
    /// expanded player lives in `Mode::Media`.
    pub mini: Option<Box<MediaState>>,
    /// The command line is acting as the address bar (`Ctrl-L`, path bar
    /// click, pasted path): it edits a `cd` target shown in the path bar.
    pub address_bar: bool,
    /// Help overlay: filter text and scroll offset.
    pub help_query: String,
    pub help_scroll: usize,
    pub width: u16,
    pub height: u16,
    /// Number of entries visible per page in the current layout.
    pub list_viewport: usize,
    /// Number of tile columns in the grid layout (1 in narrow layouts).
    pub grid_cols: usize,
    pub home: PathBuf,
    pub pending_nav: Option<PathBuf>,
    /// Entry to focus once the pending directory listing arrives (cursor
    /// returns to the child after going up, `:find` jumps, ...).
    pub pending_focus: Option<PathBuf>,
    pub hit_map: HitMap,
    /// Last single click on a grid row, for same-entry double-click detection.
    pub last_click: Option<(Instant, usize)>,
    pub double_click: Duration,
    /// Sidebar visibility: None = automatic per terminal size.
    pub show_sidebar: Option<bool>,
    /// In-flight mouse drag; None when idle.
    pub drag: Option<DragState>,
    /// In-flight background marquee selection; None when idle.
    pub marquee: Option<MarqueeState>,
    /// Preview panel visibility: None = automatic per terminal size.
    pub show_preview: Option<bool>,
    /// Sidebar entries in render order; rebuilt every frame.
    pub sidebar_items: Vec<SidebarItem>,
    /// Miller parent-pane rows in render order; rebuilt every frame.
    pub parent_rows: Vec<PathBuf>,
    /// Device mounts captured once at startup (never re-read per frame).
    pub mounts: Vec<MountInfo>,
    pub bookmarks: Vec<PathBuf>,
    pub preview: PreviewState,
    pub picker: ratatui_image::picker::Picker,
    pub next_media_session: u64,
    pub clipboard: ClipboardState,
    /// True hover state (row and/or control target); rebuilt per frame.
    pub hover: HoverState,
    /// Mode of an in-flight paste started from the context menu; consumed
    /// by operation_finished to prune moved sources out of the clipboard.
    pub pending_paste_mode: Option<ClipMode>,
    /// Frame clock: set by the event loop before each reduce/draw pass.
    pub now: Instant,
    /// Wall clock in unix seconds (relative timestamps in listings).
    pub wall_clock: i64,
    /// Implicit-animation engine driving hover/selection/cascade motion.
    pub anim: Animator,
    /// Persisted preferences (layout, appearance, associations, marks).
    pub settings: Settings,
    /// Set when `settings` changed and should be written back.
    pub settings_dirty: bool,
    /// Directory listings used by the Miller-columns parent pane.
    pub side_listings: std::collections::HashMap<PathBuf, Vec<EntryView>>,
    /// Terminal supports 24-bit color; otherwise a final pass maps the
    /// frame onto the 256-color palette.
    pub truecolor: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaSurface {
    pub rect: Rect,
    pub terminal_cells: (u16, u16),
    pub cell_pixels: (u16, u16),
}

impl AppState {
    /// True while a video surface may be live on the terminal: mpv paints
    /// frames straight to stdout. Ratatui keeps drawing modal chrome around
    /// the surface, but the moment this flips false the event loop forces a
    /// full redraw so no stale graphics remain.
    pub fn media_owns_terminal(&self) -> bool {
        matches!(
            &self.mode,
            Mode::Media(media)
                if media.kind == crate::media::MediaKind::Video
                    && media.backend.in_terminal()
                    && matches!(
                        media.phase,
                        MediaPhase::Starting
                            | MediaPhase::Playing
                            | MediaPhase::Paused
                            | MediaPhase::Stopped
                            | MediaPhase::Stopping
                    )
        )
    }

    /// Text-mode video is painting the whole terminal right now: the TUI
    /// must not draw until it pauses or stops.
    pub fn text_video_live(&self) -> bool {
        matches!(
            &self.mode,
            Mode::Media(media)
                if media.kind == crate::media::MediaKind::Video
                    && media.backend == crate::media::VideoBackend::Tct
                    && media.phase == MediaPhase::Playing
                    && media.sub_picker.is_none()
        )
    }

    pub fn new(cwd: PathBuf, home: PathBuf) -> Self {
        AppState {
            browser: Browser::new(cwd),
            mode: Mode::Browser,
            command_input: String::new(),
            message: None,
            error_epoch: 0,
            operation: None,
            tag_defs: Vec::new(),
            last_tag: None,
            should_quit: false,
            pending_keys: Vec::new(),
            pending_count: None,
            history: History::default(),
            previous_dir: None,
            tabs: vec![Tab {
                browser: Browser::new(PathBuf::new()),
                history: History::default(),
            }],
            active_tab: 0,
            closed_tabs: Vec::new(),
            undo: Vec::new(),
            dir_sizes: std::collections::HashMap::new(),
            links: Vec::new(),
            mpv_available: false,
            completions: Vec::new(),
            completion_index: None,
            completion_cycle: false,
            history_cursor: None,
            command_draft: String::new(),
            has_display: false,
            cover: None,
            mini: None,
            address_bar: false,
            help_query: String::new(),
            help_scroll: 0,
            width: 80,
            height: 24,
            list_viewport: 10,
            grid_cols: 1,
            home,
            pending_nav: None,
            pending_focus: None,
            hit_map: HitMap::default(),
            last_click: None,
            double_click: Duration::from_millis(DEFAULT_DOUBLE_CLICK_MS),
            show_sidebar: None,
            show_preview: None,
            sidebar_items: Vec::new(),
            parent_rows: Vec::new(),
            mounts: Vec::new(),
            bookmarks: Vec::new(),
            preview: PreviewState::default(),
            picker: ratatui_image::picker::Picker::from_fontsize((8, 16)),
            drag: None,
            marquee: None,
            next_media_session: 1,
            clipboard: ClipboardState::default(),
            hover: HoverState::default(),
            pending_paste_mode: None,
            now: Instant::now(),
            wall_clock: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            anim: Animator::new(false),
            settings: Settings::default(),
            settings_dirty: false,
            side_listings: std::collections::HashMap::new(),
            truecolor: true,
        }
    }

    /// The active layout.
    pub fn view(&self) -> crate::settings::ViewMode {
        self.settings.view
    }

    pub fn mode_name(&self) -> &'static str {
        if matches!(self.mode, Mode::Browser) && self.browser.visual {
            return "VISUAL";
        }
        self.mode.name()
    }

    /// Commits a user-visible error and bumps the epoch the event loop
    /// watches to force a full redraw.
    pub fn set_error(&mut self, text: impl Into<String>) {
        self.message = Some(StatusMessage::error(text));
        self.error_epoch = self.error_epoch.wrapping_add(1);
    }

    /// Key identifying the focused entry for preview caching
    /// (path, mtime, size); None when nothing is focused.
    pub fn focused_preview_key(&self) -> Option<(PathBuf, i64, u64)> {
        let view = self.browser.focused()?;
        Some((
            view.entry.path.clone(),
            view.entry.modified,
            view.entry.size,
        ))
    }
}

impl std::fmt::Debug for PreviewContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewContent::Text { lines, truncated } => f
                .debug_struct("Text")
                .field("lines", &lines.len())
                .field("truncated", truncated)
                .finish(),
            PreviewContent::Directory(names) => f.debug_tuple("Directory").field(names).finish(),
            PreviewContent::Unavailable(msg) => f.debug_tuple("Unavailable").field(msg).finish(),
            PreviewContent::Image(_) => write!(f, "Image(..)"),
        }
    }
}

impl std::fmt::Debug for PreviewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewState")
            .field("key", &self.key)
            .field("has_content", &self.content.is_some())
            .finish()
    }
}
