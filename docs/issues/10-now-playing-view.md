---
title: "Album-art-dominant Now Playing view (design goal: \"big album art, Poweramp-style\")"
labels: ["enhancement", "priority:P2", "area:ux"]
milestone: "v0.2.0"
status: "draft"
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

- [ ] Now Playing opens from the player bar and keyboard, and closes cleanly
- [ ] Large cover renders without layout shift; fallback note tile for
      tracks without art
- [ ] Lyrics (existing popover content) available inside the view
- [ ] Works at min window size 900×600 (responsive)
- [ ] Frontend tests where logic is added; `npm run build` clean

## References

- Roadmap: checklist item M5