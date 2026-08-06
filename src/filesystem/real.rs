use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{DirEntry, EntryKind, FileSystem, MutationBackend};

pub struct RealFileSystem;

impl RealFileSystem {
    pub fn new() -> Self {
        RealFileSystem
    }
}

impl Default for RealFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
fn entry_from_metadata(
    name: std::ffi::OsString,
    path: PathBuf,
    meta: &fs::Metadata,
) -> io::Result<DirEntry> {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::fs::MetadataExt;

    let ft = meta.file_type();
    let kind = if ft.is_dir() {
        EntryKind::Directory
    } else if ft.is_file() {
        EntryKind::File
    } else if ft.is_symlink() {
        EntryKind::Symlink {
            broken: fs::metadata(&path).is_err(),
        }
    } else if ft.is_socket() {
        EntryKind::Socket
    } else if ft.is_fifo() {
        EntryKind::Pipe
    } else if ft.is_block_device() {
        EntryKind::BlockDevice
    } else if ft.is_char_device() {
        EntryKind::CharDevice
    } else {
        EntryKind::Unknown
    };
    let hidden = name.to_string_lossy().starts_with('.');
    let executable = kind == EntryKind::File && meta.mode() & 0o111 != 0;
    Ok(DirEntry {
        name,
        path,
        kind,
        size: meta.size(),
        mode: meta.mode() & 0o7777,
        modified: meta.mtime(),
        executable,
        hidden,
        device: Some(meta.dev()),
        inode: Some(meta.ino()),
    })
}

impl FileSystem for RealFileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        for item in fs::read_dir(path)? {
            let item = item?;
            let name = item.file_name();
            let item_path = item.path();
            let meta = fs::symlink_metadata(&item_path)?;
            #[cfg(unix)]
            {
                out.push(entry_from_metadata(name, item_path, &meta)?);
            }
            #[cfg(not(unix))]
            {
                let _ = meta;
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "tui-explorer supports Linux only",
                ));
            }
        }
        Ok(out)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

pub struct RealMutations;

impl RealMutations {
    pub fn new() -> Self {
        RealMutations
    }
}

impl Default for RealMutations {
    fn default() -> Self {
        Self::new()
    }
}

fn copy_recursive(src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if dst.exists() || fs::symlink_metadata(dst).is_ok() {
        if !replace {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("destination exists: {}", dst.display()),
            ));
        }
        if meta.is_dir() && dst.starts_with(src) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot copy a directory into itself",
            ));
        }
        remove_any(dst)?;
    }
    if meta.is_dir() {
        fs::create_dir(dst)?;
        for item in fs::read_dir(src)? {
            let item = item?;
            copy_recursive(&item.path(), &dst.join(item.file_name()), true)?;
        }
        Ok(())
    } else if meta.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, dst)
        }
        #[cfg(not(unix))]
        {
            let _ = target;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tui-explorer supports Linux only",
            ))
        }
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

impl MutationBackend for RealMutations {
    fn copy_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        copy_recursive(src, dst, replace)
    }

    fn move_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        if (dst.exists() || fs::symlink_metadata(dst).is_ok()) && !replace {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("destination exists: {}", dst.display()),
            ));
        }
        match fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(err) if err.raw_os_error() == Some(18) => {
                copy_recursive(src, dst, replace)?;
                remove_any(src)
            }
            Err(err) => Err(err),
        }
    }

    fn delete_entry(&self, path: &Path, recursive: bool) -> io::Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if meta.is_dir() {
            if recursive {
                fs::remove_dir_all(path)
            } else {
                fs::remove_dir(path)
            }
        } else {
            fs::remove_file(path)
        }
    }

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn create_file(&self, path: &Path) -> io::Result<()> {
        match fs::File::options().create_new(true).write(true).open(path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let file = fs::File::options().write(true).open(path)?;
                file.set_modified(std::time::SystemTime::now())
            }
            Err(e) => Err(e),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists() || fs::symlink_metadata(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tui-explorer-real-fs-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_dir_is_recursive() {
        let root = tmp_dir("mkdir");
        let mutations = RealMutations::new();
        let nested = root.join("a/b/c");
        mutations.create_dir(&nested).unwrap();
        assert!(nested.is_dir());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn create_file_touches_new_and_existing() {
        let root = tmp_dir("touch");
        let mutations = RealMutations::new();
        let file = root.join("new.txt");
        mutations.create_file(&file).unwrap();
        assert!(file.is_file());
        assert_eq!(fs::metadata(&file).unwrap().len(), 0);
        let before = fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        mutations.create_file(&file).unwrap();
        let after = fs::metadata(&file).unwrap().modified().unwrap();
        assert!(after >= before);
        fs::remove_dir_all(&root).unwrap();
    }
}
