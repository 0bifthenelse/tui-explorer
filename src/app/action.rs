use std::path::PathBuf;

use crate::browser::EntryView;
use crate::operations::{OperationPlan, OperationReport};
use crate::tags::TagDef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Left,
    DoubleLeft,
    Right,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictDecision {
    Cancel,
    Skip,
    Replace,
}

#[derive(Clone, Debug)]
pub struct DirectorySnapshot {
    pub path: PathBuf,
    pub entries: Vec<EntryView>,
    pub defs: Vec<TagDef>,
}

#[derive(Clone, Debug)]
pub enum Action {
    LoadInitial,
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    GotoFirst,
    GotoLast,
    KeyG,
    OpenFocused,
    OpenParent,
    ToggleSelect,
    ToggleVisual,
    ToggleHidden,
    QuickTag,
    OpenTagPicker,
    EnterCommand,
    CommandChar(char),
    CommandBackspace,
    CommandSubmit,
    Cancel,
    ToggleHelp,
    Quit,
    Confirm,
    Reject,
    PickerMove(isize),
    PickerToggle,
    PickerNew,
    PickerChar(char),
    PickerBackspace,
    PickerSubmitNew,
    PickerCancelInput,
    PickerDelete,
    ContextMove(isize),
    ContextChoose,
    ConflictChoice(ConflictDecision),
    Mouse {
        kind: MouseKind,
        x: u16,
        y: u16,
    },
    Resize {
        width: u16,
        height: u16,
    },
    DirectoryLoaded {
        result: Result<DirectorySnapshot, String>,
    },
    OperationProgress {
        current: PathBuf,
        done: usize,
        total: usize,
    },
    OperationFinished {
        report: OperationReport,
    },
    ConflictsFound {
        plan: Box<OperationPlan>,
        conflicts: Vec<(PathBuf, PathBuf)>,
    },
    OpenFailed(String),
    ErrorMessage(String),
    TagsApplied {
        message: String,
        last_tag: Option<String>,
    },
}
