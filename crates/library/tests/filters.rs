//! Genre / decade filters and the sort control (roadmap v0.3.0 #20, #21).
//!
//! Public API only: the point of these tests is the *behaviour* a user sees —
//! which rows come back, in what order, and what the filter chips offer.

use std::path::PathBuf;

use iwaks_core::track::{AudioMetadata, Track};
use iwaks_library::db::{Library, TrackFilter, TrackSort};

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("iwaks-filter-{}-{name}", std::process::id()));
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

/// A track with the fields the filters/sorts care about.
///
/// Every row shares one artist and album on purpose: with the artist/album
/// order that leaves the title as the tie-break, which is what the default
/// sort tests below assert.
fn track(
    path: &str,
    title: &str,
    genre: Option<&str>,
    year: Option<i64>,
    ms: i64,
    modified: i64,
) -> Track {
    let mut t = Track::from_metadata(path, &AudioMetadata::new("flac"), 42, modified);
    t.title = title.to_string();
    t.artist = Some("Artist".to_string());
    t.album = Some("Album".to_string());
    t.genre = genre.map(str::to_string);
    t.year = year;
    t.duration_ms = ms;
    t
}

/// Library with a small, deliberately mixed set:
///
/// | title | genre | year | ms | modified_at |
/// |-------|-------|------|----|-------------|
/// | Bravo | Rock  | 1994 | 900 | 300 |
/// | Alpha | Rock  | 1999 | 300 | 100 |
/// | Zulu  | Jazz  | 1994 | 600 | 200 |
/// | Yankee| Metal | 1989 | 600 | 400 |
/// | Xray  | nil   | nil  | 600 | 500 |
fn fixture() -> Library {
    let mut lib = Library::open(":memory:").expect("open");
    let rows = [
        track("/m/bravo.flac", "Bravo", Some("Rock"), Some(1994), 900, 300),
        track("/m/alpha.flac", "Alpha", Some("Rock"), Some(1999), 300, 100),
        track("/m/zulu.flac", "Zulu", Some("Jazz"), Some(1994), 600, 200),
        track(
            "/m/yankee.flac",
            "Yankee",
            Some("Metal"),
            Some(1989),
            600,
            400,
        ),
        track("/m/xray.flac", "Xray", None, None, 600, 500),
    ];
    for t in rows {
        lib.upsert_track(&t).expect("insert");
    }
    lib
}

fn titles(
    lib: &Library,
    query: Option<&str>,
    filter: &TrackFilter,
    sort: TrackSort,
) -> Vec<String> {
    lib.tracks_filtered(query, filter, sort)
        .expect("query")
        .into_iter()
        .map(|t| t.title)
        .collect()
}

fn genre(value: &str) -> TrackFilter {
    TrackFilter {
        genre: Some(value.to_string()),
        decade: None,
    }
}

fn decade(value: i64) -> TrackFilter {
    TrackFilter {
        genre: None,
        decade: Some(value),
    }
}

// ---------- genre filter ----------

#[test]
fn no_filter_returns_everything_in_the_default_order() {
    let lib = fixture();
    // One shared artist/album, so the artist→album order falls through to the
    // title tie-break.
    assert_eq!(
        titles(&lib, None, &TrackFilter::default(), TrackSort::default()),
        ["Alpha", "Bravo", "Xray", "Yankee", "Zulu"]
    );
}

#[test]
fn genre_filter_keeps_only_that_genre() {
    let lib = fixture();
    assert_eq!(
        titles(&lib, None, &genre("Rock"), TrackSort::default()),
        ["Alpha", "Bravo"]
    );
}

#[test]
fn an_unknown_genre_matches_nothing() {
    let lib = fixture();
    assert!(titles(&lib, None, &genre("Polka"), TrackSort::default()).is_empty());
}

#[test]
fn a_genre_is_matched_against_the_stored_spelling() {
    let lib = fixture();
    // The chips hand back exactly what the library stores, so a different
    // case finds nothing rather than quietly matching.
    assert!(titles(&lib, None, &genre("rock"), TrackSort::default()).is_empty());
}

#[test]
fn a_genre_containing_quotes_is_data_not_sql() {
    let mut lib = Library::open(":memory:").expect("open");
    let nasty = "Rock 'n' Roll\"; DROP TABLE tracks;--";
    lib.upsert_track(&track(
        "/m/a.flac",
        "Safe",
        Some(nasty),
        Some(2001),
        1000,
        1,
    ))
    .expect("insert");

    // The predicate is bound, so the quotes are part of a value, not SQL: the
    // row is findable by that exact genre and the table is still there.
    assert_eq!(
        titles(&lib, None, &genre(nasty), TrackSort::default()),
        ["Safe"]
    );
    assert!(titles(&lib, None, &genre("Rock"), TrackSort::default()).is_empty());
    assert_eq!(lib.track_count().unwrap(), 1, "table survived");
}

// ---------- decade filter ----------

#[test]
fn decade_filter_is_inclusive_of_its_own_years() {
    let lib = fixture();
    // 1990s: Alpha (1999) and Bravo/Zulu (1994) — not Yankee (1989).
    assert_eq!(
        titles(&lib, None, &decade(1990), TrackSort::default()),
        ["Alpha", "Bravo", "Zulu"]
    );
}

#[test]
fn decades_with_nothing_in_them_return_nothing() {
    let lib = fixture();
    assert!(titles(&lib, None, &decade(1970), TrackSort::default()).is_empty());
}

#[test]
fn genre_and_decade_narrow_together() {
    let lib = fixture();
    let both = TrackFilter {
        genre: Some("Rock".to_string()),
        decade: Some(1990),
    };
    assert_eq!(
        titles(&lib, None, &both, TrackSort::Title),
        ["Alpha", "Bravo"]
    );

    // Rock from the 80s: nothing (the only 1989 track is Metal).
    let miss = TrackFilter {
        genre: Some("Rock".to_string()),
        decade: Some(1980),
    };
    assert!(titles(&lib, None, &miss, TrackSort::default()).is_empty());
}

#[test]
fn undated_tracks_never_match_a_decade() {
    let lib = fixture();
    for start in [1970, 1980, 1990, 2000, 2010] {
        let rows = titles(&lib, None, &decade(start), TrackSort::default());
        assert!(!rows.contains(&"Xray".to_string()), "Xray has no year");
    }
}

// ---------- search composed with filter and sort ----------

#[test]
fn search_still_finds_tracks_under_a_filter() {
    let lib = fixture();
    // "a" matches all three Rock titles; the decade cuts it to the 90s.
    assert_eq!(
        titles(&lib, Some("a"), &genre("Rock"), TrackSort::default()),
        ["Alpha", "Bravo"]
    );
    assert_eq!(
        titles(&lib, Some("a"), &genre("Jazz"), TrackSort::default()),
        ["Zulu"]
    );
}

#[test]
fn search_keeps_relevance_until_a_sort_is_chosen() {
    let lib = fixture();

    // Default sort + search = FTS rank order, i.e. unchanged from before #21.
    let ranked = titles(
        &lib,
        Some("a"),
        &TrackFilter::default(),
        TrackSort::default(),
    );
    let legacy: Vec<String> = lib
        .search("a")
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(ranked, legacy, "relevance still decides the order");

    // An explicit sort takes over from relevance.
    let longest = lib
        .tracks_filtered(Some("a"), &TrackFilter::default(), TrackSort::Longest)
        .unwrap();
    assert!(longest.len() > 1, "the probe term matches several tracks");
    let ordered: Vec<i64> = longest.iter().map(|t| t.duration_ms).collect();
    let mut sorted = ordered.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(ordered, sorted, "explicit sort wins over rank");
}

// ---------- sort ----------

#[test]
fn title_sort_is_case_insensitive() {
    let lib = fixture();
    assert_eq!(
        titles(&lib, None, &TrackFilter::default(), TrackSort::Title),
        ["Alpha", "Bravo", "Xray", "Yankee", "Zulu"]
    );
}

#[test]
fn longest_sort_puts_the_biggest_file_first() {
    let lib = fixture();
    assert_eq!(
        titles(&lib, None, &TrackFilter::default(), TrackSort::Longest),
        ["Bravo", "Xray", "Yankee", "Zulu", "Alpha"]
    );
}

#[test]
fn added_sort_puts_the_newest_file_first() {
    let lib = fixture();
    assert_eq!(
        titles(&lib, None, &TrackFilter::default(), TrackSort::Added),
        ["Xray", "Yankee", "Bravo", "Zulu", "Alpha"]
    );
}

#[test]
fn sort_composes_with_a_filter() {
    let lib = fixture();
    assert_eq!(
        titles(&lib, None, &genre("Rock"), TrackSort::Longest),
        ["Bravo", "Alpha"]
    );
}

#[test]
fn the_legacy_helpers_still_return_the_historical_results() {
    let lib = fixture();
    let all: Vec<String> = lib
        .all_tracks()
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(
        all,
        ["Alpha", "Bravo", "Xray", "Yankee", "Zulu"],
        "all_tracks() keeps the historical artist/album order"
    );
    let found: Vec<String> = lib
        .search("zulu")
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(found, ["Zulu"]);
}

// ---------- facets for the chips ----------

#[test]
fn genres_skip_missing_and_blank_values() {
    let mut lib = fixture();
    lib.upsert_track(&track(
        "/m/blank.flac",
        "Blank",
        Some("   "),
        Some(2000),
        500,
        600,
    ))
    .expect("insert");
    assert_eq!(lib.genres().unwrap(), ["Jazz", "Metal", "Rock"]);
}

#[test]
fn genres_dedupe_spellings_that_differ_only_in_case() {
    let mut lib = fixture(); // already holds two `Rock` rows
    lib.upsert_track(&track(
        "/m/rock-lower.flac",
        "Rock lowercase",
        Some("rock"),
        Some(2000),
        500,
        700,
    ))
    .expect("insert");
    lib.upsert_track(&track(
        "/m/jazz-lower.flac",
        "Jazz lowercase",
        Some("jazz"),
        Some(2000),
        500,
        800,
    ))
    .expect("insert");
    // One chip per genre, not one per spelling: `Rock`/`rock` collapse to the
    // stored `Rock`, `Jazz`/`jazz` to `Jazz` (`.genres()` promises the
    // case-insensitive dedup).
    assert_eq!(lib.genres().unwrap(), ["Jazz", "Metal", "Rock"]);
}

#[test]
fn decades_skip_undated_and_zero_years_newest_first() {
    let mut lib = fixture();
    lib.upsert_track(&track(
        "/m/zero.flac",
        "Zero",
        Some("Rock"),
        Some(0),
        500,
        700,
    ))
    .expect("insert");
    assert_eq!(lib.decades().unwrap(), [1990, 1980]);
}

// ---------- migration ----------

#[test]
fn a_v3_database_gains_the_filter_indexes_and_keeps_its_rows() {
    let tmp = TempDir::new("migrate-v3");
    let path = tmp.path("iwaks.db").to_string_lossy().to_string();

    {
        let mut lib = Library::open(&path).expect("open");
        lib.upsert_track(&track(
            "/m/a.flac",
            "Kept",
            Some("Rock"),
            Some(1994),
            500,
            42,
        ))
        .expect("insert");
    }

    // Rebuild the file as an older build would leave it: rows present, but at
    // v3 without the filter indexes.
    {
        let lib = Library::open(&path).expect("reopen");
        lib.conn()
            .execute_batch(
                "DROP INDEX IF EXISTS idx_tracks_genre; DROP INDEX IF EXISTS idx_tracks_year;",
            )
            .expect("downgrade");
        lib.conn()
            .pragma_update(None, "user_version", 3)
            .expect("set v3");
    }

    let lib = Library::open(&path).expect("upgrade");
    let version: i32 = lib
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("user_version");
    assert_eq!(version, 4, "migrated forward");
    for index in ["idx_tracks_genre", "idx_tracks_year"] {
        let found: String = lib
            .conn()
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name=?1",
                [index],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("{index} missing: {e}"));
        assert_eq!(found, index);
    }
    assert_eq!(
        titles(&lib, None, &genre("Rock"), TrackSort::default()),
        ["Kept"],
        "rows survived the migration"
    );
}
