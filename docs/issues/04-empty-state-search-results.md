---
title: "Empty state must distinguish \"no search results\" from \"empty library\""
labels: ["bug", "priority:P1", "area:ux"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

Typing a search that matches nothing renders the **onboarding / empty
library** state ("Your music lives here — Point Iwaks at a folder"), with a
"Scan a folder" button. For a user with 20k tracks who made a typo, the app
claims the library is empty.

## Evidence

- `src/App.tsx:1401-1411` — `isEmpty = showing.length === 0` with no check
  on whether `query` is non-blank (Songs and grid views).
- `src/App.tsx:1671-1683` — onboarding empty state branch renders when
  `isEmpty`.
- `src/App.tsx:1603` — "showing results for …" label only exists on the
  non-empty path.

## Impact

Misleading orientation flow; a vanished-library false alarm whenever any
search (or a filter) yields zero rows. Undermines trust in the library
view.

## Suggested direction

1. When `query.trim() !== ""` and results are empty → a dedicated
   "No results for '<query>'" state (keep search box focused, offer
   clearing).
2. Keep the onboarding "Scan a folder" empty state only for a genuinely
   empty library with no active query.
3. Apply consistently across Songs, Albums/Artists/Folders grids, and
   playlist detail.

## Definition of Done

- [ ] Zero-result search shows a "No results" state, not onboarding
- [ ] Truly empty library (no query) still shows onboarding
- [ ] Copy strings in code (single source of truth)
- [ ] Frontend tests (vitest) cover both states

## References

- Roadmap: checklist item M4
- Related: `docs/issues/05-time-column-alignment.md`