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

/** Matches iwaks_library::playlists::Playlist (serde camelCase). */
export interface Playlist {
  id: number;
  name: string;
  trackCount: number;
}

/** Matches iwaks_player::queue::RepeatMode (serde lowercase). */
export type RepeatMode = "off" | "all" | "one";

/** Mirrors iwaks_player::PlayerState (serde camelCase). */
export interface PlayerState {
  current: Track | null;
  index: number | null;
  listLen: number;
  position: number;
  duration: number;
  paused: boolean;
  stopped: boolean;
  volume: number;
  mute: boolean;
  repeat: RepeatMode;
  /** Shuffle on: the queue walks a random permutation of the list. */
  shuffle: boolean;
  /** Playback-rate multiplier (0.25–4.0). */
  speed: number;
  /** Seconds left on the sleep timer, or `null` when none is armed. */
  sleepRemaining: number | null;
  /** Master EQ gain in dB (±12). */
  eqPreamp: number;
  /** 10 band gains in dB at the ISO frequencies (all 0 = flat). */
  eq: number[];
  /** ReplayGain mode applied by libmpv. */
  replaygain: ReplayGainMode;
}

/** Mirrors iwaks_player::ReplayGainMode (serde lowercase; mpv: no|track|album). */
export type ReplayGainMode = "off" | "track" | "album";

/** Mirrors iwaks_tags::lyrics::TimedLine. */
export interface LyricsLine {
  time: number;
  text: string;
}

/** Mirrors iwaks_tags::lyrics::Lyrics. */
export interface Lyrics {
  timed: LyricsLine[];
  plain: string | null;
}