//! Opening entries. Enter resolves, in order:
//! 1. folders (and links to folders) are entered;
//! 2. a remembered "open with" command for the extension runs;
//! 3. built-in handlers: shortcut files open their URL, audio/video play
//!    in the player, text and images open in quick look;
//! 4. anything else asks which program to use (and remembers it).

use std::path::{Path, PathBuf};

use crate::app::effects::Effect;
use crate::app::state::{AppState, Mode, OpenWithState};
use crate::icons::{IconKind, IconResolver};
use crate::media::classify_path;
use crate::settings::{Association, association_key};

/// Terminal programs that must take over the TUI rather than detach.
const TERMINAL_PROGRAMS: &[&str] = &[
    "vi", "vim", "nvim", "nano", "micro", "hx", "helix", "emacs", "kak", "less", "more", "most",
    "bat", "man", "htop", "btop", "top", "mc", "ranger", "lf", "nnn", "mutt", "neomutt", "w3m",
    "lynx", "elinks", "ncdu", "sh", "bash", "zsh", "fish", "python", "python3", "ipython",
    "sqlite3", "cat", "hexyl", "xxd", "jq", "tig", "lazygit", "gitui", "visidata", "vd", "termpdf",
    "tpdf", "timg", "chafa", "viu", "mpv",
];

/// Whether a command should detach (GUI program with a display present).
pub fn default_detach(command: &str) -> bool {
    let has_display =
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !has_display {
        return false;
    }
    let program = crate::input::command::split_words(command)
        .ok()
        .and_then(|w| w.into_iter().next())
        .unwrap_or_default();
    let base = Path::new(&program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(program);
    !TERMINAL_PROGRAMS.contains(&base.as_str())
}

/// Kinds shown by the built-in quick look instead of an external program.
fn quick_look_kind(kind: IconKind) -> bool {
    use IconKind::*;
    matches!(
        kind,
        SourceFile
            | Rust
            | TypeScript
            | JavaScript
            | C
            | Cpp
            | Python
            | Shell
            | Html
            | Css
            | Json
            | Toml
            | Yaml
            | Markdown
            | Text
            | Subtitle
            | Image
            | CargoToml
            | CargoLock
            | PackageJson
            | Lockfile
            | Makefile
            | Docker
            | Config
            | Git
    )
}

/// Runs the remembered association for `path`, if any.
pub fn association_effect(state: &AppState, path: &Path) -> Option<Effect> {
    let key = association_key(path)?;
    let assoc = state.settings.associations.get(&key)?;
    launch(path, assoc)
}

pub fn launch(path: &Path, assoc: &Association) -> Option<Effect> {
    let words = crate::input::command::split_words(&assoc.command).ok()?;
    let (program, args) = words.split_first()?;
    Some(if assoc.detach {
        Effect::SpawnDetached {
            path: path.to_path_buf(),
            program: program.clone(),
            args: args.to_vec(),
        }
    } else {
        Effect::OpenPathWith {
            path: path.to_path_buf(),
            program: program.clone(),
            args: args.to_vec(),
        }
    })
}

/// Opens a file entry (not a folder) following the resolution order.
pub fn open_file(state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    if let Some(effect) = association_effect(state, &path) {
        return vec![effect];
    }
    if let Some(url) = crate::app::links::shortcut_url(&path) {
        return crate::app::links::open_url(state, &url);
    }
    if let Some(kind) = classify_path(&path) {
        return crate::app::reduce::start_media_session(state, path, kind);
    }
    let entry = state
        .browser
        .entries
        .iter()
        .find(|e| e.entry.path == path)
        .map(|e| e.entry.clone());
    if let Some(entry) = entry {
        let kind = IconResolver::default().resolve(&entry);
        let executable_script = entry.executable && kind == IconKind::Executable;
        if quick_look_kind(kind)
            && !executable_script
            && state.browser.focused().map(|f| &f.entry.path) == Some(&path)
        {
            state.mode = Mode::QuickLook(Box::default());
            return Vec::new();
        }
    }
    prompt(state, path);
    Vec::new()
}

/// The open-with prompt, pre-filled with the remembered command or the
/// first suggestion detected on PATH.
pub fn prompt(state: &mut AppState, target: PathBuf) {
    let remembered = association_key(&target)
        .and_then(|k| state.settings.associations.get(&k))
        .map(|a| a.command.clone());
    let suggestions = suggestions_for(&target);
    let remember = classify_path(&target).is_none();
    state.mode = Mode::OpenWith(Box::new(OpenWithState {
        target,
        input: remembered.unwrap_or_default(),
        suggestions,
        suggestion: None,
        remember,
    }));
}

/// Likely openers for a file type that exist on PATH.
pub fn suggestions_for(path: &Path) -> Vec<String> {
    let ext = association_key(path).unwrap_or_default();
    let candidates: &[&str] = match ext.as_str() {
        "pdf" | "epub" | "djvu" | "cbz" => &["zathura", "evince", "okular", "mupdf", "xdg-open"],
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "tiff" => {
            &["imv", "feh", "eog", "sxiv", "nsxiv", "gimp", "xdg-open"]
        }
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "mp3" | "flac" | "ogg" | "opus" | "wav" => {
            &["mpv", "vlc", "celluloid", "xdg-open"]
        }
        "zip" | "tar" | "gz" | "tar.gz" | "xz" | "tar.xz" | "7z" | "rar" | "zst" | "tar.zst"
        | "bz2" | "tar.bz2" => &["file-roller", "ark", "xarchiver", "atool", "xdg-open"],
        "html" | "htm" => &["firefox", "chromium", "xdg-open"],
        "doc" | "docx" | "odt" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" => {
            &["libreoffice", "xdg-open"]
        }
        _ => &["nvim", "vim", "nano", "code", "xdg-open"],
    };
    let mut found: Vec<String> = Vec::new();
    if let Ok(editor) = std::env::var("EDITOR")
        && !editor.trim().is_empty()
        && !matches!(
            ext.as_str(),
            "pdf" | "png" | "jpg" | "jpeg" | "mp4" | "mkv" | "zip"
        )
    {
        found.push(editor);
    }
    for c in candidates {
        if on_path(c) && !found.iter().any(|f| f == c) {
            found.push(c.to_string());
        }
    }
    found.truncate(6);
    found
}

/// True when `program` is an executable file somewhere on `PATH`.
pub fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(program);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            candidate.is_file()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_programs_never_detach() {
        // Without a display nothing detaches; with one, GUI programs do.
        // SAFETY of env mutation is not needed: we only read here.
        let has_display =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        assert!(!default_detach("nvim -R"));
        assert_eq!(default_detach("zathura"), has_display);
    }

    #[test]
    fn launch_builds_detached_or_attached_effects() {
        let assoc = Association {
            command: "zathura --fork".into(),
            detach: true,
        };
        match launch(Path::new("/d/a.pdf"), &assoc) {
            Some(Effect::SpawnDetached { program, args, .. }) => {
                assert_eq!(program, "zathura");
                assert_eq!(args, vec!["--fork".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        let assoc = Association {
            command: "nvim".into(),
            detach: false,
        };
        assert!(matches!(
            launch(Path::new("/d/a.rs"), &assoc),
            Some(Effect::OpenPathWith { .. })
        ));
    }
}
