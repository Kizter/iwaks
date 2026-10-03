---
title: "<issue title>"
labels: ["<bug|enhancement>", "priority:P1", "area:<area>"]
milestone: "v0.2.0"
status: "draft"
---

<!-- One issue per file. Set `status: ready` once confirmed. Frontmatter
labels/milestone are suggestions applied at publish time; see
docs/ROADMAP-2026-v0.2.0.md → "How to publish". -->

## Summary
<!-- 2-3 sentences: what is wrong / what is missing, from a user's perspective. -->

## Evidence
<!-- File:line references from the v0.1.1 codebase (post-release grill session). -->

## Impact
<!-- Who it hurts, how often, how badly (data loss, annoyance, blocked flow). -->

## Suggested direction
<!-- 2-4 bullets: concrete fix approach. Direction only — no full code. -->

## Definition of Done
- [ ] Observable behavior fixed / implemented (describe outcome)
- [ ] Tests: RED → GREEN → REFACTOR; ≥80% coverage on changed area (TDD)
- [ ] `cargo fmt` + `cargo clippy` clean (Rust) / `npm run build` clean (frontend)
- [ ] `npm run tauri build` still succeeds
- [ ] Immutability convention respected (no mutation of existing objects)
- [ ] Docs updated when behavior/UX changes

## References
- Roadmap (meta issue): `docs/ROADMAP-2026-v0.2.0.md` — checklist item N
- Related: `docs/issues/0X-*.md`