//! Pure domain logic for Iwaks. No I/O, no external state.
//!
//! Everything here is deterministic and unit-testable (TDD RED/GREEN).

pub mod fts;
pub mod scan;
pub mod time;
pub mod track;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::fts::build_fts_query;
    use crate::scan::{classify_file, find_stale, is_supported_audio, FileState};
    use crate::time::format_duration;
    use crate::track::{AudioMetadata, Track};
    use std::path::Path;

    #[test]
    fn public_api_links_together() {
        assert!(is_supported_audio(Path::new("a.flac")));
        assert_eq!(classify_file(None, None, 1, 1), FileState::New);
        assert_eq!(find_stale(&["x".into()], &[]).len(), 1);
        assert!(build_fts_query("x").is_some());
        assert_eq!(format_duration(0), "0:00");
        let t = Track::from_metadata("p.flac", &AudioMetadata::new("flac"), 1, 1);
        assert_eq!(t.title, "p");
        let _unused: HashSet<String> = HashSet::new();
    }
}
