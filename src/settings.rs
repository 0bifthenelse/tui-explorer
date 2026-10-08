//! Persisted session preferences (`session.json` in the data directory):
//! layout, sort, toggles, appearance, remembered "open with" associations,
//! marks, frecency, command history and the media volume.
//!
//! The file is written atomically (temp file + rename) and read
//! leniently: unknown keys are ignored and missing keys take defaults, so
//! older or hand-edited files never prevent startup.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    /// One row per entry with metadata columns.
    #[default]
    List,
    /// Icon tiles.
    Grid,
    /// Miller columns: parent | current | preview.
    Columns,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::List => "List",
            ViewMode::Grid => "Grid",
            ViewMode::Columns => "Columns",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ViewMode::List => ViewMode::Grid,
            ViewMode::Grid => ViewMode::Columns,
            ViewMode::Columns => ViewMode::List,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "list" | "details" | "l" => Some(ViewMode::List),
            "grid" | "icons" | "tiles" | "g" => Some(ViewMode::Grid),
            "columns" | "miller" | "column" | "c" => Some(ViewMode::Columns),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GridSize {
    Small,
    #[default]
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoOutput {
    /// Kitty graphics when available, else a GUI window when a display is
    /// present, else truecolor text output.
    #[default]
    Auto,
    Kitty,
    Sixel,
    Window,
    Tct,
}

impl VideoOutput {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(VideoOutput::Auto),
            "kitty" => Some(VideoOutput::Kitty),
            "sixel" => Some(VideoOutput::Sixel),
            "window" | "gui" => Some(VideoOutput::Window),
            "tct" | "text" | "terminal" => Some(VideoOutput::Tct),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VideoOutput::Auto => "auto",
            VideoOutput::Kitty => "kitty",
            VideoOutput::Sixel => "sixel",
            VideoOutput::Window => "window",
            VideoOutput::Tct => "tct",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleRender {
    /// Crisp caption strip drawn by the TUI from mpv's `sub-text`.
    #[default]
    Tui,
    /// mpv blends subtitles into the video frame itself.
    Mpv,
}

/// A remembered "open with" command for one extension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Association {
    /// Program plus arguments, split quote-aware at launch time.
    pub command: String,
    /// Launch detached (GUI programs) instead of suspending the TUI.
    #[serde(default)]
    pub detach: bool,
}

/// One frecency record: visit count and last visit (unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    pub path: PathBuf,
    pub count: u32,
    pub last: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub view: ViewMode,
    pub grid_size: GridSize,
    pub sort: String,
    pub show_hidden: bool,
    pub show_sidebar: Option<bool>,
    pub show_preview: Option<bool>,
    pub animations: bool,
    pub ascii: bool,
    pub nerd_icons: bool,
    pub video_output: VideoOutput,
    pub subtitle_render: SubtitleRender,
    pub volume: u8,
    /// Extension (lowercase, compound like `tar.gz` allowed, `url` for web
    /// links) to the remembered command.
    pub associations: BTreeMap<String, Association>,
    /// Single-character marks (`m<key>` / `` `<key> ``).
    pub marks: BTreeMap<String, PathBuf>,
    pub visits: Vec<Visit>,
    pub command_history: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            view: ViewMode::List,
            grid_size: GridSize::Large,
            sort: "name".to_string(),
            show_hidden: false,
            show_sidebar: None,
            show_preview: None,
            animations: true,
            ascii: false,
            nerd_icons: false,
            video_output: VideoOutput::Auto,
            subtitle_render: SubtitleRender::Tui,
            volume: 80,
            associations: BTreeMap::new(),
            marks: BTreeMap::new(),
            visits: Vec::new(),
            command_history: Vec::new(),
        }
    }
}

pub const MAX_VISITS: usize = 400;
pub const MAX_HISTORY: usize = 200;

impl Settings {
    /// Records a directory visit for frecency ranking.
    pub fn record_visit(&mut self, path: &Path, now: i64) {
        if let Some(visit) = self.visits.iter_mut().find(|v| v.path == path) {
            visit.count = visit.count.saturating_add(1);
            visit.last = now;
        } else {
            self.visits.push(Visit {
                path: path.to_path_buf(),
                count: 1,
                last: now,
            });
        }
        if self.visits.len() > MAX_VISITS {
            self.visits
                .sort_by(|a, b| frecency(b, now).total_cmp(&frecency(a, now)));
            self.visits.truncate(MAX_VISITS);
        }
    }

    /// Visits ranked by frecency (most relevant first).
    pub fn ranked_visits(&self, now: i64) -> Vec<&Visit> {
        let mut visits: Vec<&Visit> = self.visits.iter().collect();
        visits.sort_by(|a, b| frecency(b, now).total_cmp(&frecency(a, now)));
        visits
    }

    pub fn push_history(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        self.command_history.retain(|l| l != line);
        self.command_history.push(line.to_string());
        if self.command_history.len() > MAX_HISTORY {
            let excess = self.command_history.len() - MAX_HISTORY;
            self.command_history.drain(..excess);
        }
    }
}

/// zoxide-style frecency: visit count weighted by recency buckets.
pub fn frecency(visit: &Visit, now: i64) -> f64 {
    let age = (now - visit.last).max(0);
    let weight = if age < 3600 {
        4.0
    } else if age < 86_400 {
        2.0
    } else if age < 604_800 {
        0.5
    } else {
        0.25
    };
    f64::from(visit.count) * weight
}

/// Lowercase extension key for associations: compound archive suffixes
/// first (`tar.gz`), then the plain extension; `None` for extensionless.
pub fn association_key(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    for compound in ["tar.gz", "tar.xz", "tar.bz2", "tar.zst"] {
        if name.ends_with(&format!(".{compound}")) {
            return Some(compound.to_string());
        }
    }
    let (base, ext) = name.rsplit_once('.')?;
    if base.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_string())
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        SettingsStore { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Settings {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, settings: &Settings) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
        write_atomic(&self.path, text.as_bytes())
    }
}

/// Writes `bytes` to a sibling temp file, flushes it, then renames it over
/// `path`, so readers never observe a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_mode_parse_and_cycle() {
        assert_eq!(ViewMode::parse("Miller"), Some(ViewMode::Columns));
        assert_eq!(ViewMode::parse("nope"), None);
        assert_eq!(ViewMode::List.next().next().next(), ViewMode::List);
    }

    #[test]
    fn lenient_load_fills_defaults() {
        let parsed: Settings = serde_json::from_str(r#"{"view":"grid","bogus":1}"#).unwrap();
        assert_eq!(parsed.view, ViewMode::Grid);
        assert_eq!(parsed.volume, 80);
        assert!(parsed.animations);
    }

    #[test]
    fn association_keys() {
        assert_eq!(
            association_key(Path::new("/a/b.TAR.GZ")).as_deref(),
            Some("tar.gz")
        );
        assert_eq!(
            association_key(Path::new("/a/doc.pdf")).as_deref(),
            Some("pdf")
        );
        assert_eq!(association_key(Path::new("/a/.bashrc")), None);
        assert_eq!(association_key(Path::new("/a/Makefile")), None);
    }

    #[test]
    fn frecency_prefers_recent_and_frequent() {
        let mut s = Settings::default();
        s.record_visit(Path::new("/old"), 0);
        for _ in 0..3 {
            s.record_visit(Path::new("/often"), 1_000_000);
        }
        s.record_visit(Path::new("/new"), 1_000_000);
        let ranked = s.ranked_visits(1_000_100);
        assert_eq!(ranked[0].path, PathBuf::from("/often"));
        assert_eq!(ranked.last().unwrap().path, PathBuf::from("/old"));
    }

    #[test]
    fn history_dedups_and_caps() {
        let mut s = Settings::default();
        s.push_history("cd /");
        s.push_history("mkdir x");
        s.push_history("cd /");
        assert_eq!(s.command_history, vec!["mkdir x", "cd /"]);
        for i in 0..300 {
            s.push_history(&format!("c{i}"));
        }
        assert_eq!(s.command_history.len(), MAX_HISTORY);
    }

    #[test]
    fn atomic_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("tui-explorer-settings-{}", std::process::id()));
        let store = SettingsStore::new(dir.join("session.json"));
        let mut s = Settings {
            view: ViewMode::Columns,
            ..Settings::default()
        };
        s.marks.insert("a".into(), PathBuf::from("/tmp"));
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
