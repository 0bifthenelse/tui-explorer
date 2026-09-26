use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XdgDirs {
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
}

fn valid_absolute(value: Option<String>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    if path.is_absolute() { Some(path) } else { None }
}

pub fn resolve(get_env: &dyn Fn(&str) -> Option<String>) -> XdgDirs {
    let home = valid_absolute(get_env("HOME")).unwrap_or_else(|| PathBuf::from("/"));
    let data = valid_absolute(get_env("XDG_DATA_HOME"))
        .unwrap_or_else(|| home.join(".local").join("share"));
    let config = valid_absolute(get_env("XDG_CONFIG_HOME")).unwrap_or_else(|| home.join(".config"));
    let cache = valid_absolute(get_env("XDG_CACHE_HOME")).unwrap_or_else(|| home.join(".cache"));
    XdgDirs {
        data,
        config,
        cache,
    }
}

pub fn database_path(dirs: &XdgDirs) -> PathBuf {
    dirs.data.join("tui-explorer").join("tags.sqlite3")
}

pub fn bookmarks_path(dirs: &XdgDirs) -> PathBuf {
    dirs.data.join("tui-explorer").join("bookmarks.txt")
}

pub fn config_path(dirs: &XdgDirs) -> PathBuf {
    dirs.config.join("tui-explorer").join("config.toml")
}

pub fn cache_dir(dirs: &XdgDirs) -> PathBuf {
    dirs.cache.join("tui-explorer")
}

pub fn log_path(dirs: &XdgDirs) -> PathBuf {
    cache_dir(dirs).join("tui-explorer.log")
}

pub fn theme_path(dirs: &XdgDirs) -> PathBuf {
    dirs.config.join("tui-explorer").join("theme")
}

/// Reads the persisted theme index, or `None` when the file is missing,
/// unreadable, or does not hold a plain decimal index.
pub fn load_theme(path: &Path) -> Option<usize> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()
}

/// Writes the theme index as `"<index>\n"`, creating private parents first.
pub fn save_theme(path: &Path, index: usize) -> std::io::Result<()> {
    ensure_private_parent(path)?;
    std::fs::write(path, format!("{index}\n"))
}

#[cfg(unix)]
pub fn ensure_private_parent(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(parent) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn uses_xdg_data_home() {
        let dirs = resolve(&env(&[
            ("HOME", "/home/u"),
            ("XDG_DATA_HOME", "/xdg/data"),
            ("XDG_CONFIG_HOME", "/xdg/config"),
            ("XDG_CACHE_HOME", "/xdg/cache"),
        ]));
        assert_eq!(
            database_path(&dirs),
            PathBuf::from("/xdg/data/tui-explorer/tags.sqlite3")
        );
        assert_eq!(
            config_path(&dirs),
            PathBuf::from("/xdg/config/tui-explorer/config.toml")
        );
        assert_eq!(cache_dir(&dirs), PathBuf::from("/xdg/cache/tui-explorer"));
    }

    #[test]
    fn falls_back_to_home() {
        let dirs = resolve(&env(&[("HOME", "/home/u")]));
        assert_eq!(
            database_path(&dirs),
            PathBuf::from("/home/u/.local/share/tui-explorer/tags.sqlite3")
        );
        assert_eq!(
            config_path(&dirs),
            PathBuf::from("/home/u/.config/tui-explorer/config.toml")
        );
        assert_eq!(
            cache_dir(&dirs),
            PathBuf::from("/home/u/.cache/tui-explorer")
        );
    }

    #[test]
    fn rejects_relative_or_empty_xdg() {
        let dirs = resolve(&env(&[
            ("HOME", "/home/u"),
            ("XDG_DATA_HOME", "relative/path"),
            ("XDG_CACHE_HOME", ""),
        ]));
        assert_eq!(
            database_path(&dirs),
            PathBuf::from("/home/u/.local/share/tui-explorer/tags.sqlite3")
        );
        assert_eq!(
            cache_dir(&dirs),
            PathBuf::from("/home/u/.cache/tui-explorer")
        );
    }

    #[test]
    fn never_uses_usr() {
        let dirs = resolve(&env(&[]));
        for path in [database_path(&dirs), config_path(&dirs), cache_dir(&dirs)] {
            assert!(!path.starts_with("/usr"), "{}", path.display());
        }
    }

    #[test]
    fn theme_roundtrip_under_sandbox() {
        let dir = crate::filesystem::sandbox::fixture("config-theme");
        let path = theme_path(&XdgDirs {
            data: dir.clone(),
            config: dir.clone(),
            cache: dir.clone(),
        });
        assert!(load_theme(&path).is_none());
        save_theme(&path, 7).expect("theme save");
        assert_eq!(load_theme(&path), Some(7));
        assert_eq!(std::fs::read_to_string(&path).expect("theme read"), "7\n");
        std::fs::write(&path, "not-a-number").expect("malformed theme write");
        assert_eq!(load_theme(&path), None);
    }
}
