//! Integration tests for the library crate — public API only.

use std::io::Write;
use std::path::{Path, PathBuf};

use iwaks_core::track::{AudioMetadata, Track};
use iwaks_library::db::Library;
use iwaks_library::scan::{scan, ScanOptions, ScanProgress};

// ---------- helpers ----------

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("iwaks-lib-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
    fn path(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Minimal valid 16-bit PCM WAV, silent samples.
fn make_wav(path: &Path, sample_rate: u32, channels: u16, secs: f32) {
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
    buf.extend_from_slice(&1u16.to_le_bytes());
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

/// A fixture tree exercising: two readable tracks, one corrupt file, one
/// non-audio file that must be ignored.
fn build_fixture(tmp: &TempDir) -> PathBuf {
    let root = tmp.path("music");
    std::fs::create_dir_all(root.join("Album A")).expect("mkdir");
    make_wav(
        &root.join("Album A/Pink Floyd - Wish You Were Here.wav"),
        44_100,
        2,
        2.0,
    );
    make_wav(&root.join("Album A/Radiohead - Creep.wav"), 44_100, 2, 1.0);
    std::fs::write(
        root.join("broken.wav"),
        b"this is not really audio data at all, sorry",
    )
    .expect("write broken");
    std::fs::write(root.join("notes.txt"), b"ignore me").expect("write txt");
    root
}

fn scan_once(lib: &mut Library, root: &Path, clean_missing: bool) -> ScanProgress {
    let opts = ScanOptions {
        root: root.to_path_buf(),
        clean_missing,
    };
    let mut last = ScanProgress::default();
    let report = scan(lib, &opts, &mut |p| last = p.clone()).expect("scan runs");
    assert_eq!(report, last, "final callback equals report");
    report
}

fn fake_track(path: &str, title: &str) -> Track {
    let mut t = Track::from_metadata(path, &AudioMetadata::new("flac"), 1000, 42);
    t.title = title.to_string();
    t
}

// ---------- scan ----------

#[test]
fn scan_adds_tracks_with_fallback_titles() {
    let tmp = TempDir::new("scan-add");
    let root = build_fixture(&tmp);
    let mut lib = Library::open(":memory:").expect("open");

    let report = scan_once(&mut lib, &root, true);

    assert_eq!(report.added, 2, "added");
    assert_eq!(report.errors, 1, "broken.wav fails to parse");
    assert_eq!(report.skipped, 0);
    assert_eq!(lib.track_count().unwrap(), 2);

    let tracks = lib.all_tracks().unwrap();
    let wish = tracks
        .iter()
        .find(|t| t.path.ends_with("Wish You Were Here.wav"))
        .expect("wish present");
    assert_eq!(wish.title, "Pink Floyd - Wish You Were Here");
    assert_eq!(wish.format, "wav");
    assert!(
        (wish.duration_ms - 2000).abs() <= 150,
        "dur {}",
        wish.duration_ms
    );
    assert_eq!(wish.sample_rate, Some(44_100));
    assert!(wish.file_size > 0);
}

#[test]
fn scan_is_incremental_on_second_pass() {
    let tmp = TempDir::new("scan-incr");
    let root = build_fixture(&tmp);
    let mut lib = Library::open(":memory:").expect("open");

    scan_once(&mut lib, &root, true);
    let second = scan_once(&mut lib, &root, true);

    assert_eq!(second.added, 0);
    assert_eq!(second.updated, 0);
    assert_eq!(second.skipped, 2, "both healthy files unchanged");
    assert_eq!(
        second.errors, 1,
        "broken file is retried (not yet classified)"
    );
    assert_eq!(lib.track_count().unwrap(), 2);
}

#[test]
fn scan_detects_changed_files() {
    let tmp = TempDir::new("scan-change");
    let root = build_fixture(&tmp);
    let mut lib = Library::open(":memory:").expect("open");
    scan_once(&mut lib, &root, true);

    // Replace Creep.wav with a different length -> size changes -> changed.
    make_wav(&root.join("Album A/Radiohead - Creep.wav"), 44_100, 2, 1.5);
    let third = scan_once(&mut lib, &root, true);

    assert_eq!(third.updated, 1);
    assert_eq!(third.skipped, 1);
    assert_eq!(lib.track_count().unwrap(), 2);
}

#[test]
fn scan_removes_missing_files_only_when_clean_missing() {
    let tmp = TempDir::new("scan-remove");
    let root = build_fixture(&tmp);
    let mut lib = Library::open(":memory:").expect("open");
    scan_once(&mut lib, &root, true);

    std::fs::remove_file(root.join("Album A/Pink Floyd - Wish You Were Here.wav")).expect("remove");

    let keep = scan_once(&mut lib, &root, false);
    assert_eq!(keep.removed, 0);
    assert_eq!(
        lib.track_count().unwrap(),
        2,
        "stale row kept when clean_missing=false"
    );

    let clean = scan_once(&mut lib, &root, true);
    assert_eq!(clean.removed, 1);
    assert_eq!(lib.track_count().unwrap(), 1);
}

// ---------- search (FTS5) ----------

#[test]
fn search_matches_prefix_across_columns() {
    let tmp = TempDir::new("scan-search");
    let root = build_fixture(&tmp);
    let mut lib = Library::open(":memory:").expect("open");
    scan_once(&mut lib, &root, true);

    assert_eq!(lib.search("pink").unwrap().len(), 1);
    assert_eq!(lib.search("floyd").unwrap().len(), 1);
    assert_eq!(lib.search("radiohead").unwrap().len(), 1);
    assert_eq!(lib.search("CREEP").unwrap().len(), 1, "case-insensitive");
    assert_eq!(
        lib.search("pink radiohead").unwrap().len(),
        0,
        "AND semantics"
    );
    assert_eq!(lib.search("zzz").unwrap().len(), 0);
    assert!(
        lib.search("").unwrap().is_empty(),
        "empty query yields none"
    );
}

#[test]
fn upsert_updated_row_is_searchable_with_new_title() {
    let mut lib = Library::open(":memory:").expect("open");
    let p = r"C:\music\a.flac";

    lib.upsert_track(&fake_track(p, "Old Name")).unwrap();
    lib.upsert_track(&fake_track(p, "New Name")).unwrap();

    assert_eq!(lib.track_count().unwrap(), 1, "same path upserts");
    assert_eq!(lib.all_tracks().unwrap()[0].title, "New Name");
    assert_eq!(lib.search("new").unwrap().len(), 1, "FTS trigger updated");
    assert_eq!(lib.search("old").unwrap().len(), 0, "old token gone");
}

// ---------- db basics ----------

#[test]
fn delete_tracks_removes_by_path() {
    let mut lib = Library::open(":memory:").expect("open");
    lib.upsert_track(&fake_track("a.flac", "A")).unwrap();
    lib.upsert_track(&fake_track("b.flac", "B")).unwrap();

    assert_eq!(lib.delete_tracks(&["a.flac".to_string()]).unwrap(), 1);
    assert_eq!(lib.track_count().unwrap(), 1);
}

#[test]
fn database_persists_across_reopen() {
    let tmp = TempDir::new("reopen");
    let db_path = tmp.path("lib.db");
    let db = db_path.to_string_lossy().to_string();

    {
        let mut lib = Library::open(&db).expect("open first");
        lib.upsert_track(&fake_track("persist.flac", "Persist"))
            .unwrap();
    }
    let lib = Library::open(&db).expect("open second");
    assert_eq!(lib.track_count().unwrap(), 1);
    assert_eq!(lib.all_tracks().unwrap()[0].title, "Persist");
}

#[test]
fn future_schema_version_is_rejected() {
    let tmp = TempDir::new("schema");
    let db_path = tmp.path("future.db");
    let db = db_path.to_string_lossy().to_string();

    {
        let lib = Library::open(&db).expect("open");
        lib.conn()
            .pragma_update(None, "user_version", 99)
            .expect("bump version");
    }
    let err = Library::open(&db).expect_err("v99 must be rejected");
    assert!(err.to_string().contains("99"), "{err}");
}
