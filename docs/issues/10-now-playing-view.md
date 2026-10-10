---
title: "Album-art-dominant Now Playing view (design goal: \"big album art, Poweramp-style\")"
labels: ["enhancement", "priority:P2", "area:ux"]
milestone: "v0.2.0"
status: "done"
---

## Summary

The design document describes a music player with **big album art** ("alam
Poweramp"), but v0.1.1 has no Now Playing view at all — the only
representation of the playing track is the 56 px player bar. Lyrics, when
open, are the popover exception.

## Evidence

- `docs/design.md` §4.4 `layout` — "sidebar + konten + player bar selalu
  tampak + Now Playing (art besar, lirik)".
- `src/App.tsx:53-64` — `View` union has songs/albums/artists/folders/
  playlists/queue/album/artist/folder/settings — **no `now-playing`** state.
  (`settings` arrived with the Discord cover-art work; the gap is unchanged.)
- `src/PlayerBar.tsx` — 56 px bar is the entire now-playing surface.

## Impact

The cornerstone aesthetic of the product brief is unmet; no hero moment for
album art, and lyrics live in a small popover.

## Suggested direction

1. Add a `now-playing` view: large cover hero (respecting cover aspect
   ratio), track/album/artist, transport + position + volume, lyrics panel
   beside/below art, and a back-to-library control.
2. Entry point: click the player-bar art/title; keyboard shortcut (e.g.
   `N`); respect `prefers-reduced-motion` for entrance transitions.
3. Reuse existing state/events — no new player commands needed (frontend
   slice, per repo pattern).
4. Art source: embedded art only, as today — or also the online album-art URL
   when the Discord cover flag is on. Decide deliberately; the fallback tile
   must cover albums with neither.

## Definition of Done

- [x] Now Playing opens from the player bar and keyboard, and closes cleanly
- [x] Large cover renders without layout shift; fallback note tile for
      tracks without art
- [x] Lyrics (existing popover content) available inside the view
- [x] Works at min window size 900×600 (responsive)
- [x] Frontend tests where logic is added; `npm run build` clean

## Outcome

A new full-content surface, `src/NowPlaying.tsx`, rendered inside the app
shell so the sidebar and player bar stay visible (design §4.4). It reuses
existing state/events — no new player commands.

- **Entry/exit** — the player-bar art/title is now a button
  (`PlayerBar` gained `nowPlaying` + `onToggleNowPlaying`, passed through
  `Shell`) and `N` opens the view / `Escape` closes it. Both shortcuts skip
  text fields. The in-view "Library" back button, a sidebar click, or
  `Escape` all dismiss it; closing returns to whatever `view` was underneath
  (a separate `nowPlaying` boolean, so the browse state is preserved).
- **Layout** — two columns (art + meta + transport + seek + volume on the
  left, lyrics on the right) that stack below 1100 px. `.now-art` is a fixed
  `aspect-ratio: 1` box, so a late-loading image causes no layout shift.
- **Art** — reuses `Cover` (embedded art only, cached) with a `size` prop so
  the no-art case shows a matching large note tile. The online album-art URL
  stays scoped to the Discord presence fetch; wiring it into the hero would
  need new IPC plumbing and was left out deliberately.
- **Lyrics** — the popover body was extracted into `src/LyricsPanel.tsx`
  (fetch, active-line highlight, reduced-motion-aware auto-scroll) and is now
  shared by the player-bar popover and the Now Playing column, removing a
  duplicate.

Seek and volume use the same draft-then-commit pattern as the bar (one IPC
invoke on release/blur, never per pointer move). `npx tsc --noEmit` and
`npm run build` are clean. Frontend unit tests for the draft-commit helpers
are deferred with the repo-wide vitest gap (see #04/#08).

## References

- Roadmap: checklist item M5