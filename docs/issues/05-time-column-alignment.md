---
title: "Align list header \"Time\" with the duration column in all lists"
labels: ["bug", "priority:P2", "area:ux"]
milestone: "v0.2.0"
status: "draft"
---

## Summary

The column header "Title / Time" never lines up with the rows: the header
grid (`44px 1fr 128px`) has only **two** children, so "Time" is
auto-placed into the `1fr` track and ends ~140 px left of the actual
duration numbers. The Queue list is worse: rows use a 4-column grid while
the header still uses 3.

## Evidence

- `src/App.css:401-408` — `.track-head` grid `44px 1fr 128px`.
- `src/App.tsx:1664-1667` — header markup has only `<span>Title</span>` +
  `<span class="track-head-dur">Time</span>` (2 children for a 3-col grid).
- `src/App.css:1672` — Queue rows use `28px 44px 1fr 240px` (4 columns)
  vs the same 3-column header.

## Impact

Visible polish defect across every list view; misleads the eye when
matching row durations to the header. Small fix, high visibility.

## Suggested direction

1. Give the header's "Time" an explicit `grid-column: 3` (and match the
   Queue header to the Queue row grid).
2. Consider one shared header component/grid definition per list type so
   header and rows cannot drift apart again.

## Definition of Done

- [ ] "Time" header aligns with duration numbers at 900–1920 px widths
- [ ] Queue header grid matches Queue rows
- [ ] No regression to row rendering (position/cover/menu columns)

## References

- Roadmap: checklist item M4