---
title: "Align list header \"Time\" with the duration column in all lists"
labels: ["bug", "priority:P2", "area:ux"]
milestone: "v0.2.0"
status: "done"
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

- [x] "Time" header aligns with duration numbers at 900–1920 px widths
- [x] Queue header grid matches Queue rows
- [x] No regression to row rendering (position/cover/menu columns)

## Outcome

The fix itself landed with the issue; what changed afterwards is that it cannot
regress silently. The column tracks moved into two custom properties —
`--track-cols` and `--queue-cols` — and a header and its rows now read the same
declaration instead of each carrying a literal. That was suggested direction #2
("one shared definition so header and rows cannot drift apart"), and it is the
part that matters.

**The three boxes above were manual "look at it and confirm" items.** jsdom has
no layout engine, so no unit test can measure where "Time" landed at 1440 px; a
test claiming otherwise would be theatre. What *is* verifiable — and what caused
the defect — is the invariant behind the pixels, so `src/App.css.test.ts` asserts
it: both selectors resolve to the same token, the queue tracks are four columns
wide with the duration in the last, `.queue-head-dur` is pinned to `grid-column:
4` rather than auto-placed into `1fr`, and the headers reserve their hidden
leading cell.

Both defects were confirmed to fail the suite by re-introducing them on purpose:
a three-column literal on the queue header, and (in the same file's palette
guard) a colour literal smuggled into a component rule.

The stylesheet parser in that test is brace-counting and comment-aware, because
this file has 19 comments containing `{` or `}` — a naive regex pairs every
selector with the wrong body and reports confident nonsense.

## References

- Roadmap: checklist item M4