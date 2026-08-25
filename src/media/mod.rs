use std::path::Path;

pub mod aiff;
pub mod audio;
pub mod mpv;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCommand {
    Load,
    TogglePause,
    SeekRelative(i64),
    /// Absolute position in seconds, clamped by the appliers.
    SeekAbsolute(f64),
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

/// Codecs decoded in-process by symphonia (rodio sink owns playback).
pub const NATIVE_AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "flac", "ogg", "oga", "mp3", "m4a", "aif", "aiff", "aifc",
];
/// Audio routed through the mpv fallback process (no symphonia decoder).
pub const MPV_AUDIO_EXTENSIONS: &[&str] = &["opus", "wma"];
/// Every extension classified as audio; union of native + mpv lists.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "flac", "ogg", "oga", "mp3", "m4a", "aif", "aiff", "aifc", "opus", "wma",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBackend {
    /// In-process rodio + symphonia decode.
    Native,
    /// Out-of-process mpv fallback.
    Mpv,
}

/// Routes an already-lowercased extension to its playback backend.
pub fn audio_backend_for_extension(ext_lowercase: &str) -> Option<AudioBackend> {
    if NATIVE_AUDIO_EXTENSIONS.contains(&ext_lowercase) {
        Some(AudioBackend::Native)
    } else if MPV_AUDIO_EXTENSIONS.contains(&ext_lowercase) {
        Some(AudioBackend::Mpv)
    } else {
        None
    }
}

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

    use super::{
        AUDIO_EXTENSIONS, AudioBackend, MPV_AUDIO_EXTENSIONS, MediaKind, NATIVE_AUDIO_EXTENSIONS,
        VIDEO_EXTENSIONS, audio_backend_for_extension, classify_path,
    };

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

    #[test]
    fn audio_extensions_is_sorted_dedup_union_of_backends() {
        let mut union: Vec<&str> = NATIVE_AUDIO_EXTENSIONS
            .iter()
            .chain(MPV_AUDIO_EXTENSIONS)
            .copied()
            .collect();
        union.sort_unstable();
        union.dedup();
        let mut listed = AUDIO_EXTENSIONS.to_vec();
        listed.sort_unstable();
        assert_eq!(listed, union);
    }

    #[test]
    fn backend_routing_covers_every_audio_extension_exactly_once() {
        for extension in AUDIO_EXTENSIONS {
            assert!(
                audio_backend_for_extension(extension).is_some(),
                "{extension} missing backend routing"
            );
        }
        assert_eq!(
            audio_backend_for_extension("m4a"),
            Some(AudioBackend::Native)
        );
        assert_eq!(audio_backend_for_extension("opus"), Some(AudioBackend::Mpv));
        assert_eq!(audio_backend_for_extension("txt"), None);
    }
}
