//! Recursive `:find` / `:grep` search and folder measurements (`du`, item
//! counts). Pure functions over a [`FileSystem`] so the production worker
//! and the test harness share one implementation.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::app::state::FindHit;
use crate::filesystem::{EntryKind, FileSystem};

/// Most hits returned by one search.
pub const MAX_HITS: usize = 2000;
/// Deepest folder level searched below the root.
pub const MAX_DEPTH: usize = 16;
/// Upper bound on folders visited by one walk.
pub const MAX_DIRS: usize = 20_000;
/// Largest file searched by `:grep`.
pub const MAX_GREP_BYTES: u64 = 4 * 1024 * 1024;
/// Hits kept per file for `:grep`.
pub const MAX_LINES_PER_FILE: usize = 20;

/// Folders never descended into (noise and huge build trees).
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "__pycache__",
];

/// Case-insensitive glob (`*`, `?`) or substring match on a file name.
pub fn name_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let name = name.to_lowercase();
    if pattern.contains(['*', '?']) {
        glob(pattern.as_bytes(), name.as_bytes())
    } else {
        name.contains(&pattern)
    }
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

/// Smart-case line match: case-insensitive unless the query has capitals.
fn line_matches(query: &str, line: &str) -> bool {
    if query.chars().any(char::is_uppercase) {
        line.contains(query)
    } else {
        line.to_lowercase().contains(query)
    }
}

/// Breadth-first search below `root`. Names match `query` as a glob or
/// substring; with `content`, regular files are read through `read` and
/// every matching line becomes a hit. Hidden folders are skipped unless the
/// query itself starts with a dot. `cancelled` is polled between folders.
pub fn find(
    fs: &dyn FileSystem,
    root: &Path,
    query: &str,
    content: bool,
    read: &dyn Fn(&Path, u64) -> Option<String>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<FindHit> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return hits;
    }
    let want_hidden = query.starts_with('.');
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root.to_path_buf(), 0)]);
    let mut visited = 0usize;
    while let Some((dir, depth)) = queue.pop_front() {
        visited += 1;
        if visited > MAX_DIRS || hits.len() >= MAX_HITS || cancelled() {
            break;
        }
        let Ok(mut entries) = fs.read_dir(&dir) else {
            continue;
        };
        entries.sort_by(|a, b| crate::browser::natural_cmp(&a.display_name(), &b.display_name()));
        for entry in entries {
            if hits.len() >= MAX_HITS {
                break;
            }
            let name = entry.display_name();
            let is_dir = entry.kind.is_dir();
            if is_dir {
                let skip = SKIP_DIRS.contains(&name.as_str()) || (entry.hidden && !want_hidden);
                if !skip && depth < MAX_DEPTH {
                    queue.push_back((entry.path.clone(), depth + 1));
                }
            }
            if content {
                if matches!(entry.kind, EntryKind::File)
                    && entry.size <= MAX_GREP_BYTES
                    && (!entry.hidden || want_hidden)
                    && let Some(text) = read(&entry.path, MAX_GREP_BYTES)
                {
                    for (n, line) in text
                        .lines()
                        .enumerate()
                        .filter(|(_, line)| line_matches(query, line))
                        .take(MAX_LINES_PER_FILE)
                    {
                        let shown: String = line.trim().chars().take(240).collect();
                        hits.push(FindHit {
                            path: entry.path.clone(),
                            is_dir: false,
                            line: Some((n + 1, shown)),
                        });
                    }
                }
            } else if (!entry.hidden || want_hidden) && name_matches(query, &name) {
                hits.push(FindHit {
                    path: entry.path.clone(),
                    is_dir: entry.is_dir_like(),
                    line: None,
                });
            }
        }
    }
    hits
}

/// Reads a text file for `:grep`: `None` for binaries (NUL bytes) and
/// files over `limit`.
pub fn read_text(path: &Path, limit: u64) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Total size of everything below `path` (symlinks are not followed).
pub fn disk_usage(fs: &dyn FileSystem, path: &Path, cancelled: &dyn Fn() -> bool) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        visited += 1;
        if visited > MAX_DIRS * 10 || cancelled() {
            break;
        }
        let Ok(entries) = fs.read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            match entry.kind {
                EntryKind::Directory => stack.push(entry.path),
                EntryKind::Symlink { .. } => {}
                _ => total = total.saturating_add(entry.size),
            }
        }
    }
    total
}

/// Number of entries directly inside `path`.
pub fn count_children(fs: &dyn FileSystem, path: &Path) -> Option<u32> {
    fs.read_dir(path).ok().map(|e| e.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MemoryFileSystem;

    #[test]
    fn glob_and_substring_names() {
        assert!(name_matches("*.rs", "main.rs"));
        assert!(!name_matches("*.rs", "main.rsx"));
        assert!(name_matches("ma?n*", "MAIN.rs"));
        assert!(name_matches("read", "README.md"));
        assert!(!name_matches("zz", "README.md"));
    }

    #[test]
    fn find_walks_recursively_and_skips_noise() {
        let fs = crate::testing::builders::demo_fs();
        let hits = find(
            &fs,
            Path::new("/home/demo"),
            "*.rs",
            false,
            &|_, _| None,
            &|| false,
        );
        assert!(!hits.is_empty());
        assert!(
            hits.iter()
                .all(|h| h.path.to_string_lossy().ends_with(".rs"))
        );
    }

    #[test]
    fn grep_reports_line_numbers() {
        let fs = crate::testing::builders::demo_fs();
        let hits = find(
            &fs,
            Path::new("/home/demo"),
            "needle",
            true,
            &|_, _| Some("hay\nthe needle here\nhay".to_string()),
            &|| false,
        );
        assert!(!hits.is_empty());
        assert_eq!(hits[0].line.as_ref().map(|(n, _)| *n), Some(2));
    }

    #[test]
    fn disk_usage_sums_sizes() {
        let fs: MemoryFileSystem = crate::testing::builders::demo_fs();
        assert!(disk_usage(&fs, Path::new("/home/demo"), &|| false) > 0);
    }
}
