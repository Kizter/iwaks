---
title: "Crossfade (two libmpv cores + volume ramp)"
labels: ["enhancement", "priority:P3", "area:player"]
milestone: "v0.3.0"
status: "planned"
---

> **Plan only.** This is the riskiest item on the v0.3.0 roadmap and it is
> written down before any code, so the decision to attempt it is deliberate and
> reversible at every stage.

## Summary

Implement the crossfade `design.md` §4.2 specifies — "dua instance libmpv +
volume ramp, toggle, default off" — which has never been implemented (the
shipped design is single-instance, `crates/player/src/player.rs`).

## Motivation

Albums that run tracks together sound clipped without it. It is the one
promised core audio feature that does not exist.

## Options considered

| Option | What it is | Verdict |
|--------|-----------|---------|
| **A. Two libmpv cores, one pump thread** | Prefetch the next track into an idle core, ramp both volumes, swap | **Chosen** — the only true crossfade, and it preserves the "only the pump thread may call libmpv" invariant |
| B. `af=lavfi=[acrossfade]` on one core | One line of config | Rejected: `acrossfade` merges *within one input*; mpv appends the next file to the same stream, so it does not fade across `loadfile` boundaries reliably |
| C. Fade-out / fade-in, no overlap | Ramp volume down at the end of a track and up at the start of the next | Kept as the fallback if Stage 1 proves the invariant cannot hold. Audible dip in the middle, no overlap, so it is not a crossfade — it should be labelled "fade", not "crossfade" |
| D. Casting-grade DSP in Rust | Own resampler/mixer | Rejected: reintroduces the network audio path §1 rules out |

### The key insight that makes A tractable

The obvious reading of "two instances" is two pump threads, which would double
the deadlock surface this crate already has history with (issue #03's self-join,
the END_FILE property-read hazard). It does not have to be that way:

**One pump thread can drive two mpv cores.** `mpv_wait_event(handle, 0)` is
non-blocking, so the existing loop — drain commands, poll events, emit at
≤10 Hz — polls core A and core B in the same iteration with the same `WAIT_SLICE`
sleep. The architecture that took the most care (one thread owns libmpv, state
is read from a mutex, commands go through one queue) survives intact; what
changes is that `api: Mutex<Option<Arc<Mpv>>>` becomes two cores plus an
`active` index.

## Scope, in four shippable stages

Each stage is its own commit and each leaves the app working.

### Stage 1 — `Engine`: two cores, one thread, no behaviour change
- New `crates/player/src/engine.rs` owning `corem: [Option<Arc<Mpv>>; 2]` + `active`
- `Player.api` calls become `self.engine.active()` / `.idle()`
- Same startup options applied to both cores (`ao`, `audio-exclusive`,
  `audio-client-name`, `volume-max`, `gapless-audio`, replaygain, `af` EQ)
- **DoD:** all 54 existing player tests pass unchanged; no user-visible change

### Stage 2 — prefetch
- When the active track's remaining time ≤ fade duration, `loadfile` the next
  track into the idle core with `pause=yes`
- The EQ filter chain is applied to the prefetched core *before* its first frame
  can play, otherwise the fade-in plays flat while the rest of the album does not
- **DoD:** prefetch is observable in state (`prefetched: bool`); a prefetched
  track never changes what is audible on its own

### Stage 3 — the ramp and the swap
- At the fade start, un-pause the incoming core and ramp both volumes from the
  user's volume and 0 (equal-power curve, so the sum stays flat)
- Swap `active` at the midpoint, stop the outgoing core, report the new
  `current` track
- `Cmd` variants for the whole window, so `pause`/`seek`/`next`/`prev`/`stop`
  mid-fade route to the right core and cannot race the swap
- **DoD:** `next`/`prev`/shuffle during a fade keep playing audio; state stays
  consistent (one current track, correct `index`, `list_len`)

### Stage 4 — setting, UI, persistence
- `audio.crossfade_seconds` setting (0 = off, default off per design §4.2) with a
  duration control in Settings → Audio output
- Session persistence records the active track as before; a restore never
  prefetches across the boundary
- **DoD:** the choice survives a restart; default is off

## Risks, and what is done about each

| Risk | Mitigation |
|------|------------|
| **Exclusive output cannot host two cores.** WASAPI exclusive takes the device; a second mpv instance on it will fail or be forced to shared. | Refuse to prefetch when `audio_exclusive` is on and log it once. Crossfade silently degrading to normal gapless is better than an exclusive-mode failure — and issue #24 makes exclusive an explicit opt-in, so the combination is rare and visible. |
| **Wall-clock tests are flaky.** A real fade is a timing test. | Assert on *outcomes* (active index swapped, both instance volumes at the expected endpoints, state coherent) with a short fade (200 ms) and generous `wait_for` timeouts, not on exact sample positions. |
| **Prefetch plays the wrong thing if the queue changes** (reshuffle, remove, clear during the fade window). | The prefetch is re-validated against the queue's current track before un-pausing; a mismatch cancels the fade instead of playing it. |
| **Doubles the audio memory** (two decoded streams live). | Two cores is the cost of the feature; Stage 1 can measure it. If it matters, Stage 2 becomes "prefetch only while within the fade window", which bounds it to the last few seconds of a track. |
| **Two cores contend for the audio device in shared mode.** | WASAPI shared mode is designed for multiple clients; this is the supported configuration. Flagged as the thing to verify by ear first after Stage 3. |
| **A crash in the fade path leaves both cores running.** | The swap is a single guarded `active` reassignment; the outgoing core is stopped immediately after, and `Player::shutdown` stops both. |

## Acceptance criteria

- [ ] Stage 1 lands with all existing tests green and no behaviour change
- [ ] Prefetch never plays without being asked, and always with the current EQ
- [ ] A completed crossfade reports the new track, correct index and position
- [ ] Transport commands during a fade never leave both cores audible
- [ ] Exclusive mode disables crossfade instead of failing
- [ ] Setting is off by default and survives a restart
- [ ] `cargo fmt`/`clippy`/`test` and the frontend gates green at every stage

## Out of scope

- Per-playlist or automatic crossfade
- Crossfade for DSD-over-PCM paths until the plain path is proven
- Any DSP outside mpv (see option D)

## Open decisions for the reviewer

1. **Default duration.** 3 s is the common choice; 5 s suits long-form mixes.
   Proposed: 3 s, adjustable 0.5–10 s.
2. **Gapless vs crossfade are mutually exclusive by nature** — overlapping cores
   cannot be sample-accurate with the incoming track's first sample, so
   enabling crossfade gives up mpv's gapless playback. This should be stated in
   the UI, not just in this file.
3. **Whether Stage 1 alone is worth landing** if the rest is deferred: it is a
   pure refactor that removes the last place where "one mpv" is assumed, and it
   makes the audio output issue (#23) and future work easier.

## References

- `docs/design.md` §4.2 (the decision being implemented), §1 (core features)
- `crates/player/src/player.rs` (`api: Mutex<Option<Arc<Mpv>>>`, `pump`,
  `Cmd`, `read_state`)
- `crates/player/src/ffi.rs` (`Mpv::wait_event`, `command`, `get_*`)
- `docs/issues/23-output-device-picker.md` (device state in `PlayerState`),
  `docs/issues/24-wasapi-exclusive-toggle.md` (the exclusive interaction)