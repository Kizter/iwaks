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

### Stage 1 — `Engine`: two cores, one thread, no behaviour change — **done**
- New `crates/player/src/engine.rs` owning `cores: [Option<Arc<Mpv>>; 2]` + `active`
- `Player.api` calls become `self.engine.lock().active()`
- Same startup options applied to both cores (`ao`, `audio-exclusive`,
  `audio-client-name`, `volume-max`, `gapless-audio`, replaygain, `af` EQ)
- **DoD:** all 54 existing player tests pass unchanged; no user-visible change

**Outcome.** Landed as a pure refactor: `crates/player/src/engine.rs` holds the
slots and the per-core startup setup (moved out of `Player::setup_mpv`, which no
longer exists), and `Player` keeps a `Mutex<Engine>`. Only the *primary* core is
created — the second slot stays empty until a prefetch needs it, so this stage
costs no extra audio memory and cannot change what the user hears.

- The **pump still takes its `Arc<Mpv>` directly**, so the hot loop is unchanged;
  the engine is only consulted where a second core would be needed. That keeps
  the diff reviewable: 20 lines added, 61 removed in `player.rs`, and no command,
  state field or frontend type moved.
- The pump thread constructs the `Engine` and installs it with one
  `*engine.lock() = engine` before `ready_tx.send(Ok(()))`, keeping the same
  guarantee the old single handle had: after `Player::start` returns `Ok`, every
  other thread can reach the core.
- `ReplayGainMode::as_mpv` had to become `pub(crate)` — the setup now lives in a
  sibling module, and the mpv spelling stays private to the crate.
- 4 new unit tests for the pure slot logic (empty engine, swap, idle ≠ active,
  `Default`). They need no libmpv DLL, which is why they can run in CI; `Mpv` is
  neither `Debug` nor `PartialEq`, so presence is asserted with `is_none`/
  `is_some` rather than `assert_eq!`.
- `Player::idle_api()` was written and then removed: it had no caller until
  Stage 2, and stage 1 is not the place for speculative code.

Gates: `cargo fmt`/`clippy` clean; `cargo test --workspace` 316 passed
(54 existing player tests unchanged + 4 engine tests), 5 ignored live.

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
- **Two settings, not one.** `design.md` §4.2 says "toggle, default off", so
  "enabled" and "how long" are separate concerns — a single `seconds = 0`
  sentinel would also mean "off", making a 1 s crossfade impossible to express:

  | Setting | Type | Default |
  |---------|------|---------|
  | `audio.crossfade` | bool | **off** |
  | `audio.crossfade_seconds` | f64, 1.0–10.0 | **3.0** |

  Three seconds is the default because the feature ships **off**, so the default
  only matters for the first seconds after someone enables it — which makes it
  cheap to be wrong. 3 s is also the safe ceiling for a mixed library: a longer
  fade swallows whole tracks whose intros are short. The range starts at 1 s
  because below that it sounds like a glitch, not a fade.
- Session persistence records the active track as before; a restore never
  prefetches across the boundary
- **DoD:** the choice survives a restart; default is off; a 1 s crossfade is
  selectable (the two-setting split above is what makes that possible)

## Decisions

**D1 — default 3 s, range 1–10 s, split into an enable flag and a duration.**
(Decided.) See Stage 4.

**D2 — the crossfade ↔ gapless trade-off belongs in the control's own note, not
in a modal.** (Proposed, not yet implemented.) `design.md` §4.2 promises gapless;
crossfade cannot keep it, and that has to be stated where the user turns the
feature on. The app already has this pattern — the Exclusive mode switch
explains its cost in the same place — so this should be one sentence in
`.set-note`, not a dialog:

> Overlaps the end of one song with the start of the next. Sample-accurate
> transitions are given up while it is on — tracks are no longer gapless — and
> exclusive output is unavailable, so the two can't be combined.

**D3 — whether Stage 1 alone is worth landing** if the rest is deferred. (Open.)
It is a pure refactor that removes the last place "one mpv" is assumed.

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

- [x] Stage 1 lands with all existing tests green and no behaviour change
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

1. ~~Default duration~~ — **decided**: 3 s, range 1–10 s, and a separate enable
   flag (see D1).
2. ~~Whether the gapless trade-off is surfaced in the UI~~ — **proposed**: yes, one
   sentence in the control's note (see D2). Not implemented yet; Stage 1 touches
   no UI.
3. ~~Whether Stage 1 alone is worth landing~~ — **still open** (see D3).

## References

- `docs/design.md` §4.2 (the decision being implemented), §1 (core features)
- `crates/player/src/player.rs` (`api: Mutex<Option<Arc<Mpv>>>`, `pump`,
  `Cmd`, `read_state`)
- `crates/player/src/ffi.rs` (`Mpv::wait_event`, `command`, `get_*`)
- `docs/issues/23-output-device-picker.md` (device state in `PlayerState`),
  `docs/issues/24-wasapi-exclusive-toggle.md` (the exclusive interaction)