import type { EqPreset } from "./types";

/** Labels for the 10 EQ bands (ISO frequencies, matches EQ_BANDS in Rust). */
export const EQ_LABELS = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];

/** All-zero curve = flat response. */
export const EQ_ZERO = Array(EQ_LABELS.length).fill(0) as number[];

/**
 * Built-in template curves (dB per band, 31 Hz → 16 kHz). Boosted templates
 * lower the preamp so the output never clips after the master gain.
 */
export const BUILTIN_PRESETS: EqPreset[] = [
  { name: "Flat", preamp: 0, gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { name: "Acoustic", preamp: 0, gains: [3, 3, 2, 1, 1, 1, 2, 2, 3, 3] },
  { name: "Bass Boost", preamp: -2, gains: [7, 6, 5, 3, 1, 0, 0, 0, 0, 0] },
  { name: "Classical", preamp: 0, gains: [4, 3, 2, 1, -1, -1, 0, 2, 3, 4] },
  { name: "Electronic", preamp: -1, gains: [4, 3, 1, 0, -2, -1, 0, 1, 4, 5] },
  { name: "Jazz", preamp: 0, gains: [3, 2, 1, 2, -1, -1, 0, 1, 2, 3] },
  { name: "Pop", preamp: -1, gains: [-1, 0, 1, 3, 3, 1, 0, -1, -1, -1] },
  { name: "Rock", preamp: -1, gains: [4, 3, 1, -1, -2, -1, 1, 3, 4, 4] },
  { name: "Treble Boost", preamp: -2, gains: [0, 0, 0, 0, 0, 0, 3, 5, 6, 7] },
  { name: "Vocal", preamp: -1, gains: [-3, -2, -1, 1, 3, 4, 3, 1, 0, -1] },
];

/**
 * 1-decimal identity of an EQ curve. Two curves with the same signature sound
 * identical, so the dropdown shows the matching preset even after the player
 * rounds a slider value.
 */
export function eqSignature(preamp: number, gains: number[]): string {
  const d = (v: number) => Math.round(v * 10) / 10;
  return `${d(preamp)}:${gains.map((g) => d(g)).join(",")}`;
}

/** The first preset matching the curve, or `null` when it is custom work. */
export function matchPreset(
  preamp: number,
  gains: number[],
  presets: EqPreset[],
): EqPreset | null {
  const sig = eqSignature(preamp, gains);
  return presets.find((p) => eqSignature(p.preamp, p.gains) === sig) ?? null;
}