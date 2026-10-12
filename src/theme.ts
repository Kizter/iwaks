/**
 * Light/dark theming (issue #25).
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
