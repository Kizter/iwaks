//! Track model & metadata normalization (pure).

use serde::{Deserialize, Serialize};

/// Raw metadata read from audio tags. All optional except duration/format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    pub duration_ms: i64,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub bitrate: Option<i64>,
    /// Canonical short format name, e.g. "flac", "mp3", "dsf".
    pub format: String,
}

impl AudioMetadata {
    pub fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
            ..Default::default()
        }
    }
}

/// A track as stored/returned by the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    pub duration_ms: i64,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub bitrate: Option<i64>,
    pub format: String,
    pub file_size: i64,
    /// Unix seconds of last file modification (used for incremental scan).
    pub modified_at: i64,
}

impl Track {
    /// Build a Track from scanned metadata, applying fallbacks
    /// (title falls back to the file name stem). `id` starts at 0 and is
    /// assigned by the database. Pure — no filesystem access.
    pub fn from_metadata(
        path: &str,
        meta: &AudioMetadata,
        file_size: i64,
        modified_at: i64,
    ) -> Track {
        let title = match meta.title.as_deref() {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => default_title(path),
        };
        Track {
            id: 0,
            path: path.to_string(),
            title,
            artist: meta.artist.clone(),
            album: meta.album.clone(),
            album_artist: meta.album_artist.clone(),
            genre: meta.genre.clone(),
            year: meta.year,
            track_no: meta.track_no,
            disc_no: meta.disc_no,
            duration_ms: meta.duration_ms,
            sample_rate: meta.sample_rate,
            bit_depth: meta.bit_depth,
            bitrate: meta.bitrate,
            format: meta.format.clone(),
            file_size,
            modified_at,
        }
    }
}

/// File-name fallback title: the path's file stem (pure string math,
/// no filesystem access), or the whole path if there is no stem.
fn default_title(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(path);
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_with(title: Option<&str>) -> AudioMetadata {
        let mut m = AudioMetadata::new("flac");
        if let Some(t) = title {
            m.title = Some(t.to_string());
        }
        m.duration_ms = 90_000;
        m
    }

    #[test]
    fn title_falls_back_to_file_stem() {
        let t = Track::from_metadata(r"C:\music\song.flac", &meta_with(None), 12, 1);
        assert_eq!(t.title, "song");
    }

    #[test]
    fn title_uses_tag_when_present() {
        let t = Track::from_metadata(r"C:\music\song.flac", &meta_with(Some("Judul")), 12, 1);
        assert_eq!(t.title, "Judul");
    }

    #[test]
    fn blank_tag_title_falls_back_to_stem() {
        let t = Track::from_metadata(r"C:\music\song.flac", &meta_with(Some("   ")), 12, 1);
        assert_eq!(t.title, "song");
    }

    #[test]
    fn non_music_path_has_no_stem_fallbacks_to_path() {
        let t = Track::from_metadata("no-extension", &meta_with(None), 1, 1);
        assert_eq!(t.title, "no-extension");
    }

    #[test]
    fn carries_metadata_through() {
        let m = meta_with(Some("T"));
        let t = Track::from_metadata("p", &m, 99, 42);
        assert_eq!(t.duration_ms, 90_000);
        assert_eq!(t.format, "flac");
        assert_eq!(t.file_size, 99);
        assert_eq!(t.modified_at, 42);
        assert_eq!(t.id, 0);
    }
}
