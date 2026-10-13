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
  /** Bumped whenever the queue order changes (reshuffle/reorder/list swap). */
  queueVersion: number;
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
  /** Active mpv `audio-device` (empty = system default). */
  audioDevice: string;
  /** True while a second core holds the next track — loaded, and either paused
   * (crossfade armed but the window not open yet) or fading in. */
  prefetched: boolean;
}

/** Mirrors iwaks_player::ReplayGainMode (serde lowercase; mpv: no|track|album). */
export type ReplayGainMode = "off" | "track" | "album";

/** Mirrors iwaks_player::Crossfade (serde camelCase). */
export interface CrossfadeSettings {
  /** Crossfade on. Separate from `seconds` so a short fade stays expressible. */
  enabled: boolean;
  /** Overlap in seconds (1–10). Only read when `enabled`. */
  seconds: number;
}

/** Mirrors iwaks_library::persist::EqPreset (serde camelCase). */
export interface EqPreset {
  name: string;
  preamp: number;
  /** 10 band gains in dB at the ISO frequencies. */
  gains: number[];
}

/** Mirrors iwaks_library::db::TrackFilter (serde camelCase). */
export interface TrackFilter {
  /** Exact stored genre spelling, or `null` for every genre. */
  genre: string | null;
  /** Inclusive decade start — `1990` matches 1990–1999. */
  decade: number | null;
}

/** Mirrors iwaks_library::db::TrackSort (serde camelCase). */
export type TrackSort = "artistAlbum" | "title" | "added" | "longest";

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

/**
 * Mirrors iwaks_core::track::TagEdits (serde camelCase). Blank title removes
 * the title tag; `null` fields remove those tags from the file.
 */
export interface TagEdits {
  title: string;
  artist: string | null;
  album: string | null;
  albumArtist: string | null;
  genre: string | null;
  year: number | null;
  trackNo: number | null;
  discNo: number | null;
}