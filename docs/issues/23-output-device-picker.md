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

3 new player tests: enumeration answers twice, a selected device shows up in
the published state while a bogus one does not, the system-default choice never
wedges the thread, and the JSON contract of the property.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (312 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`.

## References

- `crates/player/src/player.rs` (`AudioDevice`, `Cmd::ListDevices`,
  `Cmd::SetAudioDevice`, `read_device_list`)
- `src-tauri/src/lib.rs` (`list_audio_devices`, `set_audio_device`,
  `get_audio_device`, `startup_options`)
- `src/Settings.tsx` (Audio output group)