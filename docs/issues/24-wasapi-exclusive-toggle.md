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

## References

- `src-tauri/src/lib.rs` (`SETTING_AUDIO_EXCLUSIVE`, `startup_options`,
  `get_audio_exclusive`, `set_audio_exclusive`)
- `crates/player/src/player.rs` (`Options::audio_exclusive`)
- `src/Settings.tsx` (Exclusive mode switch)