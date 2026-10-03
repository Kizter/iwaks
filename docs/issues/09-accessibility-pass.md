---
title: "Accessibility pass: icon contrast, minimum type sizes, keyboard queue reorder"
labels: ["enhancement", "priority:P1", "area:a11y"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Several WCAG 2.2 AA gaps found in the grill:

- **Icon contrast:** `--ink-soft #a37a84` (secondary icons) is **3.18:1**
  on the background — below the 4.5:1 UI threshold; the CSS comment claims
  AA but only `--ink` was verified.
- **Type too small:** format badges at `0.6rem` (≈9.6px) and several labels
  at `0.64–0.72rem` fall below the ~11–12px comfort/AA floor.
- **Keyboard queue reorder:** Queue drag-and-drop has no keyboard
  alternative (arrows to move rows).
- **Focus visibility:** relies on a single global outline; inconsistent
  across rows, popovers, and list items.

## Evidence

- `src/App.css:10` — `--ink-soft: #a37a84` (3.18:1).
- `src/App.css:596` — badge `font-size: 0.6rem`; multiple 0.64–0.72rem
  labels (lines 967, 974, 1164, 1389, 1394, 1404).
- `src/App.tsx` — Queue `onReorder` via HTML5 drag & drop only (no keyboard
  path).
- `src/App.css` — one global `:focus-visible` outline rule.

## Impact

AA failure for interface graphics and small text; queue manipulation
impossible without a mouse; keyboard users lose track of focus.

## Suggested direction

1. Darken secondary icons (e.g. `--ink-soft` → ≥4.5:1) or ensure icons are
   never the sole carrier of meaning at 3.18:1.
2. Raise badges/labels to a minimum 0.72–0.75rem (≥11px).
3. Keyboard queue reorder: Up/Down (or Alt+Up/Down) moves the focused row;
   announce with live region or aria.
4. Systematic `:focus-visible` style per component family (rows, cards,
   icon buttons, popovers).

## Definition of Done

- [ ] Computed contrast ≥4.5:1 for every non-large UI element changed
- [ ] Minimum font size ≥11px for functional text (except decorative)
- [ ] Queue rows reorder via keyboard; focus never lost
- [ ] `prefers-reduced-motion` behavior preserved
- [ ] Automated contrast checks if tooling available (or documented manual
      pass on all views)

## References

- Roadmap: checklist item M4
- Related: `docs/issues/11-visual-consistency-polish.md`