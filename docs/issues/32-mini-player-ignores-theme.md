---
title: "Mini player window ignores the stored app theme"
labels: ["bug", "priority:P2", "area:ux"]
milestone: "v0.3.0"
status: "done"
---

## Summary

When the window is minimized, the mini player does not match the theme chosen in
the main window (the Main menu). Pin **Light** on a dark desktop and the mini
player still renders dark, and switching the theme does not update a mini window
that is already open.

## Evidence

The theme is applied **per document**, and only one of the two documents applies
it:

- `src/App.tsx:1237-1255` (pre-fix) — the main window read the stored mode with
  `getTheme()` and called `applyTheme(choice)`; for `system` it also subscribed to
  the OS via `watchSystemTheme`.
- `src/MiniPlayer.tsx` — the whole file never called `getTheme` or `applyTheme`;
  there was no theme code in it at all.
- `src/main.tsx:8` — the mini layout is selected purely by the URL hash
  (`#mini`), so the theme step in `App.tsx` was never reached for that window.
- `index.html:8-21` — a pre-paint script sets `data-theme` from the **OS**
  preference. This was the only thing the mini window's document ever got.
- `src/App.css:49,99` — both palettes hang off `:root[data-theme="light"]` /
  `:root[data-theme="dark"]`, and `.mini-player` (`src/App.css:1671-1683`) uses
  `var(--surface)` / `var(--text)`. The CSS was already theme-aware; it was the
  attribute that was wrong.

The mini window is created as a separate window at minimize time
(`src-tauri/src/lib.rs:753-774`, `WebviewUrl::App("index.html#mini")`), so it is
a fresh WebView2 document that re-runs the pre-paint script and nothing else.

The stored value on the user machine is `ui.theme = 'system'`, so the mismatch
surfaces as soon as a theme is **pinned**, or when the theme is changed while the
mini window is open.

## Root cause

`applyTheme` was called from `App.tsx` only. The mini window is a second
document: it resolved the OS preference at load and never learned the stored
choice, nor received a change notification.

## Fix

- `src/theme.ts` — `initTheme()`: reads the stored mode, applies it, follows the
  OS while the mode is `system`, and re-applies when the mode changes in the
  other window. Returns an unsubscribe.
- `src/theme.ts` — `applyThemeMode()`: apply a mode *and* own the OS watcher. The
  watcher is a single piece of module state, so a second caller cancels the first
  one's instead of racing it for `data-theme`. This is what stops a pinned theme
  from being overridden by an OS change a moment later.
- `src/main.tsx` — `initTheme()` before the first render, for whichever root this
  is. One call site now serves both windows.
- `src/App.tsx` — the per-root theme effect is gone, along with its imports; the
  comment left behind says where theming moved and why (a second document needs
  the same start-up path, so it cannot live in one root's effect).
- `src/Settings.tsx` — `pickTheme` applies through `applyThemeMode` and no longer
  manages its own watcher (two watchers would have fought once `applyThemeMode`
  existed). It still applies locally first, so the choice lands without waiting
  for the round trip.
- `src-tauri/src/lib.rs` — `set_theme` now emits `theme-changed` to **every**
  window after the write succeeds.
- `src/api.ts` — `listenTheme()`, an `UnlistenFn` wrapper like `listenPlayer`.

**Deviation from the fix sketched above, on purpose.** The plan was to have
`pickTheme` broadcast the mode over the Tauri event channel from the frontend.
Emitting in the `set_theme` command instead means: the frontend needs no
`emit` capability (the mini window only has `core:default`), every writer of the
setting is announced exactly once, and the window that made the choice reacts
through the same path as the other one.

## Verification

- `src/theme.test.ts` (new, 6 tests): the stored mode is applied whoever is
  rendering; a cross-window `theme-changed` re-applies; a payload naming an
  unknown mode is ignored rather than written into `data-theme`; switching back
  to `system` resumes following the OS *live*; a pinned mode stops following the
  OS (the single-owner watcher); and a rejected `getTheme` falls back to the OS
  preference.
- `npm test`: 36 tests across 5 files.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` (342 passed), `npm run build` (tsc + vite): green.

## Not verified here

The two windows themselves. jsdom has a single document, so the tests prove the
*mechanism* — a listener that re-applies, one owner of the OS watcher — not that
two WebViews agree. The pre-existing behaviour of `system` in the main window is
covered by the same tests. Confirming it by eye is a manual step on a dev build:

1. Minimize (or open the mini player), then pin **Light** with the desktop dark:
   both windows should be light.
2. With the mini player open, change the theme in Settings: it should follow
   without being reopened.
3. Pin **system** and flip the OS theme: both should follow.

## Acceptance criteria

- [x] The stored mode is applied in the mini window, not just the OS preference
- [x] Changing the theme updates an already-open mini window (via a broadcast)
- [x] Pinning a mode stops the OS preference from overriding it, in both windows
- [x] `system` still follows the OS, including live changes
- [x] Full gates green
- [ ] Manual pass on a dev build (the three steps above)

## References

- `src/theme.ts` (`initTheme`, `applyThemeMode`, `applyTheme`, `watchSystemTheme`)
- `src/main.tsx`, `src/App.tsx`, `src/Settings.tsx` (`pickTheme`)
- `src/api.ts` (`getTheme`, `setTheme`, `listenTheme`)
- `src-tauri/src/lib.rs` (`get_theme`, `set_theme`, `open_mini_window`)
- `index.html:8-21` (pre-paint script)
- Related: `docs/issues/25-dark-mode.md`
