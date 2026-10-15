import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyThemeMode, initTheme } from "./theme";

/**
 * Issue #32: the mini player is a second document with its own stylesheet, and
 * it neither read the stored theme nor heard about a change made in the main
 * window — it sat on whatever the OS preference said forever. Issue #25's
 * behaviour is here too, because the fix moved where the OS watcher is owned.
 */
const api = vi.hoisted(() => ({
  getTheme: vi.fn(),
  listenTheme: vi.fn(),
}));

vi.mock("./api", () => api);

const theme = () => document.documentElement.dataset.theme;

/** jsdom has no `matchMedia`, and resolving `system` needs one. */
function stubSystemTheme(dark: boolean) {
  const listeners = new Set<() => void>();
  const media = {
    matches: dark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_event: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_event: string, listener: () => void) => listeners.delete(listener),
  };
  vi.stubGlobal("matchMedia", () => media);
  return {
    /** Flip the OS preference and notify, the way the platform does. */
    change(value: boolean) {
      media.matches = value;
      for (const listener of listeners) listener();
    },
  };
}

beforeEach(() => {
  api.getTheme.mockReset();
  api.listenTheme.mockReset();
  api.listenTheme.mockResolvedValue(() => {});
  delete document.documentElement.dataset.theme;
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("applyThemeMode", () => {
  it("applies a pinned mode", () => {
    stubSystemTheme(false);
    applyThemeMode("dark");
    expect(theme()).toBe("dark");
    applyThemeMode("light");
    expect(theme()).toBe("light");
  });

  it("follows the OS while the mode is system, and stops once it is pinned", () => {
    const os = stubSystemTheme(true);
    applyThemeMode("system");
    expect(theme(), "system resolves to the OS preference").toBe("dark");

    os.change(false);
    expect(theme(), "and keeps following it").toBe("light");

    applyThemeMode("dark");
    os.change(false);
    expect(theme(), "a pinned mode must not still be listening").toBe("dark");
  });
});

describe("initTheme", () => {
  it("applies the stored mode, whoever is doing the rendering", async () => {
    stubSystemTheme(false);
    api.getTheme.mockResolvedValue("dark");

    const stop = initTheme();
    await vi.waitFor(() => expect(theme()).toBe("dark"));
    stop();
  });

  it("re-applies when the theme is changed in the other window", async () => {
    stubSystemTheme(false);
    api.getTheme.mockResolvedValue("light");
    let announced: ((mode: string) => void) | undefined;
    api.listenTheme.mockImplementation((onMode: (mode: string) => void) => {
      announced = onMode;
      return Promise.resolve(() => {});
    });

    const stop = initTheme();
    await vi.waitFor(() => expect(theme()).toBe("light"));

    // This is what the mini player never received.
    announced?.("dark");
    expect(theme()).toBe("dark");

    // A payload from a build that knows a mode this one does not: ignore it
    // rather than writing garbage into `data-theme`.
    announced?.("solarized");
    expect(theme()).toBe("dark");
    stop();
  });

  it("keeps following the OS after a switch back to system", async () => {
    const os = stubSystemTheme(false);
    api.getTheme.mockResolvedValue("dark");
    let announced: ((mode: string) => void) | undefined;
    api.listenTheme.mockImplementation((onMode: (mode: string) => void) => {
      announced = onMode;
      return Promise.resolve(() => {});
    });

    const stop = initTheme();
    await vi.waitFor(() => expect(theme()).toBe("dark"));

    announced?.("system");
    expect(theme(), "system adopts the OS preference again").toBe("light");
    os.change(true);
    expect(theme(), "and follows it live").toBe("dark");
    stop();
  });

  it("falls back to the OS preference when nothing is stored", async () => {
    stubSystemTheme(true);
    api.getTheme.mockRejectedValue(new Error("no backend"));
    api.listenTheme.mockRejectedValue(new Error("no event channel"));

    const stop = initTheme();
    await vi.waitFor(() => expect(theme()).toBe("dark"));
    stop();
  });
});
