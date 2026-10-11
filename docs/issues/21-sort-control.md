---
title: "Sort control for the track list"
labels: ["enhancement", "priority:P3", "area:library"]
milestone: "v0.3.0"
status: "done"
---

## Summary

Track order was hard-coded to artist → album → title. Add a sort control with
that order plus title, newest first, and longest first.

## Motivation

"Newest first" is how people check what a scan just added; "longest first" is
how you find the long mixes. Neither was reachable, and no header was
clickable.

## Scope

- `TrackSort` in the library crate, threaded through the same query builder as
  the filter (issue #20)
- A select in the filter bar
- Sorting composes with search and with the genre/decade filters

## Acceptance criteria

- [x] Four orders, all deterministic
- [x] The historical artist → album order is the default and is unchanged
- [x] Search keeps FTS relevance until an explicit sort is chosen
- [x] Sort composes with the filters from #20
- [x] New tests; full gates green

## Outcome

`TrackSort` is a serde enum (`artistAlbum` | `title` | `added` | `longest`)
passed alongside `TrackFilter`, so one query builder serves browsing, searching,
filtering and ordering — there is no second code path that could sort
differently.

- **Relevance is only a default** — with no explicit sort, a search still comes
  back in FTS rank order, which is what search has always done; choosing a sort
  takes over from relevance. A test asserts the default path returns exactly
  what `search()` returns, so this cannot silently change search behaviour.
- **Deterministic ties** — every order ends with `lower(title)`, so tracks with
  the same artist/album/duration have a stable position instead of shuffling
  between refetches.
- **Column prefixing** — the FTS join makes bare column names ambiguous
  (`tracks_fts` also has `title`/`artist`/`album`), so `order_by` prefixes each
  column with the table alias and has one shared implementation for both
  paths. That is the one non-obvious piece of this change: `ArtistAlbum` used
  to exist as a single string constant and would have been ambiguous the moment
  it was reused for a joined query.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (300 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`.

## References

- `crates/library/src/db.rs` (`TrackSort`, `order_by`, `tracks_filtered`)
- `crates/library/tests/filters.rs`
- `src/App.tsx` (`SORT_OPTIONS`, filter bar), `src/types.ts`