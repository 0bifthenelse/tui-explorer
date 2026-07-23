//! Sidebar content: places, mounts, persistent tags and bookmarks.
//!
//! The sidebar is rebuilt from live data every frame so it always reflects
//! the current filesystem and tag database. Items are stored in render order
//! on `AppState::sidebar_items` so mouse hit regions can resolve clicks.

use std::path::{Path, PathBuf};

use crate::app::state::AppState;
use crate::ui::format::format_size;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarItem {
    Place {
        label: String,
        path: PathBuf,
    },
    Mount {
        path: PathBuf,
        fs: String,
        used: u64,
        total: u64,
    },
    Tag {
        name: String,
        token: String,
    },
    Bookmark {
        path: PathBuf,
    },
}

/// Sections in display order; each is a header plus its items.
pub struct SidebarSections {
    pub places: Vec<SidebarItem>,
    pub mounts: Vec<SidebarItem>,
    pub tags: Vec<SidebarItem>,
    pub bookmarks: Vec<SidebarItem>,
}

fn push_place(places: &mut Vec<SidebarItem>, label: &str, path: PathBuf) {
    if path.is_dir()
        && !places
            .iter()
            .any(|p| matches!(p, SidebarItem::Place { path: p2, .. } if *p2 == path))
    {
        places.push(SidebarItem::Place {
            label: label.to_string(),
            path,
        });
    }
}

pub fn places(home: &Path) -> Vec<SidebarItem> {
    let mut out = Vec::new();
    push_place(&mut out, "Home", home.to_path_buf());
    push_place(&mut out, "Root", PathBuf::from("/"));
    for label in [
        "Desktop",
        "Documents",
        "Downloads",
        "Music",
        "Pictures",
        "Videos",
    ] {
        push_place(&mut out, label, home.join(label));
    }
    out
}

#[derive(Clone, Debug)]
pub struct MountInfo {
    pub path: PathBuf,
    pub fs: String,
    pub used: u64,
    pub total: u64,
}

/// Parse /proc/self/mounts for real device mounts and stat their usage.
/// Bounded to a handful of entries; failures simply yield fewer mounts.
pub fn read_mounts() -> Vec<MountInfo> {
    let Ok(table) = std::fs::read_to_string("/proc/self/mounts") else {
        return Vec::new();
    };
    let mut out: Vec<MountInfo> = Vec::new();
    for line in table.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(target), Some(fs)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if !device.starts_with("/dev/") {
            continue;
        }
        let target = target.replace("\\040", " ");
        if target.starts_with("/snap") || target.starts_with("/boot") {
            continue;
        }
        if out.iter().any(|m| m.path == Path::new(&target)) {
            continue;
        }
        let (used, total) = mount_usage(Path::new(&target)).unwrap_or((0, 0));
        out.push(MountInfo {
            path: PathBuf::from(target),
            fs: fs.to_string(),
            used,
            total,
        });
        if out.len() >= 6 {
            break;
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn mount_usage(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let total = stat.f_blocks.saturating_mul(stat.f_frsize);
    let free = stat.f_bavail.saturating_mul(stat.f_frsize);
    Some((total.saturating_sub(free), total))
}

pub fn mount_label(m: &MountInfo) -> String {
    if m.total == 0 {
        format!("{} {}", m.path.display(), m.fs)
    } else {
        format!(
            "{} {}  {} / {}",
            m.path.display(),
            m.fs,
            format_size(m.used),
            format_size(m.total)
        )
    }
}

pub fn build_sections(state: &AppState) -> SidebarSections {
    SidebarSections {
        places: places(&state.home),
        mounts: state
            .mounts
            .iter()
            .map(|m| SidebarItem::Mount {
                path: m.path.clone(),
                fs: m.fs.clone(),
                used: m.used,
                total: m.total,
            })
            .collect(),
        tags: state
            .tag_defs
            .iter()
            .map(|d| SidebarItem::Tag {
                name: d.name.clone(),
                token: d.display_token.clone(),
            })
            .collect(),
        bookmarks: state
            .bookmarks
            .iter()
            .map(|p| SidebarItem::Bookmark { path: p.clone() })
            .collect(),
    }
}

/// Flatten sections into the render-order item list stored on the state.
pub fn flatten(sections: &SidebarSections) -> Vec<SidebarItem> {
    let mut out = Vec::new();
    out.extend(sections.places.iter().cloned());
    out.extend(sections.mounts.iter().cloned());
    out.extend(sections.tags.iter().cloned());
    out.extend(sections.bookmarks.iter().cloned());
    out
}

/// Persistent bookmark store: one absolute path per line in a plain file
/// under the XDG data directory. No secrets, no binary format.
#[derive(Clone, Debug)]
pub struct BookmarkStore {
    path: PathBuf,
}

impl BookmarkStore {
    pub fn new(path: PathBuf) -> Self {
        BookmarkStore { path }
    }

    pub fn load(&self) -> Vec<PathBuf> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && l.starts_with('/'))
            .map(PathBuf::from)
            .collect()
    }

    pub fn save(&self, bookmarks: &[PathBuf]) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut text = String::new();
        for b in bookmarks {
            text.push_str(&b.display().to_string());
            text.push('\n');
        }
        std::fs::write(&self.path, text)
    }

    /// Toggle a path; returns the new bookmark list.
    pub fn toggle(&self, bookmarks: &mut Vec<PathBuf>, path: &Path) -> std::io::Result<bool> {
        let added = if let Some(pos) = bookmarks.iter().position(|b| b == path) {
            bookmarks.remove(pos);
            false
        } else {
            bookmarks.push(path.to_path_buf());
            true
        };
        self.save(bookmarks)?;
        Ok(added)
    }
}

/// In-memory bookmark store for tests.
#[derive(Default, Debug)]
pub struct MemoryBookmarks {
    pub saved: Vec<PathBuf>,
}

impl MemoryBookmarks {
    pub fn toggle(&mut self, bookmarks: &mut Vec<PathBuf>, path: &Path) -> bool {
        let added = if let Some(pos) = bookmarks.iter().position(|b| b == path) {
            bookmarks.remove(pos);
            false
        } else {
            bookmarks.push(path.to_path_buf());
            true
        };
        self.saved = bookmarks.clone();
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tui-explorer-bm-{}", std::process::id()));
        let store = BookmarkStore::new(dir.join("bookmarks.txt"));
        let mut bm = Vec::new();
        assert!(store.toggle(&mut bm, Path::new("/tmp")).unwrap());
        assert!(!store.toggle(&mut bm, Path::new("/tmp")).unwrap());
        assert!(bm.is_empty());
        store.toggle(&mut bm, Path::new("/var")).unwrap();
        assert_eq!(store.load(), vec![PathBuf::from("/var")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mount_label_formats() {
        let m = MountInfo {
            path: PathBuf::from("/data"),
            fs: "ext4".to_string(),
            used: 512 * 1024 * 1024 * 1024,
            total: 1024 * 1024 * 1024 * 1024,
        };
        assert_eq!(mount_label(&m), "/data ext4  512G / 1.0T");
    }
}
