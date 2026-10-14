---
title: "Crossfade could hard-cut or play silence around short tracks; a stored overlap reached the pump unclamped"
labels: ["bug", "priority:P1", "area:player", "area:settings"]
milestone: "v0.3.0"
status: "done"
---

## Summary

A two-axis code review of the v0.3.0 build (`5402789...HEAD`, 15 commits) found
four confirmed defects. Three are in the crossfade sequencer (issue #27 — the
flagship) and one in the audio-device settings path. All four are fixed and
covered by tests; this document records what the review found and how each item
was resolved.

## Findings

### 1. HIGH — a short middle track played its neighbour as a hard cut (crossfade wedged)

`maybe_prefetch` loads the next-next track into the third core while the previous
ramp is still draining — the exact case issue #27's "three cores, not two" was
added for — but `start_fade` refuses to run two ramps at once
(`if self.fade.lock().unwrap().is_some() { return; }`). The loaded core then sits
parked, the engine reports it as "already waiting", and no second ramp ever
starts from it. When the short track's EOF arrives, `advance_to` → `Cmd::Load`
discards the parked core and plays a hard cut — crossfade silently lost in the
precise scenario the third core exists to serve.

**Fix (`player.rs`):** `finish_fade` now calls `clear_prefetch()` followed by
`maybe_prefetch()` once the ramp is over. The parked core is dropped and the
decision is made fresh against the current track's remaining time, so a short
middle track still fades into what follows. Verified by
`a_short_middle_track_rearms_the_next_crossfade`, which asserts the ramp is
running again (a second prefetch logged) after B is past A's ramp end.

### 2. HIGH — a next track shorter than half the window played silence with no auto-advance

`start_fade` un-pauses the incoming core, but nothing clamped the window against
the **incoming** track's own length. A track shorter than `window / 2` hit its
EOF before the ramp's midpoint; `handle_event` dropped that EOF (the core was
neither the fading-out slot nor the active slot), and the midpoint handover
adopted a finished core: index moved, position stuck at 0, queue never advanced.

**Fix:** two layers, playable one first.

- `start_fade` (player) clamps the window to `min(window, incoming.duration / 2)`
  and refuses to fade at all when that leaves less than `MIN_CROSSFADE_SECONDS`
  (0.5 s). mpv fills `duration` for a freshly loaded core asynchronously, so an
  unknown length still fades — the second layer catches that race:
- `handle_event` (player) treats an `EndFile` from the fade-in slot **before the
  handover** as "this ramp cannot complete": it calls `abandon_fade()` and lets
  the still-playing outgoing track end normally, which advances the queue.

Verified by `a_short_next_track_never_plays_silence` (a 0.9 s B under a 3 s
window must play out and then stop, never stall).

### 3. MED — a hand-edited stored overlap reached the pump unclamped

`stored_crossfade` (startup reader, `src-tauri/lib.rs`) filtered only
`is_finite()`, while the two command readers (`get_crossfade`, `set_crossfade`)
clamp to 1–10 s via `clamp_crossfade_seconds`. A hand-written `600` in the
settings row reached the pump at startup as a 600 s fade window, where
"about to end" means every track. Issue #27 states the read is clamped; the
review found it was not.

**Fix:** `stored_crossfade` now maps the parsed row through
`clamp_crossfade_seconds` (which already defaults non-finite input). Covered by
`the_startup_crossfade_reader_clamps_a_hand_edited_row` in `src-tauri/lib.rs`.

### 4. MIN — `set_audio_device` applied before persisting

The command applied the device to the session first and wrote the setting second
— the reverse of `set_crossfade`, which deliberately stores first ("a write that
fails must not leave the UI and the session disagreeing"). A failed write left a
device playing that the UI then rolled back.

**Fix:** `set_audio_device` persists first, then applies (same shape as
`set_crossfade`), and tolerates the player being absent.

## Out of scope (judgement calls)

These were noted by the review but deliberately left out of the bug-fix pass —
none is a behaviour defect. All four were picked up afterwards, in a follow-up
cleanup pass (see below).

- `set_theme(mode: String)` accepts any string from IPC; validation lived only in
  the frontend (`isThemeMode`). Value is written, not executed, so it is cosmetic.
- Palette comments in `src/App.css` are Indonesian while the rest of the repo is
  English — a review-copy accident, not a behaviour issue.
- `genres()`'s doc string claims "case-insensitively deduped" but `SELECT
  DISTINCT` is case-sensitive (`"Rock"` vs `"rock"` are two chips).
- `player.rs` has grown past 1500 lines (divergent change) and the
  `bump_queue_version` + `ClearPrefetch` trio repeats — refactors, not defects.

## Follow-up cleanup (all four addressed)

- **Theme validation** — `src-tauri/lib.rs` gained `is_theme_mode`; `set_theme`
  now rejects an unknown mode and `get_theme` falls back to `system` when a
  hand-edited row holds junk, so the guard no longer depends on the frontend
  alone. Covered by `only_the_three_offered_theme_modes_are_stored`.
- **Palette comments** — every Indonesian comment in `src/App.css` (light and
  dark palettes, the section header, the `pasca-uji` section label) is now
  English. No selector or value changed; `App.css.test.ts` still passes.
- **`genres()` case-insensitive dedup** — the query now groups by
  `lower(genre)` and selects `MIN(genre)`, so a library holding both `Rock` and
  `rock` offers one chip instead of two (the doc's promise). Covered by
  `genres_dedupe_spellings_that_differ_only_in_case`.
- **`player.rs` size + repeated trio** — the 1560-line test module moved to
  `crates/player/src/player/tests.rs` (the module is now a directory module;
  `player.rs` dropped from 3214 to 1663 lines), and the
  `bump; ClearPrefetch; bump` sequence became a named `invalidate_queue()`.
  Nothing reads `queue_version` between the two former bumps (the `ClearPrefetch`
  handler does not emit, and no test asserts an exact count — only that a change
  happened), so a single bump is the same signal, and the queue-view refetch is
  unchanged.

Gates re-run green after the cleanup: `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace` (339 passed
/ 0 failed / 5 ignored — the live libmpv suite included, 76 player tests),
`npx vitest run` 24/24, `npm run build`.

## Evidence

Both review axes were run as parallel sub-agents over
`git diff 5402789...HEAD`; each finding above was then re-verified by reading
the cited code before any change was made. Standards axis (repo AGENTS.md +
Fowler's smell baseline) and Spec axis (ROADMAP-2026-v0.3.0 + issues 15–27)
converged independently on findings 1–3.

## Outcome

Gates green after the fixes:

- `cargo fmt --all --check` clean.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo test --workspace`: 335 passed / 0 failed / 5 ignored (the ignored ones
  are the network-gated cover live tests, untouched).

The two new live crossfade tests run under real libmpv in CI's `ao=null` setup,
as the existing crossfade suite does (`have_libmpv()` gate); the device test
still requires `IWAKS_LIVE_AUDIO=1`.

## References

- Related: `docs/issues/27-crossfade.md` (the feature these fixes complete),
  `docs/issues/23-output-device-picker.md` (the picker `set_audio_device` serves)