use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

pub mod real;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink { broken: bool },
    Socket,
    Pipe,
    BlockDevice,
    CharDevice,
    Unknown,
}

impl EntryKind {
    pub fn is_dir(&self) -> bool {
        matches!(self, EntryKind::Directory)
    }
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: OsString,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: u64,
    pub mode: u32,
    pub modified: i64,
    pub executable: bool,
    pub hidden: bool,
    pub device: Option<u64>,
    pub inode: Option<u64>,
}

impl DirEntry {
    pub fn display_name(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }
}

pub trait FileSystem: Send + Sync {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn exists(&self, path: &Path) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    Skip,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordedMutation {
    Copy {
        src: PathBuf,
        dst: PathBuf,
        replace: bool,
    },
    Move {
        src: PathBuf,
        dst: PathBuf,
        replace: bool,
    },
    Delete {
        path: PathBuf,
        recursive: bool,
    },
}

pub trait MutationBackend: Send {
    fn copy_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()>;
    fn move_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()>;
    fn delete_entry(&self, path: &Path, recursive: bool) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
}
