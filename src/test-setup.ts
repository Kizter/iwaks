/**
 * Test setup shared by every frontend test.
 *
 * Two things have to be arranged before any component mounts:
 *
 * 1. jest-dom's matchers (`toBeInTheDocument`, `toHaveTextContent`, ...), which
 *    are what make an assertion read as a claim about the screen instead of a
 *    claim about `innerHTML`.
 * 2. A Tauri IPC stub. The app calls `invoke()` from `@tauri-apps/api/core`
 *    during mount; under jsdom there is no `window.__TAURI_INTERNALS__`, so
 *    every one of those calls would throw before a test got as far as
 *    asserting anything. The stub rejects with a recognisable message, so a
 *    component that needs real data says so in its own test instead of relying
 *    on a silent empty render.
 */
import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

/** Marker every stubbed IPC call rejects with, so tests can recognise it. */
export const NO_BACKEND = "no Tauri backend in this test";

if (!("__TAURI_INTERNALS__" in window)) {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    invoke: (cmd: string) =>
      Promise.reject(new Error(`${NO_BACKEND} (${cmd})`)),
    transformCallback: () => 0,
    unregisterListener: () => {},
    convertFileSrc: (p: string) => p,
  };
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});