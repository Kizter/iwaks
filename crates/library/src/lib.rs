//! SQLite library: schema, FTS5 search, incremental scanner.
//! M1 scope: tracks table + tracks_fts, scan, basic queries.
//! (albums/artists join tables land with the browse UI in M4.)

pub mod db;
pub mod scan;
