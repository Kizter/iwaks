//! Playlists (M4 slice 1): SQLite `playlists` + `playlist_tracks` CRUD and
//! `.m3u` import/export. Track order is stored per-entry in `position`.
//! The `(playlist_id, track_id)` primary key dedupes entries; foreign keys
//! (enabled at open) cascade playlist rows out when a track is pruned.

use std::path::Path;

use iwaks_core::track::Track;

use crate::db::{Library, LibraryError};

/// A playlist row with its current track count (0 when empty).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
}

/// Trimmed non-empty playlist name.
fn clean_name(name: &str) -> Result<String, LibraryError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(LibraryError::Invalid(
            "playlist name must not be empty".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

impl Library {
    fn playlist_exists(&self, id: i64) -> Result<bool, LibraryError> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE id = ?1)",
            [id],
            |r| r.get::<_, bool>(0),
        )?)
    }

    /// Create a playlist; returns its row id.
    pub fn create_playlist(&mut self, name: &str) -> Result<i64, LibraryError> {
        let name = clean_name(name)?;
        self.conn()
            .prepare("INSERT INTO playlists (name) VALUES (?1)")?
            .execute([name])?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Rename a playlist (trimmed, non-empty). Errors when it doesn't exist.
    pub fn rename_playlist(&mut self, id: i64, name: &str) -> Result<(), LibraryError> {
        let name = clean_name(name)?;
        let n = self
            .conn()
            .prepare("UPDATE playlists SET name = ?1 WHERE id = ?2")?
            .execute(rusqlite::params![name, id])?;
        if n == 0 {
            return Err(LibraryError::NotFound(format!("playlist {id}")));
        }
        Ok(())
    }

    /// Delete a playlist and its entries (idempotent — deleting a playlist
    /// that is already gone is a no-op success).
    pub fn delete_playlist(&mut self, id: i64) -> Result<(), LibraryError> {
        self.conn()
            .prepare("DELETE FROM playlists WHERE id = ?1")?
            .execute([id])?;
        Ok(())
    }

    /// Every playlist with its track count, alphabetical (case-insensitive).
    pub fn list_playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        let mut stmt = self.conn().prepare(
            "SELECT p.id, p.name, count(pt.track_id)
             FROM playlists p
             LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
             GROUP BY p.id
             ORDER BY lower(p.name)",
        )?;
        let rows = stmt.query_map([], playlist_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One playlist with its track count, or `None` when it doesn't exist.
    pub fn get_playlist(&self, id: i64) -> Result<Option<Playlist>, LibraryError> {
        let mut stmt = self.conn().prepare(
            "SELECT p.id, p.name, count(pt.track_id)
             FROM playlists p
             LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
             WHERE p.id = ?1
             GROUP BY p.id",
        )?;
        let mut rows = stmt.query_map([id], playlist_row)?;
        Ok(rows.next().transpose()?)
    }

    /// Tracks of a playlist in stored order, or `None` when it doesn't exist.
    pub fn get_playlist_tracks(&self, id: i64) -> Result<Option<Vec<Track>>, LibraryError> {
        if !self.playlist_exists(id)? {
            return Ok(None);
        }
        let mut stmt = self.conn().prepare(
            "SELECT t.id, t.path, t.title, t.artist, t.album, t.album_artist, t.genre,
                    t.year, t.track_no, t.disc_no, t.duration_ms, t.sample_rate, t.bit_depth,
                    t.bitrate, t.format, t.file_size, t.modified_at
             FROM playlist_tracks pt
             JOIN tracks t ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position",
        )?;
        let rows = stmt.query_map([id], crate::db::row_to_track)?;
        Ok(Some(rows.collect::<rusqlite::Result<Vec<_>>>()?))
    }

    /// Append one track entry (no duplicates). Errors when the playlist
    /// doesn't exist.
    pub fn add_track_to_playlist(
        &mut self,
        playlist_id: i64,
        track_id: i64,
    ) -> Result<(), LibraryError> {
        if !self.playlist_exists(playlist_id)? {
            return Err(LibraryError::NotFound(format!("playlist {playlist_id}")));
        }
        self.conn().execute(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, (SELECT coalesce(max(position), -1) + 1
                              FROM playlist_tracks WHERE playlist_id = ?1))",
            rusqlite::params![playlist_id, track_id],
        )?;
        Ok(())
    }

    /// Append every `track_id` in order (no duplicates) in **one transaction**
    /// — used when saving the session queue as a playlist. Returns how many
    /// entries were actually inserted. Errors when the playlist doesn't
    /// exist; a failing entry (e.g. unknown track id → FK) rolls the whole
    /// batch back, so nothing half-applies.
    pub fn add_tracks_to_playlist(
        &mut self,
        playlist_id: i64,
        track_ids: &[i64],
    ) -> Result<usize, LibraryError> {
        if !self.playlist_exists(playlist_id)? {
            return Err(LibraryError::NotFound(format!("playlist {playlist_id}")));
        }
        if track_ids.is_empty() {
            return Ok(0);
        }
        let tx = self.conn_mut().unchecked_transaction()?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
                 VALUES (?1, ?2, (SELECT coalesce(max(position), -1) + 1
                                  FROM playlist_tracks WHERE playlist_id = ?1))",
            )?;
            for track_id in track_ids {
                inserted += stmt.execute(rusqlite::params![playlist_id, track_id])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Remove one track entry from a playlist (idempotent).
    pub fn remove_track_from_playlist(
        &mut self,
        playlist_id: i64,
        track_id: i64,
    ) -> Result<(), LibraryError> {
        self.conn().execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            rusqlite::params![playlist_id, track_id],
        )?;
        Ok(())
    }

    /// Replace the playlist's order with `ordered_ids` (reorders and drops
    /// entries not listed) in one transaction.
    pub fn reorder_playlist(
        &mut self,
        playlist_id: i64,
        ordered_ids: &[i64],
    ) -> Result<(), LibraryError> {
        if !self.playlist_exists(playlist_id)? {
            return Err(LibraryError::NotFound(format!("playlist {playlist_id}")));
        }
        let tx = self.conn_mut().unchecked_transaction()?;
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position)
                 VALUES (?1, ?2, ?3)",
            )?;
            for (pos, track_id) in ordered_ids.iter().enumerate() {
                stmt.execute(rusqlite::params![playlist_id, track_id, pos as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// `.m3u` content for a playlist, or `None` when it doesn't exist.
    pub fn export_m3u(&self, id: i64) -> Result<Option<String>, LibraryError> {
        let Some(tracks) = self.get_playlist_tracks(id)? else {
            return Ok(None);
        };
        let mut out = String::from("#EXTM3U\n");
        for t in tracks {
            out.push_str(&format!(
                "#EXTINF:{},{}\n{}\n",
                t.duration_ms / 1000,
                t.title,
                t.path
            ));
        }
        Ok(Some(out))
    }

    /// Import an `.m3u` file as a new playlist named after the file stem.
    /// Lines that don't resolve to a library track (missing file, relative
    /// path outside the file's folder, unsupported entry) are skipped.
    /// Returns the new playlist id.
    pub fn import_m3u(&mut self, path: &Path) -> Result<i64, LibraryError> {
        let content = std::fs::read_to_string(path)?;
        let base = path.parent().unwrap_or(Path::new("."));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Imported");
        let playlist_id = self.create_playlist(name)?;

        let mut found = Vec::new();
        {
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM tracks WHERE lower(replace(path, '\\', '/')) = ?1")?;
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let entry = std::path::PathBuf::from(line);
                let resolved = if entry.is_absolute() {
                    entry
                } else {
                    base.join(entry)
                };
                let key = crate::scan::norm(&resolved.to_string_lossy());
                if let Ok(track_id) = stmt.query_row([&key], |r| r.get::<_, i64>(0)) {
                    found.push(track_id);
                }
            }
        }
        for track_id in found {
            self.add_track_to_playlist(playlist_id, track_id)?;
        }
        // Keep the source path for provenance (informational).
        self.conn()
            .prepare("UPDATE playlists SET m3u_path = ?1 WHERE id = ?2")?
            .execute(rusqlite::params![
                path.to_string_lossy().as_ref(),
                playlist_id
            ])?;
        Ok(playlist_id)
    }
}

fn playlist_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Playlist> {
    Ok(Playlist {
        id: row.get(0)?,
        name: row.get(1)?,
        track_count: row.get(2)?,
    })
}
