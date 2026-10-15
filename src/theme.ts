/**
 * Light/dark theming (issues #25, #32).
 *
 * The palettes live in `App.css` as two blocks that both declare the same
 * custom properties; `data-theme` on `<html>` picks one. Nothing else in the
 * stylesheet knows a theme exists, so adding a theme cannot leave a stray
 * hardcoded colour behind — there are none outside the two blocks.
 *
 * "System" is resolved in JavaScript rather than with a
 * `@media (prefers-color-scheme)` override, so the dark palette is declared
 * exactly once instead of once per trigger.
 */

import { getTheme, listenTheme } from "./api";

/** What the user picked: follow the OS, or pin one theme. */
export type ThemeMode = "system" | "light" | "dark";

export const THEME_MODES: Array<{ value: ThemeMode; label: string }> = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/** The two themes `data-theme` can carry. */
export type ResolvedTheme = "light" | "dark";

const QUERY = "(prefers-color-scheme: dark)";

/** True when the OS currently asks for a dark UI. */
export function prefersDark(): boolean {
  return typeof window !== "undefined" && window.matchMedia(QUERY).matches;
}

/** The theme a mode means right now. Anything unknown falls back to `light`. */
export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "dark") return "dark";
  if (mode === "light") return "light";
  return prefersDark() ? "dark" : "light";
}

/** Is this a mode we understand? (Stored values come from disk.) */
export function isThemeMode(value: string): value is ThemeMode {
  return value === "system" || value === "light" || value === "dark";
}

/** Put a resolved theme on `<html>`. */
export function applyTheme(mode: ThemeMode): ResolvedTheme {
  const resolved = resolveTheme(mode);
  document.documentElement.dataset.theme = resolved;
  return resolved;
}

/**
 * Follow the OS while the mode is `system`.
 *
 * Returns an unsubscribe function. A pinned mode does nothing, so switching to
 * Light never re-themes the app behind the user's back.
 */
export function watchSystemTheme(onChange: () => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const media = window.matchMedia(QUERY);
  const listener = () => onChange();
  // `addEventListener` is the modern API; older WebViews only have addListener.
  if (media.addEventListener) {
    media.addEventListener("change", listener);
    return () => media.removeEventListener("change", listener);
  }
  media.addListener(listener);
  return () => media.removeListener(listener);
}

/**
 * How to stop the current OS-preference watcher, if there is one.
 *
 * Module state on purpose: there is one watcher per document, and a second
 * caller applying a mode must cancel the first one's rather than race it for
 * `data-theme`.
 */
let unwatchOs: () => void = () => {};

/**
 * Apply a mode to this document and follow the OS only while it is `system`.
 *
 * Every window applies themes through here — `initTheme` at start-up, and the
 * settings page when the user picks one — so the watcher has a single owner.
 */
export function applyThemeMode(mode: ThemeMode): void {
  unwatchOs();
  applyTheme(mode);
  unwatchOs = mode === "system" ? watchSystemTheme(() => applyTheme("system")) : () => {};
}

/**
 * Theme this document from the stored choice, and keep it current (issue #32).
 *
 * Called once from `main.tsx`, before either root renders. The main window used
 * to do this itself, which left the mini player on the OS preference forever:
 * it is a separate document, so it neither read the stored mode nor heard about
 * a change made in the other window.
 *
 * Returns a function that stops listening for changes.
 */
export function initTheme(): () => void {
  let disposed = false;
  let unwatchEvents = () => {};

  getTheme()
    .then((mode) => {
      if (!disposed) applyThemeMode(isThemeMode(mode) ? mode : "system");
    })
    .catch(() => {
      // Nothing stored, or no backend: the pre-paint script already applied the
      // OS preference, and `system` keeps following it.
      if (!disposed) applyThemeMode("system");
    });

  void listenTheme((mode) => {
    if (!disposed && isThemeMode(mode)) applyThemeMode(mode);
  })
    .then((un) => {
      if (disposed) un();
      else unwatchEvents = un;
    })
    .catch(() => {
      // No event channel (a plain browser, or a test). Nothing to release.
    });

  return () => {
    disposed = true;
    unwatchEvents();
  };
}
