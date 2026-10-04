---
title: "Correct `.m4a` format badge (AAC ≠ ALAC) and stop leaking raw backend errors"
labels: ["bug", "priority:P2", "area:copy"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Two copy/accuracy issues:

1. **Badge mislabels lossy files as lossless.** `formatBadge` maps every
   `.m4a` to "ALAC". `.m4a` is a container — most files in it are AAC
   (lossy) — so a music player built around hi-res/lossless incorrectly
   advertises AAC as lossless.
2. **Raw backend errors leak into the UI.** Strings like
   `"Playback is unavailable — libmpv DLL missing or failed to initialize"`
   and production error strings (e.g. `"database error: …"`) surface
   directly in banners/popovers instead of user-friendly copy.

## Evidence

- `src/format.ts:26` — `m4a: "ALAC"` in the badge map.
- `src-tauri/src/lib.rs:37-38` — `PLAYER_UNAVAILABLE` message with
  implementation detail ("libmpv DLL missing or failed to initialize").
- `src/App.tsx:1127` — banner renders `String(e)` from backend errors;
  other `catch` paths do the same.

## Impact

Misinformation for a core selling point (lossless), and implementation
leaks that read as broken to end users.

## Suggested direction

1. Badge: label container as `M4A`, or better, derive from codec
   (ALAC/AAC) at scan time; keep the 2-3 visible letters.
2. Map known error categories to friendly messages at the API boundary
   (frontend `api.ts` or Tauri command layer); keep full detail in logs.
3. Centralize copy strings (single source of truth).

## Definition of Done

- [ ] AAC-in-`.m4a` shows a lossy-correct label; ALAC shows ALAC
- [ ] Sample test for the badge function (pure — easy TDD)
- [ ] No raw `String(e)` in user-facing UI for known error classes
- [ ] Sample friendly error copy for: playback unavailable, scan failed,
      tag-write failed

## References

- Roadmap: checklist item M4