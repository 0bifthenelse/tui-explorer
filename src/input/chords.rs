//! Browser key bindings as one table: single keys, Ranger-style chords
//! (`gg`, `yy`, `dd`, `pp`, `gh`, `zh`, `m<key>`, ...), and the metadata
//! (group + description) that drives the which-key popup and the help
//! screen. Count prefixes (`5j`, `3G`) are resolved by [`resolve`].

use std::sync::OnceLock;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::action::{Action, RenameCursor, YankKind};
use crate::browser::SortKey;
use crate::browser::SortMode;
use crate::settings::ViewMode;

/// Matches any single printable key (marks: `m<key>`, `` `<key> ``).
pub const ANY: &str = "<any>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Motion,
    Go,
    Files,
    Select,
    Search,
    View,
    Sort,
    Marks,
    Tabs,
    Misc,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Motion => "Move",
            Group::Go => "Go to",
            Group::Files => "Files",
            Group::Select => "Select",
            Group::Search => "Search & filter",
            Group::View => "View",
            Group::Sort => "Sort",
            Group::Marks => "Marks & bookmarks",
            Group::Tabs => "Tabs & history",
            Group::Misc => "Misc",
        }
    }

    pub const ALL: [Group; 10] = [
        Group::Motion,
        Group::Go,
        Group::Files,
        Group::Select,
        Group::Search,
        Group::View,
        Group::Sort,
        Group::Marks,
        Group::Tabs,
        Group::Misc,
    ];
}

pub enum Bound {
    Act(Action),
    WithChar(fn(char) -> Action),
}

pub struct Binding {
    pub keys: Vec<&'static str>,
    pub bound: Bound,
    pub group: Group,
    pub desc: &'static str,
    /// Repeatable with a count prefix (motions, selection toggles).
    pub countable: bool,
}

impl Binding {
    /// Human-readable key sequence (`gg`, `m<key>`, `<C-b>`).
    pub fn label(&self) -> String {
        self.keys
            .iter()
            .map(|k| match *k {
                ANY => "<key>",
                other => other,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn action(&self, last: &str) -> Option<Action> {
        match &self.bound {
            Bound::Act(action) => Some(action.clone()),
            Bound::WithChar(build) => last.chars().next().map(build),
        }
    }
}

fn b(keys: &[&'static str], action: Action, group: Group, desc: &'static str) -> Binding {
    Binding {
        keys: keys.to_vec(),
        bound: Bound::Act(action),
        group,
        desc,
        countable: false,
    }
}

fn counted(keys: &[&'static str], action: Action, group: Group, desc: &'static str) -> Binding {
    Binding {
        countable: true,
        ..b(keys, action, group, desc)
    }
}

fn with_char(
    keys: &[&'static str],
    build: fn(char) -> Action,
    group: Group,
    desc: &'static str,
) -> Binding {
    Binding {
        keys: keys.to_vec(),
        bound: Bound::WithChar(build),
        group,
        desc,
        countable: false,
    }
}

fn go(path: &str) -> Action {
    Action::GoTo(path.to_string())
}

fn sort(key: SortKey, desc: bool) -> Action {
    Action::SetSort(SortMode::new(key, desc))
}

/// The browser binding table (built once).
pub fn bindings() -> &'static [Binding] {
    static TABLE: OnceLock<Vec<Binding>> = OnceLock::new();
    TABLE.get_or_init(build)
}

fn build() -> Vec<Binding> {
    use Group::*;
    vec![
        // Motion
        counted(&["j"], Action::MoveDown, Motion, "down"),
        counted(&["<Down>"], Action::MoveDown, Motion, "down"),
        counted(&["k"], Action::MoveUp, Motion, "up"),
        counted(&["<Up>"], Action::MoveUp, Motion, "up"),
        b(
            &["h"],
            Action::OpenParent,
            Motion,
            "parent folder (grid: left)",
        ),
        b(
            &["<Left>"],
            Action::OpenParent,
            Motion,
            "parent folder (grid: left)",
        ),
        b(
            &["l"],
            Action::OpenFocused,
            Motion,
            "open / enter (grid: right)",
        ),
        b(
            &["<Right>"],
            Action::OpenFocused,
            Motion,
            "open / enter (grid: right)",
        ),
        b(&["<BS>"], Action::OpenParent, Motion, "parent folder"),
        b(
            &["<CR>"],
            Action::OpenFocused,
            Motion,
            "open / enter / play",
        ),
        b(&["e"], Action::OpenFocused, Motion, "open / enter / play"),
        b(&["g", "g"], Action::GotoFirst, Motion, "first entry"),
        b(&["G"], Action::GotoLast, Motion, "last entry ([n]G: n-th)"),
        b(&["<Home>"], Action::GotoFirst, Motion, "first entry"),
        b(&["<End>"], Action::GotoLast, Motion, "last entry"),
        counted(&["J"], Action::HalfPageDown, Motion, "half page down"),
        counted(&["K"], Action::HalfPageUp, Motion, "half page up"),
        counted(&["<C-d>"], Action::HalfPageDown, Motion, "half page down"),
        counted(&["<C-u>"], Action::HalfPageUp, Motion, "half page up"),
        counted(&["<PgDn>"], Action::PageDown, Motion, "page down"),
        counted(&["<PgUp>"], Action::PageUp, Motion, "page up"),
        // Go to
        b(&["g", "h"], go("~"), Go, "home"),
        b(&["g", "r"], go("/"), Go, "root /"),
        b(&["g", "/"], go("/"), Go, "root /"),
        b(&["g", "e"], go("/etc"), Go, "/etc"),
        b(&["g", "u"], go("/usr"), Go, "/usr"),
        b(&["g", "d"], go("/dev"), Go, "/dev"),
        b(&["g", "o"], go("/opt"), Go, "/opt"),
        b(&["g", "v"], go("/var"), Go, "/var"),
        b(&["g", "m"], go("/media"), Go, "/media"),
        b(&["g", "M"], go("/mnt"), Go, "/mnt"),
        b(&["g", "s"], go("/srv"), Go, "/srv"),
        b(&["g", "p"], go("/tmp"), Go, "/tmp"),
        b(&["g", "D"], go("~/Downloads"), Go, "~/Downloads"),
        b(&["g", "l"], Action::FollowLink, Go, "follow symlink target"),
        b(&["g", "x"], Action::OpenUrlFromFile, Go, "open URL in file"),
        b(&["<C-l>"], Action::OpenAddressBar, Go, "address bar"),
        // Tabs & history
        b(&["g", "n"], Action::TabNew, Tabs, "new tab"),
        b(&["g", "t"], Action::TabNext, Tabs, "next tab"),
        b(&["g", "T"], Action::TabPrev, Tabs, "previous tab"),
        b(&["g", "c"], Action::TabClose, Tabs, "close tab"),
        b(&["u", "q"], Action::TabRestore, Tabs, "restore closed tab"),
        b(&["<C-t>"], Action::TabNew, Tabs, "new tab"),
        b(&["<C-w>"], Action::TabClose, Tabs, "close tab"),
        b(&["<A-1>"], Action::TabSelect(0), Tabs, "tab 1"),
        b(&["<A-2>"], Action::TabSelect(1), Tabs, "tab 2"),
        b(&["<A-3>"], Action::TabSelect(2), Tabs, "tab 3"),
        b(&["<A-4>"], Action::TabSelect(3), Tabs, "tab 4"),
        b(&["<A-5>"], Action::TabSelect(4), Tabs, "tab 5"),
        b(&["<A-6>"], Action::TabSelect(5), Tabs, "tab 6"),
        b(&["<A-7>"], Action::TabSelect(6), Tabs, "tab 7"),
        b(&["<A-8>"], Action::TabSelect(7), Tabs, "tab 8"),
        b(&["<A-9>"], Action::TabSelect(8), Tabs, "tab 9"),
        b(&["H"], Action::HistoryBack, Tabs, "history back"),
        b(&["L"], Action::HistoryForward, Tabs, "history forward"),
        b(&["'", "'"], Action::PreviousDir, Tabs, "previous folder"),
        b(&["`", "`"], Action::PreviousDir, Tabs, "previous folder"),
        // Files
        b(&["y", "y"], Action::CopySelection, Files, "copy"),
        b(&["d", "d"], Action::CutSelection, Files, "cut"),
        b(
            &["p", "p"],
            Action::PasteHere { overwrite: false },
            Files,
            "paste",
        ),
        b(
            &["p", "o"],
            Action::PasteHere { overwrite: true },
            Files,
            "paste, overwrite",
        ),
        b(
            &["p", "l"],
            Action::PasteSymlinks,
            Files,
            "paste as symlinks",
        ),
        b(
            &["u", "d"],
            Action::ClearClipboard,
            Files,
            "clear clipboard",
        ),
        b(&["<C-c>"], Action::CopySelection, Files, "copy"),
        b(&["<C-x>"], Action::CutSelection, Files, "cut"),
        b(
            &["<C-v>"],
            Action::PasteHere { overwrite: false },
            Files,
            "paste",
        ),
        b(
            &["y", "p"],
            Action::Yank(YankKind::Path),
            Files,
            "yank path",
        ),
        b(
            &["y", "d"],
            Action::Yank(YankKind::Dir),
            Files,
            "yank folder path",
        ),
        b(
            &["y", "n"],
            Action::Yank(YankKind::Name),
            Files,
            "yank name",
        ),
        b(
            &["y", "N"],
            Action::Yank(YankKind::Stem),
            Files,
            "yank name w/o ext",
        ),
        b(&["d", "T"], Action::TrashSelection, Files, "move to trash"),
        b(&["<Del>"], Action::TrashSelection, Files, "move to trash"),
        b(
            &["d", "D"],
            Action::DeleteSelection,
            Files,
            "delete forever",
        ),
        b(
            &["<S-Del>"],
            Action::DeleteSelection,
            Files,
            "delete forever",
        ),
        b(&["d", "u"], Action::DiskUsage, Files, "folder sizes"),
        b(&["u", "u"], Action::Undo, Files, "undo"),
        b(&["<C-z>"], Action::Undo, Files, "undo"),
        b(
            &["c", "w"],
            Action::RenameStart(RenameCursor::Replace),
            Files,
            "rename (new name)",
        ),
        b(
            &["<F2>"],
            Action::RenameStart(RenameCursor::End),
            Files,
            "rename",
        ),
        b(
            &["A"],
            Action::RenameStart(RenameCursor::End),
            Files,
            "rename, cursor at end",
        ),
        b(
            &["I"],
            Action::RenameStart(RenameCursor::Start),
            Files,
            "rename, cursor at start",
        ),
        b(
            &["a"],
            Action::RenameStart(RenameCursor::BeforeExt),
            Files,
            "rename before extension",
        ),
        b(
            &["+"],
            Action::EnterCreate,
            Files,
            "create file (trailing / = folder)",
        ),
        b(&["r"], Action::OpenWithPrompt, Files, "open with…"),
        b(&["E"], Action::EditFocused, Files, "edit in $EDITOR"),
        b(&["X"], Action::EncryptToggle, Files, "encrypt / decrypt"),
        b(&["t"], Action::QuickTag, Files, "toggle last tag"),
        b(&["T"], Action::OpenTagPicker, Files, "tags…"),
        // Select
        counted(
            &["<Space>"],
            Action::ToggleSelect,
            Select,
            "toggle + move down",
        ),
        b(&["v"], Action::ToggleVisual, Select, "visual range select"),
        b(&["V"], Action::InvertSelection, Select, "invert selection"),
        b(
            &["u", "v"],
            Action::ClearSelection,
            Select,
            "clear selection",
        ),
        b(&["<C-a>"], Action::SelectAll, Select, "select all"),
        // Search
        b(
            &["/"],
            Action::EnterSearch,
            Search,
            "search (n / N next / prev)",
        ),
        b(&["n"], Action::SearchNext, Search, "next match"),
        b(&["N"], Action::SearchPrev, Search, "previous match"),
        b(&["f"], Action::EnterFind, Search, "find: type to jump"),
        b(
            &["<C-f>"],
            Action::EnterFilter,
            Search,
            "filter this folder",
        ),
        b(
            &["z", "f"],
            Action::EnterFilter,
            Search,
            "filter this folder",
        ),
        // View
        b(
            &["z", "l"],
            Action::SetView(ViewMode::List),
            View,
            "list layout",
        ),
        b(
            &["z", "g"],
            Action::SetView(ViewMode::Grid),
            View,
            "grid layout",
        ),
        b(
            &["z", "c"],
            Action::SetView(ViewMode::Columns),
            View,
            "columns layout",
        ),
        b(&["z", "v"], Action::CycleView, View, "cycle layouts"),
        b(&["z", "+"], Action::GridZoom(true), View, "large tiles"),
        b(&["z", "-"], Action::GridZoom(false), View, "compact tiles"),
        b(&["z", "h"], Action::ToggleHidden, View, "hidden files"),
        b(&["."], Action::ToggleHidden, View, "hidden files"),
        b(&["z", "p"], Action::TogglePreview, View, "preview panel"),
        b(&["z", "s"], Action::ToggleSidebar, View, "sidebar"),
        b(&["b"], Action::ToggleSidebar, View, "sidebar"),
        b(&["z", "a"], Action::ToggleAnimations, View, "animations"),
        b(&["i"], Action::QuickLook, View, "quick look"),
        b(&["<F5>"], Action::Refresh, View, "reload folder"),
        // Sort
        b(&["o", "n"], sort(SortKey::Name, false), Sort, "name"),
        b(
            &["o", "N"],
            sort(SortKey::Name, true),
            Sort,
            "name, reversed",
        ),
        b(
            &["o", "s"],
            sort(SortKey::Size, true),
            Sort,
            "size, largest first",
        ),
        b(
            &["o", "S"],
            sort(SortKey::Size, false),
            Sort,
            "size, smallest first",
        ),
        b(
            &["o", "m"],
            sort(SortKey::Modified, true),
            Sort,
            "modified, newest first",
        ),
        b(
            &["o", "M"],
            sort(SortKey::Modified, false),
            Sort,
            "modified, oldest first",
        ),
        b(&["o", "t"], sort(SortKey::Type, false), Sort, "type"),
        b(
            &["o", "e"],
            sort(SortKey::Extension, false),
            Sort,
            "extension",
        ),
        b(&["o", "r"], Action::ReverseSort, Sort, "reverse order"),
        // Marks & bookmarks
        with_char(&["m", ANY], Action::SetMark, Marks, "set mark"),
        with_char(&["`", ANY], Action::JumpMark, Marks, "jump to mark"),
        with_char(&["'", ANY], Action::JumpMark, Marks, "jump to mark"),
        with_char(&["u", "m", ANY], Action::DeleteMark, Marks, "delete mark"),
        b(&["B"], Action::OpenBookmarks, Marks, "bookmarks hub"),
        b(
            &["<C-b>"],
            Action::ToggleBookmark,
            Marks,
            "bookmark this folder",
        ),
        // Misc
        b(&[":"], Action::EnterCommand, Misc, "command line"),
        b(&["!"], Action::EnterShell, Misc, "shell command"),
        b(&["s"], Action::EnterShell, Misc, "shell command"),
        b(&["S"], Action::Subshell, Misc, "shell here"),
        b(
            &["M"],
            Action::MediaExpand,
            Misc,
            "show the player (mini player)",
        ),
        b(&["?"], Action::ToggleHelp, Misc, "help"),
        b(&["q"], Action::Quit, Misc, "quit"),
        b(&["Z", "Z"], Action::Quit, Misc, "quit"),
        b(&["Z", "Q"], Action::Quit, Misc, "quit"),
        b(&["<Esc>"], Action::Cancel, Misc, "cancel / clear"),
    ]
}

/// Converts a key event into a binding token.
pub fn token(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let named = |name: &str| -> String {
        if shift && !matches!(name, "Tab") {
            format!("<S-{name}>")
        } else {
            format!("<{name}>")
        }
    };
    Some(match key.code {
        KeyCode::Char(c) if ctrl => format!("<C-{}>", c.to_ascii_lowercase()),
        KeyCode::Char(c) if alt => format!("<A-{c}>"),
        KeyCode::Char(' ') => "<Space>".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "<CR>".to_string(),
        KeyCode::Esc => "<Esc>".to_string(),
        KeyCode::Backspace => "<BS>".to_string(),
        KeyCode::Delete => named("Del"),
        KeyCode::Up => "<Up>".to_string(),
        KeyCode::Down => "<Down>".to_string(),
        KeyCode::Left => "<Left>".to_string(),
        KeyCode::Right => "<Right>".to_string(),
        KeyCode::PageUp => "<PgUp>".to_string(),
        KeyCode::PageDown => "<PgDn>".to_string(),
        KeyCode::Home => "<Home>".to_string(),
        KeyCode::End => "<End>".to_string(),
        KeyCode::Tab => "<Tab>".to_string(),
        KeyCode::BackTab => "<S-Tab>".to_string(),
        KeyCode::F(n) => format!("<F{n}>"),
        _ => return None,
    })
}

fn token_matches(pattern: &str, token: &str) -> bool {
    pattern == token || (pattern == ANY && token.chars().count() == 1)
}

/// Outcome of feeding the pending sequence plus one more token.
#[derive(Debug)]
pub enum Resolution {
    /// Fully matched: run the action (count already applied).
    Run(Action),
    /// Valid prefix of one or more bindings: keep waiting.
    Pending,
    /// No binding starts this way.
    Unknown,
}

/// True when `tokens` is a strict prefix of at least one binding.
pub fn is_prefix(tokens: &[String]) -> bool {
    bindings().iter().any(|binding| {
        binding.keys.len() > tokens.len()
            && tokens
                .iter()
                .zip(&binding.keys)
                .all(|(t, k)| token_matches(k, t))
    })
}

/// Resolves a key sequence (pending tokens + the new one) with an optional
/// count prefix.
pub fn resolve(tokens: &[String], count: Option<usize>) -> Resolution {
    let full = bindings().iter().find(|binding| {
        binding.keys.len() == tokens.len()
            && tokens
                .iter()
                .zip(&binding.keys)
                .all(|(t, k)| token_matches(k, t))
            // A literal match wins over the wildcard of a longer family.
            && !(binding.keys.contains(&ANY) && literal_exists(tokens))
    });
    if let Some(binding) = full {
        let last = tokens.last().map(String::as_str).unwrap_or("");
        let Some(action) = binding.action(last) else {
            return Resolution::Unknown;
        };
        let action = match (count, &action) {
            (Some(n), Action::GotoLast) => Action::GotoIndex(n.saturating_sub(1)),
            (Some(n), _) if binding.countable && n > 1 => {
                Action::Repeat(n.min(10_000), Box::new(action))
            }
            _ => action,
        };
        return Resolution::Run(action);
    }
    if is_prefix(tokens) {
        Resolution::Pending
    } else {
        Resolution::Unknown
    }
}

fn literal_exists(tokens: &[String]) -> bool {
    bindings().iter().any(|binding| {
        !binding.keys.contains(&ANY)
            && binding.keys.len() == tokens.len()
            && tokens.iter().zip(&binding.keys).all(|(t, k)| t == k)
    })
}

/// Continuations of `tokens` for the which-key popup: (next key, desc).
pub fn continuations(tokens: &[String]) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for binding in bindings() {
        if binding.keys.len() > tokens.len()
            && tokens
                .iter()
                .zip(&binding.keys)
                .all(|(t, k)| token_matches(k, t))
        {
            let rest: String = binding.keys[tokens.len()..]
                .iter()
                .map(|k| if *k == ANY { "<key>" } else { k })
                .collect();
            if !out.iter().any(|(k, _)| *k == rest) {
                out.push((rest, binding.desc));
            }
        }
    }
    out
}

/// Bindings grouped for the help screen (first binding per description
/// absorbs alternates as "j / <Down>").
pub fn help_rows() -> Vec<(Group, String, &'static str)> {
    let mut rows: Vec<(Group, String, &'static str)> = Vec::new();
    for binding in bindings() {
        // Alt-1 … Alt-9 collapse into one row.
        if binding.keys.len() == 1 && binding.keys[0].starts_with("<A-") {
            if binding.keys[0] == "<A-1>" {
                rows.push((binding.group, "<A-1…9>".to_string(), "switch to tab 1–9"));
            }
            continue;
        }
        let label = binding.label();
        if let Some(row) = rows
            .iter_mut()
            .find(|(g, _, d)| *g == binding.group && *d == binding.desc)
        {
            row.1 = format!("{} / {label}", row.1);
        } else {
            rows.push((binding.group, label, binding.desc));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn single_and_chord_resolution() {
        assert!(matches!(
            resolve(&toks(&["j"]), None),
            Resolution::Run(Action::MoveDown)
        ));
        assert!(matches!(resolve(&toks(&["g"]), None), Resolution::Pending));
        assert!(matches!(
            resolve(&toks(&["g", "g"]), None),
            Resolution::Run(Action::GotoFirst)
        ));
        assert!(matches!(
            resolve(&toks(&["g", "h"]), None),
            Resolution::Run(Action::GoTo(ref p)) if p == "~"
        ));
        assert!(matches!(
            resolve(&toks(&["g", "q"]), None),
            Resolution::Unknown
        ));
    }

    #[test]
    fn wildcard_marks_and_literal_precedence() {
        assert!(matches!(
            resolve(&toks(&["m", "a"]), None),
            Resolution::Run(Action::SetMark('a'))
        ));
        // `''` is the literal previous-folder binding, not mark `'`.
        assert!(matches!(
            resolve(&toks(&["'", "'"]), None),
            Resolution::Run(Action::PreviousDir)
        ));
        assert!(matches!(
            resolve(&toks(&["'", "x"]), None),
            Resolution::Run(Action::JumpMark('x'))
        ));
        assert!(matches!(
            resolve(&toks(&["u", "m", "z"]), None),
            Resolution::Run(Action::DeleteMark('z'))
        ));
    }

    #[test]
    fn counts_repeat_motions_and_pick_rows() {
        match resolve(&toks(&["j"]), Some(5)) {
            Resolution::Run(Action::Repeat(5, inner)) => {
                assert!(matches!(*inner, Action::MoveDown))
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            resolve(&toks(&["G"]), Some(3)),
            Resolution::Run(Action::GotoIndex(2))
        ));
        // Counts never repeat non-motions.
        assert!(matches!(
            resolve(&toks(&["q"]), Some(4)),
            Resolution::Run(Action::Quit)
        ));
    }

    #[test]
    fn which_key_lists_continuations() {
        let next = continuations(&toks(&["g"]));
        assert!(next.iter().any(|(k, d)| k == "g" && *d == "first entry"));
        assert!(next.iter().any(|(k, _)| k == "h"));
        let marks = continuations(&toks(&["m"]));
        assert_eq!(marks, vec![("<key>".to_string(), "set mark")]);
    }

    #[test]
    fn tokens_for_special_keys() {
        let ctrl = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(token(&ctrl).as_deref(), Some("<C-b>"));
        let sdel = KeyEvent::new(KeyCode::Delete, KeyModifiers::SHIFT);
        assert_eq!(token(&sdel).as_deref(), Some("<S-Del>"));
        let space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(token(&space).as_deref(), Some("<Space>"));
        let upper = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(token(&upper).as_deref(), Some("G"));
    }

    #[test]
    fn no_binding_shadows_another() {
        // A complete binding must never be a strict prefix of another one,
        // otherwise the longer chord would be unreachable.
        for a in bindings() {
            for other in bindings() {
                if other.keys.len() > a.keys.len()
                    && !a.keys.contains(&ANY)
                    && a.keys.iter().zip(&other.keys).all(|(x, y)| x == y)
                {
                    panic!("{} shadows {}", a.label(), other.label());
                }
            }
        }
    }

    #[test]
    fn help_rows_cover_every_group() {
        let rows = help_rows();
        for group in Group::ALL {
            assert!(rows.iter().any(|(g, _, _)| *g == group), "{group:?}");
        }
    }
}
