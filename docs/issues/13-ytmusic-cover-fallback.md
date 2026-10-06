---
title: "Add YT Music fallback for cover art lookup (provider abstraction)"
labels: ["enhancement", "priority:P3", "area:player"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Add provider abstraction to `crates/cover` and optional YT Music fallback. Keep opt-in, strict matching, cache semantics unchanged (`Ok(None)` never cached as miss, transport errors never cached as miss).

## Motivation

Improve cover hit rate without changing default-off privacy posture.

## Scope

- Add `CoverProvider` trait; keep iTunes default
- Add YT Music provider with minimal deps (`ureq` + `serde`)
- Support fallback chain; keep `lookup()` compatible
- Add unit tests for fallback and error propagation

## Acceptance criteria

- [ ] Provider abstraction added; iTunes remains default
- [ ] YT Music fallback implemented
- [ ] Fallback order correct; existing cover tests pass; new tests added
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo test --workspace` green
- [ ] No privacy model change (network only when opt-in)

## References

- `crates/cover/src/lib.rs`, `crates/cover/src/itunes.rs`
- `crates/library/src/cover.rs`
- `src-tauri/src/cover.rs`
