---
title: "Persist player settings and restore the last session on restart"
labels: ["enhancement", "priority:P1", "area:persistence"]
milestone: "v0.2.0"
status: "done"
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

- [x] Settings survive an app restart (volume, repeat, shuffle, speed,
      ReplayGain, EQ, sleep timer)
- [x] (If in scope) last session queue + position restored on launch
- [x] DB migration 2→3 (already merged) stays backward compatible — covered
      by `crates/library/tests/cover.rs`; re-confirm it when adding keys
- [x] Writes are debounced (no DB write per slider event)
- [x] Tests cover migration + save/restore roundtrip (≥80% area coverage)

## Outcome

Two JSON blobs in the existing `settings` table — no schema change:

- `player.settings` → `PlayerSettings` (volume, mute, repeat, shuffle, speed,
  replaygain, EQ preamp + 10 bands, sleep seconds).
- `player.session` → `SessionSnapshot` (track paths in play order, current
  index, position in seconds, shuffle).

Storage lives in `crates/library/src/persist.rs`
(`save_player_settings` / `load_player_settings` / `save_session` /
`load_session`, typed via `serde_json`; a new `LibraryError::Serde` variant).
The app layer (`src-tauri/src/persist.rs`) owns the `PlayerState` ↔ storage
mapping, so neither the player nor the library crate gains a dependency on the
other.

- **Debounce**: mutating commands call `Persist::touch()` (a `mpsc` ping). A
  worker thread drains the burst and writes once `700 ms` after the last ping,
  so dragging the volume/EQ slider writes a single time.
- **Restore** (`setup`, right after `Player::start`): apply settings first,
  then `play_tracks` the resolved session at the stored index + `seek`, so
  repeat/shuffle survive and the current track follows to its new slot if
  files were added/removed. Files that no longer exist are skipped.
- **Mute** gained an explicit `Player::set_mute(bool)` (the old API only had
  `toggle_mute`, which cannot restore an arbitrary persisted value).
- **Exit**: a final synchronous `write_now` runs on `RunEvent::Exit` before the
  player shuts down, so a clean close never loses the live position.

Tests: `crates/library/tests/persist.rs`
(`settings_and_session_survive_reopen`, `saving_settings_replaces_the_previous_value`);
`src-tauri/src/persist.rs` unit tests (`settings_map_every_persisted_field`,
`enum_names_roundtrip`, `session_follows_play_order_and_current_slot`,
`session_is_none_without_something_to_resume`).

Deliberate choice: restoration begins playing at the stored position (a music
player resuming its last session) rather than paused; paused-state persistence
was left out as it is not in the DoD.

## References

- Roadmap: checklist item M1