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

/** Format label for the small cover placeholder, e.g. "FLAC", "DSF". */
export function formatBadge(format: string): string {
  const map: Record<string, string> = {
    m4a: "ALAC",
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