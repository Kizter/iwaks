import { useEffect, useState } from "react";
import { readCover } from "./api";
import type { Track } from "./types";

/** Embedded artwork per track path, cached for the session (null = has none). */
const coverCache = new Map<string, string | null>();

/** Music-note fallback shown when a track has no embedded cover art. */
export function NoteIcon() {
  return (
    <svg className="cover-note" viewBox="0 0 16 16" width="18" height="18" aria-hidden="true">
      <path
        fill="currentColor"
        d="M7.5 2v9.2A2.1 2.1 0 1 0 9.1 13.1V3.1l4.4-1.03v7.5a2.1 2.1 0 1 0 1.6 2.06V1.3L7.5 2z"
      />
    </svg>
  );
}

/** Track thumbnail: embedded art when present, else a colored note tile. */
export function Cover({ track }: { track: Track }) {
  const [src, setSrc] = useState<string | null | undefined>(undefined); // undefined = loading
  const fmt = track.format.toLowerCase();

  useEffect(() => {
    let alive = true;
    if (coverCache.has(track.path)) {
      setSrc(coverCache.get(track.path) ?? null);
      return;
    }
    readCover(track.path)
      .then((u) => {
        coverCache.set(track.path, u);
        if (alive) setSrc(u);
      })
      .catch(() => {
        coverCache.set(track.path, null);
        if (alive) setSrc(null);
      });
    return () => {
      alive = false;
    };
  }, [track.path]);

  return (
    <span className={`cover cover-${fmt}`} aria-hidden="true">
      {src ? <img className="cover-art" src={src} alt="" loading="lazy" /> : <NoteIcon />}
    </span>
  );
}