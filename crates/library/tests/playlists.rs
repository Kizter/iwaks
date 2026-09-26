//! Integration tests for playlists (M4 slice 1) — public API only.

use std::path::PathBuf;

use iwaks_core::track::{AudioMetadata, Track};
use iwaks_library::db::Library;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("iwaks-pl-test-{}-{name}", std::process::id()));
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

fn fake_track(path: &str, title: &str) -> Track {
    let mut t = Track::from_metadata(path, &AudioMetadata::new("flac"), 1000, 42);
    t.title = title.to_string();
    t.duration_ms = 92_000;
    t
}

fn insert_track(lib: &mut Library, path: &str, title: &str) -> i64 {
    lib.upsert_track(&fake_track(path, title)).expect("upsert")
}

// ---------- CRUD ----------

#[test]
fn create_list_rename_delete_playlist() {
    let mut lib = Library::open(":memory:").expect("open");
    let id = lib.create_playlist("  Road Trip  ").expect("create trims");
    assert!(id > 0);

    let listed = lib.list_playlists().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Road Trip");
    assert_eq!(listed[0].track_count, 0);

    assert_eq!(lib.get_playlist(id).unwrap().unwrap().name, "Road Trip");
    lib.rename_playlist(id, "Night Drive").unwrap();
    assert_eq!(lib.get_playlist(id).unwrap().unwrap().name, "Night Drive");

    lib.delete_playlist(id).unwrap();
    assert!(lib.list_playlists().unwrap().is_empty());
    assert_eq!(lib.get_playlist(id).unwrap(), None);
    // Deleting again is an idempotent success.
    lib.delete_playlist(id).unwrap();
}

#[test]
fn create_and_rename_reject_blank_name() {
    let mut lib = Library::open(":memory:").expect("open");
    assert!(lib.create_playlist("   ").is_err());
    assert!(lib.create_playlist("").is_err());
    assert!(lib.list_playlists().unwrap().is_empty());
}

#[test]
fn rename_missing_playlist_is_not_found() {
    let mut lib = Library::open(":memory:").expect("open");
    assert!(lib.rename_playlist(999, "X").is_err());
}

#[test]
fn add_to_missing_playlist_is_not_found() {
    let mut lib = Library::open(":memory:").expect("open");
    insert_track(&mut lib, "C:/music/a.flac", "A");
    assert!(lib.add_track_to_playlist(999, 1).is_err());
}

// ---------- membership & order ----------

#[test]
fn add_dedupes_and_remove_keeps_order() {
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    let b = insert_track(&mut lib, "C:/music/b.flac", "B");
    let c = insert_track(&mut lib, "C:/music/c.flac", "C");
    let pid = lib.create_playlist("P").unwrap();

    lib.add_track_to_playlist(pid, a).unwrap();
    lib.add_track_to_playlist(pid, b).unwrap();
    lib.add_track_to_playlist(pid, c).unwrap();
    lib.add_track_to_playlist(pid, b).unwrap(); // dedupe

    let tracks = lib.get_playlist_tracks(pid).unwrap().unwrap();
    let titles: Vec<_> = tracks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        ["A", "B", "C"],
        "order = insertion order, dupes dropped"
    );
    assert_eq!(lib.get_playlist(pid).unwrap().unwrap().track_count, 3);

    lib.remove_track_from_playlist(pid, b).unwrap();
    let after = lib.get_playlist_tracks(pid).unwrap().unwrap();
    let titles: Vec<_> = after.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["A", "C"]);
    // Removing again is an idempotent success.
    lib.remove_track_from_playlist(pid, b).unwrap();
}

#[test]
fn reorder_playlist_and_truncates() {
    let mut lib = Library::open(":memory:").expect("open");
    let mut ids = Vec::new();
    for n in 1..=4 {
        ids.push(insert_track(
            &mut lib,
            &format!("C:/music/t{n}.flac"),
            &n.to_string(),
        ));
    }
    let pid = lib.create_playlist("P").unwrap();
    for id in &ids {
        lib.add_track_to_playlist(pid, *id).unwrap();
    }

    lib.reorder_playlist(pid, &[ids[2], ids[0], ids[3], ids[1]])
        .unwrap();
    let order: Vec<i64> = lib
        .get_playlist_tracks(pid)
        .unwrap()
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(order, [ids[2], ids[0], ids[3], ids[1]]);

    // A shorter order truncates the playlist to exactly those entries.
    lib.reorder_playlist(pid, &[ids[3], ids[0]]).unwrap();
    let order: Vec<i64> = lib
        .get_playlist_tracks(pid)
        .unwrap()
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(order, [ids[3], ids[0]]);
}

// ---------- cascades ----------

#[test]
fn delete_playlist_cascades_entries() {
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    let pid = lib.create_playlist("P").unwrap();
    lib.add_track_to_playlist(pid, a).unwrap();

    lib.delete_playlist(pid).unwrap();
    assert_eq!(lib.get_playlist_tracks(pid).unwrap(), None);
    let count: i64 = lib
        .conn()
        .query_row("SELECT count(*) FROM playlist_tracks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "entries cascade with the playlist row");
}

#[test]
fn deleting_track_cascades_playlist_entry() {
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    let pid = lib.create_playlist("P").unwrap();
    lib.add_track_to_playlist(pid, a).unwrap();

    lib.delete_tracks(&["C:/music/a.flac".to_string()]).unwrap();
    assert_eq!(lib.track_count().unwrap(), 0);
    assert!(lib.get_playlist_tracks(pid).unwrap().unwrap().is_empty());
}

// ---------- m3u ----------

#[test]
fn export_m3u_shape_and_roundtrip() {
    let tmp = TempDir::new("m3u-roundtrip");
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "Song A");
    let b = insert_track(&mut lib, "C:/music/b.flac", "Song B");
    let pid = lib.create_playlist("Trip").unwrap();
    lib.add_track_to_playlist(pid, a).unwrap();
    lib.add_track_to_playlist(pid, b).unwrap();

    let content = lib.export_m3u(pid).unwrap().expect("playlist exists");
    assert!(content.starts_with("#EXTM3U\n"));
    assert!(content.contains("#EXTINF:92,Song A\nC:/music/a.flac\n"));
    assert!(content.contains("#EXTINF:92,Song B\nC:/music/b.flac\n"));

    let file = tmp.path("export.m3u");
    std::fs::write(&file, &content).unwrap();
    let roundtrip = lib.import_m3u(&file).unwrap();
    assert_ne!(roundtrip, pid);
    let rt_tracks = lib.get_playlist_tracks(roundtrip).unwrap().unwrap();
    let titles: Vec<_> = rt_tracks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["Song A", "Song B"], "export → import keeps order");
    assert_eq!(lib.get_playlist(roundtrip).unwrap().unwrap().name, "export");
}

#[test]
fn import_m3u_skips_missing_entries() {
    let tmp = TempDir::new("m3u-skip");
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    let file = tmp.path("mix.m3u");
    std::fs::write(
        &file,
        "#EXTM3U\n#EXTINF:92,Ignored\ngone.flac\nC:/music/a.flac\nalso-gone.flac\n",
    )
    .unwrap();

    let pid = lib.import_m3u(&file).unwrap();
    let tracks = lib.get_playlist_tracks(pid).unwrap().unwrap();
    assert_eq!(tracks.len(), 1, "only the resolvable entry is added");
    assert_eq!(tracks[0].id, a);
    assert_eq!(lib.get_playlist(pid).unwrap().unwrap().track_count, 1);
}

#[test]
fn import_m3u_resolves_relative_entries() {
    let tmp = TempDir::new("m3u-rel");
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, &tmp.path("rel/a.flac").to_string_lossy(), "A");
    let b = insert_track(&mut lib, &tmp.path("rel/b.flac").to_string_lossy(), "B");
    std::fs::create_dir_all(tmp.path("rel")).unwrap();
    let file = tmp.path("rel/play.m3u");
    std::fs::write(&file, "a.flac\nb.flac\n").unwrap();

    let pid = lib.import_m3u(&file).unwrap();
    let tracks = lib.get_playlist_tracks(pid).unwrap().unwrap();
    assert_eq!(
        tracks.len(),
        2,
        "relative entries resolve against the m3u dir"
    );
    assert_eq!(tracks[0].id, a);
    assert_eq!(tracks[1].id, b);
    let _ = b;
}

// ---------- migration ----------

#[test]
fn migration_v1_to_v2_preserves_tracks_and_adds_playlists() {
    let tmp = TempDir::new("migrate-v2");
    let db_path = tmp.path("lib.db");
    let mut lib = Library::open(&db_path.to_string_lossy()).expect("fresh open (v2)");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    assert_eq!(lib.track_count().unwrap(), 1);

    // Simulate a v1 database: drop the v2 tables, rewind the version.
    lib.conn()
        .execute_batch(
            "DROP TABLE IF EXISTS playlist_tracks;
             DROP TABLE IF EXISTS playlists;
             PRAGMA user_version = 1;",
        )
        .unwrap();
    drop(lib);

    let mut reopened = Library::open(&db_path.to_string_lossy()).expect("migrate 1 → 2");
    assert_eq!(
        reopened.track_count().unwrap(),
        1,
        "tracks survive migration"
    );
    let pid = reopened
        .create_playlist("After Migration")
        .expect("playlists usable post-migration");
    reopened.add_track_to_playlist(pid, a).unwrap();
    assert_eq!(reopened.get_playlist(pid).unwrap().unwrap().track_count, 1);

    let version: i32 = reopened
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
}

// ---------- bulk add (save-as-playlist, M4 slice 2) ----------

#[test]
fn add_tracks_bulk_appends_in_order_and_dedupes() {
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    let b = insert_track(&mut lib, "C:/music/b.flac", "B");
    let c = insert_track(&mut lib, "C:/music/c.flac", "C");
    let pid = lib.create_playlist("P").unwrap();

    let inserted = lib
        .add_tracks_to_playlist(pid, &[a, b, c, b])
        .expect("bulk add");
    assert_eq!(inserted, 3, "duplicates are ignored, not re-added");

    let titles: Vec<String> = lib
        .get_playlist_tracks(pid)
        .unwrap()
        .unwrap()
        .iter()
        .map(|t| t.title.clone())
        .collect();
    assert_eq!(titles, ["A", "B", "C"], "insertion order preserved");
    assert_eq!(lib.get_playlist(pid).unwrap().unwrap().track_count, 3);

    // A second bulk call appends after the existing entries.
    lib.add_tracks_to_playlist(pid, &[a, b]).unwrap();
    let titles: Vec<String> = lib
        .get_playlist_tracks(pid)
        .unwrap()
        .unwrap()
        .iter()
        .map(|t| t.title.clone())
        .collect();
    assert_eq!(titles, ["A", "B", "C"], "re-adds are deduped in place");
}

#[test]
fn add_tracks_bulk_empty_list_is_a_noop() {
    let mut lib = Library::open(":memory:").expect("open");
    let pid = lib.create_playlist("P").unwrap();
    assert_eq!(lib.add_tracks_to_playlist(pid, &[]).unwrap(), 0);
    assert!(lib.get_playlist_tracks(pid).unwrap().unwrap().is_empty());
    assert_eq!(lib.get_playlist(pid).unwrap().unwrap().track_count, 0);
}

#[test]
fn add_tracks_bulk_missing_playlist_is_not_found() {
    let mut lib = Library::open(":memory:").expect("open");
    insert_track(&mut lib, "C:/music/a.flac", "A");
    let err = lib.add_tracks_to_playlist(999, &[1]).unwrap_err();
    assert_eq!(err.to_string(), "not found: playlist 999");
}

#[test]
fn add_tracks_bulk_rolls_back_on_failure() {
    let mut lib = Library::open(":memory:").expect("open");
    let a = insert_track(&mut lib, "C:/music/a.flac", "A");
    let pid = lib.create_playlist("P").unwrap();

    // A nonexistent track id violates the FK — the whole batch must roll back,
    // so the valid entry from the same call doesn't linger half-applied.
    assert!(lib.add_tracks_to_playlist(pid, &[a, 9999]).is_err());
    assert!(lib.get_playlist_tracks(pid).unwrap().unwrap().is_empty());
}
