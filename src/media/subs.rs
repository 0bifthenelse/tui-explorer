//! Local subtitle discovery: finds `.srt` / `.ass` / `.ssa` / `.vtt` /
//! `.sub` files for a video next to it, in the usual `Subs/` style
//! folders (including `Subs/<video name>/`), in the parent folder and in
//! `~/Downloads`, then ranks them by how well their name matches the video
//! and by language tag.

use std::path::{Path, PathBuf};

use crate::filesystem::FileSystem;

pub const SUB_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub"];

/// Folder names that conventionally hold a release's subtitles.
const SUB_DIRS: &[&str] = &["subs", "sub", "subtitles", "subtitle", "srt"];

/// Words that say nothing about which video a file belongs to.
const NOISE: &[&str] = &[
    "1080p", "720p", "2160p", "480p", "4k", "x264", "x265", "h264", "h265", "hevc", "web",
    "webrip", "webdl", "dl", "bluray", "brrip", "bdrip", "hdtv", "dvdrip", "aac", "ac3", "dts",
    "10bit", "hdr", "proper", "repack", "the", "a",
];

const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("eng", "English"),
    ("english", "English"),
    ("fr", "French"),
    ("fre", "French"),
    ("fra", "French"),
    ("french", "French"),
    ("de", "German"),
    ("ger", "German"),
    ("deu", "German"),
    ("german", "German"),
    ("es", "Spanish"),
    ("spa", "Spanish"),
    ("spanish", "Spanish"),
    ("it", "Italian"),
    ("ita", "Italian"),
    ("pt", "Portuguese"),
    ("por", "Portuguese"),
    ("br", "Portuguese (BR)"),
    ("nl", "Dutch"),
    ("dut", "Dutch"),
    ("ru", "Russian"),
    ("rus", "Russian"),
    ("pl", "Polish"),
    ("pol", "Polish"),
    ("sv", "Swedish"),
    ("swe", "Swedish"),
    ("ja", "Japanese"),
    ("jpn", "Japanese"),
    ("zh", "Chinese"),
    ("chi", "Chinese"),
    ("zho", "Chinese"),
    ("ko", "Korean"),
    ("kor", "Korean"),
    ("ar", "Arabic"),
    ("ara", "Arabic"),
    ("tr", "Turkish"),
    ("tur", "Turkish"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitleFile {
    pub path: PathBuf,
    /// Human language name when the file name carries a tag.
    pub language: Option<String>,
    /// Forced / SDH markers found in the name.
    pub flags: Vec<String>,
    pub score: i32,
}

impl SubtitleFile {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

pub fn is_subtitle(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| SUB_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .filter(|t| !NOISE.contains(&t.as_str()))
        .collect()
}

/// Language and flags from the tokens after the video's own name
/// (`Movie.2020.en.forced.srt` → English, [forced]).
fn tags(sub_stem: &str) -> (Option<String>, Vec<String>) {
    let mut language = None;
    let mut flags = Vec::new();
    for token in sub_stem
        .rsplit(|c: char| !c.is_alphanumeric())
        .take(3)
        .map(str::to_lowercase)
    {
        match token.as_str() {
            "forced" | "sdh" | "cc" | "hi" => flags.push(token),
            t if language.is_none() => {
                if let Some((_, lang)) = LANGUAGES.iter().find(|(code, _)| *code == t) {
                    language = Some(lang.to_string());
                }
            }
            _ => {}
        }
    }
    (language, flags)
}

/// How well a subtitle file name matches the video (0 = unrelated).
pub fn match_score(video_name: &str, sub_name: &str) -> i32 {
    let video_stem = stem(video_name).to_lowercase();
    let sub_stem = stem(sub_name).to_lowercase();
    if sub_stem == video_stem {
        return 1000;
    }
    if sub_stem.starts_with(&video_stem) {
        return 900;
    }
    let video_tokens = tokens(&video_stem);
    if video_tokens.is_empty() {
        return 0;
    }
    let sub_tokens = tokens(&sub_stem);
    let shared = video_tokens
        .iter()
        .filter(|t| sub_tokens.contains(t))
        .count();
    (shared * 600 / video_tokens.len()) as i32
}

/// Preference for the user's language (`$LANG`), then English.
fn language_bonus(language: Option<&str>) -> i32 {
    let preferred = std::env::var("LANG")
        .ok()
        .and_then(|l| l.get(..2).map(str::to_lowercase))
        .and_then(|code| {
            LANGUAGES
                .iter()
                .find(|(c, _)| *c == code)
                .map(|(_, name)| name.to_string())
        });
    match language {
        Some(lang) if preferred.as_deref() == Some(lang) => 60,
        Some("English") => 30,
        Some(_) => 10,
        None => 0,
    }
}

/// Finds and ranks subtitles for `video`. `home` locates `~/Downloads`.
pub fn discover(fs: &dyn FileSystem, video: &Path, home: &Path) -> Vec<SubtitleFile> {
    let Some(dir) = video.parent() else {
        return Vec::new();
    };
    let video_name = video
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let video_stem = stem(&video_name).to_string();
    let mut found: Vec<SubtitleFile> = Vec::new();
    // (folder, bonus, keep files that do not match the name)
    let mut scan: Vec<(PathBuf, i32, bool)> = vec![(dir.to_path_buf(), 200, true)];
    if let Ok(entries) = fs.read_dir(dir) {
        for entry in entries.iter().filter(|e| e.kind.is_dir()) {
            let name = entry.display_name().to_lowercase();
            if SUB_DIRS.contains(&name.as_str()) {
                scan.push((entry.path.clone(), 150, true));
                // Subs/<video name>/ (common for TV releases).
                if let Ok(inner) = fs.read_dir(&entry.path) {
                    for sub in inner.iter().filter(|e| e.kind.is_dir()) {
                        if match_score(&video_name, &sub.display_name()) >= 600 {
                            scan.push((sub.path.clone(), 180, true));
                        }
                    }
                }
            } else if match_score(&video_name, &entry.display_name()) >= 600 {
                scan.push((entry.path.clone(), 120, true));
            }
        }
    }
    if let Some(parent) = dir.parent() {
        scan.push((parent.to_path_buf(), 0, false));
    }
    let downloads = home.join("Downloads");
    if downloads != dir {
        scan.push((downloads, -50, false));
    }
    for (folder, bonus, keep_unmatched) in scan {
        let Ok(entries) = fs.read_dir(&folder) else {
            continue;
        };
        for entry in entries {
            let name = entry.display_name();
            if entry.kind.is_dir() || !is_subtitle(&name) {
                continue;
            }
            if found.iter().any(|f| f.path == entry.path) {
                continue;
            }
            let matched = match_score(&video_name, &name);
            if matched < 300 && !keep_unmatched {
                continue;
            }
            let rest = stem(&name)
                .to_lowercase()
                .strip_prefix(&video_stem.to_lowercase())
                .map(str::to_string)
                .unwrap_or_else(|| stem(&name).to_string());
            let (language, flags) = tags(&rest);
            let score = matched + bonus + language_bonus(language.as_deref())
                - if flags.is_empty() { 0 } else { 15 };
            found.push(SubtitleFile {
                path: entry.path.clone(),
                language,
                flags,
                score,
            });
        }
    }
    found.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| crate::browser::natural_cmp(&a.name(), &b.name()))
    });
    found.truncate(200);
    found
}

/// Ranked subset of `found` matching `query` (fuzzy on the file name).
pub fn filter(found: &[SubtitleFile], query: &str) -> Vec<SubtitleFile> {
    if query.trim().is_empty() {
        return found.to_vec();
    }
    let mut scored: Vec<(i32, &SubtitleFile)> = found
        .iter()
        .filter_map(|f| {
            let hay = format!("{} {}", f.name(), f.language.clone().unwrap_or_default());
            crate::app::fuzzy::fuzzy_score(query, &hay).map(|s| (s, f))
        })
        .collect();
    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    scored.into_iter().map(|(_, f)| f.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{DirEntry, EntryKind};
    use crate::testing::MemoryFileSystem;

    fn file(dir: &Path, name: &str) -> DirEntry {
        let mut e = DirEntry::synthetic(dir, name, false);
        e.kind = EntryKind::File;
        e
    }

    fn folder(dir: &Path, name: &str) -> DirEntry {
        DirEntry::synthetic(dir, name, true)
    }

    fn fixture() -> MemoryFileSystem {
        let mut fs = MemoryFileSystem::new();
        let movies = PathBuf::from("/home/u/Movies");
        let subs = movies.join("Subs");
        let per = subs.join("Big.Buck.Demo.2020.1080p");
        fs.add_dir(&per);
        fs.add_dir(&PathBuf::from("/home/u/Downloads"));
        fs.add_entry(&movies, file(&movies, "Big.Buck.Demo.2020.1080p.mkv"));
        fs.add_entry(&movies, file(&movies, "Big.Buck.Demo.2020.1080p.srt"));
        fs.add_entry(&movies, file(&movies, "Other.Film.srt"));
        fs.add_entry(&movies, folder(&movies, "Subs"));
        fs.add_entry(&subs, file(&subs, "Big.Buck.Demo.2020.1080p.fr.srt"));
        fs.add_entry(&subs, folder(&subs, "Big.Buck.Demo.2020.1080p"));
        fs.add_entry(&per, file(&per, "2_English.srt"));
        let dl = PathBuf::from("/home/u/Downloads");
        fs.add_entry(&dl, file(&dl, "big buck demo 2020 en.ass"));
        fs.add_entry(&dl, file(&dl, "unrelated.srt"));
        fs
    }

    #[test]
    fn discovers_next_to_video_in_subs_folders_and_downloads() {
        let fs = fixture();
        let found = discover(
            &fs,
            Path::new("/home/u/Movies/Big.Buck.Demo.2020.1080p.mkv"),
            Path::new("/home/u"),
        );
        let names: Vec<String> = found.iter().map(|f| f.name()).collect();
        assert_eq!(
            names[0], "Big.Buck.Demo.2020.1080p.srt",
            "exact match first: {names:?}"
        );
        assert!(names.contains(&"Big.Buck.Demo.2020.1080p.fr.srt".to_string()));
        assert!(
            names.contains(&"2_English.srt".to_string()),
            "Subs/<name>/: {names:?}"
        );
        assert!(names.contains(&"big buck demo 2020 en.ass".to_string()));
        assert!(!names.contains(&"unrelated.srt".to_string()));
        let fr = found
            .iter()
            .find(|f| f.name().ends_with(".fr.srt"))
            .unwrap();
        assert_eq!(fr.language.as_deref(), Some("French"));
    }

    #[test]
    fn match_score_prefers_same_stem() {
        assert_eq!(match_score("Movie.mkv", "Movie.srt"), 1000);
        assert_eq!(match_score("Movie.mkv", "Movie.en.srt"), 900);
        assert!(match_score("The.Show.S01E02.720p.mkv", "the show s01e02.srt") >= 600);
        assert_eq!(match_score("Movie.mkv", "Unrelated.srt"), 0);
    }

    #[test]
    fn filter_narrows_by_query() {
        let fs = fixture();
        let found = discover(
            &fs,
            Path::new("/home/u/Movies/Big.Buck.Demo.2020.1080p.mkv"),
            Path::new("/home/u"),
        );
        let fr = filter(&found, "french");
        assert_eq!(fr.len(), 1);
        assert!(fr[0].name().ends_with(".fr.srt"));
    }
}
