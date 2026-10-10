---
title: "Empty state must distinguish \"no search results\" from \"empty library\""
labels: ["bug", "priority:P1", "area:ux"]
milestone: "v0.2.0"
status: "done"
---

## Summary

Typing a search that matches nothing renders the **onboarding / empty
library** state ("Your music lives here — Point Iwaks at a folder"), with a
"Scan a folder" button. For a user with 20k tracks who made a typo, the app
claims the library is empty.

## Evidence

- `src/App.tsx:1432-1442` — `isEmpty` is true whenever the filtered set is
  empty. The song and grid branches (`:1440` `groups.length === 0`, `:1441`
  `showing.length === 0`) carry **no `query` check**, so a search that matches
  nothing is indistinguishable from an empty library. The playlist-detail
  branch (`:1436`) already guards with `query.trim() === ""` — that guard is
  the pattern to copy.
- `src/App.tsx:1711` — the onboarding empty state ("Your music lives here")
  renders on `isEmpty`, so it is what a no-match search shows.
- `src/App.tsx:1643` — the "showing results for …" note exists, but only on
  the non-empty path, so it cannot disambiguate the empty case.

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

- [x] Zero-result search shows a "No results" state, not onboarding
- [x] Truly empty library (no query) still shows onboarding
- [x] Copy strings in code (single source of truth)
- [ ] Frontend tests (vitest) cover both states

## Outcome

`isEmpty` (onboarding) is now gated on `!hasQuery`; a separate `noResults`
flag renders a "No results for “<query>”" state with a Clear-search button
for Songs, Albums/Artists/Folders, and playlist detail. The playlist-detail
"no match" note was folded into the shared state for consistency.

**Deferred:** the repo has no frontend test runner (no vitest/`@types/node`);
adding one is a separate infra task tracked outside this issue.

## References

- Roadmap: checklist item M4
- Related: `docs/issues/05-time-column-alignment.md`