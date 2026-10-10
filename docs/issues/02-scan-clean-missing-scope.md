---
title: "Scope `clean_missing` prune to the scanned root; never prune on IO errors"
labels: ["bug", "priority:P1", "area:scan"]
milestone: "v0.2.0"
status: "done"
---

## Summary

`clean_missing` checks **every** stored track (all roots), not only files
under the folder being scanned, and prunes anything whose
`Path::exists()` returns false. Because `exists()` also returns false for
IO errors — unmounted drive, transient permission failure, network path
down — a single scan can delete library rows for tracks that still exist.

## Evidence

- `crates/library/src/scan.rs:95-106` — prune loop iterates the full
  `existing` map keyed from **all** DB rows and deletes when
  `!Path::new(p).exists()`.
- `src-tauri/src/lib.rs` — `scan_folder` invokes `scan()` with
  `clean_missing: true`.
- `crates/library/src/scan.rs:163-169` — `collect_audio_files` silently
  drops walk errors (`filter_map(Result::ok)`), so an unreachable root
  yields zero files while prune still runs.

## Impact

Scanning while a music drive is unplugged (or a folder is briefly
inaccessible) empties the library view for that drive. Files on disk are
**not** deleted — a later rescan restores rows — but the "my library is
gone" experience is alarming and the DB churn is real.

## Suggested direction

1. Scope prune to files *below `opts.root`* (and genuinely missing), keeping
   the multi-root accumulation behavior intact.
2. Distinguish "file is gone" from "file metadata errored": treat
   `exists()`/`metadata()` IO errors as *preserve*, not prune.
3. Add a regression test: scan of root A must not prune rows belonging to
   root B, and must not prune rows whose `metadata()` fails on IO.

## Definition of Done

- [x] Prune only rows under the scanned root (or genuinely vanished files —
      decide explicitly in the issue thread)
- [x] IO-errrored paths are preserved (simulate missing drive / deny access)
- [x] Multi-root accumulation still works (scan A then B keeps both)
- [x] Tests cover all three cases (≥80% area coverage)

## Outcome

Decision: prune a row only when **both** hold — it lives *below `opts.root`*
(`under_root`) **and** its `metadata()` fails with `NotFound` (`io_says_gone`).
Any other IO error (permission denied, transient) preserves the row, and an
unreachable root (`root_is_reachable`) skips pruning entirely so an unplugged
drive never empties the library.

- `crates/library/src/scan.rs` — scoped + IO-safe prune; unit tests for
  `under_root` / `io_says_gone`.
- `crates/library/tests/library.rs` — `scan_of_another_root_*` reworked so a
  deleted file under root A is pruned by root A's scan, not root B's; new
  `scan_of_unreachable_root_preserves_rows` covers the offline-drive case.

## References

- Roadmap: checklist item M2
- Related: `docs/issues/12-edge-cases-data-hygiene.md` (path-key dedupe)