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
    let (link_target, link_dir, target_exec) = if ft.is_symlink() {
        let target = fs::read_link(&path).ok();
        let resolved = fs::metadata(&path).ok();
        (
            target,
            resolved.as_ref().is_some_and(|m| m.is_dir()),
            resolved
                .as_ref()
                .is_some_and(|m| m.is_file() && m.mode() & 0o111 != 0),
        )
    } else {
        (None, false, false)
    };
    let executable = (kind == EntryKind::File && meta.mode() & 0o111 != 0) || target_exec;
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
        link_target,
        link_dir,
    })
}

impl FileSystem for RealFileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        for item in fs::read_dir(path)? {
            // One unreadable entry (vanished mid-listing, permission
            // quirks on FUSE mounts) must not hide the whole directory.
            let Ok(item) = item else { continue };
            let name = item.file_name();
            let item_path = item.path();
            let Ok(meta) = fs::symlink_metadata(&item_path) else {
                continue;
            };
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

    fn trash(&self, path: &Path) -> io::Result<PathBuf> {
        trash_path(path, &home_trash_dir()?)
    }

    fn symlink(&self, target: &Path, link: &Path) -> io::Result<()> {
        if fs::symlink_metadata(link).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("destination exists: {}", link.display()),
            ));
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(not(unix))]
        {
            let _ = target;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tui-explorer supports Linux only",
            ))
        }
    }

    fn set_permissions(&self, path: &Path, mode: u32) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
        }
        #[cfg(not(unix))]
        {
            let _ = (path, mode);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tui-explorer supports Linux only",
            ))
        }
    }

    fn permissions(&self, path: &Path) -> Option<u32> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::symlink_metadata(path)
                .ok()
                .map(|m| m.permissions().mode() & 0o7777)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            None
        }
    }
}

/// `$XDG_DATA_HOME/Trash` (freedesktop.org home trash).
pub fn home_trash_dir() -> io::Result<PathBuf> {
    let data = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| Path::new(v).is_absolute())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no home directory for the trash")
        })?;
    Ok(data.join("Trash"))
}

/// Moves `path` into `trash` following the freedesktop.org Trash spec:
/// the item lands in `files/<name>` (made unique) with a matching
/// `info/<name>.trashinfo` recording the original path and time, so file
/// managers (and our undo) can restore it.
pub fn trash_path(path: &Path, trash: &Path) -> io::Result<PathBuf> {
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cannot trash this path"))?
        .to_string_lossy()
        .into_owned();
    let mut chosen = name.clone();
    let mut n = 2;
    while files.join(&chosen).exists()
        || fs::symlink_metadata(files.join(&chosen)).is_ok()
        || info.join(format!("{chosen}.trashinfo")).exists()
    {
        chosen = format!("{name}.{n}");
        n += 1;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let info_path = info.join(format!("{chosen}.trashinfo"));
    let stamp = trash_timestamp();
    fs::write(
        &info_path,
        format!(
            "[Trash Info]\nPath={}\nDeletionDate={stamp}\n",
            percent_encode(&absolute.to_string_lossy())
        ),
    )?;
    let dest = files.join(&chosen);
    let moved = match fs::rename(path, &dest) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(18) => {
            copy_recursive(path, &dest, false).and_then(|()| remove_any(path))
        }
        Err(err) => Err(err),
    };
    if let Err(err) = moved {
        let _ = fs::remove_file(&info_path);
        return Err(err);
    }
    Ok(dest)
}

fn trash_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let date = crate::ui::format::format_time(secs);
    // "YYYY-MM-DD HH:MM" -> "YYYY-MM-DDTHH:MM:00"
    format!("{}:00", date.replacen(' ', "T", 1))
}

fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
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
    fn trash_writes_spec_layout_and_uniquifies() {
        let root = tmp_dir("trash");
        let trash = root.join("Trash");
        let a = root.join("my file.txt");
        fs::write(&a, b"one").unwrap();
        let dest = trash_path(&a, &trash).unwrap();
        assert!(!a.exists());
        assert_eq!(dest, trash.join("files/my file.txt"));
        let info = fs::read_to_string(trash.join("info/my file.txt.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"));
        assert!(info.contains("my%20file.txt"));
        assert!(info.contains("DeletionDate="));
        fs::write(&a, b"two").unwrap();
        let second = trash_path(&a, &trash).unwrap();
        assert_eq!(second, trash.join("files/my file.txt.2"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn symlink_and_chmod() {
        let root = tmp_dir("link");
        let target = root.join("t.txt");
        fs::write(&target, b"x").unwrap();
        let m = RealMutations::new();
        m.symlink(&target, &root.join("l")).unwrap();
        assert_eq!(fs::read_link(root.join("l")).unwrap(), target);
        assert!(m.symlink(&target, &root.join("l")).is_err());
        m.set_permissions(&target, 0o600).unwrap();
        assert_eq!(m.permissions(&target), Some(0o600));
        fs::remove_dir_all(&root).unwrap();
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
