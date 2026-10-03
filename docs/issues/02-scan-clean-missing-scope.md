---
title: "Scope `clean_missing` prune to the scanned root; never prune on IO errors"
labels: ["bug", "priority:P1", "area:scan"]
milestone: "v0.2.0"
status: "draft"
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

- [ ] Prune only rows under the scanned root (or genuinely vanished files —
      decide explicitly in the issue thread)
- [ ] IO-errrored paths are preserved (simulate missing drive / deny access)
- [ ] Multi-root accumulation still works (scan A then B keeps both)
- [ ] Tests cover all three cases (≥80% area coverage)

## References

- Roadmap: checklist item M2
- Related: `docs/issues/12-edge-cases-data-hygiene.md` (path-key dedupe)