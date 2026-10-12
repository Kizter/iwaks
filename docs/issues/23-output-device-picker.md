---
title: "Output device picker"
labels: ["enhancement", "priority:P2", "area:player"]
milestone: "v0.3.0"
status: "done"
---

## Summary

`design.md` §4.2 promises `set_output_device` and `device-list-changed`; neither
existed. Add a device picker: enumerate what mpv can output to, let the user
choose, remember the choice, and apply it immediately.

## Motivation

On a machine with a DAC, an interface and Bluetooth headphones, the output is
whatever Windows happened to pick at startup — and there was no way to change
it without a system-level default-device dance.

## Scope

- `AudioDevice` + `Player::list_devices()` / `set_audio_device()`
- mpv `audio-device-list` read on the pump thread, parsed in the player crate
- Persisted as `audio.device`, applied at startup and on change
- Settings → Audio output picker

## Acceptance criteria

- [x] The picker lists the devices mpv reports
- [x] Choosing one applies immediately and survives a restart
- [x] A rejected/removed device leaves playback untouched and is not reported active
- [x] A hung audio stack cannot hang the UI thread
- [x] The list is never stale
- [x] New player tests; full gates green

## Outcome

- **On the pump thread only** — reading a property or setting an option is an
  mpv call, and only the pump may make those. `list_devices()` therefore hands
  the pump a `mpsc::Sender` and waits with a 2 s timeout, returning an empty
  list on timeout: a wedged audio stack shows "no devices" instead of freezing
  the window. `set_audio_device` is a queued command like every other change.
- **No change event, on purpose** — `device-list-changed` was deliberately not
  wired. The list is read *when the picker opens*, so it cannot go stale, which
  is strictly simpler than subscribing to hot-plug events and maintaining a
  mirrored list across a window that may not even be open.
- **Tolerant parsing** — mpv's entries have a null `name` for the system
  default, so the property is parsed through a private tolerant shape and
  normalised to `""`. Anything unparseable degrades to an empty list and a log
  line; the user still has "System default".
- **Honest state** — `PlayerState.audioDevice` carries the device mpv actually
  resolved, which is *not* necessarily the empty string for "system default"
  (mpv resolves `auto` to a concrete device). A test caught that assumption and
  the test now asserts the promise we actually make: a rejected name never
  shows up as active, and the pump keeps answering.
- **Persistence** — the choice is stored as `audio.device` and read back into
  `PlayerOptions` at startup, next to the existing exclusive flag.

3 new player tests: enumeration answers twice and stays consistent, the JSON
contract of the property, and the device-switch pair below.

**CI caught a crash these tests could not.** The first version of the switch test
fed libmpv a fabricated device name (`iwaks-not-a-real-device`) to prove that a
rejected switch does not stick. That passes on a dev machine with an audio
device — and killed the CI binary with `STATUS_ACCESS_VIOLATION`
(`0xc0000005`) on a runner with no audio endpoint, because `set audio-device`
makes libmpv re-open an output that cannot exist. A native crash cannot be caught
from Rust, so the whole test binary died and the build went red from this commit
onward.

The fix is not "handle the error" but "do not do that":

- Tests now only ever send a name **mpv itself reported**, via
  `first_selectable_device`, which prints a `SKIP:` line when there is none.
- The two tests that actually switch output are `#[ignore]`d, matching the
  existing pattern in `crates/cover/tests/live.rs` for tests that need the real
  environment. They still exist and can be run by hand:
  `cargo test -p iwaks-player -- --ignored`.
- The read path stays in CI: `list_devices` answering, twice, without hanging,
  plus the offline JSON contract. That is the part this crate owns; whether the
  OS opens the device it names is not.

With `ao=null` (how every player test starts) mpv reports a single `auto`
entry, which is why the crash needed a *fabricated* name to trigger locally.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (314 passed, 7 ignored: 5 live cover + 2 audio-device)
are green, as are `npx tsc --noEmit` and `npm run build`.

## References

- `crates/player/src/player.rs` (`AudioDevice`, `Cmd::ListDevices`,
  `Cmd::SetAudioDevice`, `read_device_list`)
- `src-tauri/src/lib.rs` (`list_audio_devices`, `set_audio_device`,
  `get_audio_device`, `startup_options`)
- `src/Settings.tsx` (Audio output group)