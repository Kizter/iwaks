//! SQLite library: schema, FTS5 search, incremental scanner, playlists.
//! M1 scope: tracks table + tracks_fts, scan, basic queries.
//! M4 slice 1: playlists + playlist_tracks + `.m3u` import/export.

pub mod db;
pub mod playlists;
pub mod scan;
