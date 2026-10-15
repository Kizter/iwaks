---
title: "Other apps go silent while Iwaks runs (exclusive output is on)"
labels: ["bug", "priority:P1", "area:player", "area:settings"]
milestone: "v0.3.0"
status: "done"
---

## Summary

While the session runs with WASAPI **exclusive** output on, audio from other
applications (YouTube, Discord, games) is inaudible. That part is by design — the
endpoint is taken for the session.

The defect is what it took to get the sound back: turning **Exclusive mode** off
in Settings changed nothing until Iwaks was closed and started again. Reported
as: "exclusive off → other apps are audible, but only after I close the app and
run it again."

## Evidence

The user's own state, not a reproduction guess:

- `%APPDATA%\com.iwaks.app\iwaks.db`, `settings` table:
  `audio.exclusive = 'true'`. This is the stored flag, read back into
  `PlayerOptions` at startup (`src-tauri/src/lib.rs:163`).
- `%LOCALAPPDATA%\com.iwaks.app\logs\iwaks-2026-10-14.log:17`:
  `12:57:03 [WARN ] crossfade is unavailable while exclusive output is on`.
  Reachable only when `Engine::is_exclusive()` is true, i.e.
  `audio-exclusive=yes` reached mpv (`crates/player/src/engine.rs:256`).
- `audio.device` is **absent** from the same table, so this is the OS default
  endpoint — no device choice is involved.
- The restart requirement is documented, not accidental: the Settings note said
  "takes effect the next time Iwaks starts" (`src/Settings.tsx`, pre-fix), and
  `docs/issues/24` explains why — "libmpv only accepts `audio-exclusive` before
  initialisation".

## Root cause

Two issues were tangled; only the second is a defect.

1. **Exclusive on silences other apps.** As designed (`docs/design.md:89`):
   shared mode by default, exclusive is opt-in, and the note spells out the
   trade-off. Nothing to fix.

2. **The switch was startup-only — the defect.** `set_audio_exclusive` wrote the
   setting and nothing else; `startup_options` applied it at launch. That was
   deliberate, resting on #24's claim that libmpv cannot accept the option after
   initialisation. **That claim is false.** mpv accepts `set audio-exclusive` at
   runtime and re-opens the audio output itself — measured, below. So the switch
   could have been live all along, and "restart Iwaks to get your sound back" was
   never a requirement.

## Fix

- `crates/player/src/engine.rs` — `Engine::set_exclusive`. The flag is no longer
  fixed at startup, so the crossfade guard (`is_exclusive`) sees the current mode
  and a prefetch core created later inherits it.
- `crates/player/src/player/mod.rs` — `Cmd::SetAudioExclusive(bool)`, applied on
  the pump thread (the only thread allowed to touch libmpv):
  `set audio-exclusive yes|no` on the active core **and** on a prefetched idle
  core, which would otherwise open its output in the old mode when it takes over.
  A refusal is logged and the session keeps the mode mpv actually accepted — the
  state is never faked.
- `src-tauri/src/lib.rs` — `set_audio_exclusive` now stores and then applies to
  the running session, the same order `set_audio_device` uses.
- `src/Settings.tsx` — the copy no longer promises a restart; it says the switch
  applies straight away, and the crossfade note no longer says it needs one.

## Verification

Measured on the real device with `IWAKS_MPV_LOG=v`, toggling while a 4 s track
was playing (`position > 0.2`, so an `ao` was already open):

```
AO: [wasapi] 48000Hz stereo 2ch float
Set property: audio-exclusive=yes -> 1
Trying audio driver 'wasapi'
AO: [wasapi] 48000Hz stereo 2ch s16
Set property: audio-exclusive=no -> 1
Trying audio driver 'wasapi'
AO: [wasapi] 48000Hz stereo 2ch float
```

mpv tears the output down and re-opens it on each change — and the sample format
changes with the mode (`s16` exclusive, `float` shared), so this is the mode
actually changing, not a flag flipping. `-> 1` is mpv reporting success.

Tests:

- `engine::tests::exclusive_output_can_be_switched_while_the_session_runs`
- `player::tests::turning_exclusive_off_mid_session_lets_crossfade_prefetch_again`
  (`ao=null`: after the switch a prefetch appears again, which proves the change
  reached the crossfade guard rather than only the settings table)
- `player::tests::exclusive_output_can_be_switched_while_a_real_device_is_open`
  (`IWAKS_LIVE_AUDIO=1`: needs `audio-exclusive` to be accepted with an `ao`
  already open — the thing #24 assumed impossible)

## Still open (deliberately not guessed)

Whether the endpoint stays locked while playback is **paused** — mpv keeps the
`ao` open on pause so resuming is instant, so silence may outlast sound. This is
unmeasured: it needs a human ear with exclusive on and playback paused. It is
left open on purpose rather than asserted either way.

What changed for the worse-case user is the cost: the remedy is now one click,
not a restart.

## Correction to issue #24

#24's Outcome says "libmpv only accepts `audio-exclusive` before initialisation.
Switching it at runtime would mean tearing down and rebuilding the audio output
mid-playback, so the switch says so in the copy instead of pretending otherwise."

The second sentence is accurate — mpv *does* tear down and rebuild the output —
but mpv performs that itself, and the log above shows it is fast enough for a
switch. The premise ("only before initialisation") is what this issue disproved.

## Acceptance criteria

- [x] Turning exclusive off hands the device back without a restart
- [x] The change reaches the session, not just the stored setting
- [x] Exclusive off keeps the shared behaviour that was already correct
- [x] The UI no longer claims a restart is needed
- [x] Full gates green
- [ ] Confirmed whether other apps recover on pause with exclusive on (open, above)

## References

- `crates/player/src/engine.rs` (`setup_core`, `is_exclusive`, `set_exclusive`)
- `crates/player/src/player/mod.rs` (`Cmd::SetAudioExclusive`, `Options::audio_exclusive`)
- `src-tauri/src/lib.rs` (`SETTING_AUDIO_EXCLUSIVE`, `get_audio_exclusive`, `set_audio_exclusive`)
- `src/Settings.tsx` (Exclusive mode row, crossfade note)
- `docs/issues/24-wasapi-exclusive-toggle.md` (corrected here), `docs/design.md` §4.2
