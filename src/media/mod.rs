use std::path::Path;

pub mod aiff;
pub mod audio;
pub mod mpv;
pub mod subs;
pub mod tags;

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
    /// Load an external subtitle file and select it.
    AddSub(std::path::PathBuf),
    /// Turn subtitles off (`false`) or back on to the auto track (`true`).
    SetSubtitles(bool),
    /// Step through the video's subtitle tracks.
    CycleSub,
    /// Shift subtitles by this many seconds (relative).
    AddSubDelay(f64),
    /// Playback speed multiplier (absolute).
    SetSpeed(f64),
    /// Jump to a percentage of the duration (0..=100).
    SeekPercent(f64),
    SetMute(bool),
    /// Step through audio tracks (video files with several languages).
    CycleAudio,
    /// GUI window fullscreen (window backend only).
    SetFullscreen(bool),
    /// Paint the current frame again (exact zero seek) after an overlay
    /// covered a paused video.
    Redraw,
}

/// How a video session paints its frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoBackend {
    /// Kitty graphics protocol inside the player's reserved rectangle.
    #[default]
    Kitty,
    /// Sixel graphics inside the reserved rectangle.
    Sixel,
    /// mpv's own GUI window; the TUI is the remote control.
    Window,
    /// mpv's truecolor text renderer, using the whole terminal.
    Tct,
}

impl VideoBackend {
    /// Frames are painted into the terminal (not a separate window).
    pub fn in_terminal(self) -> bool {
        !matches!(self, VideoBackend::Window)
    }

    /// Only the full terminal works (frames always start at the origin).
    pub fn fullscreen_only(self) -> bool {
        matches!(self, VideoBackend::Tct)
    }

    pub fn label(self) -> &'static str {
        match self {
            VideoBackend::Kitty => "kitty",
            VideoBackend::Sixel => "sixel",
            VideoBackend::Window => "window",
            VideoBackend::Tct => "text",
        }
    }
}

/// Picks the video backend: an explicit setting wins; `auto` prefers
/// in-terminal graphics, then a GUI window, then truecolor text.
pub fn resolve_video_backend(
    setting: crate::settings::VideoOutput,
    graphics: Option<VideoBackend>,
    has_display: bool,
) -> VideoBackend {
    use crate::settings::VideoOutput;
    match setting {
        VideoOutput::Kitty => VideoBackend::Kitty,
        VideoOutput::Sixel => VideoBackend::Sixel,
        VideoOutput::Window => VideoBackend::Window,
        VideoOutput::Tct => VideoBackend::Tct,
        VideoOutput::Auto => match graphics {
            Some(backend) => backend,
            None if has_display => VideoBackend::Window,
            None => VideoBackend::Tct,
        },
    }
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

#[cfg(test)]
mod backend_tests {
    use super::*;
    use crate::settings::VideoOutput;

    #[test]
    fn auto_prefers_graphics_then_window_then_text() {
        assert_eq!(
            resolve_video_backend(VideoOutput::Auto, Some(VideoBackend::Kitty), true),
            VideoBackend::Kitty
        );
        assert_eq!(
            resolve_video_backend(VideoOutput::Auto, Some(VideoBackend::Sixel), false),
            VideoBackend::Sixel
        );
        assert_eq!(
            resolve_video_backend(VideoOutput::Auto, None, true),
            VideoBackend::Window
        );
        assert_eq!(
            resolve_video_backend(VideoOutput::Auto, None, false),
            VideoBackend::Tct
        );
        assert_eq!(
            resolve_video_backend(VideoOutput::Window, Some(VideoBackend::Kitty), false),
            VideoBackend::Window
        );
    }
}
