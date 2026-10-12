---
title: "Dark mode"
labels: ["enhancement", "priority:P2", "area:polish"]
milestone: "v0.3.0"
status: "done"
---

## Summary

Add a dark theme: a second palette behind the same tokens, a `data-theme` switch
on `<html>`, a System/Light/Dark setting, and no flash on startup.

## Motivation

`design.md` recorded dark mode as a follow-up non-goal. On a dark desktop the
parchment window is the brightest thing on screen, and at night it is
unpleasant.

## Scope

- Two palette blocks in `App.css`, one attribute to choose between them
- `src/theme.ts`: mode resolution + OS watching
- Pre-paint script in `index.html`
- `ui.theme` setting + an Appearance segmented control

## Acceptance criteria

- [x] Zero colour literals outside the two palette blocks
- [x] System follows the OS live, even with the app open
- [x] No flash of the light theme on a dark desktop
- [x] Every text/background pair meets WCAG AA (4.5:1) in **both** themes
- [x] Native controls/scrollbars follow (`color-scheme`)
- [x] Gates green

## Outcome

- **The refactor that made it cheap** — the audit said "2.4k lines of CSS" and
  expected the worst. In fact only **five** colour literals existed outside
  `:root`: four `color: #fff` (all on `background: var(--ink)`) and one
  gradient. They became `--on-ink` and `--art-grad-from/to`, and the stylesheet
  now has **zero** literals outside the palette blocks — so a third theme, or a
  future audit, is a two-block edit.
- **One switch, not two triggers** — `@media (prefers-color-scheme: dark)` plus
  `[data-theme="dark"]` would mean declaring the dark palette twice.
  `system` is resolved in JS instead (`resolveTheme`), so the palette exists
  exactly once, and `watchSystemTheme` keeps following the OS while the window
  is open. The listener is torn down the moment a theme is pinned.
- **No flash** — a small inline script in `index.html` applies the OS preference
  before the first frame. The stored choice arrives over IPC a moment later and
  overrides it, so the only case that can still flash is a user who pinned
  *dark* on a *light* desktop — which is the correct trade, since honouring the
  OS first is what makes the common case instant.
- **Colour-scheme** — each palette block sets `color-scheme`, so scrollbars,
  the `<select>` popup and focus rings follow the theme too.

### Measured contrast (WCAG 2.x, relative luminance)

Every pair a user actually reads, in both palettes:

| Pair | Light | Dark |
|------|-------|------|
| text on bg | 10.33 | 14.02 |
| text on surface | 11.07 | 12.98 |
| text-dim on bg | 5.02 | 7.09 |
| text-dim on surface | 5.39 | 6.56 |
| text-dim on mint | 4.80 | 6.21 |
| ink on bg | 4.72 | 9.25 |
| ink on surface | 5.06 | 8.56 |
| ink-hover on bg | 6.20 | 11.83 |
| ink-soft on bg | 5.18 | 7.97 |
| ink-soft on mint | 4.95 | 6.98 |
| ink-strong on ice | 6.15 | 10.14 |
| ink-strong on mint | 6.97 | 11.04 |
| on-ink on ink | 5.49 | 9.25 |
| danger on its wash | 5.08 | 5.98 |
| text on ice | 8.72 | 11.28 |

All ≥ 4.5:1 (WCAG AA for normal text) in both themes; the dark palette is the
stronger of the two on every pair, because a light-on-dark ground has more room
than dark-on-light at equal lightness. The measured numbers are quoted in the
palette comments so the next change has a baseline to check against.

`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace` (312 passed, 5 ignored live) are green, as are
`npx tsc --noEmit` and `npm run build`.

## References

- `src/App.css` (two palette blocks, `--on-ink`, `.segmented`)
- `src/theme.ts`, `index.html` (pre-paint script)
- `src-tauri/src/lib.rs` (`SETTING_THEME`, `get_theme`, `set_theme`)
- `src/Settings.tsx` (Appearance group), `src/App.tsx` (apply on mount)