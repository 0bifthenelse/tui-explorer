//! Reproduction + regression coverage for every natively decoded audio format.
//!
//! Fixtures are deterministic 440 Hz sine tones (1 s, mono, bit-exact) generated
//! by `scripts/gen-audio-fixtures.sh` (dev-only ffmpeg helper). Reproduction half
//! written against HEAD bce5bc9 to pin the M4A failure mode before any fix;
//! observed errors are recorded in the session log.

use std::path::Path;
use std::time::Duration;

use rodio::Source;
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
    let mut source = SymphoniaSource::new(&path)
        .unwrap_or_else(|error| panic!("{name}: open failed: {error}"));

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
    let mut source = SymphoniaSource::new(&path)
        .unwrap_or_else(|error| panic!("{name}: open failed: {error}"));
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

#[test]
fn m4a_alac_opens_and_decodes() {
    assert_opens_and_decodes("tone_alac.m4a");
    assert_seek_then_decode_continues("tone_alac.m4a");
}

/// AIFF has no symphonia demuxer; at reproduction time this pins the probe
/// failure. Flipped to construct the dedicated AiffSource in the completion
/// half once src/media/aiff.rs lands.
#[test]
fn aiff_big_endian_pcm_opens_and_decodes() {
    let path = fixture("tone.aiff");
    let result = SymphoniaSource::new(&path);
    assert!(result.is_err(), "unexpected: symphonia probed aiff");
}

#[test]
fn aiff_sowt_little_endian_pcm_opens_and_decodes() {
    let path = fixture("tone_sowt.aiff");
    let result = SymphoniaSource::new(&path);
    assert!(result.is_err(), "unexpected: symphonia probed aiff sowt");
}
