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
    /// Symlink target as stored in the link (not resolved).
    pub link_target: Option<PathBuf>,
    /// The symlink resolves to a directory: it opens and sorts like one.
    pub link_dir: bool,
}

impl DirEntry {
    pub fn display_name(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }

    /// A metadata-less entry for display-only purposes (preview listings).
    pub fn synthetic(dir: &Path, name: &str, is_dir: bool) -> DirEntry {
        DirEntry {
            name: OsString::from(name),
            path: dir.join(name),
            kind: if is_dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: 0,
            mode: 0o644,
            modified: 0,
            executable: false,
            hidden: name.starts_with('.'),
            device: None,
            inode: None,
            link_target: None,
            link_dir: false,
        }
    }

    /// Directory or a symlink resolving to one.
    pub fn is_dir_like(&self) -> bool {
        self.kind.is_dir() || self.link_dir
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
    CreateDir {
        path: PathBuf,
    },
    CreateFile {
        path: PathBuf,
    },
    Trash {
        path: PathBuf,
    },
    Symlink {
        target: PathBuf,
        link: PathBuf,
    },
    Chmod {
        path: PathBuf,
        mode: u32,
    },
}

pub trait MutationBackend: Send {
    fn copy_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()>;
    fn move_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()>;
    fn delete_entry(&self, path: &Path, recursive: bool) -> io::Result<()>;
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn create_file(&self, path: &Path) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    /// Moves `path` to the trash; returns where it now lives.
    fn trash(&self, path: &Path) -> io::Result<PathBuf>;
    /// Creates a symbolic link at `link` pointing to `target`.
    fn symlink(&self, target: &Path, link: &Path) -> io::Result<()>;
    /// Sets permission bits (`chmod`).
    fn set_permissions(&self, path: &Path, mode: u32) -> io::Result<()>;
    /// Current permission bits, when known.
    fn permissions(&self, path: &Path) -> Option<u32>;
}

/// `name (2).ext`, `name (3).ext`, ... next to `dst`: the first candidate
/// for which `exists` is false. Used by "keep both" pastes.
pub fn unique_destination(dst: &Path, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    if !exists(dst) {
        return dst.to_path_buf();
    }
    let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = split_ext(&name);
    for n in 2..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    parent.join(format!("{stem} (copy){ext}"))
}

/// Splits `archive.tar.gz` into (`archive`, `.tar.gz`) and `notes.md` into
/// (`notes`, `.md`); dotfiles keep their leading dot in the stem.
pub fn split_ext(name: &str) -> (String, String) {
    for compound in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst"] {
        if name.len() > compound.len() && name.to_ascii_lowercase().ends_with(compound) {
            let cut = name.len() - compound.len();
            return (name[..cut].to_string(), name[cut..].to_string());
        }
    }
    match name.rfind('.') {
        Some(idx) if idx > 0 => (name[..idx].to_string(), name[idx..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_destination_counts_up() {
        let taken = ["/d/a.txt", "/d/a (2).txt"];
        let exists = |p: &Path| taken.iter().any(|t| Path::new(t) == p);
        assert_eq!(
            unique_destination(Path::new("/d/a.txt"), &exists),
            PathBuf::from("/d/a (3).txt")
        );
        assert_eq!(
            unique_destination(Path::new("/d/free.txt"), &exists),
            PathBuf::from("/d/free.txt")
        );
    }

    #[test]
    fn split_ext_handles_compound_and_dotfiles() {
        assert_eq!(split_ext("a.tar.gz"), ("a".into(), ".tar.gz".into()));
        assert_eq!(split_ext(".bashrc"), (".bashrc".into(), String::new()));
        assert_eq!(split_ext("Makefile"), ("Makefile".into(), String::new()));
        assert_eq!(split_ext("x.md"), ("x".into(), ".md".into()));
    }
}
