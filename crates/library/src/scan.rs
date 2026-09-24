//! Incremental background scanner: walk a folder, read metadata for new or
//! changed files, and prune tracks whose files disappeared.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iwaks_core::scan::{classify_file, find_stale, is_supported_audio, FileState};
use iwaks_core::track::Track;
use iwaks_tags::read::read_metadata;

use crate::db::{Library, LibraryError};

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Root folder to walk recursively.
    pub root: PathBuf,
    /// Remove stored tracks whose files no longer exist on disk.
    pub clean_missing: bool,
}

/// Mirrors a scan in progress (also used as the final report).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub total_files: usize,
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub removed: usize,
    pub errors: usize,
}

pub fn scan(
    lib: &mut Library,
    opts: &ScanOptions,
    on_progress: &mut dyn FnMut(&ScanProgress),
) -> Result<ScanProgress, LibraryError> {
    let files = collect_audio_files(&opts.root);
    let total = files.len();
    let mut report = ScanProgress {
        total_files: total,
        ..Default::default()
    };
    on_progress(&report);

    // Existing DB rows keyed by normalized path -> (original path, mtime, size).
    let existing: HashMap<String, (String, i64, i64)> = lib
        .existing_files()?
        .into_iter()
        .map(|(p, m, s)| (norm(&p), (p, m, s)))
        .collect();

    let mut found: Vec<String> = Vec::with_capacity(total);

    for path in &files {
        let stat = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        let modified = modified_secs(&stat);
        let size = stat.len() as i64;
        let norm_path = norm(&path.to_string_lossy());
        found.push(path.to_string_lossy().into_owned());

        let state = match existing.get(&norm_path) {
            Some((_, db_mtime, db_size)) => {
                classify_file(Some(*db_mtime), Some(*db_size), modified, size)
            }
            None => FileState::New,
        };

        match state {
            FileState::Unchanged => report.skipped += 1,
            FileState::New | FileState::Changed => match read_metadata(path) {
                Ok(meta) => {
                    let track =
                        Track::from_metadata(&path.to_string_lossy(), &meta, size, modified);
                    lib.upsert_track(&track)?;
                    match state {
                        FileState::New => report.added += 1,
                        _ => report.updated += 1,
                    }
                }
                Err(_) => report.errors += 1,
            },
        }
        report.scanned += 1;

        if report.scanned.is_multiple_of(50) {
            on_progress(&report);
        }
    }

    if opts.clean_missing && !existing.is_empty() {
        let stale = find_stale(
            &existing
                .values()
                .map(|(p, _, _)| p.clone())
                .collect::<Vec<_>>(),
            &found,
        );
        if !stale.is_empty() {
            report.removed = lib.delete_tracks(&stale)?;
        }
    }

    on_progress(&report);
    Ok(report)
}

/// Normalized path key for DB lookups (case-insensitive on all platforms).
fn norm(p: &str) -> String {
    p.to_lowercase()
}

/// Collect all supported audio files under `root`, recursively.
fn collect_audio_files(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && is_supported_audio(e.path()))
        .map(|e| e.into_path())
        .collect()
}

fn modified_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
