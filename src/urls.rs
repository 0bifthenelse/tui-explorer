//! URL helpers: finding web links in text, reading desktop shortcut files
//! (`.url`, `.desktop`, `.webloc`), recognizing streamable media URLs, and
//! the persistent URL bookmark store (`links.tsv`).

use std::path::{Path, PathBuf};

/// Byte ranges of `http(s)://` URLs inside `text`. Trailing punctuation
/// that usually ends a sentence (`.`, `,`, `)`) is not part of the URL.
pub fn find_urls(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &text[i..];
        let start = if rest.starts_with("https://") || rest.starts_with("http://") {
            Some(i)
        } else {
            None
        };
        let Some(start) = start else {
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            continue;
        };
        let mut end = start;
        for (offset, c) in text[start..].char_indices() {
            if c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '|' | '{' | '}') {
                break;
            }
            end = start + offset + c.len_utf8();
        }
        while end > start {
            let last = text[..end].chars().next_back().unwrap_or(' ');
            if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']') {
                // Keep a closing paren that balances one inside the URL.
                if last == ')'
                    && text[start..end].matches('(').count()
                        >= text[start..end].matches(')').count()
                {
                    break;
                }
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        let scheme_len = if text[start..].starts_with("https://") {
            8
        } else {
            7
        };
        if end > start + scheme_len {
            out.push((start, end));
        }
        i = end.max(start + 1);
    }
    out
}

/// Every URL in `text`, deduplicated in order of appearance.
pub fn urls_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (s, e) in find_urls(text) {
        let url = text[s..e].to_string();
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// True for strings that look like a web URL.
pub fn is_web_url(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("https://") || t.starts_with("http://")) && t.len() > 10 && !t.contains(' ')
}

/// Extracts the target of a shortcut file by extension and content:
/// Windows `.url` (`URL=`), freedesktop `.desktop` with `Type=Link`, and
/// macOS `.webloc` property lists.
pub fn shortcut_target(name: &str, content: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".url") {
        return content
            .lines()
            .filter_map(|l| l.trim().strip_prefix("URL="))
            .map(str::trim)
            .find(|u| !u.is_empty())
            .map(String::from);
    }
    if lower.ends_with(".desktop") {
        let is_link = content
            .lines()
            .any(|l| l.trim().eq_ignore_ascii_case("Type=Link"));
        if !is_link {
            return None;
        }
        return content
            .lines()
            .filter_map(|l| l.trim().strip_prefix("URL="))
            .map(str::trim)
            .find(|u| !u.is_empty())
            .map(String::from);
    }
    if lower.ends_with(".webloc") {
        let key = content.find("<key>URL</key>")?;
        let rest = &content[key..];
        let open = rest.find("<string>")? + "<string>".len();
        let close = rest[open..].find("</string>")?;
        let url = rest[open..open + close].trim();
        return (!url.is_empty()).then(|| url.replace("&amp;", "&"));
    }
    None
}

/// File names treated as link shortcuts.
pub fn is_shortcut_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".url") || lower.ends_with(".webloc") || lower.ends_with(".desktop")
}

/// URLs mpv (via yt-dlp) can stream into the built-in player.
pub fn is_streamable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    const HOSTS: &[&str] = &[
        "youtube.com/watch",
        "youtu.be/",
        "youtube.com/shorts/",
        "vimeo.com/",
        "twitch.tv/",
        "soundcloud.com/",
        "dailymotion.com/video",
        "bandcamp.com/track",
    ];
    if HOSTS.iter().any(|h| lower.contains(h)) {
        return true;
    }
    let path = lower.split(['?', '#']).next().unwrap_or("");
    const EXT: &[&str] = &[
        ".m3u8", ".mp4", ".mkv", ".webm", ".mp3", ".ogg", ".flac", ".m4a", ".opus", ".mov",
    ];
    EXT.iter().any(|e| path.ends_with(e))
}

/// One URL bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub title: String,
    pub url: String,
}

/// Title for a URL when none was given: host plus first path segment.
pub fn default_title(url: &str) -> String {
    let without = url.split("://").nth(1).unwrap_or(url);
    let host = without.split('/').next().unwrap_or(without);
    let host = host.strip_prefix("www.").unwrap_or(host);
    host.to_string()
}

/// `links.tsv`: one `title<TAB>url` per line.
#[derive(Clone, Debug)]
pub struct LinkStore {
    path: PathBuf,
}

impl LinkStore {
    pub fn new(path: PathBuf) -> Self {
        LinkStore { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Vec<Link> {
        std::fs::read_to_string(&self.path)
            .map(|text| parse_links(&text))
            .unwrap_or_default()
    }

    pub fn save(&self, links: &[Link]) -> std::io::Result<()> {
        crate::settings::write_atomic(&self.path, serialize_links(links).as_bytes())
    }
}

/// Decodes `%XX` escapes (file:// URIs); invalid escapes stay literal.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_links(text: &str) -> Vec<Link> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (title, url) = match line.split_once('\t') {
                Some((t, u)) => (t.trim().to_string(), u.trim().to_string()),
                None => (default_title(line.trim()), line.trim().to_string()),
            };
            is_web_url(&url).then_some(Link { title, url })
        })
        .collect()
}

pub fn serialize_links(links: &[Link]) -> String {
    let mut out = String::new();
    for link in links {
        out.push_str(&link.title.replace('\t', " "));
        out.push('\t');
        out.push_str(&link.url);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_urls_and_trims_punctuation() {
        let text = "see https://mpv.io/manual/. and (http://a.b/c_(d)) or https://x.y/z, done";
        assert_eq!(
            urls_in(text),
            vec![
                "https://mpv.io/manual/",
                "http://a.b/c_(d)",
                "https://x.y/z"
            ]
        );
        assert!(find_urls("http:// nope").is_empty());
        assert!(find_urls("no links here").is_empty());
        // Byte ranges stay valid around multibyte text.
        let t = "日本 https://例え.jp/パス 終";
        let (s, e) = find_urls(t)[0];
        assert_eq!(&t[s..e], "https://例え.jp/パス");
    }

    #[test]
    fn shortcut_files() {
        assert_eq!(
            shortcut_target(
                "a.url",
                "[InternetShortcut]\r\nURL=https://example.com/x\r\n"
            ),
            Some("https://example.com/x".to_string())
        );
        assert_eq!(
            shortcut_target(
                "d.desktop",
                "[Desktop Entry]\nType=Link\nURL=https://docs.rs\n"
            ),
            Some("https://docs.rs".to_string())
        );
        assert_eq!(
            shortcut_target("app.desktop", "[Desktop Entry]\nType=Application\nExec=x\n"),
            None
        );
        let plist =
            "<plist><dict><key>URL</key><string>https://e.com/?a=1&amp;b=2</string></dict></plist>";
        assert_eq!(
            shortcut_target("s.webloc", plist),
            Some("https://e.com/?a=1&b=2".to_string())
        );
    }

    #[test]
    fn streamable_detection() {
        assert!(is_streamable("https://www.youtube.com/watch?v=abc"));
        assert!(is_streamable("https://cdn.x/live/index.m3u8?token=1"));
        assert!(!is_streamable("https://docs.rs/ratatui"));
    }

    #[test]
    fn links_roundtrip() {
        let links = vec![
            Link {
                title: "Docs".into(),
                url: "https://docs.rs".into(),
            },
            Link {
                title: "mpv".into(),
                url: "https://mpv.io".into(),
            },
        ];
        assert_eq!(parse_links(&serialize_links(&links)), links);
        assert_eq!(
            parse_links("https://www.rust-lang.org/learn\n# comment\nbogus\n"),
            vec![Link {
                title: "rust-lang.org".into(),
                url: "https://www.rust-lang.org/learn".into()
            }]
        );
    }

    #[test]
    fn percent_decode_handles_escapes() {
        assert_eq!(percent_decode("/a%20b/c%C3%A9"), "/a b/cé");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
