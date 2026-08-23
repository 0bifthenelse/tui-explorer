use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::browser::Browser;
use crate::media::{AfterStop, MediaKind, MediaPhase};
use crate::operations::{OperationKind, OperationPlan};
use crate::sidebar::{MountInfo, SidebarItem};
use crate::tags::TagDef;
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
}

/// State for the fuzzy bookmark navigator (`B`): a query over the already
/// loaded `AppState::bookmarks`, with the ranked result list it produced.
#[derive(Clone, Debug)]
pub struct BookmarkNavState {
    pub query: String,
    pub matches: Vec<PathBuf>,
    pub selected: usize,
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
    Delete { plan: Box<OperationPlan> },
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
    Copy,
    Move,
    Delete,
    Tags,
}

impl ContextItem {
    pub fn label(&self) -> &'static str {
        match self {
            ContextItem::Open => "Open",
            ContextItem::OpenWith => "Open with",
            ContextItem::Rename => "Rename",
            ContextItem::Copy => "Copy",
            ContextItem::Move => "Move",
            ContextItem::Delete => "Delete",
            ContextItem::Tags => "Tags",
        }
    }

    pub fn all() -> &'static [ContextItem] {
        &[
            ContextItem::Open,
            ContextItem::OpenWith,
            ContextItem::Rename,
            ContextItem::Copy,
            ContextItem::Move,
            ContextItem::Delete,
            ContextItem::Tags,
        ]
    }
}

#[derive(Clone, Debug)]
pub struct ContextMenuState {
    pub target: PathBuf,
    pub items: &'static [ContextItem],
    pub selected: usize,
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaSurface {
    pub rect: Rect,
    pub terminal_cells: (u16, u16),
    pub cell_pixels: (u16, u16),
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
}

impl MediaState {
    pub fn preparing(session: u64, path: PathBuf, kind: MediaKind) -> Self {
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
        }
    }
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
        }
    }

    pub fn is_overlay(&self) -> bool {
        !matches!(self, Mode::Browser | Mode::Command)
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
    pub pending_g: bool,
    pub width: u16,
    pub height: u16,
    /// Number of entries visible per page in the current layout.
    pub list_viewport: usize,
    /// Number of tile columns in the grid layout (1 in narrow layouts).
    pub grid_cols: usize,
    pub home: PathBuf,
    pub pending_nav: Option<PathBuf>,
    pub hit_map: HitMap,
    /// Last single click on a grid row, for same-entry double-click detection.
    pub last_click: Option<(Instant, usize)>,
    pub double_click: Duration,
    /// Sidebar visibility: None = automatic per terminal size.
    pub show_sidebar: Option<bool>,
    /// Preview panel visibility: None = automatic per terminal size.
    pub show_preview: Option<bool>,
    /// Sidebar entries in render order; rebuilt every frame.
    pub sidebar_items: Vec<SidebarItem>,
    /// Device mounts captured once at startup (never re-read per frame).
    pub mounts: Vec<MountInfo>,
    pub bookmarks: Vec<PathBuf>,
    pub preview: PreviewState,
    pub picker: ratatui_image::picker::Picker,
    pub next_media_session: u64,
}

impl AppState {
    /// True exactly while video media owns the terminal: mpv writes pixels
    /// directly to stdout, so Ratatui must neither clear nor draw.
    pub fn media_owns_terminal(&self) -> bool {
        matches!(
            &self.mode,
            Mode::Media(media)
                if media.kind == crate::media::MediaKind::Video
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
            pending_g: false,
            width: 80,
            height: 24,
            list_viewport: 10,
            grid_cols: 1,
            home,
            pending_nav: None,
            hit_map: HitMap::default(),
            last_click: None,
            double_click: Duration::from_millis(DEFAULT_DOUBLE_CLICK_MS),
            show_sidebar: None,
            show_preview: None,
            sidebar_items: Vec::new(),
            mounts: Vec::new(),
            bookmarks: Vec::new(),
            preview: PreviewState::default(),
            picker: ratatui_image::picker::Picker::from_fontsize((8, 16)),
            next_media_session: 1,
        }
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
            PreviewContent::Image(_) => write!(f, "Image(..)"),
            PreviewContent::Directory(names) => f.debug_tuple("Directory").field(names).finish(),
            PreviewContent::Unavailable(msg) => f.debug_tuple("Unavailable").field(msg).finish(),
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
