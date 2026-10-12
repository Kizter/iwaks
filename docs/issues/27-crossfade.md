---
title: "Crossfade (two libmpv cores + volume ramp)"
labels: ["enhancement", "priority:P3", "area:player"]
milestone: "v0.3.0"
status: "in progress"
---

> **Plan, executed in stages.** This was the riskiest item on the v0.3.0
> roadmap, written down before any code so the decision to attempt it stayed
> deliberate and reversible at every stage. Stages 1–3 have landed (the feature
> is code-complete but off by default); stage 4 — the two settings, the UI and
> persistence — is what makes it reachable by a user.

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

### Stage 2 — prefetch — **done**
- When the active track's remaining time ≤ fade duration, `loadfile` the next
  track into the idle core with `pause=yes`
- The EQ filter chain is applied to the prefetched core *before* its first frame
  can play, otherwise the fade-in plays flat while the rest of the album does not
- **DoD:** prefetch is observable in state (`prefetched: bool`); a prefetched
  track never changes what is audible on its own

**Outcome.** The pump's existing ~10 Hz tick now emits and *then* considers a
prefetch, so the state the user sees and the prefetch decision come from the
same snapshot. Loading is `pause=yes`, which is this stage's entire guarantee:
nothing about it is audible, and stage 3 is what changes that.

- **Off by default, so this stage is invisible in the shipped app.**
  `Crossfade::default()` is `{enabled: false, seconds: 3.0}` per design §4.2.
  `maybe_prefetch` returns immediately when `window()` is `None`. A test asserts
  the negative: play a track that is already inside a 3 s window and confirm
  nothing is prefetched after several pump ticks.
- **"The next track" is resolved by `after_end()`**, i.e. the track that will
  actually play when this one finishes, honouring repeat-all wrap. Repeat-one is
  skipped explicitly: `after_end()` returns the *same* track, and prefetching
  the current track into a second core would be pointless work.
- **The incoming core is filtered like the audible one.** The `af` chain and
  the ReplayGain mode are applied *before* `loadfile`, so the first frame is
  already EQ'd. The chain actually handed over is recorded on the `Engine`
  (`idle_af`) — not only for tests: if the user moves a slider during the fade
  window, stage 3 needs to know whether the incoming core is stale.
- **A stale prefetch is dropped, never faded into.** `play_tracks`,
  `reorder_queue`, `remove_queue_at` and `clear_queue` all queue
  `Cmd::ClearPrefetch`.
- **The clear runs on the pump, and that mattered.** The first version dropped
  the idle core directly from the caller's thread. `Mpv::drop` calls
  `mpv_terminate`, so that would have run libmpv off the pump thread — exactly
  the invariant this crate protects. Queue mutations now push a command, and the
  only place a core is released from a foreign thread is `shutdown()`, *after*
  the pump has been joined, where there is nothing left to race.
- `MIN_CROSSFADE_SECONDS = 0.5` floors a nonsensical duration (negative, NaN)
  instead of letting it produce a window that opens after the track ended.

6 new tests: off by default, prefetch happens and changes nothing audible, the
idle core carries the current EQ, a queue edit drops the prefetch, switching
crossfade off releases the core, and pure validation of the preferences.

Gates: `cargo fmt`/`clippy` clean; `cargo test --workspace` 320 passed, 8 ignored
(5 live cover + 3 audio device), verified with `RUST_TEST_THREADS=1`; `npx
tsc --noEmit` and `npm run build` clean.

### Stage 3 — the ramp and the swap — **done**
- At the fade start, un-pause the incoming core and ramp both volumes from the
  user's volume and 0 (equal-power curve, so the sum stays flat)
- Swap `active` at the midpoint, stop the outgoing core, report the new
  `current` track
- `Cmd` variants for the whole window, so `pause`/`seek`/`next`/`prev`/`stop`
  mid-fade route to the right core and cannot race the swap
- **DoD:** `next`/`prev`/shuffle during a fade keep playing audio; state stays
  consistent (one current track, correct `index`, `list_len`)

**Outcome.** The ramp runs on the existing pump — no second thread — by polling
every core with the non-blocking `mpv_wait_event(handle, 0)` the plan predicted,
and blocking on the audible core only when nothing is pending anywhere. The
consequences were worked out before the code was written, and two of them
changed the shape of the thing:

- **Three cores, not two.** The session hands over at the midpoint, so after it
  the *new* core is active while the old one is still audible, and the track
  after that wants prefetching in the same window. With two slots there is no
  free core for that, so every other transition would degrade to a hard cut.
  `CORES = 3` costs one more decoded stream, and only while crossfade is on.
- **The user's volume is no longer libmpv's.** During a ramp each core is driven
  to `user_volume × gain`, so `read_state` reading the `volume` property back
  would have walked the slider down with the fade and then treated the faded
  level as the user's choice. `Player` now owns `user_volume` and writes the
  ramped value to the cores, which keeps the reported volume the user's setting.

Everything else that had to be decided:

- **The handover is idempotent and also runs when the ramp is cut short.** If the
  fading track reaches its end before the ramp does — the window is clamped to
  what is left, but `duration` can still be wrong — the incoming track is
  already audible. Landing the fade hands it the session instead of leaving
  audio playing that nothing can control.
- **Pausing touches every audible core and freezes the ramp clock.** Pausing only
  the active one would leave the other playing under a player the user believes
  is stopped; letting the clock run would spend the ramp while paused. Mute, EQ,
  ReplayGain and speed all reach the prefetching core too, for the same reason.
- **`maybe_prefetch` stands down for the first half of a ramp.** Before the
  handover the free slot *is* the core fading in, and reloading it would throw
  the incoming track away. After the handover the third core is genuinely free,
  so a very short next track is still prefetched in time.
- **Exclusive output refuses the prefetch, and says so once.** `audio_exclusive`
  is a startup-only option, so the guard lives in the player (`Engine::is_exclusive`)
  rather than in the Tauri layer where the order of `apply_settings` calls could
  get it wrong. Crossfade then degrades to ordinary gapless playback.
- **Transport commands follow `active`,** which is the core the user is looking
  at: before the midpoint that is the fading-out track (still what the queue
  shows), after it the incoming one. `next`/`prev`/`load`/`stop` abandon the
  ramp first, so no command can leave two cores audible.

5 new tests: the handover reports the new track/index/position and leaves one
core, interrupting a ramp leaves one core, pausing freezes the ramp and does not
move the queue, equal-power is checked as maths (endpoints, midpoint, monotonic),
and exclusive output never prefetches. The prefetch-only tests from stage 2 moved
to a 6 s fixture: the midpoint lands `window / 2` after the prefetch, and a
2 s fixture left a 1 s margin — too little for a loaded CI runner.

Gates: `cargo fmt`/`clippy` clean; `cargo test --workspace` 325 passed, 8 ignored
(5 live cover + 3 audio device), verified with `RUST_TEST_THREADS=1`; the
crossfade tests repeated three times to check the timing-sensitive ones.

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

**D3 — whether Stage 1 alone is worth landing** if the rest is deferred. (Closed:
it is moot.) Stages 1–3 are landing together, so there is no "stage 1 alone" left
to weigh — the refactor is the smallest possible diff for the ramp, and shipping
it on its own would have meant a featureless extra core slot.

## Risks, and what is done about each

| Risk | Mitigation |
|------|------------|
| **Exclusive output cannot host two cores.** WASAPI exclusive takes the device; a second mpv instance on it will fail or be forced to shared. | Refuse to prefetch when `audio_exclusive` is on and log it once. Crossfade silently degrading to normal gapless is better than an exclusive-mode failure — and issue #24 makes exclusive an explicit opt-in, so the combination is rare and visible. |
| **Wall-clock tests are flaky.** A real fade is a timing test. | Assert on *outcomes* (which track is current, how many cores are live, whether a ramp is in flight) with generous `wait_for` timeouts, not on exact sample positions or ramp values. The equal-power curve itself is checked as pure maths. |
| **Prefetch plays the wrong thing if the queue changes** (reshuffle, remove, clear during the fade window). | Stage 3 turned out not to need a deferred re-validation: the prefetch and the un-pause happen in the same pump iteration, and every queue mutation pushes `Cmd::ClearPrefetch`, which abandons the ramp. |
| **Doubles the audio memory** (two decoded streams live). | Three cores in the worst case, but only during a ramp; the third is created on demand, so a session that never crossfades still holds exactly one. |
| **Two cores contend for the audio device in shared mode.** | WASAPI shared mode is designed for multiple clients; this is the supported configuration. Flagged as the thing to verify by ear first after Stage 3. |
| **A crash in the fade path leaves both cores running.** | The swap is a single guarded `active` reassignment; the outgoing core is stopped and released by `finish_fade`, every interrupting command calls `abandon_fade`, and `Player::shutdown` drains both non-playing roles. |

## Acceptance criteria

- [x] Stage 1 lands with all existing tests green and no behaviour change
- [x] Prefetch never plays without being asked, and always with the current EQ
- [x] A completed crossfade reports the new track, correct index and position
- [x] Transport commands during a fade never leave both cores audible
- [x] Exclusive mode disables crossfade instead of failing
- [ ] Setting is off by default and survives a restart
- [x] `cargo fmt`/`clippy`/`test` and the frontend gates green at every stage

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
3. ~~Whether Stage 1 alone is worth landing~~ — **moot** (see D3): stages 1–3 land
   together.

## References

- `docs/design.md` §4.2 (the decision being implemented), §1 (core features)
- `crates/player/src/player.rs` (`api()` → the engine's active core, `pump`,
  `Cmd`, `read_state`, `Fade`)
- `crates/player/src/engine.rs` (`CORES`, the role a slot holds, `setup_core`)
- `crates/player/src/ffi.rs` (`Mpv::wait_event`, `command`, `get_*`)
- `docs/issues/23-output-device-picker.md` (device state in `PlayerState`),
  `docs/issues/24-wasapi-exclusive-toggle.md` (the exclusive interaction)