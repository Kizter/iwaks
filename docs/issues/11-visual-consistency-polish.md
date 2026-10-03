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

- `src/App.css` — font-size values spread across the file (e.g. lines 44,
  122, 131, 156, 241, 251, 367, 374, 388, 398, 456, 465, 572, 580, 596,
  654, 679, 729, 738, 815, 854, 876, 931, 940, 967, 974, 1016, 1031,
  1069, 1087, 1094, 1153, 1164, 1278, 1313, 1361, 1389, 1394, 1404, 1587,
  1626, 1678, 1689, 1756, 1770).
- `src/App.css` — border-radius values at lines 68, 117, 152, 181, 229,
  281, 301, 326, 351, 359, 385, 432, 452, 496, 523, 601, 615, 648, 718,
  771, 833, 850, 871, 918, 944, 984, 997, 1006, 1029, 1052, 1068, 1130,
  1138, 1178, 1210, 1256, 1296, 1325, 1357, 1423, 1443, 1452, 1489, 1516,
  1536, 1550, 1580, 1612, 1642, 1697, 1719, 1752.
- `src/App.tsx:222-239` — inline `PlusIcon` / `CloseIcon` SVGs vs
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