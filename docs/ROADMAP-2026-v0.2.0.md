---
title: "Roadmap v0.2.0 — persistence, scan data-safety, player reliability, UX & a11y"
labels: ["roadmap", "enhancement", "priority:P1"]
milestone: "v0.2.0"
status: "draft"
---

# Roadmap v0.2.0

> **Meta issue.** Track the v0.2.0 work as one checklist that points at the
> individual issues in `docs/issues/`. Each issue is self-contained and can
> be assigned/closed independently.

## Vision

Make Iwaks feel finished: settings and playback survive restarts, a scan can
never silently empty the library, the player never hangs at exit, and the UI
stops misleading (search results, column alignment, labels, contrast).

## Principles (from repo conventions — AGENTS.md)

- **TDD**: RED → GREEN → REFACTOR; ≥80% coverage on changed areas.
- **Immutability**: create new objects; never mutate existing ones.
- **Surgical**: narrowest responsible layer; no scope creep (YAGNI).
- **DB migrations**: additive, `PRAGMA user_version`-keyed, backward
  compatible (existing v2 DBs must open cleanly).
- **Errors to users are friendly**; full detail stays in logs.

## Non-functional requirements (explicit)

| Area | Requirement |
|------|-------------|
| Performance | No IPC/state churn on slider drag; no per-tick DB writes; queue refetch only when the queued list actually changes |
| Reliability | App exit must never hang (any player-init failure path); a scan under IO stress must never prune rows for files that exist |
| Scale | Works at the design target (1k–20k tracks): scan, list, search, queue restore |
| Security/privacy | No secrets; data stays local; no new telemetry; sanitize paths surfaced in errors |
| Maintenance | One template (`docs/issues/TEMPLATE.md`), consistent labels/DoD per issue; single source of truth for copy strings |

## Execution order

1. **M1 — Persistence & session restore** (`01`) — the highest user value.
2. **M2 — Scan data-safety** (`02`) + **M3 — Player init deadlock** (`03`) —
   reliability pair; both small, both green quickly.
3. **M4 — UX & copy batch** (`04`, `05`, `06`, `07`, `08`, `09`).
4. **M5 — Design goals & polish** (`10`, `11`, `12`) — largest scope; ship
   last or as follow-ups.

## Checklist

- [x] **01** Persist player settings & restore last session
      <!-- docs/issues/01-persistence-settings.md -->
- [x] **02** Scope `clean_missing` prune to scanned root; never prune on IO
      errors <!-- docs/issues/02-scan-clean-missing-scope.md -->
- [x] **03** Fix self-join deadlock when `Player::start` fails
      <!-- docs/issues/03-player-start-deadlock.md -->
- [x] **04** Empty state distinguishes "no results" from "empty library"
      <!-- docs/issues/04-empty-state-search-results.md -->
- [x] **05** Align list header "Time" with duration column
      <!-- docs/issues/05-time-column-alignment.md -->
- [x] **06** Queue view refreshes after reshuffle
      <!-- docs/issues/06-queue-reshuffle-refresh.md -->
- [x] **07** Volume slider commits on release, not per event
      <!-- docs/issues/07-volume-slider-commit.md -->
- [x] **08** `.m4a` badge corrected; friendly error copy
      <!-- docs/issues/08-m4a-badge-and-error-copy.md -->
- [x] **09** Accessibility pass (contrast, min type, keyboard reorder)
      <!-- docs/issues/09-accessibility-pass.md -->
- [x] **10** Album-art-dominant Now Playing view
      <!-- docs/issues/10-now-playing-view.md -->
- [x] **11** Visual consistency (font scale, radii tokens, one icon system)
      <!-- docs/issues/11-visual-consistency-polish.md -->
- [x] **12** Edge cases (dup playlist names, path dedupe, seek-blur, m3u
      encoding) <!-- docs/issues/12-edge-cases-data-hygiene.md -->
- [x] **13** YT Music fallback for cover art lookup (provider abstraction)
      ([#13](https://github.com/Kizter/iwaks/issues/13))
      <!-- docs/issues/13-ytmusic-cover-fallback.md -->

## Issue index

| ID | Sev | Area | Slug | Labels |
|----|-----|------|------|--------|
| 01 | P1 | persistence | `01-persistence-settings` | enhancement, priority:P1 |
| 02 | P1 | scan | `02-scan-clean-missing-scope` | bug, priority:P1 |
| 03 | P1 | player | `03-player-start-deadlock` | bug, priority:P1 |
| 04 | P1 | ux | `04-empty-state-search-results` | bug, priority:P1 |
| 05 | P2 | ux | `05-time-column-alignment` | bug, priority:P2 |
| 06 | P2 | ux | `06-queue-reshuffle-refresh` | bug, priority:P2 |
| 07 | P2 | player | `07-volume-slider-commit` | enhancement, priority:P2 |
| 08 | P2 | copy | `08-m4a-badge-and-error-copy` | bug, priority:P2 |
| 09 | P1 | a11y | `09-accessibility-pass` | enhancement, priority:P1 |
| 10 | P2 | ux | `10-now-playing-view` | enhancement, priority:P2 |
| 11 | P3 | polish | `11-visual-consistency-polish` | enhancement, priority:P3 |
| 12 | P3 | polish | `12-edge-cases-data-hygiene` | bug, priority:P3 |
| 13 | P3 | player | `13-ytmusic-cover-fallback` | enhancement, priority:P3 |

## How to publish

1. Create labels (once): `bug`, `enhancement`, `roadmap`,
   `priority:P1..P3`, `area:persistence|scan|player|ux|a11y|copy|polish`.
2. Create the milestone `v0.2.0`.
3. For each `docs/issues/0N-*.md` (after review): head from the frontmatter
   becomes the issue title; body from `## Summary` onward becomes the issue
   body (`gh issue create --title "…" --body-file path`). Add labels +
   milestone.
4. Publish this roadmap **last**, converting its checklist into
   `- [ ] #NN` references to the real issue numbers.
5. Update frontmatter `status: draft → ready` as items are confirmed.

## Out of scope (v0.2.0)

- DLNA/Chromecast casting and Bluetooth (post-v1, design §4.5)
- Dark mode (recorded follow-up)
- Visualizer (removed in M4 slice 6)
- Anything not in the issue index — add an issue first

## Understanding summary & assumptions

Origin: post-release grill of v0.1.1 (two independent design + functional
reviews, key claims re-verified against source). Session decisions:

| # | Decision | Alternatives | Why |
|---|----------|--------------|-----|
| D1 | Draft issues as markdown first, publish later | Publish via gh now; REST token | No `gh` CLI; review-safe; repo keeps an audit trail |
| D2 | Roadmap meta + ~12 per-area issues | 5 big issues; 1 per finding | Granular independent tracking with a single index |
| D3 | One file per issue + TEMPLATE.md | Single huge file; GitHub issue templates | Ready for `gh issue create --body-file`; diff/review per issue |
| D4 | Issue bodies in English | Indonesian | Public repo, English UI |
| D5 | DoD per issue (TDD ≥80%, fmt/clippy, build) | Loose acceptance | Matches AGENTS.md conventions |
| D6 | Execution order M1 → M3 → M4 → M5 | Priority-only sort | Value first, then reliability, then UX |
| D7 | Out-of-scope list explicit | Implicit | Prevents scope creep in meta issue |

Assumptions carried forward: scan-root prune semantics are resolved inside
`02` (issue thread decides preserve-vs-prune details); persistence scope
(settings only vs queue restore) decided inside `01`.

---

_Generated from the v0.1.1 grill session (2026-09-29). See `docs/design.md`
→ Amendemen M5 (roadmap v0.2.0)._