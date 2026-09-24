//! Read audio file metadata via `lofty`.

use std::path::Path;

use iwaks_core::track::AudioMetadata;
use lofty::prelude::*;

use crate::TagError;

/// Read metadata from an audio file (tags + stream properties).
/// Falls back gracefully: absent tags yield `None` fields; only genuinely
/// unreadable/unsupported files return `Err`.
pub fn read_metadata(path: &Path) -> Result<AudioMetadata, TagError> {
    let tagged = lofty::read_from_path(path)?;
    let props = tagged.properties();

    let mut meta = AudioMetadata::new(stream_format(path));
    meta.duration_ms = props.duration().as_millis() as i64;
    meta.sample_rate = props.sample_rate().map(i64::from);
    meta.bit_depth = props.bit_depth().map(i64::from);
    meta.bitrate = props.overall_bitrate().map(i64::from);

    if let Some(tag) = tagged.primary_tag() {
        meta.title = tag.title().map(|v| v.into_owned());
        meta.artist = tag.artist().map(|v| v.into_owned());
        meta.album = tag.album().map(|v| v.into_owned());
        meta.album_artist = tag.get_string(&ItemKey::AlbumArtist).map(str::to_string);
        meta.genre = tag.genre().map(|v| v.into_owned());
        meta.year = tag.year().map(i64::from);
        meta.track_no = tag.track().map(i64::from);
        meta.disc_no = tag.disk().map(i64::from);
    }
    Ok(meta)
}

/// Canonical short format name from the file extension, e.g. "wav", "flac",
/// "mp3". The scanner only reaches here for recognized extensions.
fn stream_format(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "audio".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Minimal, valid 16-bit PCM WAV with silent samples, written by hand
    /// (no external crate needed).
    fn write_silent_wav(path: &Path, sample_rate: u32, channels: u16, secs: f32) {
        let bits: u16 = 16;
        let block_align = channels * bits / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len =
            ((byte_rate as f32 * secs) as u32 / u32::from(block_align)) * u32::from(block_align);

        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.resize(buf.len() + data_len as usize, 0);

        let mut f = std::fs::File::create(path).expect("create fixture");
        f.write_all(&buf).expect("write fixture");
    }

    /// Unique temp dir per test binary run, cleaned up on drop.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("iwaks-tags-test-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
        fn path(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_wav_stream_properties() {
        let tmp = TempDir::new("wav-prop");
        let p = tmp.path("a.wav");
        write_silent_wav(&p, 44_100, 2, 2.0);

        let m = read_metadata(&p).expect("read should succeed");
        assert_eq!(m.format, "wav");
        assert!(
            (m.duration_ms - 2000).abs() <= 100,
            "duration {}",
            m.duration_ms
        );
        assert_eq!(m.sample_rate, Some(44_100));
        assert_eq!(m.bit_depth, Some(16));
        assert!(
            m.bitrate.is_some(),
            "bitrate should be available for pcm wav"
        );
    }

    #[test]
    fn untagged_wav_returns_empty_tags() {
        let tmp = TempDir::new("wav-untagged");
        let p = tmp.path("b.wav");
        write_silent_wav(&p, 8_000, 1, 0.2);

        let m = read_metadata(&p).expect("read should succeed");
        assert_eq!(m.title, None);
        assert_eq!(m.artist, None);
        assert_eq!(m.album, None);
        assert_eq!(m.genre, None);
        assert_eq!(m.year, None);
        assert_eq!(m.track_no, None);
    }

    #[test]
    fn reads_tags_roundtrip() {
        let tmp = TempDir::new("wav-tags");
        let p = tmp.path("c.wav");
        write_silent_wav(&p, 44_100, 1, 0.5);

        let tagged_ok = crate::write::write_test_tags(&p, "Judul A", "Artis B").is_ok();
        let m = read_metadata(&p).expect("read should succeed");

        if tagged_ok {
            assert_eq!(m.title.as_deref(), Some("Judul A"));
            assert_eq!(m.artist.as_deref(), Some("Artis B"));
        } else {
            assert_eq!(m.title, None);
        }
    }

    #[test]
    fn missing_file_is_error() {
        assert!(read_metadata(Path::new("Z:/definitely/not/here.wav")).is_err());
    }

    #[test]
    fn non_audio_file_is_error() {
        let tmp = TempDir::new("not-audio");
        let p = tmp.path("d.txt");
        std::fs::write(&p, b"not audio").expect("write txt");
        assert!(read_metadata(&p).is_err(), ".txt must not parse");
    }
}
