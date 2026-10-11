---
title: "Global keyboard shortcuts: seek, track jump, focus search"
labels: ["enhancement", "priority:P3", "area:a11y"]
milestone: "v0.3.0"
status: "done"
---

## Summary

`seek_relative` existed in the backend with no frontend caller. Wire it into a
global shortcut set (seek, track jump, focus search) and document every
shortcut in Settings.

## Motivation

Power users drive a player from the keyboard. Only Space, N, Escape and the
queue reorder existed; the seek command shipped dead.

## Scope

- `←` / `→` seek ∓10 s through the existing `playerSeekRelative`
- `Shift` + `←` / `→` previous / next track
- `/` focuses (and selects) the search box
- A keyboard-shortcuts group in Settings so the keys are discoverable

## Acceptance criteria

- [x] Seek and track-jump shortcuts work through the existing commands
- [x] `/` focuses search; no-op on views where search is hidden
- [x] No shortcut fires while typing in a field
- [x] `Alt` + arrows still belong to the queue reorder
- [x] All shortcuts documented in Settings
- [x] `npx tsc --noEmit` and `npm run build` clean

## Outcome

`seek_relative` now has its first caller. The handler lives beside the existing
Space and Now Playing handlers and shares their guards: it returns early for
`INPUT`/`TEXTAREA`/`contentEditable`, and ignores `Ctrl`/`Meta`/`Alt` so the
queue's `Alt`+arrow reorder and any browser/OS combo keep working.

`/` resolves the input through the DOM (`document.querySelector(".search
input")`) instead of a captured ref, so it stays correct as the toolbar
re-renders between views — and on the Queue and Playlists grid, where the box is
hidden by issue #16, the missing input is simply a no-op rather than a stale
focus call.

Settings gained a "Keyboard shortcuts" group listing all seven bindings with
`<kbd>` chips, so nothing here depends on discovery by accident.

`npx tsc --noEmit` and `npm run build` are clean. No Rust change.

## References

- `src/App.tsx` (`SEEK_STEP`, transport shortcut effect)
- `src/Settings.tsx` (`SHORTCUTS`, shortcuts group)
- `src/api.ts` (`playerSeekRelative`, previously unused)
- `src/App.css` (`kbd`, `.set-keys`)