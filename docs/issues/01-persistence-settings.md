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

The storage mechanism now exists — `settings(key, value)` landed in
`SCHEMA_V3` for the online-album-art flag — but **nothing about the player is
written to it**. What remains is wiring player state through the table that is
already there.

## Evidence

- `crates/library/src/db.rs` — `SCHEMA_V3` has a `settings(key, value)` table
  and `crates/library/src/cover.rs` provides `setting` / `set_setting`. As of
  v0.1.1 it had none at all (`tracks`, `playlists`, `playlist_tracks`, FTS
  only); the table arrived with the Discord cover-art work.
- `crates/player/src/player.rs` — `Player::start` still hardcodes
  `volume: 80`, `speed: 1.0`, `repeat: Off`, `shuffle: false`,
  `eq: vec![0.0; 10]`, `sleep_remaining: None`. No player state reaches
  `set_setting`.
- `src-tauri/src/lib.rs` — no restore path in `setup`; the frontend has no
  `localStorage` usage (`src/App.tsx`, `src/PlayerBar.tsx`).

## Impact

High. Every app restart loses user preferences and playback context; the
session queue of a long listening session is gone after a crash or restart.

## Suggested direction

1. Reuse the existing `settings` table — the migration to v3 and its backward
   compatibility are already merged and tested. No new schema work.
2. Persist volume, mute, repeat, shuffle, speed, ReplayGain mode, EQ, and
   (decide) last queue + position on change — debounced writes, not per event.
3. Restore at `setup`: apply settings to player start, optionally reload last
   queue + position (mpv `seek` after load).
4. Frontend: keep current event-driven flow; no change needed beyond Rust.

## Definition of Done

- [ ] Settings survive an app restart (volume, repeat, shuffle, speed,
      ReplayGain, EQ, sleep timer)
- [ ] (If in scope) last session queue + position restored on launch
- [ ] DB migration 2→3 (already merged) stays backward compatible — covered
      by `crates/library/tests/cover.rs`; re-confirm it when adding keys
- [ ] Writes are debounced (no DB write per slider event)
- [ ] Tests cover migration + save/restore roundtrip (≥80% area coverage)

## References

- Roadmap: checklist item M1