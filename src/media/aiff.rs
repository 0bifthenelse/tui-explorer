//! Native AIFF/AIFC decoder for the extensions symphonia cannot probe
//! (`aif`, `aiff`, `aifc`). Streams samples straight off disk without
//! buffering the whole file, mirroring [`super::audio::SymphoniaSource`]'s
//! public shape: `new`/`total_duration` plus an interleaved `f32` iterator
//! implementing `rodio::Source`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use rodio::Source;

/// Sample encodings supported inside FORM/AIFF and FORM/AIFC files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleFormat {
    /// Unsigned 8-bit PCM (classic `NONE` at 8 bits).
    Uint8,
    /// Signed big-endian integer PCM (`twos`, or `NONE` above 8 bits).
    IntBe(u16),
    /// Signed little-endian integer PCM (`sowt`).
    IntLe(u16),
    /// Big-endian 32-bit IEEE float (`FL32`).
    Float32Be,
}

/// Parsed COMM chunk payload.
struct CommChunk {
    channels: u16,
    frames: u32,
    bits: u16,
    sample_rate: u32,
    compression: String,
}

/// Byte range of usable sound data inside the SSND chunk.
struct SsndRegion {
    start: u64,
    len: u64,
}

pub struct AiffSource {
    data: File,
    sound_start: u64,
    bytes_per_sample: usize,
    channels: usize,
    sample_rate: u32,
    total_frames: u64,
    position_sample: u64,
    total_duration: Option<Duration>,
    scratch: Vec<u8>,
    sample_format: SampleFormat,
    finished: bool,
}

impl AiffSource {
    pub fn new(path: &Path) -> Result<Self, String> {
        let mut data =
            File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        let file_len = data
            .metadata()
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?
            .len();

        let mut header = [0u8; 12];
        data.read_exact(&mut header)
            .map_err(|_| "not an aiff file: missing FORM header".to_string())?;
        if &header[0..4] != b"FORM" {
            return Err("not an aiff file: FORM magic missing".to_string());
        }
        let aifc = &header[8..12] == b"AIFC";
        if !aifc && &header[8..12] != b"AIFF" {
            return Err(format!(
                "not an aiff file: unknown form type {}",
                String::from_utf8_lossy(&header[8..12])
            ));
        }

        let mut comm: Option<CommChunk> = None;
        let mut ssnd: Option<SsndRegion> = None;
        let mut pos = 12u64;
        while pos + 8 <= file_len {
            data.seek(SeekFrom::Start(pos))
                .map_err(|error| format!("cannot seek aiff file: {error}"))?;
            let mut chunk_header = [0u8; 8];
            data.read_exact(&mut chunk_header)
                .map_err(|_| "truncated aiff chunk header".to_string())?;
            let id = &chunk_header[0..4];
            let size = u64::from(u32::from_be_bytes(chunk_header[4..8].try_into().unwrap()));
            let body = pos + 8;
            if body + size > file_len {
                return Err(format!(
                    "truncated aiff chunk {}: declares {size} bytes, {} available",
                    String::from_utf8_lossy(id),
                    file_len - body
                ));
            }
            match id {
                b"COMM" => {
                    if size > 1024 {
                        return Err(format!("implausible aiff COMM chunk: {size} bytes"));
                    }
                    let mut payload = vec![0u8; size as usize];
                    data.read_exact(&mut payload)
                        .map_err(|_| "truncated aiff COMM chunk".to_string())?;
                    comm = Some(parse_comm(&payload, aifc)?);
                }
                b"SSND" => {
                    let mut fields = [0u8; 8];
                    data.read_exact(&mut fields)
                        .map_err(|_| "truncated aiff SSND chunk".to_string())?;
                    let offset = u64::from(u32::from_be_bytes(fields[0..4].try_into().unwrap()));
                    let payload_len = size - 8;
                    if offset > payload_len {
                        return Err(
                            "truncated aiff SSND chunk: offset beyond sound data".to_string()
                        );
                    }
                    ssnd = Some(SsndRegion {
                        start: body + 8 + offset,
                        len: payload_len - offset,
                    });
                }
                _ => {}
            }
            pos = body + size + (size & 1);
        }

        let comm = comm.ok_or_else(|| "no COMM chunk in aiff file".to_string())?;
        let ssnd = ssnd.ok_or_else(|| "no SSND chunk in aiff file".to_string())?;
        if comm.sample_rate == 0 {
            return Err("invalid aiff COMM: zero sample rate".to_string());
        }
        let sample_format = resolve_sample_format(&comm.compression, comm.bits)?;
        // Guard the declared magnitude before deriving offsets or allocating.
        let sample_count = u64::from(comm.frames) * u64::from(comm.channels);
        if sample_count > 1 << 32 {
            return Err("aiff stream too large: frames x channels exceeds 2^32".to_string());
        }
        let bytes_per_sample = comm.bits as usize / 8;
        let required = sample_count * bytes_per_sample as u64;
        if required > ssnd.len {
            return Err(format!(
                "truncated aiff sound data: need {required} bytes, have {}",
                ssnd.len
            ));
        }

        data.seek(SeekFrom::Start(ssnd.start))
            .map_err(|error| format!("cannot seek aiff sound data: {error}"))?;
        Ok(Self {
            sound_start: ssnd.start,
            bytes_per_sample,
            channels: comm.channels as usize,
            sample_rate: comm.sample_rate,
            total_frames: u64::from(comm.frames),
            position_sample: 0,
            total_duration: Some(Duration::from_secs_f64(
                f64::from(comm.frames) / f64::from(comm.sample_rate),
            )),
            scratch: vec![0u8; bytes_per_sample],
            sample_format,
            finished: false,
            data,
        })
    }

    /// Declared duration from the COMM frame count.
    pub fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
}

impl Iterator for AiffSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.finished {
            return None;
        }
        let total_samples = self.total_frames * self.channels as u64;
        if self.position_sample >= total_samples {
            self.finished = true;
            return None;
        }
        if self.data.read_exact(&mut self.scratch).is_err() {
            self.finished = true;
            return None;
        }
        self.position_sample += 1;
        Some(decode_sample(&self.scratch, self.sample_format))
    }
}

impl Source for AiffSource {
    fn current_span_len(&self) -> Option<usize> {
        if self.finished {
            return None;
        }
        let total_samples = self.total_frames * self.channels as u64;
        let remaining = total_samples.saturating_sub(self.position_sample);
        Some(usize::try_from(remaining).unwrap_or(usize::MAX))
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels as rodio::ChannelCount
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        let frame = (position.as_secs_f64() * f64::from(self.sample_rate)).round() as u64;
        let frame = frame.min(self.total_frames);
        let bytes_per_frame = self.bytes_per_sample * self.channels;
        let byte = self.sound_start + frame * bytes_per_frame as u64;
        self.data.seek(SeekFrom::Start(byte)).map_err(|_| {
            rodio::source::SeekError::NotSupported {
                underlying_source: std::any::type_name::<Self>(),
            }
        })?;
        self.position_sample = frame * self.channels as u64;
        self.finished = false;
        Ok(())
    }
}

fn parse_comm(payload: &[u8], aifc: bool) -> Result<CommChunk, String> {
    if payload.len() < 18 {
        return Err("truncated aiff COMM chunk".to_string());
    }
    let channels = u16::from_be_bytes(payload[0..2].try_into().unwrap());
    let frames = u32::from_be_bytes(payload[2..6].try_into().unwrap());
    let bits = u16::from_be_bytes(payload[6..8].try_into().unwrap());
    let sample_rate = extended80_to_u32(&payload[8..18]);
    if channels == 0 {
        return Err("invalid aiff COMM: zero channels".to_string());
    }
    if !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(format!("unsupported aiff sample width: {bits} bits"));
    }
    let compression = if aifc {
        if payload.len() < 22 {
            return Err("truncated aifc COMM chunk".to_string());
        }
        String::from_utf8_lossy(&payload[18..22])
            .trim_end_matches('\0')
            .to_string()
    } else {
        "NONE".to_string()
    };
    Ok(CommChunk {
        channels,
        frames,
        bits,
        sample_rate,
        compression,
    })
}

/// Maps a COMM compression type plus bit depth onto a sample encoding.
fn resolve_sample_format(compression: &str, bits: u16) -> Result<SampleFormat, String> {
    match (compression, bits) {
        ("NONE", 8) => Ok(SampleFormat::Uint8),
        ("NONE", 16 | 24 | 32) => Ok(SampleFormat::IntBe(bits)),
        ("twos", 8 | 16 | 24 | 32) => Ok(SampleFormat::IntBe(bits)),
        ("sowt", 16 | 24 | 32) => Ok(SampleFormat::IntLe(bits)),
        ("FL32", 32) => Ok(SampleFormat::Float32Be),
        ("NONE" | "twos" | "sowt" | "FL32", _) => Err(format!(
            "unsupported aiff sample width: {compression} at {bits} bits"
        )),
        (name, _) => Err(format!("unsupported aiff compression: {name}")),
    }
}

/// Decodes an 80-bit IEEE extended float (the AIFF sample-rate encoding):
/// value = mantissa / 2^63 x 2^(exponent - 16383).
fn extended80_to_u32(bytes: &[u8]) -> u32 {
    let exponent = i16::from_be_bytes([bytes[0], bytes[1]]);
    let mantissa = u64::from_be_bytes(bytes[2..10].try_into().unwrap());
    if (exponent == 0 && mantissa == 0) || exponent == 0x7FFF {
        return 0;
    }
    let value = mantissa as f64 * 2f64.powi(i32::from(exponent) - 16383 - 63);
    value.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

/// Sign-extends a big-endian integer of 1-4 bytes into i32.
fn be_signed(width: usize, bytes: &[u8]) -> i32 {
    let mut value: u32 = 0;
    for &byte in &bytes[..width] {
        value = (value << 8) | u32::from(byte);
    }
    let shift = (32 - width * 8) as u32;
    ((value << shift) as i32) >> shift
}

/// Sign-extends a little-endian integer of 1-4 bytes into i32.
fn le_signed(width: usize, bytes: &[u8]) -> i32 {
    let mut value: u32 = 0;
    for &byte in bytes[..width].iter().rev() {
        value = (value << 8) | u32::from(byte);
    }
    let shift = (32 - width * 8) as u32;
    ((value << shift) as i32) >> shift
}

fn scale_for(bits: u16) -> f32 {
    match bits {
        8 => 128.0,
        16 => 32_768.0,
        24 => 8_388_608.0,
        _ => 2_147_483_648.0,
    }
}

fn decode_sample(bytes: &[u8], format: SampleFormat) -> f32 {
    match format {
        SampleFormat::Uint8 => (f32::from(bytes[0]) - 128.0) / 128.0,
        SampleFormat::IntBe(bits) => be_signed(bytes.len(), bytes) as f32 / scale_for(bits),
        SampleFormat::IntLe(bits) => le_signed(bytes.len(), bytes) as f32 / scale_for(bits),
        SampleFormat::Float32Be => f32::from_be_bytes(bytes.try_into().expect("32-bit float")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(name)
    }

    #[test]
    fn parses_twos_big_endian_mono() {
        let source = AiffSource::new(&fixture_path("tone.aiff")).expect("tone.aiff must parse");
        assert_eq!(source.channels(), 1);
        assert_eq!(source.sample_rate(), 22_050);
        assert!(source.total_duration().unwrap() >= Duration::from_millis(900));
        let wanted = (0.3 * f64::from(source.sample_rate())) as usize;
        let samples: Vec<f32> = source.take(wanted).collect();
        assert_eq!(samples.len(), wanted);
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    #[test]
    fn parses_sowt_little_endian_mono() {
        let source =
            AiffSource::new(&fixture_path("tone_sowt.aiff")).expect("sowt fixture must parse");
        assert_eq!(source.channels(), 1);
        assert_eq!(source.sample_rate(), 22_050);
        let wanted = (0.3 * f64::from(source.sample_rate())) as usize;
        assert_eq!(source.take(wanted).count(), wanted);
    }

    #[test]
    fn seek_lands_on_the_same_samples_as_sequential_read() {
        let mut sequential = AiffSource::new(&fixture_path("tone.aiff")).expect("parse");
        let skipped: Vec<f32> = sequential.by_ref().skip(11_025).take(48).collect();
        let mut seeked = AiffSource::new(&fixture_path("tone.aiff")).expect("parse");
        seeked
            .try_seek(Duration::from_secs_f64(0.5))
            .expect("seek must succeed");
        let window: Vec<f32> = seeked.take(48).collect();
        assert_eq!(skipped, window, "seeked samples must match sequential read");
    }

    #[test]
    fn truncated_header_is_rejected_cleanly() {
        let full = std::fs::read(fixture_path("tone.aiff")).expect("fixture readable");
        let victim = std::env::temp_dir().join("tui_explorer_aiff_truncation_probe.aiff");
        std::fs::write(&victim, &full[..20]).expect("write probe");
        let result = AiffSource::new(&victim);
        let _ = std::fs::remove_file(&victim);
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("truncated header must be rejected"),
        };
        assert!(error.contains("aiff"), "typed error expected, got: {error}");
    }

    #[test]
    fn wrong_magic_is_rejected_cleanly() {
        let victim = std::env::temp_dir().join("tui_explorer_aiff_bad_magic.aiff");
        std::fs::write(&victim, b"RIFFxxxxWAVEjunk").expect("write probe");
        let result = AiffSource::new(&victim);
        let _ = std::fs::remove_file(&victim);
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("wrong magic must be rejected"),
        };
        assert!(error.contains("FORM"), "got: {error}");
    }
}
