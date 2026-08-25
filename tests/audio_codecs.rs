//! Reproduction + regression coverage for every natively decoded audio format.
//!
//! Fixtures are deterministic 440 Hz sine tones (1 s, mono, bit-exact) generated
//! by `scripts/gen-audio-fixtures.sh` (dev-only ffmpeg helper). The reproduction
//! half was written against HEAD bce5bc9 to pin the M4A failure mode before any
//! fix; the completion half asserts every native format decodes, seeks, and
//! fails bounded on truncation.

use std::path::Path;
use std::time::Duration;

use rodio::Source;
use tui_explorer::media::aiff::AiffSource;
use tui_explorer::media::audio::SymphoniaSource;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/audio");

fn fixture(name: &str) -> std::path::PathBuf {
    let path = Path::new(FIXTURES).join(name);
    assert!(path.exists(), "missing fixture {}", path.display());
    path
}

/// Decodes up to `seconds` of audio, returning the achieved duration.
/// Fails if the stream ends early.
fn decode_seconds(source: &mut SymphoniaSource, seconds: f64, what: &str) -> f64 {
    let rate = f64::from(source.sample_rate()) * f64::from(source.channels());
    let needed = (seconds * rate).ceil() as usize;
    let mut produced = 0usize;
    while produced < needed {
        match source.next() {
            Some(_) => produced += 1,
            None => panic!(
                "{what}: stream ended after {:.3}s of {:.3}s requested ({} samples)",
                produced as f64 / rate,
                seconds,
                produced
            ),
        }
    }
    produced as f64 / rate
}

fn assert_opens_and_decodes(name: &str) {
    let path = fixture(name);
    let mut source =
        SymphoniaSource::new(&path).unwrap_or_else(|error| panic!("{name}: open failed: {error}"));

    // Container-reported duration must be sane when present.
    if let Some(duration) = source.total_duration() {
        assert!(
            duration > Duration::from_millis(200),
            "{name}: implausible total_duration {duration:?}"
        );
    }

    decode_seconds(&mut source, 0.3, name);
}

fn assert_seek_then_decode_continues(name: &str) {
    let path = fixture(name);
    let mut source =
        SymphoniaSource::new(&path).unwrap_or_else(|error| panic!("{name}: open failed: {error}"));
    match source.try_seek(Duration::from_secs(1)) {
        Ok(()) => {}
        Err(rodio::source::SeekError::NotSupported { .. }) => return,
        Err(other) => panic!("{name}: unexpected seek error: {other}"),
    }
    // Iteration must terminate cleanly after a seek (no panic, no hang).
    let mut after_seek = 0usize;
    while source.next().is_some() {
        after_seek += 1;
        assert!(
            after_seek <= source.sample_rate() as usize * 8,
            "{name}: unbounded post-seek stream"
        );
    }
}

#[test]
fn wav_pcm_opens_decodes_seeks() {
    assert_opens_and_decodes("wav_pcm.wav");
    assert_seek_then_decode_continues("wav_pcm.wav");
}

#[test]
fn flac_opens_decodes_seeks() {
    assert_opens_and_decodes("tone.flac");
    assert_seek_then_decode_continues("tone.flac");
}

#[test]
fn mp3_opens_decodes_seeks() {
    assert_opens_and_decodes("tone.mp3");
    assert_seek_then_decode_continues("tone.mp3");
}

#[test]
fn ogg_vorbis_opens_decodes_seeks() {
    assert_opens_and_decodes("tone.ogg");
    assert_seek_then_decode_continues("tone.ogg");
}

#[test]
fn m4a_aac_lc_opens_and_decodes() {
    assert_opens_and_decodes("tone_aac.m4a");
    assert_seek_then_decode_continues("tone_aac.m4a");
}

/// AIFF is handled by the dedicated parser (no symphonia demuxer).
#[test]
fn aiff_big_endian_pcm_opens_and_decodes() {
    let path = fixture("tone.aiff");
    let mut source =
        AiffSource::new(&path).unwrap_or_else(|error| panic!("tone.aiff: open failed: {error}"));
    assert_eq!(source.sample_rate(), 22050);
    if let Some(duration) = source.total_duration() {
        assert!(duration > Duration::from_millis(200));
    }
    let rate = f64::from(source.sample_rate()) * f64::from(source.channels());
    let needed = (0.3 * rate).ceil() as usize;
    let produced = (0..needed).filter(|_| source.next().is_some()).count();
    assert_eq!(produced, needed, "aiff: short decode");
}

#[test]
fn aiff_sowt_little_endian_pcm_opens_and_decodes() {
    let path = fixture("tone_sowt.aiff");
    let mut source = AiffSource::new(&path)
        .unwrap_or_else(|error| panic!("tone_sowt.aiff: open failed: {error}"));
    let rate = f64::from(source.sample_rate()) * f64::from(source.channels());
    let needed = (0.3 * rate).ceil() as usize;
    let produced = (0..needed).filter(|_| source.next().is_some()).count();
    assert_eq!(produced, needed, "sowt: short decode");
}

/// Truncated containers must end cleanly or fail bounded — never spin or
/// panic. Every truncated fixture must terminate within a generous sample
/// budget (well beyond the 1 s of real audio).
#[test]
fn truncated_fixtures_terminate_without_hanging_or_panic() {
    for name in [
        "wav_pcm_trunc.wav",
        "tone_trunc.flac",
        "tone_trunc.mp3",
        "tone_trunc.ogg",
        "tone_aac_trunc.m4a",
        "tone_alac_trunc.m4a",
    ] {
        let path = fixture(name);
        let Ok(mut source) = SymphoniaSource::new(&path) else {
            // A clean construction-time rejection is a valid outcome.
            continue;
        };
        let budget = source.sample_rate() as usize * 8 * usize::from(source.channels());
        let mut taken = 0usize;
        while source.next().is_some() {
            taken += 1;
            assert!(taken <= budget, "{name}: unbounded stream after truncation");
        }
    }
}

/// The opus/wma family routes to the mpv fallback, never the native
/// decoder; the native set routes natively.
#[test]
fn unsupported_native_codecs_route_to_mpv_fallback() {
    use tui_explorer::media::{AudioBackend, audio_backend_for_extension};
    for ext in ["opus", "wma"] {
        assert_eq!(
            audio_backend_for_extension(ext),
            Some(AudioBackend::Mpv),
            "{ext} must use the mpv fallback"
        );
    }
    for ext in [
        "wav", "flac", "ogg", "oga", "mp3", "m4a", "aif", "aiff", "aifc",
    ] {
        assert_eq!(
            audio_backend_for_extension(ext),
            Some(AudioBackend::Native),
            "{ext} must decode natively"
        );
    }
}
