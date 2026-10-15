---
title: "Output device picker shows only \"System default\" until a second click"
labels: ["bug", "priority:P1", "area:settings", "area:player"]
milestone: "v0.3.0"
status: "done"
---

## Summary

In Settings → Audio output, the **Output device** control listed only
"System default" and could not be opened at all. The user's report: "when the
dropdown is clicked it doesn't work, only System default". They had therefore
never been able to choose a device, and the DB has no `audio.device` row.

This is **not** a recurrence of issue #29. There, the Rust side could not read
`audio-device-list` at all (`MPV_ERROR_PROPERTY_FORMAT`, -9). That is fixed, and
the logs prove enumeration works on this machine.

## Evidence

- Enumeration works in the build that wrote the logs —
  `%LOCALAPPDATA%\com.iwaks.app\logs\iwaks-2026-10-14.log:7-10`:
  `audio-device-list: 10 entries, 9 pickable` (four times, consistent). So nine
  devices were read and handed back by `read_device_list`; the list was never
  empty, and the defect is entirely in the front end.
- `%APPDATA%\com.iwaks.app\iwaks.db`, `settings` table: there is **no**
  `audio.device` key, so `getAudioDevice()` resolves `""` and the control sits on
  its default row.
- `src/Settings.tsx` (pre-fix): the fetch was wired to the control's own open
  gesture (`onFocus={loadDevices} onClick={loadDevices}`) and the control was
  `disabled={devicesLoading || device === null}` — the fetch disabled the very
  control that started it.

## Root cause

**Confirmed** by reading the wiring, and it explains the "it never works" report
better than the earlier reading of this document ("a second click works"):

1. Opening a `<select>` fires the pointer gesture at the control; `onFocus` (and
   `onClick`) called `loadDevices()`, which set `devicesLoading = true`.
2. That made the select `disabled` mid-gesture. WebView2 dismisses the popup of a
   control that becomes disabled during the gesture, and a click landing in that
   window is swallowed.
3. **Every click repeats it.** `onFocus` does not fire again once the element is
   focused, but `onClick` does, so the second click re-ran the fetch and dismissed
   the popup exactly as the first did. That is why the user saw "tidak bisa"
   rather than the flakiness this document originally assumed.
4. Independently of the disabled race: even a successful fetch resolved *after*
   the popup was built, so the popup could only contain the static
   `<option value="">System default</option>`.

## Fix

- `src/Settings.tsx` — the list is read in the mount effect that already reads
  `getAudioDevice()` / `getAudioExclusive()`, so the options are in the DOM
  before anyone clicks. The note ("the list is read again each time you open this
  page") now describes the code exactly.
- The select is no longer disabled while loading. It is disabled only until the
  stored choice arrives, so it never flashes "System default" over a saved
  device.
- The `onFocus` / `onClick` refresh is gone: the fetch is what closed the popup.
- A rejected `getAudioDevice()` no longer leaves the picker disabled forever —
  that was the one remaining path to a permanently dead control.

## Verification

New tests in `src/Settings.test.tsx` (6, all passing):

- lists the devices mpv returned without needing a click first
- is enabled — a select that cannot be opened is the bug
- stays enabled while the list is still being read
- does not re-read the list merely because the picker was opened
- leaves the picker usable when the stored device cannot be read
- still saves the device the user picks

**Not verified by these tests:** the native popup is WebView2's, and jsdom has
none. The tests assert the two facts the failure was made of — options present
before the click, control enabled during a load — not that a popup opens.
Confirming the click end to end is a manual step on a dev build.

## Acceptance criteria

- [x] The options are present the moment the page opens, before any click
- [x] No interaction is swallowed by a loading state
- [x] The list is read once per page open, matching the on-screen promise
- [x] A failed read cannot leave the control unusable
- [ ] Choosing a device persists `audio.device` and survives a restart
      (existing path, unchanged by this fix; needs a manual pass)
- [ ] Manual pass on a dev build: one click on a fresh launch lists the devices

## Note

`crates/player/src/player/mod.rs:408-410` warns that an empty result is what
"happens when no audio output has been opened yet", while `docs/issues/29`
records the opposite ("both before and after playback starts"). One of the two is
wrong; worth settling so the next person does not chase the wrong one. This
issue is unaffected either way — the list was never empty here.

## References

- `src/Settings.tsx` (mount effect, `pickDevice`, the Output device row)
- `src/Settings.test.tsx` (new regression tests)
- `crates/player/src/player/mod.rs:406-435` (`read_device_list`)
- `crates/player/src/ffi.rs:384` (`get_option_list`)
- Related: `docs/issues/23-output-device-picker.md`, `docs/issues/29-audio-device-list-empty.md`
