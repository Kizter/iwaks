//! Durable state for the album-art feature: a generic key/value settings
//! store, plus a cache of album-art lookups so playback does not re-ask the
//! provider for every track of an album.
//!
//! The cache stores *negative* results too (`CoverLookup::Missing`). Without
//! that, a library of local releases the provider has never heard of would
//! re-query on every single track change. Callers must only record an outcome
//! the provider actually returned — never a transport failure — otherwise one
//! offline minute would be cached forever.

use crate::db::{Library, LibraryError};

/// What the provider said about one album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverLookup {
    /// A cover exists at this URL, which Discord's servers can fetch.
    Found(String),
    /// The provider has no matching release. Do not ask again.
    Missing,
}

impl Library {
    /// Read a stored setting, `None` when the key was never written.
    pub fn setting(&self, key: &str) -> Result<Option<String>, LibraryError> {
        let mut stmt = self
            .conn()
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
    }

    /// Write a setting, replacing any previous value for that key.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), LibraryError> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// The remembered outcome for `album_key`: `None` = never looked up, so
    /// ask the provider.
    pub fn cached_cover(&self, album_key: &str) -> Result<Option<CoverLookup>, LibraryError> {
        let mut stmt = self
            .conn()
            .prepare("SELECT artwork_url FROM cover_cache WHERE album_key = ?1")?;
        let mut rows = stmt.query([album_key])?;
        Ok(rows.next()?.map(cover_row).transpose()?)
    }

    /// Remember a lookup outcome. A blank key is ignored: it means the track
    /// has no usable album tags, and caching it would merge every untagged
    /// track onto one row.
    pub fn store_cover(&self, album_key: &str, lookup: &CoverLookup) -> Result<(), LibraryError> {
        if album_key.is_empty() {
            return Ok(());
        }
        let url = match lookup {
            CoverLookup::Found(url) => Some(url),
            CoverLookup::Missing => None,
        };
        self.conn().execute(
            "INSERT INTO cover_cache (album_key, artwork_url) VALUES (?1, ?2)
             ON CONFLICT(album_key) DO UPDATE SET artwork_url = excluded.artwork_url",
            rusqlite::params![album_key, url],
        )?;
        Ok(())
    }
}

/// Row → outcome. A NULL `artwork_url` is the negative-result marker.
fn cover_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CoverLookup> {
    Ok(match row.get::<_, Option<String>>(0)? {
        Some(url) => CoverLookup::Found(url),
        None => CoverLookup::Missing,
    })
}
