//! Media transport shared by keys, buttons and colon commands: the live
//! session (expanded player or status-bar mini player), queue navigation
//! with shuffle / repeat, volume and mute (persisted), speed, percentage
//! seeks, subtitles (picker, delay, tracks) and minimize / expand.

use std::path::PathBuf;

use crate::app::effects::Effect;
use crate::app::state::{
    AppState, MediaState, MediaSurface, Mode, Repeat, StatusMessage, SubChoice, SubPickerState,
};
use crate::media::{MediaCommand, MediaKind, MediaPhase, classify_path};

fn info(state: &mut AppState, text: impl Into<String>) {
    state.message = Some(StatusMessage::info(text));
}

/// Highest volume offered (mpv and rodio both amplify past 100%).
pub const MAX_VOLUME: u8 = 130;

/// Resolves how a new video session paints (setting, terminal graphics,
/// display); text output always runs fullscreen.
pub fn choose_backend(state: &AppState, media: &mut MediaState) {
    use crate::media::{VideoBackend, resolve_video_backend};
    use ratatui_image::picker::ProtocolType;
    if media.kind != MediaKind::Video {
        return;
    }
    let graphics = match state.picker.protocol_type() {
        ProtocolType::Kitty => Some(VideoBackend::Kitty),
        ProtocolType::Sixel => Some(VideoBackend::Sixel),
        _ => None,
    };
    media.backend = resolve_video_backend(state.settings.video_output, graphics, state.has_display);
    if media.backend.fullscreen_only() {
        media.fullscreen = true;
    }
}

/// The live session: the expanded player, else the mini player.
pub fn media_mut(state: &mut AppState) -> Option<&mut MediaState> {
    match &mut state.mode {
        Mode::Media(media) => Some(media.as_mut()),
        _ => state.mini.as_deref_mut(),
    }
}

pub fn media_ref(state: &AppState) -> Option<&MediaState> {
    match &state.mode {
        Mode::Media(media) => Some(media.as_ref()),
        _ => state.mini.as_deref(),
    }
}

/// A backend is attached and accepts commands.
pub fn is_live(media: &MediaState) -> bool {
    !matches!(
        media.phase,
        MediaPhase::Preparing | MediaPhase::Stopping | MediaPhase::Error
    )
}

fn command(state: &AppState, command: MediaCommand) -> Vec<Effect> {
    match media_ref(state) {
        Some(media) if is_live(media) => vec![Effect::MediaCommand {
            session: media.session,
            command,
        }],
        _ => Vec::new(),
    }
}

fn new_session(state: &mut AppState) -> u64 {
    let session = state.next_media_session;
    state.next_media_session = state.next_media_session.wrapping_add(1).max(1);
    session
}

/// Commands that restore the listener's preferences on a fresh backend
/// (sent right after `Load`).
pub fn restore_commands(media: &MediaState) -> Vec<MediaCommand> {
    let mut out = Vec::new();
    if media.volume != 100 {
        out.push(MediaCommand::SetVolume(media.volume));
    }
    if media.muted {
        out.push(MediaCommand::SetMute(true));
    }
    if (media.speed - 1.0).abs() > f64::EPSILON {
        out.push(MediaCommand::SetSpeed(media.speed));
    }
    if media.kind == MediaKind::Video {
        if let Some(sub) = &media.sub_file {
            out.push(MediaCommand::AddSub(sub.clone()));
        }
        if media.subs_off {
            out.push(MediaCommand::SetSubtitles(false));
        }
        if media.sub_delay.abs() > f64::EPSILON {
            out.push(MediaCommand::AddSubDelay(media.sub_delay));
        }
    }
    out
}

/// A cheap deterministic shuffle step (no RNG dependency): mixes the
/// session id and the queue position.
fn shuffle_pick(seed: u64, len: usize, current: usize) -> usize {
    if len <= 1 {
        return current;
    }
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (current as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 31;
    let pick = (x % (len as u64 - 1)) as usize;
    if pick >= current { pick + 1 } else { pick }
}

/// Queue index to play after `media` ends (`None`: stop).
pub fn after_end(media: &MediaState) -> Option<usize> {
    let len = media.playlist.len();
    if len == 0 {
        return (media.repeat == Repeat::One).then_some(0);
    }
    match media.repeat {
        Repeat::One => Some(media.playlist_pos),
        _ if media.shuffle && len > 1 => Some(shuffle_pick(media.session, len, media.playlist_pos)),
        Repeat::All => Some((media.playlist_pos + 1) % len),
        Repeat::Off => (media.playlist_pos + 1 < len).then_some(media.playlist_pos + 1),
    }
}

/// Replaces the current track with queue entry `pos`, in whichever
/// player (expanded or mini) the session lives.
pub fn play_index(state: &mut AppState, pos: usize) -> Vec<Effect> {
    let in_modal = matches!(state.mode, Mode::Media(_));
    let Some(current) = media_ref(state).cloned() else {
        return Vec::new();
    };
    let path = if current.playlist.is_empty() {
        current.path.clone()
    } else {
        match current.playlist.get(pos) {
            Some(path) => path.clone(),
            None => return Vec::new(),
        }
    };
    let kind = classify_path(&path).unwrap_or(current.kind);
    let session = new_session(state);
    let mut next =
        MediaState::preparing_with_playlist(session, path, kind, current.playlist.clone(), pos);
    next.inherit_preferences(&current);
    if in_modal || kind == MediaKind::Video {
        // The player modal reserves the surface, then starts the backend.
        state.mini = None;
        state.mode = Mode::Media(Box::new(next));
        return Vec::new();
    }
    // Mini player: audio needs no surface, start straight away.
    let surface = MediaSurface {
        rect: ratatui::layout::Rect::default(),
        terminal_cells: (state.width, state.height),
        cell_pixels: state.picker.font_size(),
    };
    next.surface = Some(surface);
    next.awaiting_surface_ready = false;
    let effect = Effect::StartMedia {
        session,
        path: next.path.clone(),
        kind,
        surface,
        resume_position: None,
        resume_paused: None,
        backend: next.backend,
    };
    state.mini = Some(Box::new(next));
    vec![effect]
}

/// Next track (`n`): honors shuffle, wraps only with repeat all.
pub fn next(state: &mut AppState) -> Vec<Effect> {
    let Some(media) = media_ref(state) else {
        return Vec::new();
    };
    if matches!(media.phase, MediaPhase::Preparing | MediaPhase::Stopping) {
        return Vec::new();
    }
    let len = media.playlist.len();
    let target = if media.shuffle && len > 1 {
        Some(shuffle_pick(
            media.session ^ 0x5bd1,
            len,
            media.playlist_pos,
        ))
    } else if media.playlist_pos + 1 < len {
        Some(media.playlist_pos + 1)
    } else if media.repeat == Repeat::All && len > 0 {
        Some(0)
    } else {
        None
    };
    match target {
        Some(pos) => play_index(state, pos),
        None => {
            info(state, "end of playlist");
            Vec::new()
        }
    }
}

/// Previous track (`p`); restarts the current one past 3 seconds.
pub fn prev(state: &mut AppState) -> Vec<Effect> {
    let Some(media) = media_ref(state) else {
        info(state, "nothing is playing");
        return Vec::new();
    };
    if matches!(media.phase, MediaPhase::Preparing | MediaPhase::Stopping) {
        return Vec::new();
    }
    if media.position > 3.0 || media.playlist_pos == 0 {
        if media.playlist_pos == 0 && media.repeat == Repeat::All && media.position <= 3.0 {
            let last = media.playlist.len().saturating_sub(1);
            return play_index(state, last);
        }
        return command(state, MediaCommand::SeekAbsolute(0.0));
    }
    let pos = media.playlist_pos - 1;
    play_index(state, pos)
}

/// Appends the selection (or focus) to the live queue.
pub fn enqueue(state: &mut AppState) -> Vec<Effect> {
    let targets: Vec<PathBuf> = state
        .browser
        .action_targets()
        .into_iter()
        .filter(|p| classify_path(p).is_some())
        .collect();
    let Some(media) = media_mut(state) else {
        info(state, "play a track first, then queue more");
        return Vec::new();
    };
    if media.playlist.is_empty() {
        media.playlist.push(media.path.clone());
        media.playlist_pos = 0;
    }
    let added = targets.len();
    media.playlist.extend(targets);
    info(
        state,
        format!("queued {added} track{}", if added == 1 { "" } else { "s" }),
    );
    Vec::new()
}

pub fn add_sub(state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    let Some(media) = media_mut(state) else {
        info(state, "open a video first");
        return Vec::new();
    };
    if media.kind != MediaKind::Video {
        info(state, "subtitles apply to videos");
        return Vec::new();
    }
    media.sub_file = Some(path.clone());
    media.subs_off = false;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fx = command(state, MediaCommand::AddSub(path));
    info(state, format!("subtitles: {name}"));
    fx
}

pub fn set_volume(state: &mut AppState, volume: u8) -> Vec<Effect> {
    let volume = volume.min(MAX_VOLUME);
    state.settings.volume = volume;
    state.settings_dirty = true;
    let Some(media) = media_mut(state) else {
        return Vec::new();
    };
    media.volume = volume;
    let unmute = media.muted;
    media.muted = false;
    let mut fx = command(state, MediaCommand::SetVolume(volume));
    if unmute {
        fx.extend(command(state, MediaCommand::SetMute(false)));
    }
    fx
}

pub fn change_volume(state: &mut AppState, delta: i8) -> Vec<Effect> {
    let Some(media) = media_ref(state) else {
        return Vec::new();
    };
    let volume = (i16::from(media.volume) + i16::from(delta)).clamp(0, i16::from(MAX_VOLUME)) as u8;
    set_volume(state, volume)
}

pub fn toggle_mute(state: &mut AppState) -> Vec<Effect> {
    let Some(media) = media_mut(state) else {
        return Vec::new();
    };
    media.muted = !media.muted;
    let muted = media.muted;
    let volume = media.volume;
    info(state, if muted { "muted" } else { "unmuted" });
    let mut fx = command(state, MediaCommand::SetMute(muted));
    if !muted {
        // The in-process backend mutes by zeroing its level.
        fx.extend(command(state, MediaCommand::SetVolume(volume)));
    }
    fx
}

pub fn toggle_shuffle(state: &mut AppState) -> Vec<Effect> {
    if let Some(media) = media_mut(state) {
        media.shuffle = !media.shuffle;
        let on = media.shuffle;
        info(state, if on { "shuffle on" } else { "shuffle off" });
    }
    Vec::new()
}

pub fn cycle_repeat(state: &mut AppState) -> Vec<Effect> {
    if let Some(media) = media_mut(state) {
        media.repeat = media.repeat.next();
        let label = media.repeat.label();
        info(state, label);
    }
    Vec::new()
}

/// Speed steps: 0.5 … 2.0 in 0.25 increments (0 resets).
pub fn speed(state: &mut AppState, step: i8) -> Vec<Effect> {
    let Some(media) = media_mut(state) else {
        return Vec::new();
    };
    media.speed = if step == 0 {
        1.0
    } else {
        (media.speed + f64::from(step) * 0.25).clamp(0.5, 2.0)
    };
    let speed = media.speed;
    info(state, format!("speed {speed:.2}×"));
    command(state, MediaCommand::SetSpeed(speed))
}

pub fn seek_percent(state: &mut AppState, tenth: u8) -> Vec<Effect> {
    command(
        state,
        MediaCommand::SeekPercent(f64::from(tenth.min(9)) * 10.0),
    )
}

fn video_only(state: &mut AppState) -> bool {
    let video = media_ref(state).is_some_and(|m| m.kind == MediaKind::Video);
    if !video {
        info(state, "subtitles apply to videos");
    }
    video
}

pub fn cycle_sub(state: &mut AppState) -> Vec<Effect> {
    if !video_only(state) {
        return Vec::new();
    }
    if let Some(media) = media_mut(state) {
        media.subs_off = false;
    }
    info(state, "next subtitle track");
    command(state, MediaCommand::CycleSub)
}

pub fn toggle_subs(state: &mut AppState) -> Vec<Effect> {
    if !video_only(state) {
        return Vec::new();
    }
    let Some(media) = media_mut(state) else {
        return Vec::new();
    };
    media.subs_off = !media.subs_off;
    let on = !media.subs_off;
    info(state, if on { "subtitles on" } else { "subtitles off" });
    command(state, MediaCommand::SetSubtitles(on))
}

pub fn cycle_audio(state: &mut AppState) -> Vec<Effect> {
    info(state, "next audio track");
    command(state, MediaCommand::CycleAudio)
}

pub fn sub_delay(state: &mut AppState, tenths: i8) -> Vec<Effect> {
    if !video_only(state) {
        return Vec::new();
    }
    let step = f64::from(tenths) / 10.0;
    if let Some(media) = media_mut(state) {
        media.sub_delay = ((media.sub_delay + step) * 10.0).round() / 10.0;
        let delay = media.sub_delay;
        info(state, format!("subtitle delay {delay:+.1} s"));
    }
    command(state, MediaCommand::AddSubDelay(step))
}

/// Esc in the audio player: keep playing in the status-bar mini player.
pub fn minimize(state: &mut AppState) -> Vec<Effect> {
    let Mode::Media(media) = &state.mode else {
        return Vec::new();
    };
    if media.kind != MediaKind::Audio || !is_live(media) {
        return crate::app::reduce::reduce_inner(state, crate::app::action::Action::MediaClose);
    }
    let Mode::Media(mut media) = std::mem::replace(&mut state.mode, Mode::Browser) else {
        return Vec::new();
    };
    media.clear_slider_state();
    media.sub_picker = None;
    state.mini = Some(media);
    info(state, "playing in the background · M opens the player");
    Vec::new()
}

/// `M`: the mini player becomes the full player again.
pub fn expand(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.mode, Mode::Browser) {
        return Vec::new();
    }
    match state.mini.take() {
        Some(media) => {
            state.mode = Mode::Media(media);
            Vec::new()
        }
        None => {
            info(state, "nothing is playing");
            Vec::new()
        }
    }
}

// --- Subtitle picker -------------------------------------------------------

fn refresh_choices(picker: &mut SubPickerState) {
    let files = crate::media::subs::filter(&picker.found, &picker.query);
    let mut choices = Vec::new();
    if picker.query.trim().is_empty() {
        choices.push(SubChoice::Off);
        choices.push(SubChoice::Embedded);
    }
    choices.extend(files.into_iter().map(SubChoice::File));
    picker.selected = picker.selected.min(choices.len().saturating_sub(1));
    picker.choices = choices;
}

pub fn open_subs(state: &mut AppState) -> Vec<Effect> {
    let Mode::Media(media) = &mut state.mode else {
        return Vec::new();
    };
    if media.kind != MediaKind::Video {
        info(state, "subtitles apply to videos");
        return Vec::new();
    }
    // Video frames sit above text: pause and clear them so the picker is
    // visible; closing resumes.
    let playing = media.phase == MediaPhase::Playing;
    let mut picker = SubPickerState {
        searching: true,
        resume: playing,
        ..SubPickerState::default()
    };
    refresh_choices(&mut picker);
    media.sub_picker = Some(picker);
    let session = media.session;
    let video = media.path.clone();
    let mut fx = Vec::new();
    if playing {
        fx.push(Effect::MediaCommand {
            session,
            command: MediaCommand::TogglePause,
        });
    }
    fx.push(Effect::ClearGraphics);
    fx.push(Effect::FindSubtitles { session, video });
    fx
}

pub fn subs_found(
    state: &mut AppState,
    session: u64,
    files: Vec<crate::media::subs::SubtitleFile>,
) -> Vec<Effect> {
    if let Some(media) = media_mut(state)
        && media.session == session
        && let Some(picker) = &mut media.sub_picker
    {
        picker.found = files;
        picker.searching = false;
        // Land on the best match rather than "Off".
        picker.selected = if picker.found.is_empty() { 1 } else { 2 };
        refresh_choices(picker);
    }
    Vec::new()
}

fn picker_mut(state: &mut AppState) -> Option<&mut SubPickerState> {
    match &mut state.mode {
        Mode::Media(media) => media.sub_picker.as_mut(),
        _ => None,
    }
}

pub fn picker_char(state: &mut AppState, c: char) -> Vec<Effect> {
    if let Some(picker) = picker_mut(state) {
        picker.query.push(c);
        picker.selected = 0;
        refresh_choices(picker);
    }
    Vec::new()
}

pub fn picker_backspace(state: &mut AppState) -> Vec<Effect> {
    if let Some(picker) = picker_mut(state) {
        picker.query.pop();
        refresh_choices(picker);
    }
    Vec::new()
}

pub fn picker_move(state: &mut AppState, delta: isize) -> Vec<Effect> {
    if let Some(picker) = picker_mut(state) {
        let len = picker.choices.len();
        if len > 0 {
            picker.selected = (picker.selected as isize + delta).rem_euclid(len as isize) as usize;
        }
    }
    Vec::new()
}

pub fn picker_close(state: &mut AppState) -> Vec<Effect> {
    let resume = match &mut state.mode {
        Mode::Media(media) => media.sub_picker.take().map(|p| p.resume),
        _ => None,
    };
    match resume {
        // Resuming repaints the video by itself.
        Some(true) => command(state, MediaCommand::TogglePause),
        // A paused video needs one frame drawn again.
        Some(false) => command(state, MediaCommand::Redraw),
        None => Vec::new(),
    }
}

pub fn picker_submit(state: &mut AppState) -> Vec<Effect> {
    let choice = picker_mut(state).and_then(|p| p.choices.get(p.selected).cloned());
    let mut fx = picker_close(state);
    fx.extend(match choice {
        Some(SubChoice::Off) => {
            if let Some(media) = media_mut(state) {
                media.subs_off = true;
            }
            info(state, "subtitles off");
            command(state, MediaCommand::SetSubtitles(false))
        }
        Some(SubChoice::Embedded) => {
            if let Some(media) = media_mut(state) {
                media.subs_off = false;
            }
            info(state, "video's own subtitles");
            let mut fx = command(state, MediaCommand::SetSubtitles(true));
            fx.extend(command(state, MediaCommand::CycleSub));
            fx
        }
        Some(SubChoice::File(file)) => add_sub(state, file.path),
        None => Vec::new(),
    });
    fx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(len: usize, pos: usize) -> MediaState {
        let playlist: Vec<PathBuf> = (0..len)
            .map(|i| PathBuf::from(format!("/m/{i}.mp3")))
            .collect();
        MediaState::preparing_with_playlist(
            7,
            playlist[pos].clone(),
            MediaKind::Audio,
            playlist,
            pos,
        )
    }

    #[test]
    fn after_end_respects_repeat_and_shuffle() {
        let mut m = media(3, 2);
        assert_eq!(after_end(&m), None, "end of queue stops");
        m.repeat = Repeat::All;
        assert_eq!(after_end(&m), Some(0), "repeat all wraps");
        m.repeat = Repeat::One;
        assert_eq!(after_end(&m), Some(2), "repeat one replays");
        m.repeat = Repeat::Off;
        m.shuffle = true;
        let pick = after_end(&m).unwrap();
        assert!(pick < 3 && pick != 2, "shuffle picks another track");
    }

    #[test]
    fn shuffle_never_repeats_current() {
        for seed in 0..50 {
            for current in 0..5 {
                let pick = shuffle_pick(seed, 5, current);
                assert!(pick < 5 && pick != current);
            }
        }
    }

    #[test]
    fn restore_commands_reapply_preferences() {
        let mut m = media(1, 0);
        assert!(restore_commands(&m).is_empty());
        m.volume = 60;
        m.muted = true;
        m.speed = 1.5;
        let cmds = restore_commands(&m);
        assert!(cmds.contains(&MediaCommand::SetVolume(60)));
        assert!(cmds.contains(&MediaCommand::SetMute(true)));
        assert!(cmds.contains(&MediaCommand::SetSpeed(1.5)));
    }
}
