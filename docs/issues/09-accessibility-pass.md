---
title: "Accessibility pass: icon contrast, minimum type sizes, keyboard queue reorder"
labels: ["enhancement", "priority:P1", "area:a11y"]
milestone: "v0.2.0"
status: "done"
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

- `src/App.css:9` — `--ink-soft: #a37a84` (3.18:1).
- `src/App.css:604` — badge `font-size: 0.6rem`; multiple 0.64–0.72rem
  labels (lines 406, 823, 884, 975, 982, 1172, 1397, 1402, 1412, 1835, 1867).
- `src/App.tsx` — Queue `onReorder` via HTML5 drag & drop only (no keyboard
  path).
- `src/App.css` — one global `:focus-visible` outline rule (`:65`) plus
  input-specific overrides that replace it with a box-shadow ring.

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

- [x] Computed contrast ≥4.5:1 for every non-large UI element changed
- [x] Minimum font size ≥11px for functional text (except decorative)
- [x] Queue rows reorder via keyboard; focus never lost
- [x] `prefers-reduced-motion` behavior preserved
- [x] Automated contrast checks if tooling available (or documented manual
      pass on all views)

## Outcome

1. **Icon contrast** — `--ink-soft` darkened `#a37a84 → #7f5865`. Measured
   (WCAG formula, computed script) on every surface it is used against:
   `--bg` 5.18:1, `--sidebar` 4.81:1, `--mint` 4.95:1, `--surface` 5.55:1 —
   all ≥4.5:1 (was 3.18:1 on bg, 2.96 on `--sidebar`).
2. **Minimum type size** — every functional font-size below 11 px raised to
   `0.72rem` (11.52 px): `.track-badge` (0.6), `.eq-val` (0.64),
   `.eq-freq`/`.mp-panel-count` (0.68), and the 0.7rem labels
   (`.sidebar-label`, `.mp-source-name`, `.queue-now`, `.set-group-title`).
   Smallest remaining functional size is 0.72rem.
3. **Keyboard queue reorder** — a focused queue row moves with
   `Alt+ArrowUp` / `Alt+ArrowDown`; the move is announced via a visually
   hidden `role="status"` live region and focus follows the moved track
   (row keys keep the DOM node, plus a post-reorder `scrollIntoView`/focus
   guard). Rows expose `tabIndex`, `aria-label`, and `aria-keyshortcuts`.
4. **Focus visibility** — the global `:focus-visible` rule no longer forces
   `border-radius: 6px` on focused elements (it previously mutated the
   element's radius). Inputs keep a solid `2px + 4px` ring instead of the
   old 15%-alpha ring (~1.2:1 effective). Virtualized rows and grid cards
   get an inset `-2px` outline so scroll edges can't clip it.
5. **Reduced motion** — unchanged; the new focus/keyboard work adds no
   animation (post-reorder scroll is `block: "nearest"`, instant).

No contrast lint tooling exists in the repo (no stylelint/axe config), so
values were computed with a one-off WCAG script and `npm run build` /
`npx tsc --noEmit` gate the change.

## References

- Roadmap: checklist item M4
- Related: `docs/issues/11-visual-consistency-polish.md`