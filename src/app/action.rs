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
    /// Pointer motion with no button held (hover tracking).
    Moved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictDecision {
    Cancel,
    Skip,
    Replace,
    KeepBoth,
}

/// What `y<key>` copies to the system clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YankKind {
    /// Full path of each target.
    Path,
    /// The current directory.
    Dir,
    /// File name with extension.
    Name,
    /// File name without extension.
    Stem,
}

/// Where the cursor starts in the inline rename field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameCursor {
    /// Empty field (`cw`): type a brand-new name.
    Replace,
    /// Whole name, cursor at the end (`A`, F2).
    End,
    /// Whole name, cursor at the start (`I`).
    Start,
    /// Whole name, cursor before the extension (`a`).
    BeforeExt,
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
    /// Switch the layout (list / grid / columns).
    SetView(crate::settings::ViewMode),
    /// Cycle list → grid → columns.
    CycleView,
    /// Open the editable location bar (Ctrl-L / click on the path bar).
    OpenAddressBar,
    /// Grid tile size: true = large tiles, false = compact.
    GridZoom(bool),
    /// Sort by a key; the active key flips direction.
    SortBy(crate::browser::SortKey),
    /// Sort with an explicit key and direction (`os`, `oM`, ...).
    SetSort(crate::browser::SortMode),
    /// `or`: flip the current sort direction.
    ReverseSort,
    /// One key of a multi-key chord (`gg`, `yy`, `m<key>`, counts).
    ChordKey(String),
    /// Run an action `n` times (count prefixes such as `5j`).
    Repeat(usize, Box<Action>),
    /// Jump to the n-th visible entry (`5G`).
    GotoIndex(usize),
    /// Go to a location spec (`~`, `/etc`, `~/Downloads`).
    GoTo(String),
    HistoryBack,
    HistoryForward,
    /// `''`: the previous directory.
    PreviousDir,
    SetMark(char),
    JumpMark(char),
    DeleteMark(char),
    /// `/`: incremental search.
    EnterSearch,
    SearchNext,
    SearchPrev,
    /// `f`: type-to-find, opens when one entry matches.
    EnterFind,
    SelectAll,
    InvertSelection,
    ClearSelection,
    /// `yy` / Ctrl-C: copy targets to the internal clipboard.
    CopySelection,
    /// `dd` / Ctrl-X: cut targets to the internal clipboard.
    CutSelection,
    /// `pp` / `po` / Ctrl-V.
    PasteHere {
        overwrite: bool,
    },
    /// `pl`: paste as symbolic links.
    PasteSymlinks,
    ClearClipboard,
    /// `yp` `yd` `yn` `yN`: copy text to the system clipboard.
    Yank(YankKind),
    /// `dT` / Delete: move targets to the trash.
    TrashSelection,
    /// `dD` / Shift-Delete: delete permanently (confirmed).
    DeleteSelection,
    Undo,
    RenameStart(RenameCursor),
    /// `+`: create a file (or a folder with a trailing `/`).
    EnterCreate,
    /// `!` / `s`: run a shell command.
    EnterShell,
    /// `S`: interactive shell in the current directory.
    Subshell,
    /// `E`: open in `$EDITOR`.
    EditFocused,
    /// `i`: fullscreen quick look.
    QuickLook,
    /// `du`: compute directory sizes.
    DiskUsage,
    ToggleAnimations,
    TabNew,
    TabNext,
    TabPrev,
    TabClose,
    TabRestore,
    TabSelect(usize),
    /// `gx`: open a URL found in the focused file.
    OpenUrlFromFile,
    /// `gl`: jump to the focused symlink's target.
    FollowLink,
    /// Bracketed paste from the terminal.
    Paste(String),
    /// Edit the active inline prompt (rename, search, address bar).
    LineEdit(crate::input::line::Edit),
    /// Enter in the active inline prompt.
    PromptSubmit,
    /// A create effect succeeded: focus it and journal it for undo.
    EntryCreated(PathBuf),
    DiskUsageReady(Vec<(PathBuf, u64)>),
    ChildCountsReady(Vec<(PathBuf, u32)>),
    /// An undo job finished (not itself recorded as undoable).
    UndoFinished {
        report: OperationReport,
    },
    QuickLookScroll(isize),
    /// `:find` / `:grep` finished.
    FindResults {
        title: String,
        root: PathBuf,
        hits: Vec<crate::app::state::FindHit>,
    },
    /// Results modal: typing filters, Up/Down move, Enter jumps.
    ResultsChar(char),
    ResultsBackspace,
    ResultsMove(isize),
    ResultsSubmit,
    /// Names edited in `$EDITOR` for `:bulkrename`.
    BulkRenamePlan(Vec<(PathBuf, PathBuf)>),
    /// Open-with prompt: Tab cycles detected openers.
    OpenWithCycle(isize),
    /// Open-with prompt: Ctrl-R toggles "remember for .ext".
    OpenWithToggleRemember,
    /// Help overlay: type-to-filter and scrolling.
    HelpChar(char),
    HelpBackspace,
    HelpScroll(isize),
    MediaPrev,
    /// Audio keeps playing in the status-bar mini player.
    MediaMinimize,
    /// Back to the full player from the mini player (`M`).
    MediaExpand,
    MediaMute,
    MediaShuffle,
    MediaRepeat,
    /// Speed step (+1 faster, -1 slower, 0 reset).
    MediaSpeed(i8),
    /// Jump to n×10% of the duration (`0`–`9`).
    MediaSeekPercent(u8),
    MediaCycleSub,
    /// Subtitles on / off (`v`).
    MediaToggleSubs,
    MediaCycleAudio,
    /// Shift subtitles by n tenths of a second.
    MediaSubDelay(i8),
    /// Subtitle picker (`c`).
    MediaOpenSubs,
    TrackInfoLoaded {
        session: u64,
        info: crate::media::tags::TrackInfo,
    },
    SubsFound {
        session: u64,
        files: Vec<crate::media::subs::SubtitleFile>,
    },
    SubPickerChar(char),
    SubPickerBackspace,
    SubPickerMove(isize),
    SubPickerSubmit,
    SubPickerClose,
    /// Append the selection (or focus) to the play queue.
    MediaEnqueue,
    MediaAddSub(PathBuf),
    MediaSetVolume(u8),
    /// Tab in the command line: complete the command or path.
    CommandComplete,
    /// Up/Down in the command line: walk the history.
    CommandHistory(isize),
    /// Background listing for the Miller parent pane.
    SideListingLoaded {
        path: PathBuf,
        entries: Vec<EntryView>,
    },
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
    /// Tab / Shift-Tab: switch hub section.
    BookmarkSection(isize),
    /// Remove the selected hub item.
    BookmarkDelete,
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
    MediaSeekAbsolute(f64),
    MediaVolume(i8),
    MediaStop,
    MediaClose,
    MediaToggleFullscreen,
    MediaNext,
    ClipboardCopy {
        paths: Vec<PathBuf>,
    },
    ClipboardCut {
        paths: Vec<PathBuf>,
    },
    ClipboardPaste,
    BookmarksChanged {
        bookmarks: Vec<PathBuf>,
        message: String,
    },
    ToggleSelect,
    Mouse {
        kind: MouseKind,
        x: u16,
        y: u16,
        /// Ctrl was held; makes marquee selection additive instead of
        /// replacing the current selection set.
        ctrl: bool,
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
