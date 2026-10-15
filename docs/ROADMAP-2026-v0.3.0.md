---
title: "Roadmap v0.3.0 — loose ends, library browsing, platform depth"
labels: ["roadmap", "enhancement", "priority:P2"]
milestone: "v0.3.0"
status: "done"
---

# Roadmap v0.3.0

> **Meta issue.** Sourced from a full audit of `docs/design.md` against the
> shipped code after v0.2.0 landed (all 13 items done, all GitHub issues
> closed). Every entry below is ordered **easiest → hardest**, so the risky work
> (theme engine, dual-instance audio) is always last.
>
> **Status:** the 13 items are done, but **v0.3.0 is not shipped** — user
> testing of the release candidate found three bugs, swept in
> "Post-audit bug sweep" below. The version bump is committed locally and held.

## Vision

Close the dead ends the audit found (backend commands nothing calls, UI that
blocks or lies), then add the browsing controls the design promised, then go
deeper on platform and audio. Nothing here may destabilise what already works.

## Ordering rule

Each milestone is independently shippable and ordered by *blast radius*, not by
value: small wins first (so the roadmap always has something to show), the two
expensive subsystems last.

| M | Theme | Blast radius | Items |
|---|-------|--------------|-------|
| M1 | Loose ends | UI-only or re-wires an existing command | 15–19 |
| M2 | Library browsing | new SQL + indexes | 20–21 |
| M3 | Platform/reliability | new IPC, device enumeration, disk logging | 22–24 |
| M4 | Visual | every CSS rule + contrast re-audit | 25 |
| M5 | Audio engine | two libmpv instances, per-track fades | 26 |

## Checklist

- [x] **15** Tag editor reachable from playlist rows <!-- docs/issues/15-tag-editor-in-playlists.md -->
- [x] **16** UI rough edges: inert search box, scan-gated queue save, sticky
      banners, native `confirm` <!-- docs/issues/16-ui-rough-edges.md -->
- [x] **17** Global shortcuts: seek ±10 s, `/` focus search, prev/next
      <!-- docs/issues/17-keyboard-shortcuts.md -->
- [x] **18** Playlist reorder in the UI (wires `reorderPlaylist`)
      <!-- docs/issues/18-playlist-reorder.md -->
- [x] **19** Queue: remove one track / clear the queue
      <!-- docs/issues/19-queue-remove-clear.md -->
- [x] **20** Genre + decade filter chips (with indexes)
      <!-- docs/issues/20-genre-decade-filters.md -->
- [x] **21** Sort control (artist/album/title/added/duration)
      <!-- docs/issues/21-sort-control.md -->
- [x] **22** File logging + panic hook (needs `panic = "unwind"`)
      <!-- docs/issues/22-file-logging-panic-hook.md -->
- [x] **23** Output device picker <!-- docs/issues/23-output-device-picker.md -->
- [x] **24** WASAPI exclusive-output toggle
      <!-- docs/issues/24-wasapi-exclusive-toggle.md -->
- [x] **25** Dark mode <!-- docs/issues/25-dark-mode.md -->
- [x] **26** Equalizer preset templates + save custom setups (shipped ahead of
      this batch; GitHub `#14` is the v0.2.0 meta issue)
      <!-- docs/issues/26-equalizer-presets.md -->
- [x] **27** Crossfade <!-- docs/issues/27-crossfade.md -->

## Post-audit bug sweep

User testing of the release candidate found three bugs in work this audit had
already ticked off. They are tracked here because **v0.3.0 is not shipped** — the
version bump is committed locally and held until the sweep is done.

- **31 — other apps stayed silent until Iwaks was closed and restarted**
  (`docs/issues/31-other-apps-silenced-exclusive.md`). The Exclusive mode switch
  only wrote the setting and was applied at startup, resting on #24's claim that
  libmpv takes `audio-exclusive` only before initialisation. It does not: mpv
  takes it at runtime and re-opens the audio output itself, measured with
  `IWAKS_MPV_LOG=v` on a real device (`s16` exclusive vs `float` shared). The
  switch now reaches the running session, prefetch core included.
- **32 — the mini player window ignored the stored theme**
  (`docs/issues/32-mini-player-ignores-theme.md`). The theme was applied in
  `App.tsx`, and the mini player is a second document that only ran the pre-paint
  OS guess.
- **33 — the output device picker offered only "System default"**
  (`docs/issues/33-output-device-picker-first-click.md`). The list was fetched
  from the select's own open gesture, and the fetch disabled the control that was
  being clicked, so the popup was dismissed every time.

Each fix carries regression tests (Rust + vitest), and #24 now has a correction.
Also part of this milestone, from before the sweep: #29 (empty
`audio-device-list`) and #30 (code-review findings).

One out-of-scope item landed anyway: vitest + testing-library (`npm test`, 5
files / 36 tests), which covers the M4/M5 UI work the original audit deferred.

Left open on purpose: whether the endpoint is released while playback is
**paused** with exclusive on — mpv holds the `ao` so resuming is instant. Only a
human ear can settle it; what the fix changed is the cost of the symptom, from a
restart to one click.

## Issue index

| ID | M | Difficulty | Area | Slug |
|----|---|------------|------|------|
| 15 | M1 | trivial | ux | `15-tag-editor-in-playlists` |
| 16 | M1 | easy | ux | `16-ui-rough-edges` |
| 17 | M1 | easy | a11y | `17-keyboard-shortcuts` |
| 18 | M1 | easy | ux | `18-playlist-reorder` |
| 19 | M1 | medium | player | `19-queue-remove-clear` |
| 20 | M2 | medium | library | `20-genre-decade-filters` |
| 21 | M2 | medium | library | `21-sort-control` |
| 22 | M3 | medium | reliability | `22-file-logging-panic-hook` |
| 23 | M3 | medium-hard | player | `23-output-device-picker` |
| 24 | M3 | medium-hard | player | `24-wasapi-exclusive-toggle` |
| 25 | M4 | hard | polish | `25-dark-mode` |
| 26 | M1 | easy | player | `26-equalizer-presets` (shipped before the batch) |
| 27 | M5 | hardest | player | `27-crossfade` |

## Non-functional requirements (carry over from v0.2.0)

| Area | Requirement |
|------|-------------|
| Performance | No new IPC churn; filter/sort must use indexes, never a full-table scan per keystroke |
| Reliability | Logging must never block the UI or the audio thread; device changes must not interrupt playback |
| Safety | Every new query is parameterised; schema changes stay additive and `user_version`-keyed |
| A11y | New shortcuts documented in the shortcuts help; focus never lost; reorder announce via `aria-live` |
| Privacy | Device names and log paths stay local; no telemetry |

## How to publish

Same flow as v0.2.0: draft in `docs/issues/NN-*.md`, then `gh issue create
--title "…" --body-file`, labels + milestone, and tick the checklist above.

## Out of scope (v0.3.0)

- Casting (DLNA/Chromecast) and Bluetooth — post-v1 by decision (D12/D16)
- Bass/treble shelf filters — documented design deviation, the octave
  `equalizer` bands cover it
- Cover art cached to disk (`cover_path`) — superseded by the in-memory +
  `cover_cache` path
- Centralised `AppError` type — broad refactor across every command for little
  user-visible gain; friendly strings already ship

---

_Generated from the v0.2.0 completion audit (design.md ↔ code). Sourced from
concrete `file:line` evidence: three Tauri commands (`playerSeekRelative`,
`playerStop`, `reorderPlaylist`) with no frontend caller, `onEdit` not threaded
into the playlist list, no dark-mode mechanism behind 2.4k lines of CSS, and no
crossfade code despite three design mentions._