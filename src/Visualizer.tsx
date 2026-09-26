// Built-in generative visualizer — animated bars that do NOT analyse the
// audio file. The motion is driven purely by time and playback state, so it
// works for every format (including ones no decoder can parse) and even when
// nothing is playing (idle breathing). The current track's path only seeds
// the animation's "shape", giving each song a distinct look cheaply.

import { useEffect, useRef } from "react";
import type { PlayerState } from "./types";

/** Deterministic 32-bit hash of a string (seeds the per-track animation). */
function seedFrom(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Stable per-bar pseudo-random parameters derived from the track seed. */
function barParams(seed: number, i: number) {
  let x = seed + Math.imul(i + 1, 0x9e3779b1);
  x = Math.imul(x ^ (x >>> 15), 0x85ebca6b);
  x = Math.imul(x ^ (x >>> 13), 0xc2b2ae35);
  x ^= x >>> 16;
  const r = (x >>> 0) / 4294967295;
  return {
    phase: r * Math.PI * 2,
    speed: 0.7 + r * 1.4,
    wobble: 0.3 + r * 0.7,
  };
}

/** Beat pulse frequency while playing (~114 BPM). */
const BEAT_RAD = Math.PI * 2 * 1.9;

export function Visualizer({
  state,
  seed,
  variant = "panel",
  className,
}: {
  state: PlayerState | null;
  /** Shapes this animation: pass the current track's path. */
  seed: string | null;
  variant?: "panel" | "mini";
  className?: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const barsRef = useRef<Float32Array | null>(null);
  // Read the freshest state inside the rAF loop without restarting it
  // (~10 Hz state ticks must not reset the animation).
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const N = variant === "mini" ? 64 : 48;
    const base = seedFrom(seed ?? "iwaks");
    const params = Array.from({ length: N }, (_, i) => barParams(base, i));
    if (!barsRef.current || barsRef.current.length !== N) {
      barsRef.current = new Float32Array(N);
    }
    const bars = barsRef.current;
    bars.fill(0);

    let raf = 0;
    let last = performance.now();
    let t = (base % 1000) / 1000; // every track enters on a different phase
    let size = { w: 0, h: 0, dpr: 0 };

    const draw = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const s = stateRef.current;
      const playing = s ? !s.paused && !s.stopped : false;
      t += dt * (playing ? 1 : 0.2); // idle slows down, bars just breathe

      const beat = 0.5 + 0.5 * Math.sin(t * BEAT_RAD);
      for (let i = 0; i < N; i++) {
        const p = params[i];
        const wob = 0.5 + 0.5 * Math.sin(t * p.speed * 0.9 + p.phase);
        const hop = 0.5 + 0.5 * Math.sin(t * p.speed * 2.3 + p.phase * 1.7);
        // Lower bars carry the pulse harder; upper bars wobble freely.
        const bass = 1 - (i / N) * 0.6;
        const mix = p.wobble * wob + (1 - p.wobble) * hop;
        const level = (0.25 + 0.55 * mix) * (1 - 0.25 * bass) + beat * 0.25 * bass;
        const target = playing ? Math.min(1, level) : 0.05;
        // Fast attack when a bar rises, slower release when it falls.
        bars[i] += (target - bars[i]) * (target > bars[i] ? 0.45 : 0.08);
      }

      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      const dpr = window.devicePixelRatio || 1;
      if (size.w !== w || size.h !== h || size.dpr !== dpr) {
        size = { w, h, dpr };
        canvas.width = Math.max(1, Math.floor(w * dpr));
        canvas.height = Math.max(1, Math.floor(h * dpr));
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      const slot = w / N;
      const bw = Math.max(2, slot * 0.64);
      const grad = ctx.createLinearGradient(0, h, 0, 0);
      if (variant === "mini") {
        grad.addColorStop(0, "#c8f7ff");
        grad.addColorStop(0.55, "#0b6aa7");
        grad.addColorStop(1, "#12334d");
      } else {
        grad.addColorStop(0, "#8fd6e8");
        grad.addColorStop(0.55, "#3f9fc9");
        grad.addColorStop(1, "#026aa7");
      }
      ctx.fillStyle = grad;
      for (let i = 0; i < N; i++) {
        const bh = 2 + bars[i] * (h - 8);
        ctx.fillRect(i * slot + (slot - bw) / 2, h - bh, bw, bh);
      }
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [seed, variant]);

  return (
    <canvas
      ref={canvasRef}
      className={className}
      aria-label={variant === "mini" ? "Visualizer (mini window)" : "Visualizer"}
    />
  );
}