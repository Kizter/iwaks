//! SQLite storage for the track library.

use std::path::Path;

use iwaks_core::track::Track;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Schema v1: denormalized tracks table + FTS5 external-content index.
/// Albums/artists stay frontend-derived (grouping) — no join tables needed.
const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS tracks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    path         TEXT    NOT NULL UNIQUE,
    title        TEXT    NOT NULL,
    artist       TEXT,
    album        TEXT,
    album_artist TEXT,
    genre        TEXT,
    year         INTEGER,
    track_no     INTEGER,
    disc_no      INTEGER,
    duration_ms  INTEGER NOT NULL DEFAULT 0,
    sample_rate  INTEGER,
    bit_depth    INTEGER,
    bitrate      INTEGER,
    format       TEXT    NOT NULL,
    file_size    INTEGER NOT NULL DEFAULT 0,
    modified_at  INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album);

CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
    title, artist, album,
    content = 'tracks',
    content_rowid = 'id'
);

CREATE TRIGGER IF NOT EXISTS tracks_ai AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts(rowid, title, artist, album)
    VALUES (new.id, new.title, coalesce(new.artist, ''), coalesce(new.album, ''));
END;

CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
    INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album)
    VALUES ('delete', old.id, old.title, coalesce(old.artist, ''), coalesce(old.album, ''));
END;

CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
    INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album)
    VALUES ('delete', old.id, old.title, coalesce(old.artist, ''), coalesce(old.album, ''));
    INSERT INTO tracks_fts(rowid, title, artist, album)
    VALUES (new.id, new.title, coalesce(new.artist, ''), coalesce(new.album, ''));
END;
";

/// Schema v2: playlists (M4 slice 1). Track order lives in
/// `playlist_tracks.position`; the (playlist_id, track_id) primary key
/// dedupes entries. Foreign keys cascade deletes both ways.
const SCHEMA_V2: &str = "
CREATE TABLE IF NOT EXISTS playlists (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    name     TEXT    NOT NULL,
    m3u_path TEXT
);

CREATE TABLE IF NOT EXISTS playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_playlist_tracks_pos ON playlist_tracks(playlist_id, position);
";

/// Schema v3: app settings + the album-art lookup cache.
///
/// `settings` is a plain key/value store so features can persist their own
/// flags without a migration each; values are text (`"true"` / `"false"`).
/// `cover_cache` stores a NULL `artwork_url` to remember "the provider has no
/// such album" so the lookup is not retried on every track.
const SCHEMA_V3: &str = "
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cover_cache (
    album_key   TEXT PRIMARY KEY,
    artwork_url TEXT
);
";

/// Schema v4: indexes backing the genre / decade filters (issue #20).
///
/// Purely additive — every existing database keeps its rows; the filters only
/// become usable because `genre` and `year` can now be narrowed by index
/// instead of scanned.
const SCHEMA_V4: &str = "
CREATE INDEX IF NOT EXISTS idx_tracks_genre ON tracks(genre);
CREATE INDEX IF NOT EXISTS idx_tracks_year  ON tracks(year);
";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unsupported schema version {0} (this build supports v4)")]
    UnsupportedSchema(i32),
}

/// Optional narrowing applied to every track query (issue #20).
///
/// Both fields are `None` for "no narrowing", which is what every existing
/// caller gets. `genre` is matched against the exact spelling the library
/// stores, so the predicate stays a plain equality and can use its index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackFilter {
    pub genre: Option<String>,
    /// Inclusive start of a decade — `Some(1990)` matches 1990–1999.
    pub decade: Option<i64>,
}

/// Order the track list is returned in (issue #21).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackSort {
    /// Artist → album → track number → title (the historical order).
    #[default]
    ArtistAlbum,
    Title,
    /// Most recently modified file first.
    Added,
    Longest,
}

/// Column list shared by every track query.
const TRACK_COLUMNS: &str = "id, path, title, artist, album, album_artist, genre, year, \
     track_no, disc_no, duration_ms, sample_rate, bit_depth, bitrate, format, \
     file_size, modified_at";
/// Same list for the FTS join, where the bare names would be ambiguous.
const TRACK_COLUMNS_T: &str = "t.id, t.path, t.title, t.artist, t.album, t.album_artist, \
     t.genre, t.year, t.track_no, t.disc_no, t.duration_ms, t.sample_rate, t.bit_depth, \
     t.bitrate, t.format, t.file_size, t.modified_at";
/// The browse order, kept identical to the pre-v4 `all_tracks` query.
const ORDER_ARTIST_ALBUM: &str =
    "lower(coalesce(artist, '')), lower(coalesce(album, '')), track_no IS NULL, \
     track_no, lower(title)";

impl TrackSort {
    /// `ORDER BY` body, `alias`-prefixed for the FTS join (where bare column
    /// names would be ambiguous against `tracks_fts`).
    fn order_by(self, alias: &str) -> String {
        let col = |name: &str| {
            if alias.is_empty() {
                name.to_string()
            } else {
                format!("{alias}.{name}")
            }
        };
        match self {
            Self::ArtistAlbum => {
                if alias.is_empty() {
                    ORDER_ARTIST_ALBUM.to_string()
                } else {
                    format!(
                        "lower(coalesce({}, '')), lower(coalesce({}, '')), {} IS NULL, {}, lower({})",
                        col("artist"),
                        col("album"),
                        col("track_no"),
                        col("track_no"),
                        col("title")
                    )
                }
            }
            Self::Title => format!("lower({})", col("title")),
            Self::Added => format!("{} DESC, lower({})", col("modified_at"), col("title")),
            Self::Longest => format!("{} DESC, lower({})", col("duration_ms"), col("title")),
        }
    }
}

impl TrackFilter {
    /// Append this filter's predicates (and their bound values) to a query
    /// under construction. Values are always bound, never interpolated.
    fn push_sql(&self, clauses: &mut Vec<String>, params: &mut Vec<Box<dyn rusqlite::ToSql>>) {
        if let Some(genre) = &self.genre {
            params.push(Box::new(genre.clone()));
            clauses.push(format!("genre = ?{}", params.len()));
        }
        if let Some(decade) = self.decade {
            let first = params.len() + 1;
            params.push(Box::new(decade));
            params.push(Box::new(decade + 10));
            clauses.push(format!("year >= ?{first} AND year < ?{}", first + 1));
        }
    }
}

/// A single SQLite connection to the library database (WAL mode).
/// Not `Sync` — open per thread / per call as needed.
#[derive(Debug)]
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open (creating + migrating) the library at `path`.
    /// Pass `":memory:"` for an ephemeral database (tests).
    pub fn open(path: &str) -> Result<Self, LibraryError> {
        if path != ":memory:" {
            let dir = Path::new(path).parent().unwrap_or(Path::new("."));
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;

        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        match version {
            0 => {
                conn.execute_batch(SCHEMA_V1)?;
                conn.execute_batch(SCHEMA_V2)?;
                conn.execute_batch(SCHEMA_V3)?;
                conn.execute_batch(SCHEMA_V4)?;
                conn.pragma_update(None, "user_version", 4)?;
            }
            1 => {
                conn.execute_batch(SCHEMA_V2)?;
                conn.execute_batch(SCHEMA_V3)?;
                conn.execute_batch(SCHEMA_V4)?;
                conn.pragma_update(None, "user_version", 4)?;
            }
            2 => {
                conn.execute_batch(SCHEMA_V3)?;
                conn.execute_batch(SCHEMA_V4)?;
                conn.pragma_update(None, "user_version", 4)?;
            }
            3 => {
                conn.execute_batch(SCHEMA_V4)?;
                conn.pragma_update(None, "user_version", 4)?;
            }
            4 => {}
            other => return Err(LibraryError::UnsupportedSchema(other)),
        }
        // Cascade playlist entries / playlist rows when a track or playlist
        // is deleted from the library (scan pruning, playlist removal).
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Library { conn })
    }

    /// Insert or update a track by its unique `path`. Returns the row id.
    ///
    /// Uniqueness is decided on the *normalized* path (lowercase, `/`⇄`\`), so
    /// a path that differs from the stored one only by case or separator
    /// updates the same row instead of inserting a duplicate.
    pub fn upsert_track(&mut self, t: &Track) -> Result<i64, LibraryError> {
        self.rekey_normalized_path(&t.path)?;
        self.conn
            .prepare(
                "INSERT INTO tracks
                    (path, title, artist, album, album_artist, genre, year, track_no,
                     disc_no, duration_ms, sample_rate, bit_depth, bitrate, format,
                     file_size, modified_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT(path) DO UPDATE SET
                     title = excluded.title, artist = excluded.artist,
                     album = excluded.album, album_artist = excluded.album_artist,
                     genre = excluded.genre, year = excluded.year,
                     track_no = excluded.track_no, disc_no = excluded.disc_no,
                     duration_ms = excluded.duration_ms, sample_rate = excluded.sample_rate,
                     bit_depth = excluded.bit_depth, bitrate = excluded.bitrate,
                     format = excluded.format, file_size = excluded.file_size,
                     modified_at = excluded.modified_at
                 RETURNING id",
            )?
            .query_row(
                rusqlite::params![
                    t.path,
                    t.title,
                    t.artist,
                    t.album,
                    t.album_artist,
                    t.genre,
                    t.year,
                    t.track_no,
                    t.disc_no,
                    t.duration_ms,
                    t.sample_rate,
                    t.bit_depth,
                    t.bitrate,
                    t.format,
                    t.file_size,
                    t.modified_at
                ],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    /// Move a stored row onto `path` when it already exists under a path that
    /// differs only by case or separator. Keeps the row id (and therefore any
    /// playlist membership) intact, so the following upsert updates it rather
    /// than inserting a duplicate.
    fn rekey_normalized_path(&mut self, path: &str) -> Result<(), LibraryError> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM tracks
                 WHERE lower(replace(path, '\\', '/')) = lower(replace(?1, '\\', '/'))
                   AND path <> ?1",
                [path],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            self.conn
                .prepare("UPDATE tracks SET path = ?1 WHERE id = ?2")?
                .execute(rusqlite::params![path, id])?;
        }
        Ok(())
    }

    /// All tracks, sorted by artist → album → title (case-insensitive).
    pub fn all_tracks(&self) -> Result<Vec<Track>, LibraryError> {
        self.tracks_filtered(None, &TrackFilter::default(), TrackSort::default())
    }

    /// Tracks narrowed by `filter` and ordered by `sort`; `query` switches
    /// from the plain listing to FTS5 full-text search.
    ///
    /// Every value is bound, never interpolated, so a genre with a quote in it
    /// is just data. With no filter and the default sort this is exactly the
    /// historical `all_tracks` / `search` behaviour, including keeping FTS
    /// relevance as the order whenever the user has not asked for one.
    pub fn tracks_filtered(
        &self,
        query: Option<&str>,
        filter: &TrackFilter,
        sort: TrackSort,
    ) -> Result<Vec<Track>, LibraryError> {
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        let (from, columns, alias) = match query {
            Some(q) => {
                // A query that FTS cannot express (punctuation only) matches
                // nothing, exactly like the old `search` did.
                let Some(fts) = iwaks_core::fts::build_fts_query(q) else {
                    return Ok(Vec::new());
                };
                params.push(Box::new(fts));
                clauses.push("tracks_fts MATCH ?1".to_string());
                (
                    "tracks_fts f JOIN tracks t ON t.id = f.rowid",
                    TRACK_COLUMNS_T,
                    "t",
                )
            }
            None => ("tracks", TRACK_COLUMNS, ""),
        };
        filter.push_sql(&mut clauses, &mut params);

        let mut sql = format!("SELECT {columns} FROM {from}");
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY ");
        // Relevance wins only while the user has not picked an order.
        if query.is_some() && sort == TrackSort::default() {
            sql.push_str("rank");
        } else {
            sql.push_str(&sort.order_by(alias));
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), row_to_track)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every genre present in the library, for the filter chips: distinct,
    /// blanks and `NULL` excluded, case-insensitively deduped on the spelling
    /// that is actually stored (so the value can be bound back as-is).
    pub fn genres(&self) -> Result<Vec<String>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT genre FROM tracks
             WHERE genre IS NOT NULL AND trim(genre) <> ''
             ORDER BY lower(genre)",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Decades that hold at least one dated track, newest first.
    pub fn decades(&self) -> Result<Vec<i64>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT (year / 10) * 10 FROM tracks
             WHERE year IS NOT NULL AND year > 0
             ORDER BY 1 DESC",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Full-text search over title/artist/album. `query` is free-text user
    /// input; an empty/meaningless query yields an empty result (callers
    /// fall back to `all_tracks` when the input is blank).
    pub fn search(&self, query: &str) -> Result<Vec<Track>, LibraryError> {
        self.tracks_filtered(Some(query), &TrackFilter::default(), TrackSort::default())
    }

    /// Every stored (original path, modified_at, file_size) triple.
    pub fn existing_files(&self) -> Result<Vec<(String, i64, i64)>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, modified_at, file_size FROM tracks")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Delete tracks by exact stored path. Returns number deleted.
    pub fn delete_tracks(&mut self, paths: &[String]) -> Result<usize, LibraryError> {
        let mut stmt = self.conn.prepare("DELETE FROM tracks WHERE path = ?1")?;
        let mut deleted = 0;
        for p in paths {
            deleted += stmt.execute([p])?;
        }
        Ok(deleted)
    }

    pub fn track_count(&self) -> Result<i64, LibraryError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))?)
    }

    /// Low-level access for power tooling and tests (e.g. schema checks).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Mutable low-level access (crate-internal — used by playlist
    /// transactions today).
    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

pub(crate) fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        path: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_artist: row.get(5)?,
        genre: row.get(6)?,
        year: row.get(7)?,
        track_no: row.get(8)?,
        disc_no: row.get(9)?,
        duration_ms: row.get(10)?,
        sample_rate: row.get(11)?,
        bit_depth: row.get(12)?,
        bitrate: row.get(13)?,
        format: row.get(14)?,
        file_size: row.get(15)?,
        modified_at: row.get(16)?,
    })
}
