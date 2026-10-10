---
title: "Fix self-join deadlock when `Player::start` fails (libmpv init / 30s timeout)"
labels: ["bug", "priority:P1", "area:player"]
milestone: "v0.2.0"
status: "done"
---

## Summary

When `Player::start` fails — libmpv init error, or the 30 s init timeout —
the caller drops its `Arc<Player>`, leaving the pump thread holding the
last reference. That thread's exit runs `Drop` → `shutdown()` →
`thread.take()` → `handle.join()` **on itself** → permanent deadlock; the
thread leaks and app shutdown hangs.

## Evidence

- `crates/player/src/player.rs:272-279` — `recv_timeout(30s)` returns `Err`
  and sets `cancelled`; **no `shutdown()` / no join** on the failure path.
- `crates/player/src/player.rs:467-477` — `shutdown()` does
  `handle.join()` unconditionally after `thread.take()`.
- `crates/player/src/player.rs:813-817` — `Drop for Player` calls
  `shutdown()`; the last `Arc` in a failed start lives on the pump thread.
- Caller side: `src-tauri/src/lib.rs` → on `Err`, `player` slot stays `None`
  and the failed `Arc` is dropped by the caller.

## Impact

App hangs at exit on machines where libmpv is present but initialization
fails (e.g. missing MSVC runtime), or when init stalls past 30 s. Healthy
v0.1.1 installs are unaffected — this bites exactly the failure paths the
playback-unavailable UI is meant to handle gracefully.

## Suggested direction

1. Record the pump thread's `ThreadId` at spawn; in `Drop`, skip the `join`
   when called from the pump thread itself (detach semantics).
2. Alternatively, keep an `Arc` alive on the caller side until the pump
   thread has fully exited, so the last drop never happens on the pump thread.
3. Add a forced-failure test: make `Mpv::new()` fail (env hook), start a
   `Player`, drop it, assert exit is clean and no hang.

## Definition of Done

- [x] Simulated init failure (and timeout) exits cleanly, no hang, no leaked
      thread
- [x] Normal shutdown path unchanged (deterministic, idempotent)
- [x] Tests for both failure paths (≥80% area coverage)

## Outcome

Implemented by suggested direction 1: `Player` records the pump thread's
`ThreadId` at spawn (`pump_thread`). `shutdown()` compares it with the caller
and **detaches** (drops the `JoinHandle`) when they match — the case where the
last `Arc` is released on the pump thread after a failed/timed-out `start` —
instead of joining itself. The normal path (join) is unchanged.

- `crates/player/src/player.rs` — `pump_thread` field, id recorded in `start`,
  self-join guard in `shutdown`, and the unit test
  `shutdown_on_pump_thread_detaches_instead_of_self_joining` (a bare `Player`
  with a still-running stand-in pump thread; shutdown must return promptly
  rather than join from the recorded pump thread).

## References

- Roadmap: checklist item M3