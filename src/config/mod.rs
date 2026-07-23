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
}
