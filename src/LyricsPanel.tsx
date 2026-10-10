import { useEffect, useMemo, useRef, useState } from "react";
import { getLyrics } from "./api";
import type { Lyrics } from "./types";

/**
 * Lyrics for one track: timed lines (auto-scrolled to the active line) when
 * available, else the plain fallback, else an explanatory empty state.
 *
 * Shared by the player-bar popover and the Now Playing view — the parent owns
 * sizing and framing; this component only renders the content.
 */
export function LyricsPanel({
  path,
  position,
}: {
  path: string | null;
  position: number;
}) {
  const [lyrics, setLyrics] = useState<Lyrics | null | undefined>(undefined);

  useEffect(() => {
    if (!path) {
      setLyrics(undefined);
      return;
    }
    let cancelled = false;
    setLyrics(undefined); // loading
    void getLyrics(path).then((l) => {
      if (!cancelled) setLyrics(l);
    });
    return () => {
      cancelled = true;
    };
  }, [path]);

  // Active line = the last one whose timestamp is <= playback position.
  const activeIdx = useMemo(() => {
    const timed = lyrics?.timed ?? [];
    if (timed.length === 0) return -1;
    let idx = -1;
    for (let i = 0; i < timed.length; i++) {
      if (timed[i].time <= position) idx = i;
      else break;
    }
    return idx;
  }, [lyrics, position]);

  const box = useRef<HTMLDivElement | null>(null);
  const activeLine = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const b = box.current;
    const el = activeLine.current;
    if (b && el) {
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      b.scrollTo({
        top: Math.max(0, el.offsetTop - b.clientHeight / 2),
        behavior: reduce ? "auto" : "smooth",
      });
    }
  }, [activeIdx]);

  if (lyrics === undefined) {
    return <p className="lyr-empty">Loading…</p>;
  }
  if (lyrics === null || (lyrics.timed.length === 0 && !lyrics.plain)) {
    return <p className="lyr-empty">No lyrics for this track</p>;
  }
  if (lyrics.timed.length > 0) {
    return (
      <div className="lyr-list" ref={box}>
        {lyrics.timed.map((line, i) => (
          <div
            key={i}
            ref={i === activeIdx ? activeLine : undefined}
            className={`lyr-line${i === activeIdx ? " on" : ""}`}
          >
            {line.text}
          </div>
        ))}
      </div>
    );
  }
  return <pre className="lyr-plain">{lyrics.plain}</pre>;
}
