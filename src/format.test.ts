import { describe, expect, it } from "vitest";
import { formatBadge, formatDuration, formatTech } from "./format";

/**
 * The badge is the only place the app claims something about a file's fidelity,
 * so a wrong answer here is a wrong claim about someone's music (issue #08).
 */
describe("formatBadge", () => {
  it("names the codecs it knows, in the casing the design uses", () => {
    expect(formatBadge("flac")).toBe("FLAC");
    expect(formatBadge("alac")).toBe("ALAC");
    expect(formatBadge("aac")).toBe("AAC");
    expect(formatBadge("m4a")).toBe("M4A");
    expect(formatBadge("wav")).toBe("WAV");
    expect(formatBadge("dsf")).toBe("DSF");
    expect(formatBadge("dff")).toBe("DFF");
  });

  it("is case-insensitive, because the scanner does not promise a casing", () => {
    expect(formatBadge("FLAC")).toBe("FLAC");
    expect(formatBadge("Flac")).toBe("FLAC");
    expect(formatBadge("ALAC")).toBe("ALAC");
  });

  it("never calls an .m4a lossless when the codec is known", () => {
    // `.m4a` is a container: AAC is lossy, ALAC is not. Showing "ALAC" for a
    // container whose contents were never probed would be a false claim, which
    // is exactly what issue #08 was about.
    expect(formatBadge("aac")).not.toContain("ALAC");
    expect(formatBadge("alac")).toContain("ALAC");
    // An unprobed container shows the neutral container name.
    expect(formatBadge("m4a")).toBe("M4A");
    expect(formatBadge("m4a")).not.toContain("ALAC");
  });

  it("falls back to the uppercased name rather than showing nothing", () => {
    expect(formatBadge("ape")).toBe("APE");
    expect(formatBadge("tta")).toBe("TTA");
    // Blank input must not crash a row: it renders as an empty badge.
    expect(formatBadge("")).toBe("");
  });
});

/**
 * The hi-res readout is the only place the app states a lossless file's full
 * fidelity claim. When the scanner could not read a rate or depth it must be
 * omitted, never guessed at (issue #10).
 */
describe("formatTech", () => {
  it("renders the lossless claim a hi-res player should make", () => {
    expect(formatTech({ format: "flac", sampleRate: 192_000, bitDepth: 24 })).toBe(
      "FLAC · 192 kHz · 24 bit",
    );
  });

  it("keeps one decimal only where it matters, and none where it does not", () => {
    expect(formatTech({ format: "flac", sampleRate: 44_100, bitDepth: 16 })).toBe(
      "FLAC · 44.1 kHz · 16 bit",
    );
    expect(formatTech({ format: "wav", sampleRate: 48_000, bitDepth: 24 })).toBe(
      "WAV · 48 kHz · 24 bit",
    );
  });

  it("omits whatever the scan could not read", () => {
    expect(formatTech({ format: "flac", sampleRate: null, bitDepth: 24 })).toBe("FLAC · 24 bit");
    expect(formatTech({ format: "flac", sampleRate: 96_000, bitDepth: null })).toBe(
      "FLAC · 96 kHz",
    );
    expect(formatTech({ format: "mp3", sampleRate: null, bitDepth: null })).toBe("MP3");
  });

  it("reads DSD in MHz, where a kilohertz line would be noise", () => {
    expect(formatTech({ format: "dsf", sampleRate: 2_822_400, bitDepth: 1 })).toBe(
      "DSF · 2.8 MHz · 1 bit",
    );
  });
});

/**
 * Durations are shown in the player bar, the track lists and Now Playing, so a
 * wrong one is visible three times over.
 */
describe("formatDuration", () => {
  it("renders minutes and seconds", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(1_000)).toBe("0:01");
    expect(formatDuration(61_000)).toBe("1:01");
    expect(formatDuration(599_000)).toBe("9:59");
  });

  it("keeps counting minutes past an hour rather than rolling over", () => {
    // Deliberate: a 90-minute live set reads "90:00", not "1:30:00". Changing
    // this would change every duration already on screen.
    expect(formatDuration(3_600_000)).toBe("60:00");
    expect(formatDuration(5_401_000)).toBe("90:01");
  });

  it("survives the values a track without a duration produces", () => {
    // A scanner that did not read the header, or a still-loading track. This
    // used to render the literal text "NaN:NaN".
    expect(formatDuration(Number.NaN)).toBe("0:00");
    expect(formatDuration(Number.POSITIVE_INFINITY)).toBe("0:00");
    expect(formatDuration(-1)).toBe("0:00");
  });
});