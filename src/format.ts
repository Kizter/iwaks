/** Format milliseconds as `m:ss` (0 → "0:00"). */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Short human summary of a finished scan, e.g. "4 added · 1 updated". */
export function scanSummary(p: {
  added: number;
  updated: number;
  removed: number;
  errors: number;
}): string {
  const parts: string[] = [];
  if (p.added > 0) parts.push(`${p.added} added`);
  if (p.updated > 0) parts.push(`${p.updated} updated`);
  if (p.removed > 0) parts.push(`${p.removed} removed`);
  if (parts.length === 0) parts.push("up to date");
  if (p.errors > 0) parts.push(`${p.errors} skipped (unreadable)`);
  return parts.join(" · ");
}

/**
 * Format label for the small cover placeholder, e.g. "FLAC", "DSF".
 *
 * `.m4a` is a container that can hold either AAC (lossy) or ALAC (lossless),
 * so the scanner reports the detected codec (`aac` / `alac`); an undetected
 * container shows the neutral "M4A" rather than claiming lossless.
 */
export function formatBadge(format: string): string {
  const map: Record<string, string> = {
    alac: "ALAC",
    m4a: "M4A",
    aiff: "AIFF",
    aif: "AIFF",
    oga: "OGG",
    wav: "WAV",
    mp3: "MP3",
    flac: "FLAC",
    opus: "OPUS",
    ogg: "OGG",
    wv: "WV",
    wma: "WMA",
    dsf: "DSF",
    dff: "DFF",
    aac: "AAC",
  };
  return map[format.toLowerCase()] ?? format.toUpperCase();
}

/**
 * Map a raw backend error to user-facing copy. Known classes get a friendly
 * sentence; anything else falls back to a generic one. The untrimmed detail
 * goes to the console for debugging — never to a banner.
 */
export function friendlyError(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  console.error(e);
  const s = raw.toLowerCase();
  if (s.includes("libmpv") || s.includes("playback is unavailable")) {
    return "Playback is unavailable right now. Try restarting Iwaks.";
  }
  if (s.includes("database") || s.includes("sqlite") || s.includes("query returned")) {
    return "Iwaks couldn't read its library database. Try again in a moment.";
  }
  if (s.includes("permission denied") || s.includes("access is denied") || s.includes("os error 5")) {
    return "Permission denied while accessing that file.";
  }
  if (s.includes("no such file") || s.includes("not found") || s.includes("os error 2")) {
    return "That file couldn't be found — it may have been moved or deleted.";
  }
  if (s.includes("unsupported") || s.includes("unrecognized format")) {
    return "That file format isn't supported.";
  }
  if (s.includes("m3u") || s.includes("playlist")) {
    return "Iwaks couldn't read that playlist file.";
  }
  return "Something went wrong. Please try again.";
}