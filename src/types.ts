// IPC types shared with the Rust backend (mirrors iwaks_core::track::Track).

export interface Track {
  id: number;
  path: string;
  title: string;
  artist: string | null;
  album: string | null;
  albumArtist: string | null;
  genre: string | null;
  year: number | null;
  trackNo: number | null;
  discNo: number | null;
  durationMs: number;
  sampleRate: number | null;
  bitDepth: number | null;
  bitrate: number | null;
  format: string;
  fileSize: number;
  modifiedAt: number;
}

export interface ScanProgress {
  totalFiles: number;
  scanned: number;
  added: number;
  updated: number;
  skipped: number;
  removed: number;
  errors: number;
}

export interface ScanEvent {
  progress: ScanProgress;
  finished: boolean;
}