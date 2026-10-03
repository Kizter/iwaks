---
title: "Persist player settings and restore the last session on restart"
labels: ["enhancement", "priority:P1", "area:persistence"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Every player setting is in-memory only. Volume resets to 80, EQ to flat,
repeat off, shuffle off, speed 1.0, ReplayGain to `track`, and the sleep
timer / session queue / last position are lost on every restart. A music
player that forgets the volume and position each launch.

## Evidence

- `crates/library/src/db.rs:10-76` — schema v1/v2 has **no settings table**
  (`tracks`, `playlists`, `playlist_tracks`, FTS only).
- `crates/player/src/player.rs:220-239` — `Player::start` hardcodes
  `volume: 80`, `speed: 1.0`, `repeat: Off`, `shuffle: false`,
  `eq: vec![0.0; 10]`, `sleep_remaining: None`.
- `src-tauri/src/lib.rs` — no restore path in `setup`; the frontend has no
  `localStorage` usage (`src/App.tsx`, `src/PlayerBar.tsx`).

## Impact

High. Every app restart loses user preferences and playback context; the
session queue of a long listening session is gone after a crash or restart.

## Suggested direction

1. DB migration v3: `settings(key TEXT PRIMARY KEY, value TEXT)` (schema
   migrator already keys on `PRAGMA user_version`).
2. Persist volume, mute, repeat, shuffle, speed, ReplayGain mode, EQ, and
   (decide) last queue + position on change — debounced writes, not per event.
3. Restore at `setup`: apply settings to player start, optionally reload last
   queue + position (mpv `seek` after load).
4. Frontend: keep current event-driven flow; no change needed beyond Rust.

## Definition of Done

- [ ] Settings survive an app restart (volume, repeat, shuffle, speed,
      ReplayGain, EQ, sleep timer)
- [ ] (If in scope) last session queue + position restored on launch
- [ ] DB migration 2→3 is tested and backward compatible (open old DB)
- [ ] Writes are debounced (no DB write per slider event)
- [ ] Tests cover migration + save/restore roundtrip (≥80% area coverage)

## References

- Roadmap: checklist item M1