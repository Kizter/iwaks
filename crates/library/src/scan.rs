//! Incremental background scanner: walk a folder, read metadata for new or
//! changed files, and prune tracks whose files disappeared.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iwaks_core::scan::{classify_file, is_supported_audio, FileState};
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

    // Prune only tracks under the scanned root whose file genuinely vanished.
    // Two guards keep one scan from wiping rows it shouldn't:
    //   * scope — rows outside `opts.root` belong to other roots, so keep them;
    //   * IO-safety — only a "not found" result prunes; a permission or
    //     transient IO error preserves the row.
    // An unreachable root (unplugged drive, unmounted share) is treated the
    // same way: nothing is pruned, so an offline root can't empty the library.
    if opts.clean_missing && root_is_reachable(&opts.root) {
        let root_key = norm(&opts.root.to_string_lossy());
        let root_key = root_key.trim_end_matches('/');
        let stale: Vec<String> = existing
            .values()
            .filter(|(p, _, _)| under_root(&norm(p), root_key) && is_genuinely_gone(p))
            .map(|(p, _, _)| p.clone())
            .collect();
        if !stale.is_empty() {
            report.removed = lib.delete_tracks(&stale)?;
        }
    }

    on_progress(&report);
    Ok(report)
}

/// Add or update specific files picked by the user (multi-select dialog).
/// No recursion, no pruning — each listed file is read & upserted once.
pub fn scan_files(
    lib: &mut Library,
    paths: &[PathBuf],
    on_progress: &mut dyn FnMut(&ScanProgress),
) -> Result<ScanProgress, LibraryError> {
    let mut report = ScanProgress {
        total_files: paths.len(),
        ..Default::default()
    };
    on_progress(&report);

    for path in paths {
        let stat = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        let modified = modified_secs(&stat);
        let size = stat.len() as i64;

        match read_metadata(path) {
            Ok(meta) => {
                let track = Track::from_metadata(&path.to_string_lossy(), &meta, size, modified);
                lib.upsert_track(&track)?;
                report.added += 1;
            }
            Err(_) => report.errors += 1,
        }
        report.scanned += 1;
        if report.scanned.is_multiple_of(20) {
            on_progress(&report);
        }
    }

    on_progress(&report);
    Ok(report)
}

/// Normalized path key for DB lookups: case-insensitive on all platforms,
/// and `/`/`\` equivalent — a file picked from a dialog can carry either
/// separator on Windows, while walking a folder yields the other.
pub(crate) fn norm(p: &str) -> String {
    p.to_lowercase().replace('\\', "/")
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

/// A root is reachable when it currently resolves to a directory. An unplugged
/// drive or unmounted share fails this check, and pruning is skipped so an
/// offline root never empties the library.
fn root_is_reachable(root: &Path) -> bool {
    std::fs::metadata(root).map(|m| m.is_dir()).unwrap_or(false)
}

/// Whether a normalized stored path lives below the (normalized, trailing
/// slash trimmed) scanned root. An empty root matches everything.
fn under_root(key: &str, root_key: &str) -> bool {
    if root_key.is_empty() {
        return true;
    }
    key == root_key || key.starts_with(&format!("{root_key}/"))
}

/// Classify one `metadata` result: only a genuine "not found" means the file
/// is gone and safe to prune. Any other error (permission denied, transient
/// IO) must preserve the row.
fn io_says_gone(result: std::io::Result<std::fs::Metadata>) -> bool {
    matches!(result, Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

fn is_genuinely_gone(path: &str) -> bool {
    io_says_gone(std::fs::metadata(path))
}

#[cfg(test)]
mod tests {
    use super::{io_says_gone, under_root};
    use std::io::ErrorKind;

    #[test]
    fn under_root_matches_only_descendants() {
        assert!(under_root("c:/music/a.flac", "c:/music"));
        assert!(under_root("c:/music/sub/a.flac", "c:/music"));
        assert!(under_root("c:/music", "c:/music"), "the root itself counts");
        assert!(
            !under_root("c:/musicbox/a.flac", "c:/music"),
            "a sibling with a shared prefix is not under the root"
        );
        assert!(!under_root("d:/other/a.flac", "c:/music"));
        assert!(under_root("anything", ""), "an empty root matches all");
    }

    #[test]
    fn only_not_found_is_treated_as_gone() {
        assert!(io_says_gone(Err(ErrorKind::NotFound.into())));
        assert!(
            !io_says_gone(Err(ErrorKind::PermissionDenied.into())),
            "denied access preserves the row"
        );
        assert!(
            !io_says_gone(Err(ErrorKind::TimedOut.into())),
            "transient IO errors preserve the row"
        );
    }
}
