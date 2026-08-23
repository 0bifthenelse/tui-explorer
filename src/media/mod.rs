use std::path::Path;

pub mod audio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaCommand {
    Load,
    TogglePause,
    SeekRelative(i64),
    SetVolume(u8),
    Stop,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaPhase {
    Preparing,
    Starting,
    Playing,
    Paused,
    Stopped,
    Stopping,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AfterStop {
    Close,
    Quit,
    RestartAfterResize { position: f64, paused: bool },
    ShowError(String),
}

pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "flac", "ogg", "oga", "mp3", "m4a"];
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "avi", "mov", "wmv", "flv", "ogv", "mpeg", "mpg",
];

pub fn classify_path(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Audio)
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{AUDIO_EXTENSIONS, MediaKind, VIDEO_EXTENSIONS, classify_path};

    #[test]
    fn classifies_every_supported_extension_case_insensitively() {
        for extension in AUDIO_EXTENSIONS {
            assert_eq!(
                classify_path(Path::new(&format!("track.{extension}"))),
                Some(MediaKind::Audio)
            );
            assert_eq!(
                classify_path(Path::new(&format!(
                    "track.{}",
                    extension.to_ascii_uppercase()
                ))),
                Some(MediaKind::Audio)
            );
        }
        for extension in VIDEO_EXTENSIONS {
            assert_eq!(
                classify_path(Path::new(&format!("clip.{extension}"))),
                Some(MediaKind::Video)
            );
            assert_eq!(
                classify_path(Path::new(&format!(
                    "clip.{}",
                    extension.to_ascii_uppercase()
                ))),
                Some(MediaKind::Video)
            );
        }
        assert_eq!(classify_path(Path::new("notes.txt")), None);
    }
}
