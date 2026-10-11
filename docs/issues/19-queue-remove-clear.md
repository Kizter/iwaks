---
title: "Queue: remove one track / clear the queue"
labels: ["enhancement", "priority:P2", "area:player"]
milestone: "v0.3.0"
status: "done"
---

## Summary

The session queue could only be reordered or saved — never shrunk. Add
remove-one and clear-queue, including the case where the removed track is the
one playing.

## Motivation

A mistake in a 200-track queue meant rebuilding it from scratch. Every other
list in the app could have an item removed.

## Scope

- `Queue::track_at` / `Queue::without_position` / `Queue::cleared` in the
  player crate
- `Player::remove_queue_at` / `Player::clear_queue`
- Tauri commands `remove_from_queue` / `clear_queue`
- Row ✕ button + `Delete` key in the Queue view, "Clear queue" in the toolbar

## Acceptance criteria

- [x] Removing a non-current track never interrupts playback
- [x] Removing the current track hands over to the track that took its slot
- [x] Removing the last playing track leaves the rest of the queue and no
      current track
- [x] Clearing empties the queue, stops playback, and keeps repeat + shuffle
- [x] Queue view refetches (queue version bump) and state stays consistent
- [x] New player tests; `cargo fmt`/`clippy`/`test` and the frontend gates clean

## Outcome

- **Queue maths** — `without_position(pos)` drops the track from the
  permutation, shifts every stored track index above it down by one, and
  re-points the current position: removing something before the current track
  shifts it back, removing the current one leaves the following track in its
  slot, and removing the last one leaves no current track. Out-of-range
  positions return the queue untouched.
- **Player** — `remove_queue_at` edits the queue and track list inside one
  critical section (queue → tracks, the lock order used everywhere else), so no
  reader can observe a queue and a track list from two different edits. When the
  removed track was playing it queues the usual `Cmd::Load` for the successor —
  the same path `next` uses, so no mpv call happens off the pump thread — and
  falls back to `Cmd::Stop` when nothing follows. Any removal bumps
  `queue_version` so the Queue view refetches.
- **Clearing** — `Queue::cleared()` keeps `repeat` and `shuffle`, so emptying
  the list cannot silently reset how the *next* list will play.
- **Tests** — 5 new tests: removing an upcoming track (playback intact, next
  skips it), removing the current track (hands over), removing the last playing
  track (queue keeps the rest, nothing selected), clearing (empty + stopped +
  both preferences kept, and shuffle still active on the next list), plus an
  offline check that out-of-range removals are no-ops. The first three assert
  through `wait_for`, because `snapshot()` only catches up when the pump drains
  the queued command — asserting immediately tested the previous state.
- **UI** — each queue row gets a ✕ (`Delete`/`Backspace` from the focused row,
  announced in the row's `aria-label`) and the toolbar gets "Clear queue".
  Both reuse the existing `btn-danger` / `row-btn` styling.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (281 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`. No new dependencies.

## References

- `crates/player/src/queue.rs` (`track_at`, `without_position`, `cleared`)
- `crates/player/src/player.rs` (`remove_queue_at`, `clear_queue` + tests)
- `src-tauri/src/lib.rs` (`remove_from_queue`, `clear_queue`)
- `src/api.ts`, `src/App.tsx` (Queue view)