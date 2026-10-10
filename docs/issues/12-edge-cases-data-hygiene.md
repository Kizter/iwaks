---
title: "Edge cases: duplicate playlist names, path-key dedupe, seek-on-blur, m3u encoding"
labels: ["bug", "priority:P3", "area:polish"]
milestone: "v0.2.0"
status: "done"
---

## Summary

Four small, independent correctness gaps surfaced in the grill:

1. **Duplicate playlist names allowed** — `playlists.name` has no unique
   constraint, so two playlists can share a name and be indistinguishable
   in the grid/picker.
2. **Path-key dedupe is inconsistent** — scan lookups use `norm()`
   (lowercase, `/`⇄`\`), but upserts use the raw path with
   `ON CONFLICT(path)`; a case/separator-variant path can create a second
   row for the same physical file.
3. **Seek drag discarded on blur** — `onBlur` clears the pending drag
   before it is committed when focus leaves mid-drag.
4. **m3u import fails on non-UTF-8** — `read_to_string` rejects the
   latin-1/UTF-16 files that are common in the wild.

## Evidence

- `crates/library/src/playlists.rs:42-48` — `create_playlist` inserts any
  trimmed non-empty name; schema at `db.rs:62-66` has no `UNIQUE(name)`.
- `crates/library/src/scan.rs:157-159` — `norm()` key; `db.rs:13`
  (`UNIQUE(path)`), `db.rs:156-175` (`upsert_track` with
  `ON CONFLICT(path)` at `:164`, matching on the raw path).
- `src/PlayerBar.tsx:146-151, 246-259` — `commitSeek` on pointerup/keyup;
  `onBlur={() => setDrag(null)}` discards uncommitted drag.
- `crates/library/src/playlists.rs:229-236` — `std::fs::read_to_string`
  (no encoding fallback).

## Impact

Low frequency, distinct annoyances: ambiguous playlists; rare duplicate
rows; a lost seek when focus leaves mid-drag; occasional m3u import
failures for non-ASCII playlists.

## Suggested direction

1. Enforce unique playlist names (case-insensitive) with a friendly error
   on collision (migration + `clean_name` check).
2. Normalize the path **before** upsert (store the `norm()` form, or keep a
   normalized key column with the raw path for display).
3. Commit the seek on `pointercancel`/`lostpointercapture` too, or drop the
   blur-clears-drag behavior.
4. Read m3u with encoding sniffing (UTF-8, then latin-1; skip UTF-16 BOM
   handling) and verify resolved paths still match library rows.

## Definition of Done

- [x] Creating a duplicate-named playlist errors with clear copy
- [x] No second row for the same file regardless of path case/separator
- [x] Seek drag committed or safely cancelled on every exit path
- [x] m3u import succeeds for UTF-8 and latin-1 (tests for both)

## Outcome

1. **Unique playlist names** — `create_playlist` / `rename_playlist` reject a
   name already in use, case-insensitively, with
   `a playlist named "<name>" already exists`. Implemented as a guard query
   (`playlist_name_taken`) rather than a schema change: no migration, and the
   m3u importer routes through `create_playlist` too.
2. **Path-key dedupe** — `Library::upsert_track` now re-keys an existing row
   whose path differs only by case/separator (`rekey_normalized_path`) before
   the insert, so the row id and any playlist membership survive and no second
   row appears. No migration needed.
3. **Seek on blur** — the seek slider commits the pending value on `blur` and
   `pointercancel` (was: `onBlur` discarded the drag).
4. **m3u encoding** — `read_text_lossy` strips a UTF-8 BOM, keeps valid UTF-8,
   and falls back to latin-1 for legacy files instead of failing.

Tests: `upsert_dedupes_case_and_separator_path_variants`,
`upsert_rekey_keeps_playlist_membership` (library); `import_m3u_decodes_utf8_bom_and_latin1`,
`create_and_rename_reject_duplicate_names` (playlists). The seek fix is
frontend-only and has no runner in the repo.

## References

- Roadmap: checklist item M5
- Related: `docs/issues/02-scan-clean-missing-scope.md` (scan root semantics)