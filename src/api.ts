import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { PlayerState, RepeatMode, ScanEvent, ScanProgress, Track } from "./types";

export function getTracks(): Promise<Track[]> {
  return invoke<Track[]>("get_tracks");
}

export function searchTracks(query: string): Promise<Track[]> {
  return invoke<Track[]>("search_tracks", { query });
}

export function scanFolder(path: string): Promise<void> {
  return invoke<void>("scan_folder", { path });
}

/** Embedded album art as a `data:` URL, or `null` when the file has none. */
export function readCover(path: string): Promise<string | null> {
  return invoke<string | null>("read_cover", { path });
}

/** Native folder picker; resolves `null` when the user cancels. */
export async function pickFolder(): Promise<string | null> {
  const picked = await open({
    directory: true,
    multiple: false,
    title: "Choose a music folder",
  });
  return typeof picked === "string" ? picked : null;
}

export interface ScanHandlers {
  onProgress: (p: ScanProgress, finished: boolean) => void;
  onError: (message: string) => void;
}

export async function listenScan(handlers: ScanHandlers): Promise<UnlistenFn> {
  const unProg = await listen<ScanEvent>("scan-progress", (e) => {
    handlers.onProgress(e.payload.progress, e.payload.finished);
  });
  const unErr = await listen<string>("scan-error", (e) => {
    handlers.onError(e.payload);
  });
  return () => {
    unProg();
    unErr();
  };
}

// ---------- playback (iwaks-player -> libmpv) ----------

export function playTracks(tracks: Track[], index: number): Promise<void> {
  return invoke<void>("play_tracks", { tracks, index });
}

export function playerTogglePlay(): Promise<void> {
  return invoke<void>("toggle_play");
}

export function playerNext(): Promise<void> {
  return invoke<void>("next_track");
}

export function playerPrev(): Promise<void> {
  return invoke<void>("prev_track");
}

export function playerSeek(seconds: number): Promise<void> {
  return invoke<void>("seek", { position: seconds });
}

export function playerSeekRelative(delta: number): Promise<void> {
  return invoke<void>("seek_relative", { delta });
}

export function playerSetVolume(volume: number): Promise<void> {
  return invoke<void>("set_volume", { volume });
}

export function playerToggleMute(): Promise<void> {
  return invoke<void>("toggle_mute");
}

export function playerSetRepeat(repeat: RepeatMode): Promise<void> {
  return invoke<void>("set_repeat", { repeat });
}

export function playerSetSpeed(speed: number): Promise<void> {
  return invoke<void>("set_speed", { speed });
}

/** Arm the sleep timer (`null` cancels); playback pauses at the deadline. */
export function playerSetSleepTimer(seconds: number | null): Promise<void> {
  return invoke<void>("set_sleep_timer", { seconds });
}

export function playerStop(): Promise<void> {
  return invoke<void>("stop_playback");
}

/** Initial state; `null` when playback is unavailable (missing libmpv). */
export function getPlayerState(): Promise<PlayerState | null> {
  return invoke<PlayerState | null>("get_player_state");
}

/** Live playback state: fires ~4×/second plus on every change/command. */
export async function listenPlayer(
  onState: (s: PlayerState) => void,
  onError: (message: string) => void,
): Promise<UnlistenFn> {
  const unState = await listen<PlayerState>("player-state", (e) => onState(e.payload));
  const unErr = await listen<string>("player-error", (e) => onError(e.payload));
  return () => {
    unState();
    unErr();
  };
}