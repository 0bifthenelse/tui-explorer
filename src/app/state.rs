use std::path::PathBuf;

use crate::browser::Browser;
use crate::operations::{OperationKind, OperationPlan};
use crate::tags::TagDef;
use crate::ui::hit::HitMap;

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

#[derive(Clone, Debug)]
pub enum Mode {
    Browser,
    Command,
    Confirm(Box<ConfirmState>),
    Conflict(Box<ConflictState>),
    TagPicker(Box<TagPickerState>),
    ContextMenu(Box<ContextMenuState>),
    Help,
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
            Mode::Help => "HELP",
        }
    }

    pub fn is_overlay(&self) -> bool {
        !matches!(self, Mode::Browser | Mode::Command)
    }
}

#[derive(Clone, Debug)]
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
    pub list_viewport: usize,
    pub home: PathBuf,
    pub pending_nav: Option<PathBuf>,
    pub hit_map: HitMap,
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
            home,
            pending_nav: None,
            hit_map: HitMap::default(),
        }
    }

    pub fn mode_name(&self) -> &'static str {
        if matches!(self.mode, Mode::Browser) && self.browser.visual {
            return "VISUAL";
        }
        self.mode.name()
    }
}
