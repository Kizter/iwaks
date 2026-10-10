---
title: "Add YT Music fallback for cover art lookup (provider abstraction)"
labels: ["enhancement", "priority:P3", "area:player"]
milestone: "v0.2.0"
status: "done"
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

- [x] Provider abstraction added; iTunes remains default
- [x] YT Music fallback implemented
- [x] Fallback order correct; existing cover tests pass; new tests added
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `cargo test --workspace` green
- [x] No privacy model change (network only when opt-in)

## Outcome

`crates/cover` now has a provider seam and a two-provider default chain, with
the cache semantics untouched.

- **Abstraction** — `CoverProvider` (`lookup` → `Ok(Some)` hit / `Ok(None)`
  settled miss / `Err` retryable) with `Itunes` (default) and `YtMusic`, plus
  `Fallback`, which tries providers in order. The free `lookup()` is unchanged
  — it is still iTunes — and a new `lookup_chain()` runs the default chain
  (`default_chain()`). `src-tauri/src/cover.rs` now resolves through
  `lookup_chain`, so the fallback is actually reachable, only when the opt-in
  flag is on.
- **Fallback semantics** — first hit wins; a *miss from every* provider is the
  only case that becomes a cacheable `Missing`. If any provider fails (network
  down), the answer stays retryable even when the others said no, so the cache
  never records an offline minute as "this album has no cover".
- **YT Music provider** — no keyless public API exists, so `ytmusic.rs` talks
  to the same internal `youtubei/v1/search` endpoint the web client uses
  (`WEB_REMIX`, a visitor id scraped from the homepage, a computed client
  version). Parsing is defensive (a recursive scan for
  `musicResponsiveListItemRenderer`) and matching is strict like iTunes:
  normalized artist *and* album must appear among the subtitle parts. Google
  image URLs are upscaled `=w60-h60` → `=w600-h600`; anything unexpected
  degrades to `Ok(None)`, never a panic.
- **Transport** — the single `ureq` call in the workspace moved into `net.rs`
  (one timeout, one error mapping), shared by both providers.
- **Tests** — 19 new offline tests: chain ordering/short-circuit and the
  retryable-vs-settled-miss rule (provider.rs), and YT parsing, matching,
  upscaling, request/visitor-id shape, and the date helper (ytmusic.rs). Two
  ignored live smoke tests cover the YT contract and the iTunes-first chain.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (274 passed, 3 ignored offline) are green. No new
dependencies: `ureq` + `serde`/`serde_json` as before.

## References

- `crates/cover/src/lib.rs`, `crates/cover/src/itunes.rs`
- `crates/library/src/cover.rs`
- `src-tauri/src/cover.rs`
