---
title: "Output device picker listed nothing, so no device could be chosen"
labels: ["bug", "priority:P1", "area:player"]
milestone: "v0.2.0"
status: "done"
---

## Summary

The **Output device** picker in Settings showed only "System default". No real
device ever appeared, so the control could not be used at all — and switching
output meant editing the settings table by hand.

## Evidence

- `crates/player/src/player.rs` — `read_device_list` read the property with
  `api.get_string("audio-device-list")` and parsed the result as JSON.
- `audio-device-list` is a **list-of-options** property. mpv returns nothing
  useful for it in `MPV_FORMAT_STRING`: the read yields an empty string, the
  JSON parse fails, and `read_device_list` returns `Vec::new()`.
- So the failure was silent by construction: the function logged a warning
  ("was not the expected JSON") and returned an empty list, which is exactly
  what "this machine has no audio device" looks like.

**Why the tests did not catch it.** The three device tests are `#[ignore]`d
(they crash libmpv on a machine with no audio endpoint), and all of them build
their player with `start_player()`, which sets `ao_null: true`. Under `ao=null`
mpv never enumerates anything, so those assertions ran over an empty list and
passed vacuously. A test that cannot fail is worse than no test.

## Fix

The property is now read as an mpv node tree, which is the only way to get a
list-of-options out of libmpv.

- `crates/player/src/ffi.rs` — `Mpv::get_option_list` requests
  `MPV_FORMAT_NODE` (6) and walks the tree, releasing it with
  `mpv_free_node_contents`.
- The walk dispatches on the node's own `format`, because `audio-device-list` has
  shipped as both an array of maps and a map of maps. `device_from_entry` then
  looks for `name` / `description` anywhere in an entry rather than at a fixed
  position, taking the **last** match (a map-of-maps entry starts with mpv's own
  option name).
- mpv's automatic entry (`name: "auto"`, "Autoselect device") is dropped: the
  picker already renders its own "System default" row with an empty value, and
  keeping both would offer the same destination twice under two different values.

### The part that cost the most time

`mpv_node_map` turned out to have the **same field order** as `mpv_node_list`
(`num, values, keys`) — not the reversed order some transcriptions of
`client.h` suggest. Reading it as reversed puts every `keys` pointer on a
`mpv_node*`, which is then dereferenced as a string: `STATUS_ACCESS_VIOLATION`,
with no way to catch it from Rust. The layouts are now one type
(`RawNodeMap = RawNodeList`) so there is exactly one place to be wrong.

`MPV_FORMAT_NODE_LIST` (8) is also **not** a way to ask for this: mpv only
converts *to* a node list, not *from* a node array, and answers
`MPV_ERROR_PROPERTY_FORMAT` (-9).

## Outcome

Verified on a machine with real output (`IWAKS_LIVE_AUDIO=1`): mpv reports 10
entries, 9 of them pickable — including "Speakers", "Realtek HD Audio 2nd
output", "CABLE Input" — and **both before and after playback starts**, so the
picker is populated on a fresh launch rather than only once music is playing.
Switching to a listed device is reflected in the published `audioDevice` state.

Tests:

- `device_from_entry` is pure, so the mapping is covered without libmpv: the array
  shape, the map-of-maps shape, the `auto` entry, an unnamed entry, and an entry
  with nothing usable in it.
- `the_device_list_is_read_from_mpvs_node_tree` opens **real** audio and asserts
  devices are listed before playback and that a listed device can be selected.
  It is gated on `IWAKS_LIVE_AUDIO=1`, because enumerating devices where no
  endpoint exists crashes libmpv — CI has no endpoint and skips it.

## Follow-up (resolved)

The three older device tests now gate on `IWAKS_LIVE_AUDIO` like the live test, so
there is one pattern instead of two and a conscious skip in CI rather than
`#[ignore]` (which in the old test world read as "looks like coverage but never
runs"). Their comment in an earlier draft of this issue called them *vacuous*;
that was true of the string-read era, where `ao=null` meant the list was always
empty and every assertion ran over nothing. Once the node read landed, they
enumerate real devices even under `ao=null` on machines that have endpoints —
verified on a machine with audio, where all three pass and one actually switches
to a listed WASAPI device.

## References

- Related: `docs/issues/23-output-device-picker.md` (the picker this completes),
  `docs/issues/24-wasapi-exclusive-toggle.md`