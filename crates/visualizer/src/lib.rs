//! Spectrum analysis for the visualizer.
//!
//! Decodes a track with `symphonia` (on the caller's thread — the app runs it
//! on a blocking worker thread so playback is never touched), then computes
//! log-spaced magnitude bins over a timeline with `rustfft`.
//!
//! The whole track is analyzed once into `Spectrum { fps, bins, frames }`;
//! the frontend maps the current playback position to a frame index, so seeks
//! and scrubbing cost nothing. Per design §4.2 this is the "sinkron via
//! timestamp" path (a live PCM tap does not exist in libmpv).

use std::path::Path;
use std::sync::Mutex;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Analysis defaults, matching design §4.2 (60–120 bins, 30 fps).
pub const DEFAULT_FPS: u32 = 30;
pub const DEFAULT_BINS: usize = 60;

/// FFT window length; ~46 ms at 44.1 kHz.
const FFT_SIZE: usize = 2048;
/// Lowest and highest analyzed frequency (Hz).
const F_MIN: f64 = 20.0;
const F_MAX: f64 = 20_000.0;
/// Stop analyzing after this many frames (≈ 30 min at 30 fps) so a marathon
/// of a track cannot balloon memory. The frontend clamps past the end.
const MAX_FRAMES: usize = 54_000;

#[derive(Debug, thiserror::Error)]
pub enum SpectrumError {
    #[error("unsupported or unreadable file")]
    Unsupported,
    #[error("no audio track found")]
    NoAudioTrack,
    #[error("no decodable samples")]
    NoSamples,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Full spectrum timeline for one track.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spectrum {
    /// Frames per second of `frames`.
    pub fps: u32,
    /// Magnitude bins per frame (log-spaced 20 Hz–20 kHz).
    pub bins: usize,
    /// `frames.len()` × `bins` magnitudes in `0..=1`, indexed `[frame][bin]`.
    pub frames: Vec<Vec<f32>>,
}

/// Frequency band edges (Hz), `bins + 1` values, log-spaced in `[f_min, f_max]`
/// with a geometric ratio. Band `b` covers `[edges[b], edges[b + 1])`.
pub fn band_edges(bins: usize, f_min: f64, f_max: f64) -> Vec<f64> {
    let ratio = (f_max / f_min).ln();
    (0..=bins)
        .map(|i| f_min * (ratio * i as f64 / bins as f64).exp())
        .collect()
}

/// Hann window of `FFT_SIZE` samples (coherent gain 0.5 at bin centers).
fn hann_window() -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / FFT_SIZE as f64).cos())
        .map(|v| v as f32)
        .collect()
}

/// Average FFT magnitude over each log band, normalized so a full-scale sine
/// reaches ~1.0; `0..=1` clamped.
fn frame_magnitudes(fft: &[Complex<f32>], edges: &[f64], sample_rate: usize) -> Vec<f32> {
    let bins = edges.len() - 1;
    let bin_hz = sample_rate as f64 / FFT_SIZE as f64;
    let norm = FFT_SIZE as f32 * 0.5;
    let mut out = vec![0f32; bins];
    for (b, pair) in edges.windows(2).enumerate() {
        // Skip DC (bin 0) and the mirrored upper half (bins > Nyquist).
        let lo = ((pair[0] / bin_hz).floor() as usize).max(1);
        let hi = ((pair[1] / bin_hz).ceil() as usize).min(FFT_SIZE / 2);
        let mut sum = 0f32;
        let mut n = 0usize;
        for c in &fft[lo..hi] {
            let mag = (c.re * c.re + c.im * c.im).sqrt();
            sum += mag;
            n += 1;
        }
        out[b] = if n > 0 {
            (sum / n as f32 / norm).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    out
}

/// Analyze raw mono samples into a spectrum timeline.
pub fn analyze_samples(samples: &[f32], sample_rate: u32, fps: u32, bins: usize) -> Spectrum {
    let sr = sample_rate as usize;
    let hop = (sr / fps.max(1) as usize).max(256);
    let nyquist = sr as f64 / 2.0;
    // Clamp the top band to Nyquist (e.g. 22.05 kHz files) but never below f_min.
    let edges = band_edges(bins, F_MIN, F_MAX.min(nyquist));

    let window = hann_window();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];

    let mut frames = Vec::new();
    let mut start = 0usize;
    while start + FFT_SIZE <= samples.len() && frames.len() < MAX_FRAMES {
        for (c, (s, w)) in buf
            .iter_mut()
            .zip(samples[start..start + FFT_SIZE].iter().zip(&window))
        {
            c.re = s * w;
            c.im = 0.0;
        }
        fft.process(&mut buf);
        frames.push(frame_magnitudes(&buf, &edges, sr));
        start += hop;
    }

    Spectrum { fps, bins, frames }
}

/// Decode `path` and return its full spectrum timeline.
pub fn analyze(path: &Path, fps: u32, bins: usize) -> Result<Spectrum, SpectrumError> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| SpectrumError::Unsupported)?;

    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or(SpectrumError::NoAudioTrack)?;
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.expect("checked above");
    let spec = SignalSpec {
        rate: sample_rate,
        channels: track.codec_params.channels.unwrap_or_default(),
    };

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| SpectrumError::Unsupported)?;
    let mut format = probed.format;
    let mut sample_buf = SampleBuffer::<f32>::new(FFT_SIZE as u64, spec);

    let mut mono: Vec<f32> = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        if let Ok(decoded) = decoder.decode(&packet) {
            let ch = decoded.spec().channels.count().max(1);
            sample_buf.copy_interleaved_ref(decoded);
            for frame in sample_buf.samples().chunks(ch) {
                mono.push(frame.iter().sum::<f32>() / frame.len() as f32);
            }
        }
    }

    if mono.is_empty() {
        return Err(SpectrumError::NoSamples);
    }
    Ok(analyze_samples(&mono, sample_rate, fps, bins))
}

/// Tiny single-entry cache (path → spectrum) so re-opening the same track's
/// visualizer is instant. Re-analyzing replaces the entry.
#[derive(Default)]
pub struct SpectrumCache {
    inner: Mutex<Option<(std::path::PathBuf, Spectrum)>>,
}

impl SpectrumCache {
    pub fn get(&self, path: &Path) -> Option<Spectrum> {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|(_, s)| s.clone())
    }

    pub fn put(&self, path: &Path, spectrum: Spectrum) {
        let mut guard = self.inner.lock().unwrap();
        *guard = Some((path.to_path_buf(), spectrum));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 1-second mono 440 Hz sine at full scale (float −1..1).
    fn sine(secs: usize, freq: f64, sr: u32) -> Vec<f32> {
        (0..secs * sr as usize)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sr as f64).sin() as f32)
            .collect()
    }

    /// Average interleaved channel samples into mono (per frame).
    fn to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
        interleaved
            .chunks(channels.max(1))
            .map(|ch| ch.iter().sum::<f32>() / ch.len() as f32)
            .collect()
    }

    /// Sum each band's magnitude across all frames, return the band whose
    /// center frequency is most energized.
    fn dominant_band(spectrum: &Spectrum, sr: u32) -> (usize, f64) {
        let mut totals = vec![0f32; spectrum.bins];
        for frame in &spectrum.frames {
            for (i, v) in frame.iter().enumerate() {
                totals[i] += v;
            }
        }
        let b = totals
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, c)| a.total_cmp(c))
            .map(|(i, _)| i)
            .unwrap();
        let edges = band_edges(spectrum.bins, F_MIN, F_MAX.min(sr as f64 / 2.0));
        let center = (edges[b] * edges[b + 1]).sqrt();
        (b, center)
    }

    // ---- pure: band geometry ----

    #[test]
    fn band_edges_are_log_spaced_and_clamped_to_nyquist() {
        let edges = band_edges(60, F_MIN, F_MAX);
        assert_eq!(edges.len(), 61);
        assert!((edges[0] - 20.0).abs() < 1.0);
        assert!((edges[60] - 20_000.0).abs() < 1.0);
        for w in edges.windows(2) {
            assert!(w[0] < w[1]);
        }
        // Geometric ratio is constant.
        let r1 = edges[10] / edges[9];
        let r2 = edges[40] / edges[39];
        assert!((r1 - r2).abs() < 1e-9);
        // Nyquist clamp: 22.05 kHz file tops out at 11025 Hz.
        let clamped = band_edges(60, F_MIN, F_MAX.min(11_025.0));
        assert!((clamped[60] - 11_025.0).abs() < 1.0);
    }

    // ---- pure: mono downmix ----

    #[test]
    fn stereo_interleave_averages_to_mono() {
        assert_eq!(
            to_mono(&[1.0, 3.0, 1.0, 3.0, 5.0, 7.0], 2),
            vec![2.0, 2.0, 6.0]
        );
        // Partial trailing frame still averages what exists.
        assert_eq!(to_mono(&[1.0, 3.0, 1.0], 2), vec![2.0, 1.0]);
        // Mono passthrough.
        assert_eq!(to_mono(&[0.25, -0.5], 1), vec![0.25, -0.5]);
    }

    // ---- pure: FFT timeline ----

    #[test]
    fn sine_peaks_in_its_frequency_band() {
        let sr = 44_100u32;
        let spectrum = analyze_samples(&sine(1, 440.0, sr), sr, DEFAULT_FPS, DEFAULT_BINS);
        assert_eq!(spectrum.bins, DEFAULT_BINS);
        assert_eq!(spectrum.fps, DEFAULT_FPS);
        assert!(!spectrum.frames.is_empty());
        let (_band, center) = dominant_band(&spectrum, sr);
        assert!(
            (350.0..550.0).contains(&center),
            "440 Hz sine peaked at {center:.0} Hz"
        );
    }

    #[test]
    fn silence_stays_near_zero() {
        let sr = 44_100u32;
        let spectrum = analyze_samples(&vec![0.0; sr as usize], sr, DEFAULT_FPS, DEFAULT_BINS);
        for frame in &spectrum.frames {
            for v in frame {
                assert!(*v < 1e-3, "silence produced magnitude {v}");
            }
        }
    }

    #[test]
    fn frame_rate_matches_fps_over_duration() {
        let sr = 44_100u32;
        let spectrum = analyze_samples(&sine(2, 440.0, sr), sr, DEFAULT_FPS, DEFAULT_BINS);
        // ~2 s × 30 fps = ~60 windows, minus the last partial window (±2).
        assert!(
            (57..=61).contains(&spectrum.frames.len()),
            "got {}",
            spectrum.frames.len()
        );
    }

    // ---- WAV fixture end-to-end ----

    fn write_wav_mono_16(path: &Path, samples: &[i16], sample_rate: u32) {
        let data_len = (samples.len() * 2) as u32;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            w.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, w).unwrap();
    }

    fn fixture_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iwaks-viz-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wav_file_analyzes_to_expected_timeline() {
        let sr = 44_100u32;
        let samples: Vec<i16> = sine(1, 440.0, sr)
            .into_iter()
            .map(|v| (v * 32767.0) as i16)
            .collect();

        let dir = fixture_dir("wav");
        let path = dir.join("tone.wav");
        write_wav_mono_16(&path, &samples, sr);

        let spectrum = analyze(&path, DEFAULT_FPS, DEFAULT_BINS).expect("analysis ok");
        assert_eq!(spectrum.bins, DEFAULT_BINS);
        assert_eq!(spectrum.fps, DEFAULT_FPS);
        assert!(
            (28..=31).contains(&spectrum.frames.len()),
            "got {}",
            spectrum.frames.len()
        );
        let (_band, center) = dominant_band(&spectrum, sr);
        assert!((350.0..550.0).contains(&center), "peaked at {center:.0} Hz");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn garbage_file_errors() {
        let dir = fixture_dir("garbage");
        let path = dir.join("tone.wav");
        std::fs::write(&path, b"this is definitely not audio").unwrap();
        let err = analyze(&path, DEFAULT_FPS, DEFAULT_BINS).unwrap_err();
        assert!(matches!(err, SpectrumError::Unsupported), "got {err:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- cache ----

    #[test]
    fn cache_returns_only_latest_path() {
        let cache = SpectrumCache::default();
        let a = PathBuf::from("a.flac");
        let b = PathBuf::from("b.flac");
        assert_eq!(cache.get(&a), None);

        let sa = Spectrum {
            fps: 30,
            bins: 60,
            frames: vec![vec![0.5; 60]],
        };
        cache.put(&a, sa.clone());
        assert_eq!(cache.get(&a), Some(sa));

        let sb = Spectrum {
            fps: 30,
            bins: 60,
            frames: vec![],
        };
        cache.put(&b, sb.clone());
        assert_eq!(cache.get(&a), None, "old path evicted");
        assert_eq!(cache.get(&b), Some(sb));
    }
}
