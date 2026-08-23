use std::path::PathBuf;

use crate::app::state::MediaSurface;
use crate::browser::EntryView;
use crate::crypto::CryptoOutcome;
use crate::media::MediaPhase;
use crate::operations::{OperationPlan, OperationReport};
use crate::preview::PreviewLoaded;
use crate::tags::TagDef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Left,
    Right,
    ScrollUp,
    ScrollDown,
    /// Left button moved while held (drag motion).
    LeftDrag,
    /// Left button released.
    LeftUp,
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
    MoveLeft,
    MoveRight,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    GotoFirst,
    GotoLast,
    KeyG,
    OpenFocused,
    OpenParent,
    Refresh,
    OpenWithPrompt,
    OpenWithChar(char),
    OpenWithBackspace,
    OpenWithSubmit,
    ToggleSidebar,
    TogglePreview,
    ToggleBookmark,
    OpenBookmarks,
    BookmarkChar(char),
    BookmarkBackspace,
    BookmarkMove(isize),
    BookmarkSubmit,
    /// `X`: start encryption, or decryption when the focused entry is a
    /// recognized encrypted output (`*.age` / `*.tar.age`).
    EncryptToggle,
    PasswordChar(char),
    PasswordBackspace,
    PasswordSubmit,
    CryptoFinished {
        done: Vec<CryptoOutcome>,
        failed: Vec<(PathBuf, String)>,
    },
    PreviewLoaded {
        key: (PathBuf, i64, u64),
        result: PreviewLoaded,
    },
    MediaSurfaceReady {
        session: u64,
        surface: MediaSurface,
    },
    MediaBackendReady {
        session: u64,
    },
    MediaStatus {
        session: u64,
        phase: MediaPhase,
        position: f64,
        duration: Option<f64>,
        volume: u8,
    },
    MediaSpectrum {
        session: u64,
        spectrum: [f32; 24],
    },
    MediaEnded {
        session: u64,
    },
    MediaFailed {
        session: u64,
        message: String,
    },
    MediaStopped {
        session: u64,
    },
    MediaTogglePause,
    MediaSeek(i64),
    MediaVolume(i8),
    MediaStop,
    MediaClose,
    BookmarksChanged {
        bookmarks: Vec<PathBuf>,
        message: String,
    },
    ToggleSelect,
    Mouse {
        kind: MouseKind,
        x: u16,
        y: u16,
    },
    DragCancel,
    ToggleVisual,
    ToggleHidden,
    SetFilter(Option<String>),
    QuickTag,
    OpenTagPicker,
    EnterCommand,
    EnterFilter,
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
