---
title: "Commit volume slider changes on release, not on every input event"
labels: ["enhancement", "priority:P2", "area:player"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Dragging the volume slider fires one IPC invoke (`playerSetVolume`) per
`input` event — dozens per second. The EQ and seek controls in the same file
already use a correct draft-then-commit pattern; volume is the odd one out.

## Evidence

- `src/PlayerBar.tsx:488` — `onChange={(e) => void playerSetVolume(...)}`
  (per-event invoke).
- `src/PlayerBar.tsx:165-190` — EQ uses `eqDraft` + commit on release.
- `src/PlayerBar.tsx:146-151, 246-259` — seek uses `drag` state + commit on
  pointer/key release.
- Cost per event is **an IPC round trip plus a queue push**, not a synchronous
  state read: `Player::set_volume` only pushes `Cmd::SetVolume`
  (`crates/player/src/player.rs`), and the pump drains the queue and emits on
  its own `TICK_EVERY = 100 ms` cadence (`:46`, used at `:706`). So the
  backend is not doing a `read_state()` per event — the waste is purely the
  invoke rate from the frontend, which is why fixing it needs no backend
  change.

## Impact

IPC chatter and re-render pressure during a drag; on lower-end machines this
contends with the pump while hi-res audio is playing. The audio path itself is
already batched by the queue, so this is churn, not glitch.

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