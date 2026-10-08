//! Track metadata (title, artist, album, year) and embedded cover art,
//! read with symphonia from ID3 / Vorbis comments / MP4 atoms.

use std::path::Path;

use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey};
use symphonia::core::probe::Hint;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<String>,
}

impl TrackTags {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.artist.is_none() && self.album.is_none()
    }

    /// "Artist · Album · Year" (whatever is known).
    pub fn byline(&self) -> String {
        [&self.artist, &self.album, &self.year]
            .iter()
            .filter_map(|v| v.as_deref())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub tags: TrackTags,
    /// Encoded cover image (JPEG / PNG) when the file embeds one.
    pub cover: Option<Vec<u8>>,
}

fn absorb(info: &mut TrackInfo, revision: &MetadataRevision) {
    for tag in revision.tags() {
        let value = tag.value.to_string().trim().to_string();
        if value.is_empty() {
            continue;
        }
        let slot = match tag.std_key {
            Some(StandardTagKey::TrackTitle) => &mut info.tags.title,
            Some(StandardTagKey::Artist) => &mut info.tags.artist,
            Some(StandardTagKey::AlbumArtist) if info.tags.artist.is_none() => {
                &mut info.tags.artist
            }
            Some(StandardTagKey::Album) => &mut info.tags.album,
            Some(StandardTagKey::Date) | Some(StandardTagKey::ReleaseDate) => {
                // Keep just the year of full dates.
                info.tags
                    .year
                    .get_or_insert_with(|| value.chars().take(4).collect());
                continue;
            }
            _ => continue,
        };
        slot.get_or_insert(value);
    }
    if info.cover.is_none() {
        let visuals = revision.visuals();
        let front = visuals
            .iter()
            .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| visuals.first());
        if let Some(visual) = front {
            info.cover = Some(visual.data.to_vec());
        }
    }
}

/// Reads tags and cover art; files symphonia cannot open yield defaults.
pub fn read(path: &Path) -> TrackInfo {
    let mut info = TrackInfo::default();
    let Ok(file) = std::fs::File::open(path) else {
        return info;
    };
    let mut hint = Hint::new();
    if let Some(ext) = path.extension() {
        hint.with_extension(&ext.to_string_lossy());
    }
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let Ok(mut probed) = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return info;
    };
    // Container-level tags read during probing (ID3v2 in front of MP3s).
    if let Some(metadata) = probed.metadata.get()
        && let Some(revision) = metadata.current()
    {
        absorb(&mut info, revision);
    }
    // Tags inside the format itself (FLAC / Vorbis comments, MP4 atoms).
    if let Some(revision) = probed.format.metadata().current() {
        absorb(&mut info, revision);
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byline_joins_known_fields() {
        let tags = TrackTags {
            title: Some("Song".into()),
            artist: Some("Band".into()),
            album: None,
            year: Some("2020".into()),
        };
        assert_eq!(tags.byline(), "Band · 2020");
        assert!(!tags.is_empty());
        assert!(TrackTags::default().is_empty());
    }

    #[test]
    fn missing_file_reads_as_empty() {
        assert_eq!(read(Path::new("/nonexistent/x.mp3")), TrackInfo::default());
    }

    #[test]
    fn reads_id3_tags_and_cover_from_a_real_file() {
        // Generated on the fly; skipped where ffmpeg is unavailable.
        let dir = std::env::temp_dir().join(format!("tui-explorer-tags-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cover = dir.join("cover.png");
        let song = dir.join("song.mp3");
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=orange:s=32x32:d=1",
            ])
            .args(["-frames:v", "1"])
            .arg(&cover)
            .status()
            .is_ok_and(|s| s.success())
            && std::process::Command::new("ffmpeg")
                .args([
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=duration=1",
                ])
                .arg("-i")
                .arg(&cover)
                .args([
                    "-map",
                    "0:a",
                    "-map",
                    "1:v",
                    "-c:v",
                    "png",
                    "-disposition:v",
                    "attached_pic",
                ])
                .args(["-id3v2_version", "3"])
                .args([
                    "-metadata",
                    "title=Orange Song",
                    "-metadata",
                    "artist=The Testers",
                ])
                .args(["-metadata", "album=Fixtures", "-metadata", "date=2024"])
                .arg(&song)
                .status()
                .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let info = read(&song);
        assert_eq!(info.tags.title.as_deref(), Some("Orange Song"));
        assert_eq!(info.tags.artist.as_deref(), Some("The Testers"));
        assert_eq!(info.tags.album.as_deref(), Some("Fixtures"));
        assert_eq!(info.tags.year.as_deref(), Some("2024"));
        let bytes = info.cover.expect("embedded cover");
        assert!(image::load_from_memory(&bytes).is_ok(), "cover decodes");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
