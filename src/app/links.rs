//! Web links: opening URLs (stream into the player or hand to a browser),
//! URLs found inside files and shortcut files, and URL bookmarks.

use std::path::Path;

use crate::app::effects::Effect;
use crate::app::state::{AppState, StatusMessage};
use crate::urls::{self, Link};

fn info(state: &mut AppState, text: impl Into<String>) {
    state.message = Some(StatusMessage::info(text));
}

/// Program + args used for web URLs: the remembered `url` association,
/// then `$BROWSER`, then `xdg-open`.
pub fn url_opener(state: &AppState) -> (Option<String>, Vec<String>) {
    let configured = state
        .settings
        .associations
        .get("url")
        .map(|a| a.command.clone())
        .or_else(|| std::env::var("BROWSER").ok())
        .filter(|c| !c.trim().is_empty());
    match configured.and_then(|c| crate::input::command::split_words(&c).ok()) {
        Some(words) if !words.is_empty() => {
            let mut words = words.into_iter();
            let program = words.next();
            (program, words.collect())
        }
        _ => (None, Vec::new()),
    }
}

/// Opens a web URL: streams go to the built-in player when mpv can play
/// them, everything else goes to the browser.
pub fn open_url(state: &mut AppState, url: &str) -> Vec<Effect> {
    let url = url.trim().to_string();
    if !urls::is_web_url(&url) {
        state.set_error(format!("not a web URL: {url}"));
        return Vec::new();
    }
    if urls::is_streamable(&url) && state.mpv_available {
        return crate::app::reduce::start_url_media(state, url);
    }
    let (program, args) = url_opener(state);
    info(state, format!("opening {}", urls::default_title(&url)));
    vec![Effect::OpenUrl { url, program, args }]
}

/// The URL a shortcut file points to, read from disk (small files only).
pub fn shortcut_url(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if !urls::is_shortcut_name(&name) {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 64 * 1024 {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    urls::shortcut_target(&name, &content)
}

/// `gx`: URLs inside the focused file (shortcut target first, then any
/// links in its text). One URL opens directly; several open the picker.
pub fn open_url_from_file(state: &mut AppState) -> Vec<Effect> {
    let Some(view) = state.browser.focused() else {
        return Vec::new();
    };
    let path = view.entry.path.clone();
    let mut found: Vec<String> = Vec::new();
    if let Some(target) = shortcut_url(&path) {
        found.push(target);
    }
    if found.is_empty() {
        // Prefer the already-loaded preview text; fall back to a bounded read.
        let text = match &state.preview.content {
            Some(crate::app::state::PreviewContent::Text { lines, .. })
                if state.preview.key.as_ref().map(|k| &k.0) == Some(&path) =>
            {
                lines.join("\n")
            }
            _ => std::fs::read(&path)
                .ok()
                .map(|b| String::from_utf8_lossy(&b[..b.len().min(256 * 1024)]).into_owned())
                .unwrap_or_default(),
        };
        found = urls::urls_in(&text);
    }
    match found.len() {
        0 => {
            info(state, "no URLs in this file");
            Vec::new()
        }
        1 => {
            let url = found.remove(0);
            open_url(state, &url)
        }
        _ => {
            let links: Vec<Link> = found
                .into_iter()
                .map(|url| Link {
                    title: urls::default_title(&url),
                    url,
                })
                .collect();
            crate::app::reduce::open_link_picker(state, links);
            Vec::new()
        }
    }
}

/// `:bookmark-url`: saves a web link.
pub fn bookmark_url(state: &mut AppState, url: String, title: Option<String>) -> Vec<Effect> {
    if !urls::is_web_url(&url) {
        state.set_error(format!("not a web URL: {url}"));
        return Vec::new();
    }
    if state.links.iter().any(|l| l.url == url) {
        info(state, "already bookmarked");
        return Vec::new();
    }
    let title = title.unwrap_or_else(|| urls::default_title(&url));
    info(state, format!("bookmarked link {title}"));
    state.links.push(Link { title, url });
    vec![Effect::SaveLinks(state.links.clone())]
}
