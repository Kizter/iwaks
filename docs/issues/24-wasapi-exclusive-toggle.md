---
title: "WASAPI exclusive-output toggle"
labels: ["enhancement", "priority:P3", "area:player"]
milestone: "v0.3.0"
status: "done"
---

## Summary

`Options.audio_exclusive` existed but nothing could set it: `Player::start` was
always called with `PlayerOptions::default()`. Expose the bit-perfect mode as a
setting.

## Motivation

`design.md` §4.2 asks for exclusive output for hi-res playback, and the code
deliberately defaulted it off (it silences every other app on the machine).
Being off by default is right; having no way to turn it on was the gap.

## Scope

- Persist `audio.exclusive` and read it into `PlayerOptions` at startup
- Settings switch with the trade-off spelled out
- No runtime switching (libmpv only accepts the option before initialisation)

## Acceptance criteria

- [x] Stored setting is applied at startup
- [x] Default stays off (opt-in, no surprise)
- [x] The UI says when the change takes effect and what it costs
- [x] Full gates green

## Outcome

The startup options are now built from the settings table
(`startup_options(&db_path)`) instead of `PlayerOptions::default()`, which
covers both audio flags and the device choice from issue #23.

- **Applied at startup, on purpose** — libmpv only accepts `audio-exclusive`
  before initialisation. Switching it at runtime would mean tearing down and
  rebuilding the audio output mid-playback, so the switch says so in the copy
  ("takes effect the next time Iwaks starts") instead of pretending otherwise.
- **Off by default, and it stays that way** — an absent or unreadable setting
  means shared mode. A broken settings table can never hand a user
  bit-perfect exclusive output they did not ask for, which is the failure mode
  that made this default controversial in the first place.
- **The trade-off is on screen** — the note explains that exclusive control
  silences other apps while Iwaks plays, so the choice is informed rather than
  a mysterious toggle.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (312 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`.

## Correction (2026-10-15)

The premise above — "libmpv only accepts `audio-exclusive` before
initialisation" — is **wrong**, and it is what turned this deliberate design into
the bug in issue #31 ("other apps stay silent until Iwaks is restarted").

mpv accepts `set audio-exclusive yes|no` at runtime and re-opens the audio output
by itself. Measured with `IWAKS_MPV_LOG=v` while a track was playing:

```
AO: [wasapi] 48000Hz stereo 2ch float
Set property: audio-exclusive=yes -> 1
Trying audio driver 'wasapi'
AO: [wasapi] 48000Hz stereo 2ch s16
```

So the tear-down-and-rebuild this Outcome describes is real, but mpv performs it,
and it is fast enough to be a switch rather than a restart. `set_audio_exclusive`
now applies to the running session; see `docs/issues/31` for the fix. The
startup application here is still correct and still needed — it is just no longer
the *only* way the flag can take effect.

## References

- `src-tauri/src/lib.rs` (`SETTING_AUDIO_EXCLUSIVE`, `startup_options`,
  `get_audio_exclusive`, `set_audio_exclusive`)
- `crates/player/src/player.rs` (`Options::audio_exclusive`)
- `src/Settings.tsx` (Exclusive mode switch)