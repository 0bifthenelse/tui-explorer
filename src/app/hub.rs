//! The bookmarks hub (`B`): folder bookmarks, web links, marks and recent
//! folders behind one fuzzy search, split into sections (Tab cycles).

use std::path::PathBuf;

use crate::app::effects::Effect;
use crate::app::fuzzy::{fuzzy_score, score_bookmark};
use crate::app::state::{AppState, BookmarkNavState, HubItem, HubSection, Mode, StatusMessage};
use crate::urls::Link;

const RECENT_LIMIT: usize = 20;

pub fn open(state: &mut AppState) {
    state.mode = Mode::Bookmarks(Box::new(BookmarkNavState {
        query: String::new(),
        section: HubSection::All,
        matches: Vec::new(),
        selected: 0,
        picker: None,
    }));
    refresh(state);
}

/// Opens the hub restricted to a list of links (URLs found in a file).
pub fn open_link_picker(state: &mut AppState, links: Vec<Link>) {
    state.mode = Mode::Bookmarks(Box::new(BookmarkNavState {
        query: String::new(),
        section: HubSection::Links,
        matches: Vec::new(),
        selected: 0,
        picker: Some(links),
    }));
    refresh(state);
}

/// Every candidate item for `section`, before filtering.
fn candidates(state: &AppState, nav: &BookmarkNavState) -> Vec<HubItem> {
    if let Some(links) = &nav.picker {
        return links.iter().cloned().map(HubItem::Link).collect();
    }
    let folders = || state.bookmarks.iter().cloned().map(HubItem::Folder);
    let links = || state.links.iter().cloned().map(HubItem::Link);
    let marks = || {
        state
            .settings
            .marks
            .iter()
            .filter_map(|(k, p)| k.chars().next().map(|c| HubItem::Mark(c, p.clone())))
    };
    let recent = || {
        state
            .settings
            .ranked_visits(state.wall_clock)
            .into_iter()
            .map(|v| v.path.clone())
            .filter(|p| !state.bookmarks.contains(p))
            .take(RECENT_LIMIT)
            .map(HubItem::Recent)
            .collect::<Vec<_>>()
    };
    match nav.section {
        HubSection::All => folders()
            .chain(links())
            .chain(marks())
            .chain(recent())
            .collect(),
        HubSection::Folders => folders().collect(),
        HubSection::Links => links().collect(),
        HubSection::Marks => marks().collect(),
        HubSection::Recent => recent(),
    }
}

fn score(item: &HubItem, query: &str) -> Option<i32> {
    match item {
        HubItem::Folder(p) | HubItem::Recent(p) => score_bookmark(query, p),
        HubItem::Mark(c, p) => {
            if query.chars().count() == 1 && query.starts_with(*c) {
                Some(10_000)
            } else {
                score_bookmark(query, p)
            }
        }
        HubItem::Link(link) => {
            let title = fuzzy_score(query, &link.title).map(|s| s + 20);
            let url = fuzzy_score(query, &link.url);
            title.max(url)
        }
    }
}

pub fn refresh(state: &mut AppState) {
    let Mode::Bookmarks(nav) = &state.mode else {
        return;
    };
    let nav = nav.as_ref().clone();
    let items = candidates(state, &nav);
    let mut scored: Vec<(i32, usize, HubItem)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| score(&item, &nav.query).map(|s| (s, i, item)))
        .collect();
    if !nav.query.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    let matches: Vec<HubItem> = scored.into_iter().map(|(_, _, item)| item).collect();
    if let Mode::Bookmarks(nav) = &mut state.mode {
        nav.selected = nav.selected.min(matches.len().saturating_sub(1));
        nav.matches = matches;
    }
}

pub fn section_step(state: &mut AppState, delta: isize) {
    if let Mode::Bookmarks(nav) = &mut state.mode {
        if nav.picker.is_some() {
            return;
        }
        let all = HubSection::ALL;
        let idx = all.iter().position(|s| *s == nav.section).unwrap_or(0) as isize;
        nav.section = all[(idx + delta).rem_euclid(all.len() as isize) as usize];
        nav.selected = 0;
    }
    refresh(state);
}

pub fn submit(state: &mut AppState) -> Vec<Effect> {
    let item = match &state.mode {
        Mode::Bookmarks(nav) => nav.matches.get(nav.selected).cloned(),
        _ => None,
    };
    let Some(item) = item else {
        return Vec::new();
    };
    state.mode = Mode::Browser;
    match item {
        HubItem::Link(link) => crate::app::links::open_url(state, &link.url),
        other => {
            let path: PathBuf = other.path().cloned().unwrap_or_default();
            crate::app::reduce::navigate_with(state, path, true)
        }
    }
}

/// Removes the selected item from its store.
pub fn delete(state: &mut AppState) -> Vec<Effect> {
    let (item, picker) = match &state.mode {
        Mode::Bookmarks(nav) => (nav.matches.get(nav.selected).cloned(), nav.picker.is_some()),
        _ => (None, false),
    };
    let Some(item) = item else {
        return Vec::new();
    };
    if picker {
        return Vec::new();
    }
    let fx = match item {
        HubItem::Folder(path) => vec![Effect::ToggleBookmark(path)],
        HubItem::Link(link) => {
            state.links.retain(|l| l.url != link.url);
            state.message = Some(StatusMessage::info(format!("removed link {}", link.title)));
            vec![Effect::SaveLinks(state.links.clone())]
        }
        HubItem::Mark(c, _) => {
            state.settings.marks.remove(&c.to_string());
            state.settings_dirty = true;
            state.message = Some(StatusMessage::info(format!("mark {c} deleted")));
            Vec::new()
        }
        HubItem::Recent(path) => {
            state.settings.visits.retain(|v| v.path != path);
            state.settings_dirty = true;
            Vec::new()
        }
    };
    refresh(state);
    fx
}
