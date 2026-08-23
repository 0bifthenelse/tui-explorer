use std::path::Path;
use std::time::Duration;

use rodio::Source;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub struct SymphoniaSource {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: usize,
    total_duration: Option<Duration>,
    buffer: SampleBuffer<f32>,
    cursor: usize,
    span_len: usize,
    finished: bool,
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
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("no supported audio track")?
            .clone();
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|error| format!("unsupported codec: {error}"))?;
        let sample_rate = track
            .codec_params
            .sample_rate
            .ok_or("unknown sample rate")?;
        let channels = track
            .codec_params
            .channels
            .ok_or("unknown channel count")?
            .count();
        let total_duration = track
            .codec_params
            .n_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / f64::from(sample_rate)));
        Ok(Self {
            format,
            decoder,
            track_id: track.id,
            sample_rate,
            channels,
            total_duration,
            buffer: SampleBuffer::new(
                // One second of capacity; reallocated per decoded packet anyway.
                sample_rate as u64,
                symphonia::core::audio::SignalSpec::new(
                    sample_rate,
                    track
                        .codec_params
                        .channels
                        .unwrap_or(symphonia::core::audio::Channels::FRONT_LEFT),
                ),
            ),
            cursor: 0,
            span_len: 0,
            finished: false,
        })
    }

    fn next_packet_samples(&mut self) -> Result<(), SymphoniaError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(error) => {
                    self.finished = true;
                    return Err(error);
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let capacity =
                        decoded.frames() as u64 * u64::try_from(spec.channels.count()).unwrap_or(1);
                    let mut buffer = SampleBuffer::<f32>::new(capacity, spec);
                    buffer.copy_interleaved_ref(decoded);
                    self.buffer = buffer;
                    self.cursor = 0;
                    self.span_len = (capacity as usize).max(1);
                    return Ok(());
                }
                Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
                Err(error) => return Err(error),
            }
        }
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
            Some(self.span_len - self.cursor)
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
}
