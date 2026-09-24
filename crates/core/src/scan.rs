//! Scan classification logic (pure).

use std::path::Path;

/// Audio extensions recognized by the scanner, matching the supported
/// format list in docs/design.md.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "flac", "wav", "mp3", "aac", "m4a", "ogg", "oga", "opus", "wv", "aiff", "aif", "wma", "dsf",
    "dff",
];

/// Whether `path` has a recognized audio extension (case-insensitive).
pub fn is_supported_audio(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => SUPPORTED_EXTENSIONS
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s)),
        None => false,
    }
}

/// What to do with a file found on disk, compared to the database row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Not in the database yet — insert.
    New,
    /// Present but modified (mtime or size changed) — re-read tags, update.
    Changed,
    /// Present and unchanged — skip.
    Unchanged,
}

/// Classify a file using the recorded DB row (mtime/size) vs filesystem stat.
/// `db_modified`/`db_size` are `None` when there is no row yet.
pub fn classify_file(
    db_modified: Option<i64>,
    db_size: Option<i64>,
    fs_modified: i64,
    fs_size: i64,
) -> FileState {
    match (db_modified, db_size) {
        (None, _) => FileState::New,
        (Some(m), Some(s)) if m == fs_modified && s == fs_size => FileState::Unchanged,
        _ => FileState::Changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_positive() {
        for ext in SUPPORTED_EXTENSIONS {
            let p = Path::new("x").join(format!("song.{ext}"));
            assert!(is_supported_audio(&p), "{ext} should be supported");
        }
    }

    #[test]
    fn unsupported_and_upper_case() {
        assert!(!is_supported_audio(Path::new("song.txt")));
        assert!(!is_supported_audio(Path::new("song.mp4")));
        assert!(!is_supported_audio(Path::new("song.flac.bak")));
        assert!(!is_supported_audio(Path::new("song")));
        assert!(is_supported_audio(Path::new("SONG.FLAC")));
    }

    #[test]
    fn classify_new_when_no_db_row() {
        assert_eq!(classify_file(None, None, 100, 10), FileState::New);
    }

    #[test]
    fn classify_unchanged_when_identical() {
        assert_eq!(
            classify_file(Some(100), Some(10), 100, 10),
            FileState::Unchanged
        );
    }

    #[test]
    fn classify_changed_on_mtime_or_size() {
        assert_eq!(
            classify_file(Some(99), Some(10), 100, 10),
            FileState::Changed
        );
        assert_eq!(
            classify_file(Some(100), Some(99), 100, 10),
            FileState::Changed
        );
    }
}
