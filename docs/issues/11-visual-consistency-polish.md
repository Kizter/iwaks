---
title: "Visual consistency polish: typography scale, radii tokens, single icon system"
labels: ["enhancement", "priority:P3", "area:polish"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

The UI uses ~19 distinct `font-size` values (several under 11 px), 7+
distinct `border-radius` values (6, 7, 8, 10, 12, 14, 26, pill, circle),
and two icon systems (Phosphor + handful of hand-rolled inline SVGs still
left in `App.tsx`). This makes the surface feel inconsistent and blocks
global theming.

## Evidence

- `src/App.css` — no type scale: **58 `font-size` declarations using 21
  distinct values** (`0.6rem`, `0.64rem`, `0.68rem`, `0.7rem`, `0.72rem`,
  `0.74rem`, `0.75rem`, `0.78rem`, `0.8rem`, `0.82rem`, `0.85rem`, `0.9rem`,
  `0.92rem`, `0.95rem`, `0.98rem`, `1.05rem`, `1.1rem`, `1.2rem`, `1.25rem`,
  `1.6rem`, `16px`).
- `src/App.css` — **58 `border-radius` declarations using 10 distinct
  values** (`6px`, `7px`, `8px`, `10px`, `12px`, `14px`, `26px`, `50%`,
  `999px`, `inherit`) with no scale mapping size to radius.
- `src/App.tsx:224-241` — inline `PlusIcon` / `CloseIcon` SVGs vs
  `@phosphor-icons/react` everywhere else.

## Impact

Perceived inconsistency; changing one style (e.g. radius or base size)
requires touching dozens of declarations instead of a token.

## Suggested direction

1. Define a small token scale: `--font-xs…--font-2xl` (6–8 steps,
   ≥11 px for functional text) and `--radius-sm/md/lg/pill`.
2. Replace remaining inline SVGs with Phosphor (project convention from
   M4 slice 6).
3. Purely cosmetic — visual diff should be near-zero; no behavior change.

## Definition of Done

- [ ] All font-sizes and radii use tokens (custom props) — zero hardcoded
      values in components
- [ ] No hand-rolled SVG icons remain (Phosphor only)
- [ ] `npm run build` clean; screenshots unchanged in intent (visual check)

## References

- Roadmap: checklist item M5
- Related: `docs/issues/09-accessibility-pass.md` (min sizes overlap)