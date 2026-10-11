---
title: "UI rough edges: inert search box, scan-gated queue save, sticky banners, native confirm"
labels: ["bug", "priority:P3", "area:ux"]
milestone: "v0.3.0"
status: "done"
---

## Summary

Four small honesty fixes from the v0.2.0 completion audit: the search box
appeared in views it cannot filter, "Save as playlist" was disabled during a
scan for no reason, success messages looked and behaved like errors, and
playlist deletion used a native `window.confirm`.

## Motivation

Each of these is a control that lies or blocks. Small individually, but together
they are the difference between "finished" and "demo".

## Scope

- Hide the search box on the Queue and Playlists-grid views
- Stop gating "Save as playlist" on `scanning`/`adding`
- Separate transient success notices from sticky error banners
- Replace `window.confirm` with the app's own modal

## Acceptance criteria

- [x] Search box hidden exactly where the query is ignored
- [x] Queue save works during a scan
- [x] Success messages auto-dismiss and are visually distinct from errors
- [x] Errors keep the Dismiss button and never auto-expire
- [x] Delete confirmation uses the app's own dialog
- [x] `npx tsc --noEmit` and `npm run build` clean

## Outcome

- **Search box** — `searchApplies = !isQueue && !isPlaylists` mirrors exactly the
  condition the `noResults` computation already used, so the box now appears
  only on views where typing actually filters something.
- **Queue save** — `disabled={scanning || adding}` dropped; saving the queue
  only reads the queue and writes a new playlist.
- **Notice vs banner** — a separate `notice` state for the five success messages
  (imported / exported / queue saved / added to playlist / playlist created),
  auto-dismissed after 4 s and announced politely with `role="status"`. Error
  banners stay sticky with `role="alert"`. Both gained an explicit tone:
  `.banner.error` uses the danger wash, `.banner.notice` the calm mint wash.
- **Delete confirmation** — `onDelete` now opens the existing `.picker`
  dialog, states that the tracks themselves are kept, and only then calls
  `deletePlaylist`. `window.confirm` is gone.

`npx tsc --noEmit` and `npm run build` are clean. No Rust change, so the cargo
gates are unaffected.

## References

- `src/App.tsx` (toolbar search, queue save, `onDelete`/`confirmDelete`, banners)
- `src/App.css` (`.banner.error`, `.banner.notice`)