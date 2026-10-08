use std::path::PathBuf;

use crate::app::action::Action;
use crate::app::state::{MediaSurface, Password};
use crate::crypto::CryptoKind;
use crate::media::{MediaCommand, MediaKind};
use crate::operations::OperationPlan;

#[derive(Debug)]
pub enum Effect {
    LoadDirectory(PathBuf),
    /// Load a listing for a side pane (Miller parent column).
    LoadSideListing(PathBuf),
    /// Persist `AppState::settings`.
    SaveSettings(Box<crate::settings::Settings>),
    /// Put text on the system clipboard (OSC 52 plus wl-copy/xclip).
    CopyText(String),
    /// Suspend the TUI and run a shell: interactive (`None`) or one
    /// command line through `$SHELL -c`, in `cwd`.
    RunShell {
        command: Option<String>,
        cwd: PathBuf,
    },
    /// Recursive sizes for folders (worker thread).
    DiskUsage(Vec<PathBuf>),
    /// Item counts for folders (worker thread).
    CountChildren(Vec<PathBuf>),
    /// Reverse earlier jobs: move `to` back to `from`, trash created paths.
    RunUndo {
        moves: Vec<(PathBuf, PathBuf)>,
        trash: Vec<PathBuf>,
    },
    Chmod(Vec<(PathBuf, u32)>),
    /// Recursive search below `root`: names (`content` false, glob or
    /// substring) or file contents.
    FindFiles {
        root: PathBuf,
        query: String,
        content: bool,
    },
    /// Edit the names of `paths` in `$EDITOR` (suspends the TUI).
    BulkRename(Vec<PathBuf>),
    /// Rename/move each (from, to) pair; journaled for undo.
    MovePairs(Vec<(PathBuf, PathBuf)>),
    /// Launch a GUI program without suspending the TUI.
    SpawnDetached {
        path: PathBuf,
        program: String,
        args: Vec<String>,
    },
    /// Remove terminal graphics placements (video frames) so text drawn
    /// in their place is visible.
    ClearGraphics,
    /// Read tags and cover art of a track (worker thread).
    LoadTrackInfo {
        session: u64,
        path: PathBuf,
    },
    /// Look for subtitle files near a video (worker thread).
    FindSubtitles {
        session: u64,
        video: PathBuf,
    },
    /// Persist web link bookmarks.
    SaveLinks(Vec<crate::urls::Link>),
    /// Open a web URL (browser or association), detached.
    OpenUrl {
        url: String,
        program: Option<String>,
        args: Vec<String>,
    },
    RunOperation(Box<OperationPlan>),
    RunRename(Box<OperationPlan>),
    OpenPathWith {
        path: PathBuf,
        program: String,
        args: Vec<String>,
    },
    CreateEntry {
        path: PathBuf,
        is_dir: bool,
    },
    /// Load preview content for the focused entry (worker thread).
    LoadPreview {
        key: (PathBuf, i64, u64),
        name: String,
        is_dir: bool,
    },
    /// Run encryption/decryption for one target (worker thread).
    Crypto {
        kind: CryptoKind,
        target: PathBuf,
        password: Password,
    },
    ToggleBookmark(PathBuf),
    TagAssign {
        name: String,
        paths: Vec<PathBuf>,
        create: bool,
    },
    TagUnassign {
        name: String,
        paths: Vec<PathBuf>,
    },
    TagCreate(String),
    TagDelete(String),
    TagMove {
        from: PathBuf,
        to: PathBuf,
    },
    StartMedia {
        session: u64,
        path: PathBuf,
        kind: MediaKind,
        surface: MediaSurface,
        resume_position: Option<f64>,
        resume_paused: Option<bool>,
        /// How video paints (ignored for audio).
        backend: crate::media::VideoBackend,
    },
    MediaCommand {
        session: u64,
        command: MediaCommand,
    },
    StopMedia {
        session: u64,
    },
    Quit,
}

pub trait EffectHandler {
    fn handle(&mut self, effect: Effect) -> Vec<Action>;
}
