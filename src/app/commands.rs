//! Execution of the Ranger-style colon commands added on top of the
//! original set (`:search`, `:create`, `:find`, `:chmod`, `:shell`,
//! `:set`, ...).

use std::path::{Path, PathBuf};

use crate::app::action::Action;
use crate::app::effects::Effect;
use crate::app::reduce::{reduce_inner, resolve_user_path, start_operation};
use crate::app::state::{AppState, StatusMessage};
use crate::browser::SortMode;
use crate::input::command::Command;
use crate::operations::{ConflictPolicy, OperationKind, OperationPlan, validate};
use crate::settings::{GridSize, SubtitleRender, VideoOutput, ViewMode};

fn info(state: &mut AppState, text: impl Into<String>) {
    state.message = Some(StatusMessage::info(text));
}

/// Runs an extended command, or hands the original-set command back.
pub fn run_or_return(state: &mut AppState, cmd: Command) -> Result<Vec<Effect>, Command> {
    match cmd {
        Command::Search { .. }
        | Command::Find { .. }
        | Command::Grep { .. }
        | Command::Create { .. }
        | Command::Trash
        | Command::Undo
        | Command::Chmod { .. }
        | Command::Symlink { .. }
        | Command::BulkRename
        | Command::Du
        | Command::Shell { .. }
        | Command::Tab { .. }
        | Command::Mark { .. }
        | Command::View { .. }
        | Command::Set { .. }
        | Command::BookmarkUrl { .. }
        | Command::Url { .. }
        | Command::Links
        | Command::Assoc { .. }
        | Command::Unassoc { .. }
        | Command::Play
        | Command::Pause
        | Command::Next
        | Command::Prev
        | Command::Queue
        | Command::Sub { .. } => Ok(run(state, cmd).unwrap_or_default()),
        other => Err(other),
    }
}

/// Runs one of the extended commands; `None` when `cmd` is not one of
/// them (the reducer handles the original set).
pub fn run(state: &mut AppState, cmd: Command) -> Option<Vec<Effect>> {
    Some(match cmd {
        Command::Search { query } => {
            state.browser.search = Some(query.clone());
            let fx = reduce_inner(state, Action::SearchNext);
            if crate::app::ranger::search_matches(state).is_empty() {
                state.browser.search = None;
                state.set_error(format!("no match for {query}"));
            }
            fx
        }
        Command::Find { pattern } => {
            info(state, format!("finding {pattern}…"));
            vec![Effect::FindFiles {
                root: state.browser.cwd.clone(),
                query: pattern,
                content: false,
            }]
        }
        Command::Grep { text } => {
            info(state, format!("searching contents for {text}…"));
            vec![Effect::FindFiles {
                root: state.browser.cwd.clone(),
                query: text,
                content: true,
            }]
        }
        Command::Create { name } => create(state, &name),
        Command::Trash => crate::app::ranger::trash_selection(state),
        Command::Undo => crate::app::ranger::undo(state),
        Command::Chmod { mode } => chmod(state, &mode),
        Command::Symlink { name } => symlink(state, name),
        Command::BulkRename => {
            let targets = state.browser.action_targets();
            if targets.is_empty() {
                info(state, "nothing to rename");
                return Some(Vec::new());
            }
            vec![Effect::BulkRename(targets)]
        }
        Command::Du => crate::app::ranger::disk_usage(state),
        Command::Shell { command } => {
            let expanded = expand_macros(state, &command);
            vec![
                Effect::RunShell {
                    command: Some(expanded),
                    cwd: state.browser.cwd.clone(),
                },
                Effect::LoadDirectory(state.browser.cwd.clone()),
            ]
        }
        Command::Tab { arg } => match arg.as_deref() {
            None | Some("new") => reduce_inner(state, Action::TabNew),
            Some("close") => reduce_inner(state, Action::TabClose),
            Some("next") => reduce_inner(state, Action::TabNext),
            Some("prev") | Some("previous") => reduce_inner(state, Action::TabPrev),
            Some("restore") => reduce_inner(state, Action::TabRestore),
            Some(n) => match n.parse::<usize>() {
                Ok(i) if i >= 1 => reduce_inner(state, Action::TabSelect(i - 1)),
                _ => {
                    state.set_error("tab expects new, close, next, prev, restore or a number");
                    Vec::new()
                }
            },
        },
        Command::Mark { key } => crate::app::ranger::set_mark(state, key),
        Command::View { mode } => match ViewMode::parse(&mode) {
            Some(view) => reduce_inner(state, Action::SetView(view)),
            None => {
                state.set_error("view expects list, grid or columns");
                Vec::new()
            }
        },
        Command::Set { key, value } => set(state, &key, value.as_deref()),
        Command::BookmarkUrl { url, title } => crate::app::links::bookmark_url(state, url, title),
        Command::Url { url } => crate::app::links::open_url(state, &url),
        Command::Links => {
            crate::app::hub::open(state);
            if let crate::app::state::Mode::Bookmarks(nav) = &mut state.mode {
                nav.section = crate::app::state::HubSection::Links;
            }
            crate::app::hub::refresh(state);
            Vec::new()
        }
        Command::Assoc { ext, command } => assoc(state, ext, command),
        Command::Unassoc { ext } => {
            if state.settings.associations.remove(&ext).is_some() {
                state.settings_dirty = true;
                info(state, format!("forgot the opener for .{ext}"));
            } else {
                info(state, format!("no opener remembered for .{ext}"));
            }
            Vec::new()
        }
        Command::Play | Command::Pause => reduce_inner(state, Action::MediaTogglePause),
        Command::Next => reduce_inner(state, Action::MediaNext),
        Command::Prev => reduce_inner(state, Action::MediaPrev),
        Command::Queue => reduce_inner(state, Action::MediaEnqueue),
        Command::Sub { path } => {
            let path = resolve_user_path(state, &path);
            reduce_inner(state, Action::MediaAddSub(path))
        }
        _ => return None,
    })
}

/// `:create a/b/c.txt` makes parents as needed; a trailing `/` creates a
/// folder. The new entry is focused once it exists.
fn create(state: &mut AppState, name: &str) -> Vec<Effect> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "/" {
        state.set_error("create expects a name");
        return Vec::new();
    }
    if trimmed.split('/').any(|seg| seg == "..") {
        state.set_error("create never leaves the current folder (..)");
        return Vec::new();
    }
    let is_dir = trimmed.ends_with('/');
    let path = resolve_user_path(state, trimmed.trim_end_matches('/'));
    if state.browser.entries.iter().any(|e| e.entry.path == path) {
        state.set_error(format!("{} already exists", path.display()));
        return Vec::new();
    }
    vec![Effect::CreateEntry { path, is_dir }]
}

/// Parses `644`, `0755`, `+x`, `u+x`, `go-w`, `a=r` against `current`.
pub fn apply_mode_spec(current: u32, spec: &str) -> Option<u32> {
    let spec = spec.trim();
    if !spec.is_empty() && spec.chars().all(|c| c.is_digit(8)) {
        return u32::from_str_radix(spec, 8).ok().filter(|m| *m <= 0o7777);
    }
    let mut mode = current & 0o7777;
    for clause in spec.split(',') {
        let op_at = clause.find(['+', '-', '='])?;
        let (who, rest) = clause.split_at(op_at);
        let op = rest.chars().next()?;
        let perms = &rest[1..];
        let mut who_mask = 0u32;
        for c in who.chars() {
            who_mask |= match c {
                'u' => 0o700,
                'g' => 0o070,
                'o' => 0o007,
                'a' => 0o777,
                _ => return None,
            };
        }
        if who_mask == 0 {
            who_mask = 0o777;
        }
        let mut bits = 0u32;
        for c in perms.chars() {
            bits |= match c {
                'r' => 0o444,
                'w' => 0o222,
                'x' => 0o111,
                _ => return None,
            };
        }
        let bits = bits & who_mask;
        match op {
            '+' => mode |= bits,
            '-' => mode &= !bits,
            '=' => mode = (mode & !who_mask) | bits,
            _ => return None,
        }
    }
    Some(mode)
}

fn chmod(state: &mut AppState, spec: &str) -> Vec<Effect> {
    let targets = state.browser.action_targets();
    let mut changes = Vec::new();
    for path in targets {
        let current = state
            .browser
            .entries
            .iter()
            .find(|e| e.entry.path == path)
            .map(|e| e.entry.mode)
            .unwrap_or(0o644);
        match apply_mode_spec(current, spec) {
            Some(mode) => changes.push((path, mode)),
            None => {
                state.set_error(format!("invalid mode {spec} (try 644, +x, u+x, go-w)"));
                return Vec::new();
            }
        }
    }
    if changes.is_empty() {
        return Vec::new();
    }
    let what = match changes.as_slice() {
        [(path, mode)] => format!(
            "{} is now {:o}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            mode & 0o7777
        ),
        many => format!("chmod {spec} on {} entries", many.len()),
    };
    info(state, what);
    vec![
        Effect::Chmod(changes),
        Effect::LoadDirectory(state.browser.cwd.clone()),
    ]
}

fn symlink(state: &mut AppState, name: String) -> Vec<Effect> {
    let Some(view) = state.browser.focused() else {
        return Vec::new();
    };
    let plan = OperationPlan {
        kind: OperationKind::Symlink,
        sources: vec![view.entry.path.clone()],
        dest_dir: Some(state.browser.cwd.clone()),
        rename_to: Some(std::ffi::OsString::from(name)),
        policy: ConflictPolicy::Ask,
    };
    match validate(&plan) {
        Ok(()) => start_operation(state, plan),
        Err(e) => {
            state.set_error(e.to_string());
            Vec::new()
        }
    }
}

/// Single-quotes `text` for POSIX shells.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Expands ranger macros: `%f` focused path, `%s` selection (or focus),
/// `%d` current folder, `%%` a literal percent.
pub fn expand_macros(state: &AppState, command: &str) -> String {
    let focused = state
        .browser
        .focused()
        .map(|v| shell_quote(&v.entry.path.display().to_string()))
        .unwrap_or_default();
    let selection = state
        .browser
        .action_targets()
        .iter()
        .map(|p| shell_quote(&p.display().to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    let dir = shell_quote(&state.browser.cwd.display().to_string());
    let mut out = String::new();
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('f') => {
                    out.push_str(&focused);
                    chars.next();
                }
                Some('s') => {
                    out.push_str(&selection);
                    chars.next();
                }
                Some('d') => {
                    out.push_str(&dir);
                    chars.next();
                }
                Some('%') => {
                    out.push('%');
                    chars.next();
                }
                _ => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn on_off(value: Option<&str>, current: bool) -> Option<bool> {
    match value.map(|v| v.to_ascii_lowercase()) {
        None => Some(!current),
        Some(v) => match v.as_str() {
            "on" | "true" | "yes" | "1" => Some(true),
            "off" | "false" | "no" | "0" => Some(false),
            "toggle" => Some(!current),
            _ => None,
        },
    }
}

fn set(state: &mut AppState, key: &str, value: Option<&str>) -> Vec<Effect> {
    let bad = |state: &mut AppState, expected: &str| {
        state.set_error(format!("set {key} expects {expected}"));
        Vec::new()
    };
    match key {
        "animations" | "motion" => match on_off(value, state.settings.animations) {
            Some(on) if on != state.settings.animations => {
                crate::app::ranger::toggle_animations(state);
                Vec::new()
            }
            Some(_) => Vec::new(),
            None => bad(state, "on or off"),
        },
        "hidden" => match on_off(value, state.browser.show_hidden) {
            Some(on) => {
                if on != state.browser.show_hidden {
                    state.browser.toggle_hidden();
                }
                state.settings_dirty = true;
                Vec::new()
            }
            None => bad(state, "on or off"),
        },
        "ascii" | "charset" => {
            let ascii = match value {
                Some("ascii") => Some(true),
                Some("unicode") => Some(false),
                other => on_off(other, state.settings.ascii),
            };
            match ascii {
                Some(ascii) => {
                    state.settings.ascii = ascii;
                    state.settings_dirty = true;
                    crate::ui::glyphs::set_charset(if ascii {
                        crate::ui::glyphs::Charset::Ascii
                    } else {
                        crate::ui::glyphs::Charset::Unicode
                    });
                    info(
                        state,
                        if ascii {
                            "ASCII glyphs"
                        } else {
                            "Unicode glyphs"
                        },
                    );
                    Vec::new()
                }
                None => bad(state, "ascii or unicode"),
            }
        }
        "icons" | "nerd" => {
            let nerd = match value {
                Some("nerd") => Some(true),
                Some("badges") | Some("text") => Some(false),
                other => on_off(other, state.settings.nerd_icons),
            };
            match nerd {
                Some(nerd) => {
                    state.settings.nerd_icons = nerd;
                    state.settings_dirty = true;
                    crate::ui::glyphs::set_icon_style(if nerd {
                        crate::ui::glyphs::IconStyle::Nerd
                    } else {
                        crate::ui::glyphs::IconStyle::Badges
                    });
                    Vec::new()
                }
                None => bad(state, "nerd or badges"),
            }
        }
        "preview" => match on_off(
            value,
            crate::ui::preview_visible(state.width, state.height, state.show_preview),
        ) {
            Some(on) => {
                state.show_preview = Some(on);
                state.settings_dirty = true;
                Vec::new()
            }
            None => bad(state, "on or off"),
        },
        "sidebar" => match on_off(
            value,
            crate::ui::sidebar_visible(state.width, state.height, state.show_sidebar),
        ) {
            Some(on) => {
                state.show_sidebar = Some(on);
                state.settings_dirty = true;
                Vec::new()
            }
            None => bad(state, "on or off"),
        },
        "view" | "layout" => match value.and_then(ViewMode::parse) {
            Some(view) => reduce_inner(state, Action::SetView(view)),
            None => bad(state, "list, grid or columns"),
        },
        "grid" => match value {
            Some("large") | Some("big") => reduce_inner(state, Action::GridZoom(true)),
            Some("small") | Some("compact") => reduce_inner(state, Action::GridZoom(false)),
            _ => {
                let _ = GridSize::Large;
                bad(state, "large or small")
            }
        },
        "sort" => match value.and_then(SortMode::parse) {
            Some(mode) => reduce_inner(state, Action::SetSort(mode)),
            None => bad(state, "name, size, modified, type or extension (-desc)"),
        },
        "video" | "video_output" => match value.and_then(VideoOutput::parse) {
            Some(out) => {
                state.settings.video_output = out;
                state.settings_dirty = true;
                info(state, format!("video output: {}", out.label()));
                Vec::new()
            }
            None => bad(state, "auto, kitty, window or tct"),
        },
        "subs" | "subtitle_render" | "subtitles" => match value {
            Some("tui") | Some("captions") => {
                state.settings.subtitle_render = SubtitleRender::Tui;
                state.settings_dirty = true;
                Vec::new()
            }
            Some("mpv") | Some("burned") => {
                state.settings.subtitle_render = SubtitleRender::Mpv;
                state.settings_dirty = true;
                Vec::new()
            }
            _ => bad(state, "tui or mpv"),
        },
        "volume" => match value.and_then(|v| v.trim_end_matches('%').parse::<u8>().ok()) {
            Some(v) => {
                state.settings.volume = v.min(130);
                state.settings_dirty = true;
                reduce_inner(state, Action::MediaSetVolume(v.min(130)))
            }
            None => bad(state, "0-130"),
        },
        _ => {
            state.set_error(format!(
                "unknown setting {key} (animations hidden ascii icons preview sidebar view grid sort video subs volume)"
            ));
            Vec::new()
        }
    }
}

fn assoc(state: &mut AppState, ext: Option<String>, command: Option<String>) -> Vec<Effect> {
    match (ext, command) {
        (None, _) => {
            let list = state
                .settings
                .associations
                .iter()
                .map(|(e, a)| format!(".{e} → {}", a.command))
                .collect::<Vec<_>>()
                .join(" · ");
            info(
                state,
                if list.is_empty() {
                    "no remembered openers (r opens with, then remembers)".to_string()
                } else {
                    list
                },
            );
            Vec::new()
        }
        (Some(ext), None) => {
            let text = match state.settings.associations.get(&ext) {
                Some(a) => format!(".{ext} opens with {}", a.command),
                None => format!("no opener remembered for .{ext}"),
            };
            info(state, text);
            Vec::new()
        }
        (Some(ext), Some(command)) => {
            let detach = crate::app::open::default_detach(&command);
            state.settings.associations.insert(
                ext.clone(),
                crate::settings::Association {
                    command: command.clone(),
                    detach,
                },
            );
            state.settings_dirty = true;
            info(state, format!(".{ext} now opens with {command}"));
            Vec::new()
        }
    }
}

/// Moves a path's name into `dir` (bulk rename / results helpers).
pub fn sibling(path: &Path, name: &str) -> PathBuf {
    path.parent()
        .map(|p| p.join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Turns the names edited in `$EDITOR` (one line per path, same order)
/// into rename pairs. Unchanged lines are skipped; empty names, `..`,
/// duplicate targets and a changed line count are rejected.
pub fn bulk_rename_pairs(
    paths: &[PathBuf],
    names: &[String],
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let names: Vec<&str> = names
        .iter()
        .map(|n| n.trim_end_matches(['\r', '\n']))
        .collect();
    // Trailing blank lines (editors add a final newline) do not count.
    let mut names = names;
    while names.len() > paths.len() && names.last().is_some_and(|n| n.trim().is_empty()) {
        names.pop();
    }
    if names.len() != paths.len() {
        return Err(format!(
            "bulk rename: expected {} names, got {} (lines must not be added or removed)",
            paths.len(),
            names.len()
        ));
    }
    let mut pairs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (path, name) in paths.iter().zip(names) {
        let old = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == old {
            seen.insert(path.clone());
            continue;
        }
        if name.trim().is_empty() {
            return Err(format!("bulk rename: empty name for {old}"));
        }
        if Path::new(name).components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }) {
            return Err(format!("bulk rename: {name} leaves the folder"));
        }
        let target = sibling(path, name);
        if !seen.insert(target.clone()) {
            return Err(format!("bulk rename: two entries would be named {name}"));
        }
        pairs.push((path.clone(), target));
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_rename_pairs_validate_names() {
        let paths = vec![PathBuf::from("/d/a.txt"), PathBuf::from("/d/b.txt")];
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            bulk_rename_pairs(&paths, &names(&["a.txt", "c.txt", ""])).unwrap(),
            vec![(PathBuf::from("/d/b.txt"), PathBuf::from("/d/c.txt"))]
        );
        assert!(bulk_rename_pairs(&paths, &names(&["a.txt"])).is_err());
        assert!(bulk_rename_pairs(&paths, &names(&["x", "x"])).is_err());
        assert!(bulk_rename_pairs(&paths, &names(&["../x", "b.txt"])).is_err());
        assert!(bulk_rename_pairs(&paths, &names(&["", "b.txt"])).is_err());
    }

    #[test]
    fn mode_specs() {
        assert_eq!(apply_mode_spec(0o644, "755"), Some(0o755));
        assert_eq!(apply_mode_spec(0o644, "+x"), Some(0o755));
        assert_eq!(apply_mode_spec(0o755, "go-w"), Some(0o755));
        assert_eq!(apply_mode_spec(0o777, "go-w"), Some(0o755));
        assert_eq!(apply_mode_spec(0o644, "u+x,g+w"), Some(0o764));
        assert_eq!(apply_mode_spec(0o777, "o=r"), Some(0o774));
        assert_eq!(apply_mode_spec(0o644, "q+x"), None);
        assert_eq!(apply_mode_spec(0o644, "99"), None);
    }

    #[test]
    fn macros_are_shell_quoted() {
        let mut state = crate::testing::builders::demo_state(80, 24);
        state.browser.set_entries(vec![crate::browser::EntryView {
            entry: crate::testing::builders::entry(
                Path::new("/home/demo"),
                "it's here.txt",
                crate::filesystem::EntryKind::File,
                1,
                0o644,
                0,
            ),
            tags: Vec::new(),
        }]);
        assert_eq!(
            expand_macros(&state, "wc -c %f && echo 100%% in %d"),
            "wc -c '/home/demo/it'\\''s here.txt' && echo 100% in '/home/demo'"
        );
    }
}
