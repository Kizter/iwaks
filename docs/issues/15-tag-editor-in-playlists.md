---
title: "Tag editor reachable from playlist rows"
labels: ["bug", "priority:P3", "area:ux"]
milestone: "v0.3.0"
status: "done"
---

## Summary

The playlist-detail track list never received an `onEdit` handler, so the
pencil button was never rendered and tags could not be edited from a playlist.

## Motivation

The tag editor worked from Songs and folder views only. A playlist row showed
"+" and "✕" but no pencil — the only destructive-looking-free action missing on
exactly the surface users browse when curating.

## Scope

- Pass `onEdit` into the playlist-detail `TrackList`
- No new state, command, or styling (`onSaved` already refreshed playlists)

## Acceptance criteria

- [x] Pencil button appears on playlist rows
- [x] Saving a tag edit refreshes the playlist detail view
- [x] `npx tsc --noEmit` and `npm run build` clean

## Outcome

One line in `src/App.tsx`: the playlist-detail `TrackList` now gets
`onEdit={(t) => setEditTarget(t)}`, matching the Songs and folder lists. The
`TagEditor`'s existing `onSaved` branch already called `refreshPlaylists()` for
`view.kind === "playlist"`, so the edit shows up without further work — the
missing prop was the only gap.

## References

- `src/App.tsx` (playlist-detail `TrackList`)
- `src/App.tsx` (`TagEditor` `onSaved`)