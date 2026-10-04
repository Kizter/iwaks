//! Integration tests for the cover-lookup cache and the generic settings
//! store — public API only.

use std::path::PathBuf;

use iwaks_library::cover::CoverLookup;
use iwaks_library::db::Library;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("iwaks-cov-test-{}-{name}", std::process::id()));
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

const ART: &str = "https://cdn.example/album/600x600bb.jpg";

// ---------- settings ----------

#[test]
fn setting_is_absent_before_it_is_written() {
    let lib = Library::open(":memory:").expect("open");
    assert_eq!(lib.setting("discord.online_cover").unwrap(), None);
}

#[test]
fn setting_round_trips_a_value() {
    let lib = Library::open(":memory:").expect("open");
    lib.set_setting("discord.online_cover", "true").unwrap();
    assert_eq!(
        lib.setting("discord.online_cover").unwrap().as_deref(),
        Some("true")
    );
}

#[test]
fn setting_overwrites_an_existing_value() {
    let lib = Library::open(":memory:").expect("open");
    lib.set_setting("discord.online_cover", "true").unwrap();
    lib.set_setting("discord.online_cover", "false").unwrap();
    assert_eq!(
        lib.setting("discord.online_cover").unwrap().as_deref(),
        Some("false")
    );
}

#[test]
fn settings_do_not_collide() {
    let lib = Library::open(":memory:").expect("open");
    lib.set_setting("a", "1").unwrap();
    lib.set_setting("b", "2").unwrap();
    assert_eq!(lib.setting("a").unwrap().as_deref(), Some("1"));
    assert_eq!(lib.setting("b").unwrap().as_deref(), Some("2"));
}

#[test]
fn settings_survive_a_reopen() {
    let tmp = TempDir::new("settings");
    let db = tmp.path("lib.db").to_string_lossy().to_string();
    {
        let lib = Library::open(&db).expect("open");
        lib.set_setting("discord.online_cover", "true").unwrap();
    }
    let lib = Library::open(&db).expect("reopen");
    assert_eq!(
        lib.setting("discord.online_cover").unwrap().as_deref(),
        Some("true")
    );
}

// ---------- cover cache ----------

#[test]
fn cover_is_unknown_before_the_first_lookup() {
    let lib = Library::open(":memory:").expect("open");
    assert_eq!(lib.cached_cover("crayon case\u{1f}surabaya").unwrap(), None);
}

#[test]
fn found_cover_is_read_back_with_its_url() {
    let lib = Library::open(":memory:").expect("open");
    let hit = CoverLookup::Found(ART.to_string());
    lib.store_cover("key", &hit).unwrap();
    match lib.cached_cover("key").unwrap() {
        Some(CoverLookup::Found(url)) => assert_eq!(url, ART),
        other => panic!("expected the stored url, got {other:?}"),
    }
}

#[test]
fn missing_cover_is_remembered_so_it_is_not_asked_again() {
    let lib = Library::open(":memory:").expect("open");
    lib.store_cover("key", &CoverLookup::Missing).unwrap();
    assert!(matches!(
        lib.cached_cover("key").unwrap(),
        Some(CoverLookup::Missing)
    ));
}

#[test]
fn a_later_lookup_replaces_the_stored_outcome() {
    let lib = Library::open(":memory:").expect("open");
    lib.store_cover("key", &CoverLookup::Missing).unwrap();
    lib.store_cover("key", &CoverLookup::Found(ART.into()))
        .unwrap();
    assert!(matches!(
        lib.cached_cover("key").unwrap(),
        Some(CoverLookup::Found(_))
    ));
}

#[test]
fn cover_keys_do_not_collide() {
    let lib = Library::open(":memory:").expect("open");
    lib.store_cover("ab\u{1f}c", &CoverLookup::Found("1".into()))
        .unwrap();
    lib.store_cover("a\u{1f}bc", &CoverLookup::Found("2".into()))
        .unwrap();
    match lib.cached_cover("ab\u{1f}c").unwrap() {
        Some(CoverLookup::Found(url)) => assert_eq!(url, "1"),
        other => panic!("wrong row, got {other:?}"),
    }
}

#[test]
fn an_empty_album_key_is_not_stored() {
    // An empty key is "not cacheable" (no tags); caching it would make every
    // untagged track share one row.
    let lib = Library::open(":memory:").expect("open");
    lib.store_cover("", &CoverLookup::Found(ART.into()))
        .unwrap();
    assert_eq!(lib.cached_cover("").unwrap(), None);
}

#[test]
fn cover_cache_survives_a_reopen() {
    let tmp = TempDir::new("cover");
    let db = tmp.path("lib.db").to_string_lossy().to_string();
    {
        let lib = Library::open(&db).expect("open");
        lib.store_cover("key", &CoverLookup::Found(ART.into()))
            .unwrap();
    }
    let lib = Library::open(&db).expect("reopen");
    assert!(matches!(
        lib.cached_cover("key").unwrap(),
        Some(CoverLookup::Found(_))
    ));
}

// ---------- migration ----------

#[test]
fn a_v2_database_gains_the_new_tables_on_open() {
    let tmp = TempDir::new("migrate");
    let db = tmp.path("lib.db").to_string_lossy().to_string();

    {
        // Rewind to a v2 library: no settings table, no cover cache.
        let lib = Library::open(&db).expect("open");
        lib.conn()
            .execute_batch("DROP TABLE settings; DROP TABLE cover_cache;")
            .expect("rewind tables");
        lib.conn()
            .pragma_update(None, "user_version", 2)
            .expect("rewind version");
    }

    let lib = Library::open(&db).expect("migrate v2 -> v3");
    assert_eq!(lib.setting("discord.online_cover").unwrap(), None);
    assert_eq!(lib.cached_cover("key").unwrap(), None);
    lib.set_setting("discord.online_cover", "true").unwrap();
    lib.store_cover("key", &CoverLookup::Missing).unwrap();
    assert!(lib.cached_cover("key").unwrap().is_some());
}
