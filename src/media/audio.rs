use std::path::Path;
use std::time::Duration;

use rodio::Source;
use symphonia::core::audio::{Channels, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Consecutive per-packet decode failures tolerated before the stream is
/// declared fatally broken instead of spinning silently to instant EOF.
const MAX_CONSECUTIVE_DECODE_ERRORS: usize = 64;

pub struct SymphoniaSource {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: usize,
    total_duration: Option<Duration>,
    buffer: SampleBuffer<f32>,
    cursor: usize,
    span_len: usize,
    finished: bool,
    consecutive_decode_errors: usize,
}

impl SymphoniaSource {
    pub fn new(path: &Path) -> Result<Self, String> {
        let source = std::fs::File::open(path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        let mut hint = Hint::new();
        if let Some(extension) = path.extension() {
            hint.with_extension(&extension.to_string_lossy());
        }
        let mss = MediaSourceStream::new(Box::new(source), Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|error| format!("unsupported audio format: {error}"))?;
        let mut format = probed.format;

        // Track selection: the first track with a declared codec is not
        // necessarily decodable (containers may list bogus or unsupported
        // entries first). Prefer the first track whose decoder actually builds.
        let mut chosen = None;
        for candidate in format.tracks() {
            if candidate.codec_params.codec == CODEC_TYPE_NULL {
                continue;
            }
            if let Ok(decoder) = symphonia::default::get_codecs()
                .make(&candidate.codec_params, &DecoderOptions::default())
            {
                chosen = Some((candidate.clone(), decoder));
                break;
            }
        }
        let Some((track, mut decoder)) = chosen else {
            return Err("no supported audio track".to_string());
        };

        // Containers such as mp4 leave the signal spec to codec-specific
        // config (AudioSpecificConfig / ALAC magic cookie), so codec_params
        // reports neither channels nor sample rate for AAC-LC and ALAC.
        // Peek-decode the first packet to discover the real spec instead of
        // failing construction; the decoded samples become the pending buffer
        // so none of them are lost.
        let mut sample_rate = track.codec_params.sample_rate;
        let mut channels = track.codec_params.channels.map(|ch| ch.count());
        let mut pending: Option<(SampleBuffer<f32>, usize, usize)> = None;
        if channels.is_none() || sample_rate.is_none() {
            let mut consecutive = 0usize;
            loop {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(error) => return Err(format!("no decodable audio packet: {error}")),
                };
                if packet.track_id() != track.id {
                    continue;
                }
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        channels.get_or_insert_with(|| spec.channels.count());
                        sample_rate.get_or_insert(spec.rate);
                        let frames = decoded.frames();
                        if frames == 0 {
                            continue; // spec learned; keep hunting for samples
                        }
                        let mut buffer = SampleBuffer::<f32>::new(frames as u64, spec);
                        buffer.copy_interleaved_ref(decoded);
                        let discovered = channels.unwrap_or(1);
                        let span_len = frames.saturating_mul(discovered);
                        // Honor priming/gapless trims announced by the demuxer.
                        let trim = packet.trim_start() as usize;
                        let cursor = trim.saturating_mul(discovered).min(span_len);
                        pending = Some((buffer, span_len, cursor));
                        break;
                    }
                    Err(SymphoniaError::DecodeError(msg)) => {
                        consecutive += 1;
                        if consecutive > MAX_CONSECUTIVE_DECODE_ERRORS {
                            return Err(format!("decode failure: {msg}"));
                        }
                    }
                    Err(error) => {
                        return Err(format!("codec failed on first packet: {error}"));
                    }
                }
            }
        }

        let sample_rate = sample_rate.ok_or("unknown sample rate")?;
        let channels = channels.ok_or("unknown channel count")?;
        let total_duration = track
            .codec_params
            .n_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / f64::from(sample_rate)));
        let mut source = Self {
            format,
            decoder,
            track_id: track.id,
            sample_rate,
            channels,
            total_duration,
            buffer: SampleBuffer::new(
                // One second of placeholder capacity; replaced per packet.
                sample_rate as u64,
                SignalSpec::new(
                    sample_rate,
                    track.codec_params.channels.unwrap_or(Channels::FRONT_LEFT),
                ),
            ),
            cursor: 0,
            span_len: 0,
            finished: false,
            consecutive_decode_errors: 0,
        };
        if let Some((buffer, span_len, cursor)) = pending {
            source.buffer = buffer;
            source.span_len = span_len;
            source.cursor = cursor;
        }
        Ok(source)
    }

    /// Pulls the next packet of the selected track into the sample buffer.
    /// Clean EOF and fatal failures both terminate iteration; the returned
    /// message distinguishes them for diagnostics.
    fn next_packet_samples(&mut self) -> Result<(), String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(io_error)) => {
                    self.finished = true;
                    if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                        return Err("end of stream".to_string());
                    }
                    return Err(format!("io error: {io_error}"));
                }
                Err(error) => {
                    self.finished = true;
                    return Err(format!("demux error: {error}"));
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    self.consecutive_decode_errors = 0;
                    let frames = decoded.frames();
                    if frames == 0 {
                        // Zero-frame packets carry no samples; skipping them
                        // avoids indexing an empty buffer downstream.
                        continue;
                    }
                    let spec = *decoded.spec();
                    let channel_count = spec.channels.count();
                    let mut buffer = SampleBuffer::<f32>::new(frames as u64, spec);
                    buffer.copy_interleaved_ref(decoded);
                    self.buffer = buffer;
                    self.span_len = frames.saturating_mul(channel_count);
                    // Honor priming/gapless trims announced by the demuxer.
                    let trim = packet.trim_start() as usize;
                    self.cursor = trim.saturating_mul(channel_count).min(self.span_len);
                    return Ok(());
                }
                Err(SymphoniaError::DecodeError(msg)) => {
                    self.consecutive_decode_errors += 1;
                    if self.consecutive_decode_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                        self.finished = true;
                        return Err(format!("decode failure: {msg}"));
                    }
                }
                Err(SymphoniaError::IoError(io_error)) => {
                    self.finished = true;
                    return Err(format!("io error: {io_error}"));
                }
                Err(error) => {
                    self.finished = true;
                    return Err(format!("decode error: {error}"));
                }
            }
        }
    }

    /// Total decoded duration when the container reports frame counts.
    /// Captured once by the media supervisor; the sink remains the sole
    /// playback-position authority.
    pub fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.finished {
            return None;
        }
        while self.cursor >= self.span_len {
            if self.next_packet_samples().is_err() || self.finished {
                self.finished = true;
                return None;
            }
        }
        let sample = self.buffer.samples()[self.cursor];
        self.cursor += 1;
        Some(sample)
    }
}

impl Source for SymphoniaSource {
    fn current_span_len(&self) -> Option<usize> {
        if self.finished {
            None
        } else {
            Some(self.span_len.saturating_sub(self.cursor))
        }
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
        let time = Time::from(position);
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|_| rodio::source::SeekError::NotSupported {
                underlying_source: std::any::type_name::<Self>(),
            })?;
        self.decoder.reset();
        self.cursor = 0;
        self.span_len = 0;
        self.finished = false;
        self.consecutive_decode_errors = 0;
        Ok(())
    }
}

/// Number of published logarithmic spectrum bands.
pub const SPECTRUM_BANDS: usize = 24;
const FFT_SIZE: usize = 2048;
const HOP_SIZE: usize = 1024;
const DB_FLOOR: f32 = -72.0;
const BAND_MIN_HZ: f32 = 20.0;
const ATTACK: f32 = 0.65;
const RELEASE: f32 = 0.18;

/// Wraps any f32 audio source, analyzes the mono mix of exactly the samples
/// handed to rodio through one preplanned FFT, and publishes band magnitudes
/// plus playback position through atomics. The pull path never blocks or
/// allocates after construction.
pub struct SpectrumSource<S> {
    inner: S,
    channels: usize,
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    scratch: Vec<rustfft::num_complex::Complex<f32>>,
    pending: Vec<f32>,
    band_edges: [f32; SPECTRUM_BANDS + 1],
    smoothed: [f32; SPECTRUM_BANDS],
    sample_rate: u32,
    position_samples: u64,
    bands_out: std::sync::Arc<SpectrumSnapshot>,
}

/// Atomic publication of the latest analysis frame.
#[derive(Default)]
pub struct SpectrumSnapshot {
    sequence: std::sync::atomic::AtomicU64,
    position_ms: std::sync::atomic::AtomicU64,
    values: [std::sync::atomic::AtomicU32; SPECTRUM_BANDS],
}

impl SpectrumSnapshot {
    /// Reads one coherent snapshot (sequence-consistent set of band values).
    pub fn read(&self) -> ([f32; SPECTRUM_BANDS], u64) {
        loop {
            let before = self.sequence.load(std::sync::atomic::Ordering::Acquire);
            let mut out = [0.0f32; SPECTRUM_BANDS];
            for (slot, value) in self.values.iter().enumerate() {
                out[slot] = f32::from_bits(value.load(std::sync::atomic::Ordering::Relaxed));
            }
            let position = self.position_ms.load(std::sync::atomic::Ordering::Relaxed);
            let after = self.sequence.load(std::sync::atomic::Ordering::Acquire);
            if before == after && after.is_multiple_of(2) {
                return (out, position);
            }
        }
    }

    fn write(&self, bands: &[f32; SPECTRUM_BANDS], position_ms: u64) {
        self.sequence
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        for (slot, value) in self.values.iter().enumerate() {
            value.store(bands[slot].to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
        self.position_ms
            .store(position_ms, std::sync::atomic::Ordering::Relaxed);
        // Odd = mid-write, even = stable; Release pairs with readers' Acquire.
        self.sequence
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }
}

impl<S> SpectrumSource<S>
where
    S: rodio::Source<Item = f32>,
{
    pub fn new(inner: S) -> (Self, std::sync::Arc<SpectrumSnapshot>) {
        let sample_rate = inner.sample_rate();
        let channels = usize::from(inner.channels());
        let snapshot = std::sync::Arc::new(SpectrumSnapshot::default());
        let mut planner = rustfft::FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|n| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();
        let nyquist = sample_rate as f32 / 2.0;
        let log_min = BAND_MIN_HZ.ln();
        let log_max = nyquist.max(BAND_MIN_HZ + 1.0).ln();
        let mut band_edges = [0.0f32; SPECTRUM_BANDS + 1];
        for (index, edge) in band_edges.iter_mut().enumerate() {
            let ratio = index as f32 / SPECTRUM_BANDS as f32;
            *edge = (log_min + ratio * (log_max - log_min)).exp().min(nyquist);
        }
        (
            SpectrumSource {
                inner,
                channels,
                fft,
                window,
                scratch: Vec::with_capacity(FFT_SIZE),
                pending: Vec::with_capacity(FFT_SIZE * 2),
                band_edges,
                smoothed: [0.0; SPECTRUM_BANDS],
                sample_rate,
                position_samples: 0,
                bands_out: snapshot.clone(),
            },
            snapshot,
        )
    }

    fn analyze_window(&mut self) {
        self.scratch.clear();
        for (index, sample) in self.pending.iter().take(FFT_SIZE).enumerate() {
            self.scratch.push(rustfft::num_complex::Complex::new(
                sample * self.window[index],
                0.0,
            ));
        }
        self.fft.process(&mut self.scratch);
        let window_sum: f32 = self.window.iter().sum();
        let bin_hz = self.sample_rate as f32 / FFT_SIZE as f32;
        let mut band_power = [0.0f32; SPECTRUM_BANDS];
        let mut band_count = [0u32; SPECTRUM_BANDS];
        for (bin, value) in self.scratch.iter().enumerate().skip(1).take(FFT_SIZE / 2) {
            let magnitude = (value.re * value.re + value.im * value.im).sqrt() * (2.0 / window_sum);
            let hz = bin as f32 * bin_hz;
            if hz < BAND_MIN_HZ {
                continue;
            }
            for (band, edge) in self.band_edges.iter().enumerate().take(SPECTRUM_BANDS) {
                if hz >= *edge && hz < self.band_edges[band + 1] {
                    band_power[band] += magnitude * magnitude;
                    band_count[band] += 1;
                    break;
                }
            }
        }
        let mut bands = [0.0f32; SPECTRUM_BANDS];
        for index in 0..SPECTRUM_BANDS {
            let rms = if band_count[index] > 0 {
                (band_power[index] / band_count[index] as f32).sqrt()
            } else {
                0.0
            };
            let db = 20.0 * rms.max(f32::EPSILON).log10();
            let normalized = ((db - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0);
            let target = normalized;
            self.smoothed[index] = if target > self.smoothed[index] {
                self.smoothed[index] + ATTACK * (target - self.smoothed[index])
            } else {
                self.smoothed[index] + RELEASE * (target - self.smoothed[index])
            };
            bands[index] = self.smoothed[index];
        }
        let position_ms =
            (self.position_samples as f64 / f64::from(self.sample_rate) * 1000.0) as u64;
        self.bands_out.write(&bands, position_ms);
    }
}

impl<S> Iterator for SpectrumSource<S>
where
    S: rodio::Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        // Average complete channel frames to mono for analysis.
        if self.channels == 0 {
            return Some(sample);
        }
        // The inner source yields interleaved samples; we buffer everything
        // and analyze once per hop on the mono mixdown.
        self.pending.push(sample);
        if self.pending.len() == FFT_SIZE {
            self.analyze_window();
        }
        self.position_samples += 1;
        if self.pending.len() >= FFT_SIZE + HOP_SIZE {
            self.pending.drain(..HOP_SIZE);
        }
        Some(sample)
    }
}

impl<S> rodio::Source for SpectrumSource<S>
where
    S: rodio::Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.pending.clear();
        self.smoothed = [0.0; SPECTRUM_BANDS];
        self.position_samples = position
            .as_secs_f64()
            .mul_add(f64::from(self.sample_rate), 0.0) as u64;
        self.inner.try_seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic sine generator standing in for a decoded file.
    struct SineSource {
        frequency: f32,
        sample_rate: u32,
        remaining: usize,
        phase: f32,
    }

    impl Iterator for SineSource {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            if self.remaining == 0 {
                return None;
            }
            self.remaining -= 1;
            let value = (self.phase * 2.0 * std::f32::consts::PI).sin();
            self.phase += self.frequency / self.sample_rate as f32;
            Some(value)
        }
    }

    impl rodio::Source for SineSource {
        fn current_span_len(&self) -> Option<usize> {
            Some(self.remaining)
        }

        fn channels(&self) -> rodio::ChannelCount {
            2
        }

        fn sample_rate(&self) -> rodio::SampleRate {
            self.sample_rate
        }

        fn total_duration(&self) -> Option<Duration> {
            Some(Duration::from_secs_f64(
                self.remaining as f64 / f64::from(self.sample_rate),
            ))
        }
    }

    #[test]
    fn sine_energy_lands_in_expected_log_band() {
        let sample_rate = 48_000u32;
        let source = SineSource {
            frequency: 440.0,
            sample_rate,
            remaining: FFT_SIZE * 8,
            phase: 0.0,
        };
        let (spectrum, snapshot) = SpectrumSource::new(source);
        for _ in spectrum.take(FFT_SIZE * 6) {}
        let (bands, _position) = snapshot.read();
        let peak = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .expect("peak band");
        assert!(
            (7..=10).contains(&peak),
            "440 Hz should land in mid-low log bands, got {peak} with {bands:?}"
        );
        assert!(bands[peak] > 0.05, "peak must carry real energy");
    }

    struct ZeroSource {
        remaining: usize,
    }

    impl Iterator for ZeroSource {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            if self.remaining == 0 {
                return None;
            }
            self.remaining -= 1;
            Some(0.0)
        }
    }

    impl rodio::Source for ZeroSource {
        fn current_span_len(&self) -> Option<usize> {
            Some(self.remaining)
        }

        fn channels(&self) -> rodio::ChannelCount {
            1
        }

        fn sample_rate(&self) -> rodio::SampleRate {
            48_000
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn silence_publishes_zero_bands() {
        let source = ZeroSource {
            remaining: FFT_SIZE * 4,
        };
        let (spectrum, snapshot) = SpectrumSource::new(source);
        for _ in spectrum.take(FFT_SIZE * 3) {}
        let (bands, _) = snapshot.read();
        assert!(
            bands.iter().all(|value| *value < 0.01),
            "silence must stay near zero, got {bands:?}"
        );
    }

    #[test]
    fn wrapper_returns_inner_samples_verbatim() {
        let source = SineSource {
            frequency: 100.0,
            sample_rate: 48_000,
            remaining: 16,
            phase: 0.0,
        };
        let (mut spectrum, _snapshot) = SpectrumSource::new(source);
        let collected: Vec<f32> = spectrum.by_ref().take(16).collect();
        assert_eq!(collected.len(), 16);
    }

    #[test]
    fn seek_delegates_and_clears_analysis_state() {
        let source = SineSource {
            frequency: 220.0,
            sample_rate: 48_000,
            remaining: FFT_SIZE * 4,
            phase: 0.0,
        };
        let (mut spectrum, _snapshot) = SpectrumSource::new(source);
        for _ in spectrum.by_ref().take(HOP_SIZE * 3) {}
        assert!(!spectrum.pending.is_empty());
        // The wrapper must clear buffered analysis state on any seek, then
        // delegate to the inner source (which rejects unknown seeks here).
        assert!(spectrum.try_seek(Duration::ZERO).is_err());
    }
    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(name)
    }

    #[test]
    fn m4a_aac_constructs_with_discovered_channels() {
        let mut source = SymphoniaSource::new(&fixture_path("tone_aac.m4a"))
            .expect("aac m4a must construct via peek-decoded channel discovery");
        assert!(
            source.channels() >= 1,
            "channels come from the decoded spec"
        );
        assert_eq!(source.sample_rate(), 22_050);
        assert_eq!(source.by_ref().take(512).count(), 512);
    }

    #[test]
    fn m4a_alac_constructs_with_discovered_channels() {
        let mut source = SymphoniaSource::new(&fixture_path("tone_alac.m4a"))
            .expect("alac m4a must construct via peek-decoded channel discovery");
        assert!(
            source.channels() >= 1,
            "channels come from the decoded spec"
        );
        assert_eq!(source.sample_rate(), 22_050);
        assert_eq!(source.by_ref().take(512).count(), 512);
    }

    #[test]
    fn ogg_zero_frame_packets_iterate_without_panic() {
        let source = SymphoniaSource::new(&fixture_path("tone.ogg")).expect("ogg must open");
        let count = source.count();
        assert!(
            count > 20_000,
            "expected ~1 s of 22.05 kHz audio, got {count}"
        );
    }

    #[test]
    fn truncated_streams_terminate_without_panicking() {
        for name in ["tone_aac_trunc.m4a", "tone_trunc.ogg"] {
            let Ok(source) = SymphoniaSource::new(&fixture_path(name)) else {
                continue; // rejection at open is a valid bounded outcome
            };
            let count = source.take(50_000_000).count();
            assert!(count <= 50_000_000, "{name} iteration must terminate");
        }
    }

    #[test]
    fn seek_keeps_stream_decodable() {
        let mut source = SymphoniaSource::new(&fixture_path("wav_pcm.wav")).expect("wav must open");
        assert_eq!(source.by_ref().take(128).count(), 128);
        // Success or NotSupported are both acceptable; corruption after the
        // seek attempt is not.
        let _ = source.try_seek(Duration::from_millis(400));
        assert!(
            source.take(128).count() > 0,
            "stream must decode after seek"
        );
    }
}
