use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::browser::Browser;
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

#[derive(Debug)]
pub enum Mode {
    Browser,
    Command,
    Confirm(Box<ConfirmState>),
    Conflict(Box<ConflictState>),
    TagPicker(Box<TagPickerState>),
    ContextMenu(Box<ContextMenuState>),
    Password(Box<PasswordState>),
    Help,
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
            Mode::Help => Mode::Help,
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
            Mode::Help => "HELP",
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
}

impl AppState {
    pub fn new(cwd: PathBuf, home: PathBuf) -> Self {
        AppState {
            browser: Browser::new(cwd),
            mode: Mode::Browser,
            command_input: String::new(),
            message: None,
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
        }
    }

    pub fn mode_name(&self) -> &'static str {
        if matches!(self.mode, Mode::Browser) && self.browser.visual {
            return "VISUAL";
        }
        self.mode.name()
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
