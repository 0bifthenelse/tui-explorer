use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::filesystem::DirEntry;

#[derive(Clone, Debug)]
pub struct EntryView {
    pub entry: DirEntry,
    pub tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortMode {
    NameDirsFirst,
    Size,
    SizeDesc,
    Modified,
    ModifiedDesc,
    NameDesc,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::NameDirsFirst => "name",
            SortMode::Size => "size",
            SortMode::SizeDesc => "size desc",
            SortMode::Modified => "modified",
            SortMode::ModifiedDesc => "modified desc",
            SortMode::NameDesc => "name desc",
        }
    }

    pub fn descending(self) -> bool {
        matches!(
            self,
            SortMode::NameDesc | SortMode::SizeDesc | SortMode::ModifiedDesc
        )
    }
}

pub fn sort_entries(entries: &mut [EntryView], mode: SortMode) {
    entries.sort_by(|a, b| {
        let dir_a = a.entry.kind.is_dir();
        let dir_b = b.entry.kind.is_dir();
        if dir_a != dir_b {
            return dir_b.cmp(&dir_a);
        }
        let primary = match mode {
            SortMode::NameDirsFirst | SortMode::NameDesc => a
                .entry
                .name
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.entry.name.to_string_lossy().to_lowercase()),
            SortMode::Size | SortMode::SizeDesc => a.entry.size.cmp(&b.entry.size),
            SortMode::Modified | SortMode::ModifiedDesc => a.entry.modified.cmp(&b.entry.modified),
        };
        let primary = if matches!(
            mode,
            SortMode::NameDesc | SortMode::SizeDesc | SortMode::ModifiedDesc
        ) {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| {
            a.entry
                .name
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.entry.name.to_string_lossy().to_lowercase())
        })
    });
}

#[derive(Clone, Debug)]
pub struct Browser {
    pub cwd: PathBuf,
    pub entries: Vec<EntryView>,
    pub selected: usize,
    pub scroll: usize,
    pub selection: BTreeSet<PathBuf>,
    pub show_hidden: bool,
    /// Case-insensitive filename filter for the current directory.
    pub filter: Option<String>,
    pub sort_mode: SortMode,
    pub visual: bool,
}

impl Browser {
    pub fn new(cwd: PathBuf) -> Self {
        Browser {
            cwd,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            selection: BTreeSet::new(),
            show_hidden: false,
            filter: None,
            sort_mode: SortMode::NameDirsFirst,
            visual: false,
        }
    }

    pub fn visible_entries(&self) -> impl Iterator<Item = (usize, &EntryView)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.show_hidden || !e.entry.hidden)
            .filter(|(_, e)| {
                self.filter.as_ref().is_none_or(|query| {
                    e.entry
                        .name
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(query)
                })
            })
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.visible_entries().map(|(i, _)| i).collect()
    }

    pub fn focused_index(&self) -> Option<usize> {
        let indices = self.visible_indices();
        if indices.is_empty() {
            return None;
        }
        let pos = self.selected.min(indices.len() - 1);
        Some(indices[pos])
    }

    pub fn focused(&self) -> Option<&EntryView> {
        self.focused_index().map(|i| &self.entries[i])
    }

    pub fn set_entries(&mut self, mut entries: Vec<EntryView>) {
        let previous_focus = self.focused().map(|e| e.entry.path.clone());
        sort_entries(&mut entries, self.sort_mode);
        self.entries = entries;
        let indices = self.visible_indices();
        if indices.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        if let Some(prev) = previous_focus {
            if let Some(pos) = indices
                .iter()
                .position(|&i| self.entries[i].entry.path == prev)
            {
                self.selected = pos;
                self.clamp_scroll(usize::MAX);
                return;
            }
        }
        self.selected = self.selected.min(indices.len() - 1);
        self.clamp_scroll(usize::MAX);
    }

    pub fn visible_len(&self) -> usize {
        self.visible_indices().len()
    }

    /// Number of entries visible with hidden-file rules but before the active
    /// filename filter, used for a useful `matches / total` status.
    pub fn listed_len(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| self.show_hidden || !entry.entry.hidden)
            .count()
    }

    fn step(&mut self, delta: isize, viewport: usize) {
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, len as isize - 1);
        self.selected = next as usize;
        self.clamp_scroll(viewport);
    }

    pub fn move_down(&mut self, viewport: usize) {
        self.step(1, viewport);
    }

    pub fn move_up(&mut self, viewport: usize) {
        self.step(-1, viewport);
    }

    pub fn page_down(&mut self, viewport: usize) {
        let jump = viewport.max(1) as isize;
        self.step(jump, viewport);
    }

    pub fn page_up(&mut self, viewport: usize) {
        let jump = viewport.max(1) as isize;
        self.step(-jump, viewport);
    }

    pub fn half_page_down(&mut self, viewport: usize) {
        let jump = (viewport.max(2) / 2) as isize;
        self.step(jump, viewport);
    }

    pub fn half_page_up(&mut self, viewport: usize) {
        let jump = (viewport.max(2) / 2) as isize;
        self.step(-jump, viewport);
    }

    pub fn goto_first(&mut self) {
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn goto_last(&mut self, viewport: usize) {
        let len = self.visible_len();
        self.selected = len.saturating_sub(1);
        self.clamp_scroll(viewport);
    }

    pub fn goto_last_grid(&mut self, cols: usize, rows: usize) {
        let len = self.visible_len();
        self.selected = len.saturating_sub(1);
        self.clamp_scroll_grid(cols, rows);
    }

    pub fn scroll_by(&mut self, delta: isize, viewport: usize) {
        self.step(delta, viewport);
    }

    /// Grid-aware movement: delta is in entries (e.g. +/-columns for
    /// vertical movement), scrolling keeps tile rows aligned to columns.
    pub fn grid_move(&mut self, delta: isize, cols: usize, rows: usize) {
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, len as isize - 1);
        self.selected = next as usize;
        self.clamp_scroll_grid(cols, rows);
    }

    /// Clamp `scroll` (entry index of the first visible tile) so the
    /// selection is visible and tile rows stay aligned to `cols`.
    pub fn clamp_scroll_grid(&mut self, cols: usize, rows: usize) {
        let len = self.visible_len();
        if len == 0 {
            self.scroll = 0;
            return;
        }
        let cols = cols.max(1);
        let rows = rows.max(1);
        let per = cols.saturating_mul(rows).max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected - (self.selected % cols);
        } else if self.selected >= self.scroll + per {
            let row = self.selected / cols;
            self.scroll = (row + 1 - rows) * cols;
        }
        if len <= per {
            self.scroll = 0;
            return;
        }
        let mut max_start = len.saturating_sub(per);
        max_start -= max_start % cols;
        self.scroll = self.scroll.min(max_start);
    }

    pub fn clamp_scroll(&mut self, viewport: usize) {
        let len = self.visible_len();
        if len == 0 {
            self.scroll = 0;
            return;
        }
        let viewport = viewport.max(1);
        if viewport == usize::MAX {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + viewport {
            self.scroll = self.selected + 1 - viewport;
        }
        let max_scroll = len.saturating_sub(viewport);
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn set_sort_mode(&mut self, mode: SortMode) {
        self.sort_mode = mode;
        let entries = std::mem::take(&mut self.entries);
        self.set_entries(entries);
    }

    pub fn set_filter(&mut self, query: Option<String>) {
        self.filter = query
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let len = self.visible_len();
        self.selected = self.selected.min(len.saturating_sub(1));
        self.scroll = 0;
        self.clamp_scroll(usize::MAX);
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
            self.scroll = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }

    pub fn toggle_select_focused(&mut self) {
        if let Some(view) = self.focused() {
            let path = view.entry.path.clone();
            if !self.selection.remove(&path) {
                self.selection.insert(path);
            }
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.visual = false;
    }

    /// Selects every entry currently in the listing (ranger's `:selectall`).
    pub fn select_all(&mut self) {
        self.selection = self.entries.iter().map(|e| e.entry.path.clone()).collect();
        self.visual = false;
    }

    /// Flips selection membership for every entry currently in the listing
    /// (ranger's per-view `:invert`). Anything already selected but outside
    /// the current listing is dropped, keeping the result well-defined.
    pub fn invert_selection(&mut self) {
        self.selection = self
            .entries
            .iter()
            .map(|e| &e.entry.path)
            .filter(|p| !self.selection.contains(*p))
            .cloned()
            .collect();
        self.visual = false;
    }

    pub fn targets(&self) -> Vec<PathBuf> {
        if !self.selection.is_empty() {
            return self.selection.iter().cloned().collect();
        }
        self.focused()
            .map(|v| vec![v.entry.path.clone()])
            .unwrap_or_default()
    }

    pub fn selected_paths_set(&self) -> &BTreeSet<PathBuf> {
        &self.selection
    }

    pub fn enter(&mut self, dir: &Path) {
        self.cwd = dir.to_path_buf();
        self.selected = 0;
        self.scroll = 0;
        self.clear_selection();
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::EntryKind;
    use std::ffi::OsString;

    fn view(name: &str, kind: EntryKind) -> EntryView {
        EntryView {
            entry: DirEntry {
                name: OsString::from(name),
                path: PathBuf::from(format!("/d/{name}")),
                kind,
                size: 0,
                mode: 0o644,
                modified: 0,
                executable: false,
                hidden: name.starts_with('.'),
                device: None,
                inode: None,
            },
            tags: Vec::new(),
        }
    }

    fn browser() -> Browser {
        let mut b = Browser::new(PathBuf::from("/d"));
        b.set_entries(vec![
            view("zeta.txt", EntryKind::File),
            view("Beta", EntryKind::Directory),
            view("alpha", EntryKind::Directory),
            view(".secret", EntryKind::File),
            view("Gamma.rs", EntryKind::File),
        ]);
        b
    }

    #[test]
    fn dirs_first_case_insensitive() {
        let b = browser();
        let names: Vec<String> = b
            .visible_entries()
            .map(|(_, e)| e.entry.display_name())
            .collect();
        assert_eq!(names, vec!["alpha", "Beta", "Gamma.rs", "zeta.txt"]);
    }

    #[test]
    fn hidden_toggle() {
        let mut b = browser();
        assert_eq!(b.visible_len(), 4);
        b.toggle_hidden();
        assert_eq!(b.visible_len(), 5);
        b.toggle_hidden();
        assert_eq!(b.visible_len(), 4);
    }

    #[test]
    fn sort_mode_changes_order_without_losing_focus() {
        let mut b = browser();
        b.set_sort_mode(SortMode::Size);
        let sizes: Vec<u64> = b.visible_entries().map(|(_, e)| e.entry.size).collect();
        assert!(sizes.windows(2).all(|pair| pair[0] <= pair[1]));
        b.set_sort_mode(SortMode::Modified);
        let modified: Vec<i64> = b.visible_entries().map(|(_, e)| e.entry.modified).collect();
        assert!(modified.windows(2).all(|pair| pair[0] <= pair[1]));
        b.set_sort_mode(SortMode::SizeDesc);
        let descending: Vec<u64> = b.visible_entries().map(|(_, e)| e.entry.size).collect();
        assert!(descending.windows(2).all(|pair| pair[0] >= pair[1]));
        b.set_filter(Some("rs".into()));
        b.set_sort_mode(SortMode::NameDesc);
        assert_eq!(b.visible_len(), 1);
    }

    #[test]
    fn listed_len_ignores_filter_but_respects_hidden_setting() {
        let mut b = browser();
        assert_eq!(b.listed_len(), 4);
        b.set_filter(Some("rs".into()));
        assert_eq!(b.listed_len(), 4);
        b.toggle_hidden();
        assert_eq!(b.listed_len(), 5);
    }

    #[test]
    fn filter_matches_names_case_insensitively_and_can_clear() {
        let mut b = browser();
        b.set_filter(Some("GAM".into()));
        assert_eq!(b.visible_len(), 1);
        assert_eq!(b.focused().unwrap().entry.display_name(), "Gamma.rs");
        b.set_filter(None);
        assert_eq!(b.visible_len(), 4);
    }

    #[test]
    fn navigation_bounds() {
        let mut b = browser();
        b.move_up(10);
        assert_eq!(b.selected, 0);
        for _ in 0..10 {
            b.move_down(10);
        }
        assert_eq!(b.selected, 3);
        b.goto_first();
        assert_eq!(b.selected, 0);
        b.goto_last(10);
        assert_eq!(b.selected, 3);
    }

    #[test]
    fn scroll_window_follows_selection() {
        let mut b = browser();
        for _ in 0..3 {
            b.move_down(2);
        }
        assert_eq!(b.selected, 3);
        assert_eq!(b.scroll, 2);
        b.move_up(2);
        b.move_up(2);
        assert_eq!(b.selected, 1);
        assert_eq!(b.scroll, 1);
        b.move_up(2);
        assert_eq!(b.selected, 0);
        assert_eq!(b.scroll, 0);
    }

    #[test]
    fn stable_focus_after_refresh() {
        let mut b = browser();
        b.move_down(10);
        let focused = b.focused().unwrap().entry.path.clone();
        b.set_entries(vec![
            view("zeta.txt", EntryKind::File),
            view("Beta", EntryKind::Directory),
            view("alpha", EntryKind::Directory),
            view(".secret", EntryKind::File),
            view("Gamma.rs", EntryKind::File),
            view("new.txt", EntryKind::File),
        ]);
        assert_eq!(b.focused().unwrap().entry.path, focused);
    }

    #[test]
    fn targets_fall_back_to_focused() {
        let mut b = browser();
        assert_eq!(b.targets().len(), 1);
        b.toggle_select_focused();
        b.move_down(10);
        b.toggle_select_focused();
        assert_eq!(b.targets().len(), 2);
        b.clear_selection();
        assert_eq!(b.targets().len(), 1);
    }

    #[test]
    fn select_all_selects_every_entry() {
        let mut b = browser();
        b.visual = true;
        b.select_all();
        assert_eq!(b.selection.len(), 5);
        assert!(!b.visual);
    }

    #[test]
    fn invert_selection_flips_membership() {
        let mut b = browser();
        b.invert_selection();
        assert_eq!(b.selection.len(), 5);
        b.invert_selection();
        assert!(b.selection.is_empty());
        b.toggle_select_focused();
        b.invert_selection();
        assert_eq!(b.selection.len(), 4);
        let focused = b.entries[b.focused_index().unwrap()].entry.path.clone();
        assert!(!b.selection.contains(&focused));
    }

    #[test]
    fn empty_directory_is_safe() {
        let mut b = Browser::new(PathBuf::from("/empty"));
        b.set_entries(Vec::new());
        b.move_down(10);
        b.page_down(10);
        b.goto_last(10);
        assert!(b.focused().is_none());
        assert!(b.targets().is_empty());
    }
}
