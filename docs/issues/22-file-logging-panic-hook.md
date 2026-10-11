---
title: "File logging + panic hook"
labels: ["enhancement", "priority:P2", "area:reliability"]
milestone: "v0.3.0"
status: "done"
---

## Summary

Every diagnostic in the app went to `eprintln!` and vanished when the window
closed. Add an append-only daily log file plus a panic hook that records the
message and source location before the process dies.

## Motivation

`design.md` §4.6 promises a log file in `%LOCALAPPDATA%/Iwaks/logs` and a panic
dialog. When something goes wrong at 2am there was nothing to attach to a bug
report.

## Scope

- New `crates/log`: dependency-free file sink, UTC timestamps, panic hook
- Init from the Tauri setup hook, before anything that can fail
- Replace `eprintln!` in the player, cover, presence and app layers
- `log_path` command + a Settings row showing the file
- Release profile: `panic = "abort"` → `"unwind"`

## Acceptance criteria

- [x] Lines are appended to `iwaks-YYYY-MM-DD.log` with a timestamp and level
- [x] Nothing is created before `init`, so tests and library crates stay quiet
- [x] An unusable log directory degrades to stderr-only, no panic
- [x] Panics are recorded with message + `file:line` before the default hook
- [x] The log path is visible in Settings
- [x] No new dependency in any crate; new tests; full gates green

## Outcome

- **A crate, not a module** — `crates/core` is documented as "pure domain logic,
  no I/O", so the sink could not live there and `iwaks-cover` has no dependency
  on core at all. `crates/log` is a new workspace member with **zero
  dependencies** and is shared by the player, cover, presence and app layers,
  so all diagnostics end up in one file. Timestamps are computed here (the
  days→civil-date conversion is ~15 lines and unit tested) rather than pulling
  in a date-time crate.
- **Safe by construction** — the log is the thing you read when something is
  already broken, so every IO error is ignored on purpose, both mutexes are
  poison-tolerant, newlines in a message are folded so one record stays one
  line, and `line()` before `init` is a stderr-only no-op. Tests can call it
  freely without creating files.
- **`panic = "unwind"`, stated explicitly** — `abort` was silently skipping the
  hook. The profile line is kept (with a comment) so nobody "optimises" it back.
- **A dialog was deliberately dropped.** The design also promised one, but
  showing a window from inside a panic hook (during unwind, possibly on the pump
  thread) can deadlock and turn a recoverable bug into a hard crash. The log
  write happens first, so the panic report survives either way.
- **Deadlock caught by a test** — the first `init()` returned `path()` while
  still holding the sink mutex, and `std::sync::Mutex` is not reentrant: the
  second `init()` hung forever. The integration test for "a second init must not
  switch files" found it. Each `init`-dependent scenario now lives in its own
  test binary, because the sink is once-per-process by design.

8 new tests: the date conversion (known dates plus a day-by-day walk across four
years), timestamp padding, level tags, the append round trip, directory
creation, and the unusable-directory fallback.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (308 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`.

## References

- `crates/log/src/lib.rs`, `crates/log/tests/{file,creates_dir,unusable}.rs`
- `src-tauri/src/lib.rs` (setup init, panic hook, `log_path`)
- `src/Settings.tsx` (Diagnostics group), `src/api.ts`
- `Cargo.toml` (new member, `panic = "unwind"`)