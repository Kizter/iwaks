---
title: "Playlist reorder in the UI"
labels: ["bug", "priority:P3", "area:ux"]
milestone: "v0.3.0"
status: "done"
---

## Summary

`reorder_playlist` shipped as a working Tauri command with no frontend caller —
playlist order could only be set by the order tracks were added. Wire it into
the playlist-detail list.

## Motivation

A curated playlist whose order you cannot change is a half-feature. The session
queue already had drag + `Alt`+arrow reorder; playlists needed the same.

## Scope

- `TrackList` grows an optional `onReorder` (rows become draggable and
  `Alt`+`↑`/`↓` moves them), mirroring `QueueList`
- Playlist detail passes a handler that writes the new order
- Filtered views: the move is resolved by track id, not row index

## Acceptance criteria

- [x] Playlist rows drag to reorder and reorder with `Alt`+`↑`/`↓`
- [x] Focus follows the moved row; the move is announced
- [x] Reordering works while a search filter is active
- [x] Other views (`Songs`, folders) are unaffected — rows stay non-draggable
- [x] A finished drag never triggers playback
- [x] `npx tsc --noEmit` and `npm run build` clean

## Outcome

- **`TrackList` opt-in** — `onReorder` is the same pattern as `onAdd`/`onRemove`:
  absent means the list behaves exactly as before, so the Songs and folder
  lists keep no drag affordance at all.
- **Filtered lists are safe** — the handler receives row indices into whatever
  the list shows, so when a search filter is active it maps the moved and
  hovered rows to their *track ids* and applies "insert where the hovered track
  was" to the full playlist order. The backend stays the source of truth, since
  `reorder_playlist` takes a complete id list.
- **No accidental playback** — a completed HTML5 drag ends with a `click`, which
  in this list means "play this row". A ref set on `dragstart` swallows exactly
  that one click and is reset on the next `mousedown`, so a *cancelled* drag
  cannot swallow a later, real click.
- **A11y** — reorderable rows carry `aria-keyshortcuts="Alt+ArrowUp
  Alt+ArrowDown"`, a positional `aria-label`, and the `aria-live` announcement
  the queue already used.

No Rust change: this is issue #18's dead-command fix on the frontend side.

## References

- `src/App.tsx` (`TrackList`/`TrackRow` reorder, `onReorderPlaylist`)
- `src/api.ts` (`reorderPlaylist`, previously unused)
- `crates/library/src/playlists.rs` (`reorder_playlist`)