//! iwaks-smtc: expose the Iwaks player as a Windows **System Media Transport
//! Controls** (SMTC) session.
//!
//! Why: tools like [Music Presence](https://github.com/ungive/discord-music-presence)
//! (Discord status) and the Windows 10/11 media flyout discover players
//! through SMTC — and libmpv never registers one, so Iwaks wouldn't appear
//! anywhere. This crate publishes a tiny WinRT `MediaPlayer` (silent source,
//! volume 0, never actually played) purely as a metadata + timeline +
//! playback-status + album-art surface for the OS. Once Music Presence is
//! installed it picks "Iwaks" up automatically, no per-player configuration
//! needed; [`launch`] also locates and starts that installation alongside
//! the app.
//!
//! Threading: **every WinRT call stays on one dedicated thread** (the session
//! thread). The rest of the app only pushes [`NowPlaying`] snapshots over a
//! channel (the player already emits ~10 Hz while playing), hands the raw
//! embedded cover bytes over the same channel whenever the track changes, and
//! registers a callback for media-key / flyout button presses. That keeps the
//! handle `Send + Sync` so Tauri can store it in managed state.
//!
//! On non-Windows targets [`SmtcSession::start`] returns `None` and the other
//! methods are no-ops.

mod win;

pub mod launch;

pub use win::SmtcSession;

/// Playback information pushed to the SMTC session on every player-state
/// tick. Built by the app from `iwaks_player::PlayerState`.
#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Seconds into the track (mpv `playback-time`; clamped upstream).
    pub position_secs: f64,
    /// Track length in seconds; 0 when unknown.
    pub duration_secs: f64,
    /// Playing (not paused, not stopped).
    pub playing: bool,
    /// Playback ended naturally (repeat off, nothing loaded).
    pub stopped: bool,
}

/// A media key / flyout button the user pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtcButton {
    Play,
    Pause,
    Next,
    Previous,
    Stop,
}

/// The three states the SMTC surface accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayStatus {
    Playing,
    Paused,
    Stopped,
}

/// Fallback title shown when a track has no usable metadata.
pub const APP_NAME: &str = "Iwaks";

impl NowPlaying {
    /// Map paused/stopped flags to the SMTC playback status.
    pub fn status(&self) -> PlayStatus {
        if self.stopped {
            PlayStatus::Stopped
        } else if self.playing {
            PlayStatus::Playing
        } else {
            PlayStatus::Paused
        }
    }

    /// (title, artist, album) with every missing piece filled in: SMTC
    /// metadata rejects empty strings, and a blank title falls back to the
    /// app name so the presence still reads "Listening to Iwaks".
    pub fn metadata(&self) -> (String, String, String) {
        let title = if self.title.trim().is_empty() {
            APP_NAME.to_string()
        } else {
            self.title.clone()
        };
        (
            title,
            self.artist.clone().unwrap_or_default(),
            self.album.clone().unwrap_or_default(),
        )
    }
}

/// Convert seconds to the 100-nanosecond `TimeSpan` unit SMTC timelines use.
/// Negative / NaN / infinite values clamp to zero (mpv can report -1 for an
/// unknown position).
pub fn secs_to_100ns(secs: f64) -> i64 {
    let clamped = if secs.is_finite() && secs > 0.0 {
        secs
    } else {
        0.0
    };
    (clamped * 10_000_000.0).round() as i64
}

/// A tiny PCM silence WAV handed to the `MediaPlayer` as its source. The
/// player never produces sound (volume 0, `Play` is never called) but an
/// attached source keeps the session registered with the OS from the start.
pub fn silent_wav(seconds: u32) -> Vec<u8> {
    const RATE: u32 = 8_000;
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    let data_len = RATE * seconds * (CHANNELS as u32) * (BITS as u32 / 8);
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    let byte_rate = RATE * (CHANNELS as u32) * (BITS as u32 / 8);
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&(CHANNELS * BITS / 8).to_le_bytes());
    wav.extend_from_slice(&BITS.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0); // silence
    wav
}

/// Map raw cover bytes to the extension of the thumbnail temp file. Sniffs
/// magic bytes (JPEG/PNG/GIF/WebP); anything else falls back to `bin` — WIC
/// and most decoders sniff content, so the extension is mostly cosmetic and
/// only needs to stay stable across covers.
pub(crate) fn cover_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "bin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_maps_playing_paused_stopped() {
        let base = NowPlaying {
            title: "Song".into(),
            artist: None,
            album: None,
            position_secs: 12.0,
            duration_secs: 200.0,
            playing: true,
            stopped: false,
        };
        assert_eq!(base.status(), PlayStatus::Playing);
        let paused = NowPlaying {
            playing: false,
            ..base.clone()
        };
        assert_eq!(paused.status(), PlayStatus::Paused);
        let stopped = NowPlaying {
            playing: false,
            stopped: true,
            ..base
        };
        assert_eq!(stopped.status(), PlayStatus::Stopped);
    }

    #[test]
    fn metadata_fills_blanks_and_falls_back_to_app_name() {
        let full = NowPlaying {
            title: "Happier Than Ever".into(),
            artist: Some("Billie Eilish".into()),
            album: Some("Happier Than Ever".into()),
            playing: true,
            stopped: false,
            position_secs: 0.0,
            duration_secs: 0.0,
        };
        assert_eq!(
            full.metadata(),
            (
                "Happier Than Ever".into(),
                "Billie Eilish".into(),
                "Happier Than Ever".into()
            )
        );

        let blank = NowPlaying {
            title: "   ".into(),
            artist: None,
            album: None,
            playing: true,
            stopped: false,
            position_secs: 0.0,
            duration_secs: 0.0,
        };
        assert_eq!(
            blank.metadata(),
            (APP_NAME.to_string(), String::new(), String::new())
        );
    }

    #[test]
    fn secs_to_100ns_rounds_and_clamps() {
        assert_eq!(secs_to_100ns(0.0), 0);
        assert_eq!(secs_to_100ns(1.0), 10_000_000);
        assert_eq!(secs_to_100ns(2.5), 25_000_000);
        assert_eq!(secs_to_100ns(0.1), 1_000_000);
        assert_eq!(secs_to_100ns(-3.0), 0);
        assert_eq!(secs_to_100ns(f64::NAN), 0);
        assert_eq!(secs_to_100ns(f64::INFINITY), 0);
    }

    #[test]
    fn silent_wav_is_valid_pcm() {
        let wav = silent_wav(2);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[20..22], &1u16.to_le_bytes()); // PCM
        assert_eq!(&wav[36..40], b"data");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        assert_eq!(data_len, 8_000 * 2 * 2); // 8 kHz, mono, 16-bit, 2 s
                                             // Audio payload is pure silence.
        assert!(wav[44..].iter().all(|b| *b == 0));
    }

    #[test]
    fn cover_extension_sniffs_magic_bytes() {
        assert_eq!(cover_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), "jpg");
        assert_eq!(cover_extension(b"\x89PNG\r\n\x1a\n"), "png");
        assert_eq!(cover_extension(b"GIF89a"), "gif");
        assert_eq!(cover_extension(b"RIFF....WEBPVP8 "), "webp");
        assert_eq!(cover_extension(b"RIFF"), "bin"); // truncated webp header
        assert_eq!(cover_extension(b"not an image"), "bin");
        assert_eq!(cover_extension(&[]), "bin");
    }
}
