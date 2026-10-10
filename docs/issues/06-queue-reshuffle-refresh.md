---
title: "Refresh the Queue view after reshuffle (queue signature misses permutation order)"
labels: ["bug", "priority:P2", "area:ux"]
milestone: "v0.2.0"
status: "done"
---

## Summary

Pressing the "reshuffle" (dice) button reorders the remaining queue in the
backend, but the Queue view keeps showing the stale order until some other
state change happens.

## Evidence

- `src/App.tsx:1064-1070` — queue refetch is keyed on `queueSig` =
  `` `${index}|${listLen}|${shuffle}` `` only.
- `crates/player/src/player.rs:509-514` — `apply_reshuffle` swaps in a new
  permutation and emits state; `index`, `listLen`, and `shuffle` are all
  unchanged, so `queueSig` does not change.
- Manual reorder is safe because `onReorderQueue` refetches explicitly
  (`src/App.tsx:1280-1282`).

## Impact

Queue view lies about the actual play order after "randomize rest" until a
coincidental state change; users infer the wrong next track.

## Suggested direction

1. Include a permutation epoch in the emitted state (e.g. a monotonically
   bumped `queueVersion` on reshuffle/reorder) and add it to `queueSig`.
2. Or refetch the queue when the reshuffle command resolves.

## Definition of Done

- [x] After dice-reshuffle, the Queue view order matches the real play order
      immediately
- [x] No extra refetch on ordinary ticks (no per-tick IPC churn)

## Outcome

Implemented by suggested direction 1: `PlayerState.queueVersion` is a
monotonic counter bumped on every queue-order mutation (reshuffle, reorder,
shuffle toggle, list swap) and included in the frontend `queueSig`. Ordinary
ticks leave it untouched, so there is no per-tick refetch. Covered by the
`reshuffle_bumps_queue_version_so_the_view_can_refetch` test in
`crates/player/src/player.rs`.

## References

- Roadmap: checklist item M4