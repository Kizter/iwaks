---
title: "Equalizer preset templates + save custom setups"
labels: ["enhancement", "priority:P2", "area:player"]
milestone: "v0.2.1"
status: "done"
---

## Summary

Add a preset dropdown to the equalizer panel (built-in templates) and let the
user save the current curve as a named custom preset, persisted across
restarts.

## Motivation

The 10-band EQ has no presets, so re-tuning per genre is manual. Templates give
one-click starting points; saved presets keep personal curves without
re-dialling them.

## Scope

- Built-in template curves (Flat, Acoustic, Bass Boost, Classical, Electronic,
  Jazz, Pop, Rock, Treble Boost, Vocal) in a new `src/eq.ts`
- Dropdown in the EQ popover: current preset detected from the live curve,
  built-ins under "Presets", saved ones under "My presets"
- "Save as…" inline form (new name replaces an existing one case-insensitively)
  and delete for the currently active saved preset
- Persistence via the `settings` table (`player.eq_presets` blob), new Tauri
  commands `get_eq_presets` / `set_eq_presets`
- No third-party libraries; UI strings in English

## Acceptance criteria

- [x] Dropdown lists built-in templates and saved presets; picking one applies it
- [x] Changing a band/preamp switches the dropdown back to "Custom"
- [x] "Save as…" persists; saving over the same name replaces, not duplicates
- [x] Saved presets survive restart (round-trip test in `crates/library`)
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `cargo test --workspace` green; `npx tsc --noEmit` + `npm run build` clean

## Outcome

The EQ popover now opens with a preset select. The select reflects the current
curve: it highlights the matching template or saved preset, and shows "Custom"
once any band or the preamp is moved off a known curve. Built-in curves live in
`src/eq.ts` (boosted templates carry negative preamps so the master gain never
clips); the user's own presets are the only ones stored.

- **Storage** — `crates/library/src/persist.rs` gains `SETTING_EQ_PRESETS`
  (`player.eq_presets`) plus `EqPreset { name, preamp, gains }` and
  `save_eq_presets`/`load_eq_presets` on `Library`, mirroring the #01 blob
  pattern. `src-tauri` exposes `get_eq_presets` / `set_eq_presets` (the
  frontend sends only the custom list; built-ins are never written).
- **Curve matching** — `eqSignature` rounds to 0.1 dB, so a preset applied from
  the (0.5-step) sliders still matches after the player round-trips the values.
  Matching prefers built-ins on an exact tie.
- **Save/delete** — saving with an existing name (case-insensitive) replaces
  that preset in place; the trash button appears only when the active curve is
  a saved one, and deletes just that entry.
- **Tests** — 2 new offline tests in `crates/library/tests/persist.rs`:
  presets survive a reopen, and saving replaces the previous list (including
  clearing with an empty list).

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (276 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`. No new dependencies.

## References

- `src/eq.ts` (built-in templates, `eqSignature`, `matchPreset`)
- `src/PlayerBar.tsx` (dropdown + save/delete UI)
- `crates/library/src/persist.rs`, `crates/library/tests/persist.rs`
- `src-tauri/src/lib.rs` (`get_eq_presets` / `set_eq_presets`)
- `src/types.ts`, `src/api.ts`, `src/App.css`