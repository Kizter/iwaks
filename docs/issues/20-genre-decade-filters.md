---
title: "Genre and decade filters"
labels: ["enhancement", "priority:P2", "area:library"]
milestone: "v0.3.0"
status: "done"
---

## Summary

`design.md` promises "search + filter"; only free-text search existed. Add
genre and decade filters, backed by indexes so they narrow by index rather than
scanning the table.

## Motivation

Browsing a large library by typing is fine for finding one song and useless for
answering "what do I have from the 90s?".

## Scope

- `TrackFilter { genre, decade }` in the library crate + `tracks_filtered`
- Schema v4: indexes on `genre` and `year`
- Facet queries `genres()` / `decades()` for the chips
- Filter bar above the track list; filters compose with the text query

## Acceptance criteria

- [x] Genre and decade filters narrow the list
- [x] Filters combine with each other and with the search box
- [x] Undated tracks never match a decade; blank genres are not offered
- [x] Filter values are bound, never interpolated into SQL
- [x] An existing database migrates forward and keeps its rows
- [x] New tests; `cargo fmt`/`clippy`/`test` and frontend gates clean

## Outcome

- **One query path** — `tracks_filtered(query, filter, sort)` assembles the
  `SELECT`/`WHERE`/`ORDER BY` in one place, so browsing and searching cannot
  drift apart; `all_tracks()` and `search()` are now one-line wrappers over it
  and every existing caller and test behaves exactly as before. Every value is
  a bound parameter, which a test pins down with a genre containing quotes and
  a `DROP TABLE`.
- **Indexable predicates** — genre is compared to the exact spelling the
  library stores (the chips hand back `SELECT DISTINCT genre`), so the predicate
  stays a plain equality and uses the new index; a decade becomes a `year >=
  ? AND year < ?` range. Schema v4 is additive: two `CREATE INDEX IF NOT
  EXISTS`, no table is touched.
- **The migration test earned its keep** — the first version of the match arms
  handled `0..=3` and rejected anything else, so a database already migrated by
  a previous run could not be reopened. The v3→v4 test caught it before it
  shipped; a `4 => {}` arm now makes "already current" a no-op.
- **Facet honesty** — `genres()` drops `NULL` and blank values, `decades()`
  drops `NULL` and `year = 0`, and both are computed from the *whole* library,
  so the chips never shrink down to the one option currently selected.
- **UI** — a filter bar under the toolbar (hidden on the Queue and Playlists
  grid, where nothing would narrow), with "Clear filters" when anything is
  active. An active filter that matches nothing now shows the no-results panel
  with a way out, instead of a silently blank list.

19 new tests in `crates/library/tests/filters.rs` (default/genre/decade/
combined/unknown/case-sensitive/SQL-injection, decade boundaries, search
composition, each facet query, and the migration).

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (300 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`. No new dependencies.

## References

- `crates/library/src/db.rs` (`SCHEMA_V4`, `TrackFilter`, `tracks_filtered`,
  `genres`, `decades`)
- `crates/library/tests/filters.rs`
- `src-tauri/src/lib.rs` (`get_tracks`, `search_tracks`, `list_genres`,
  `list_decades`)
- `src/App.tsx` (filter bar), `src/api.ts`, `src/types.ts`, `src/App.css`