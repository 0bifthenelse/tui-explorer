//! Repository contained test sandbox: fixtures, XDG root, and the guarded delete choke point.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Environment variable that pins every destructive operation to a caller supplied root.
pub const OVERRIDE_ENV: &str = "TUI_EXPLORER_SANDBOX_ROOT";

/// Returns the configured sandbox root, but only when it is an existing absolute directory.
pub fn root_override() -> Option<PathBuf> {
    let raw = std::env::var_os(OVERRIDE_ENV)?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() || !path.is_dir() {
        return None;
    }
    Some(path)
}

/// Returns the in repository sandbox root, independent of any override.
pub fn default_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/tui-explorer-sandbox")
}

/// Returns the directory holding generated test fixtures.
pub fn fixtures_root() -> PathBuf {
    default_root().join("fixtures")
}

/// Returns the per process XDG base directory for tests.
pub fn xdg_root() -> PathBuf {
    default_root().join("xdg")
}

/// Creates `path` and all missing parents.
pub fn ensure(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Reports whether `path` resolves inside `root`, failing closed on any resolution error.
pub fn contains(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(base), Ok(target)) => target.starts_with(base),
        _ => false,
    }
}

/// Deletes a file or a whole directory tree, refusing anything outside the override root.
pub fn discard(path: &Path) -> Result<(), String> {
    if let Some(root) = root_override()
        && !contains(&root, path)
    {
        return Err(format!(
            "refusing to delete {}: outside sandbox root {}",
            path.display(),
            root.display()
        ));
    }
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
    let result = if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|e| format!("cannot delete {}: {e}", path.display()))
}

/// Creates a fresh fixture directory named after the caller tag, unique per process and call.
pub fn fixture(tag: &str) -> PathBuf {
    let path = fixtures_root().join(named(tag));
    if path.symlink_metadata().is_ok() {
        discard(&path).expect("sandbox fixture removal");
    }
    ensure(&path).expect("sandbox fixture creation");
    path
}

fn named(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{tag}-{}-{nanos}", std::process::id())
}
