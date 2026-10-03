---
title: "Commit volume slider changes on release, not on every input event"
labels: ["enhancement", "priority:P2", "area:player"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Dragging the volume slider fires one IPC invoke (`playerSetVolume`) per
`input` event — dozens per second — each triggering an mpv command, a full
`read_state()`, and a `player-state` emit that re-renders the app. The EQ
and seek controls in the same file already use a correct draft-then-commit
pattern; volume is the odd one out.

## Evidence

- `src/PlayerBar.tsx:488` — `onChange={(e) => void playerSetVolume(...)}`
  (per-event invoke).
- `src/PlayerBar.tsx:165-190` — EQ uses `eqDraft` + commit on release.
- `src/PlayerBar.tsx:146-151, 246-259` — seek uses `drag` state + commit on
  pointer/key release.
- Backend cost per call: `crates/player/src/player.rs` `set_volume` + full
  `read_state()` + emit (≈4 Hz pump and sync per event).

## Impact

UI churn and IPC volume during drag; on lower-end machines this contends
with the pump while hi-res audio is playing.

## Suggested direction

1. Keep a local draft while dragging; commit once on `pointerup`/`keyup`
   (blur commits too), mirroring the EQ pattern.
2. Verify keyboard arrow interactions still commit (keyup path).

## Definition of Done

- [ ] A full drag produces ≤2 IPC invokes (start state change + commit)
- [ ] Keyboard (arrows) volume changes still work
- [ ] No flicker: thumb follows local draft immediately

## References

- Roadmap: checklist item M4