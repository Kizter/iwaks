//! SQLite storage for the track library.

use std::path::Path;

use iwaks_core::track::Track;
use rusqlite::Connection;

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
    #[error("unsupported schema version {0} (this build supports v2)")]
    UnsupportedSchema(i32),
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
                conn.pragma_update(None, "user_version", 2)?;
            }
            1 => {
                conn.execute_batch(SCHEMA_V2)?;
                conn.pragma_update(None, "user_version", 2)?;
            }
            2 => {}
            other => return Err(LibraryError::UnsupportedSchema(other)),
        }
        // Cascade playlist entries / playlist rows when a track or playlist
        // is deleted from the library (scan pruning, playlist removal).
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Library { conn })
    }

    /// Insert or update a track by its unique `path`. Returns the row id.
    pub fn upsert_track(&mut self, t: &Track) -> Result<i64, LibraryError> {
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

    /// All tracks, sorted by artist → album → title (case-insensitive).
    pub fn all_tracks(&self) -> Result<Vec<Track>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, title, artist, album, album_artist, genre, year, track_no,
                    disc_no, duration_ms, sample_rate, bit_depth, bitrate, format,
                    file_size, modified_at
             FROM tracks
             ORDER BY lower(coalesce(artist, '')), lower(coalesce(album, '')),
                      track_no IS NULL, track_no, lower(title)",
        )?;
        let rows = stmt.query_map([], row_to_track)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Full-text search over title/artist/album. `query` is free-text user
    /// input; an empty/meaningless query yields an empty result (callers
    /// fall back to `all_tracks` when the input is blank).
    pub fn search(&self, query: &str) -> Result<Vec<Track>, LibraryError> {
        let Some(fts) = iwaks_core::fts::build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.path, t.title, t.artist, t.album, t.album_artist, t.genre,
                    t.year, t.track_no, t.disc_no, t.duration_ms, t.sample_rate, t.bit_depth,
                    t.bitrate, t.format, t.file_size, t.modified_at
             FROM tracks_fts f
             JOIN tracks t ON t.id = f.rowid
             WHERE tracks_fts MATCH ?1
             ORDER BY rank",
        )?;
        let rows = stmt.query_map([fts], row_to_track)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
