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
    use crate::testutil::{write_silent_wav, TempDir};

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
