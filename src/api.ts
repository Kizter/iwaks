import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { Lyrics, Playlist, PlayerState, ReplayGainMode, RepeatMode, ScanEvent, ScanProgress, Track } from "./types";

export function getTracks(): Promise<Track[]> {
  return invoke<Track[]>("get_tracks");
}

export function searchTracks(query: string): Promise<Track[]> {
  return invoke<Track[]>("search_tracks", { query });
}

export function scanFolder(path: string): Promise<void> {
  return invoke<void>("scan_folder", { path });
}

/** Add individually-picked files (multi-select) to the library. */
export function addFiles(paths: string[]): Promise<ScanProgress> {
  return invoke<ScanProgress>("add_files", { paths });
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

const AUDIO_EXTENSIONS = [
  "flac", "wav", "alac", "m4a", "aac", "mp3", "ogg", "opus", "wv", "wavpack",
  "aiff", "aif", "wma", "dsf", "dff",
];

/** Native multi-file picker for direct file import; `null` on cancel. */
export async function pickFiles(): Promise<string[] | null> {
  const picked = await open({
    directory: false,
    multiple: true,
    title: "Add music files",
    filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }],
  });
  return Array.isArray(picked) ? picked : null;
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

/** Toggle shuffle on/off (current track stays; remaining order re-randomized). */
export function playerSetShuffle(shuffle: boolean): Promise<void> {
  return invoke<void>("set_shuffle", { shuffle });
}

/** Re-shuffle the remaining tracks (current track stays selected). */
export function playerReshuffle(): Promise<void> {
  return invoke<void>("reshuffle_tracks");
}

export function playerSetSpeed(speed: number): Promise<void> {
  return invoke<void>("set_speed", { speed });
}

/** Arm the sleep timer (`null` cancels); playback pauses at the deadline. */
export function playerSetSleepTimer(seconds: number | null): Promise<void> {
  return invoke<void>("set_sleep_timer", { seconds });
}

/** Graphic EQ: master `preamp` dB + 10 band gains (dB). */
export function playerSetEq(preamp: number, gains: number[]): Promise<void> {
  return invoke<void>("set_eq", { preamp, eq: gains });
}

export function playerSetReplayGain(mode: ReplayGainMode): Promise<void> {
  return invoke<void>("set_replaygain", { mode });
}

/** Lyrics for a track (embedded / `.lrc` sidecar), or `null` when absent. */
export function getLyrics(path: string): Promise<Lyrics | null> {
  return invoke<Lyrics | null>("get_lyrics", { path });
}

/** Toggle the always-on-top mini visualizer window (player-bar button). */
export function toggleMiniVisualizer(): Promise<void> {
  return invoke<void>("toggle_mini_visualizer");
}

export function playerStop(): Promise<void> {
  return invoke<void>("stop_playback");
}

/** Initial state; `null` when playback is unavailable (missing libmpv). */
export function getPlayerState(): Promise<PlayerState | null> {
  return invoke<PlayerState | null>("get_player_state");
}

// ---------- playlists (M4 slice 1) ----------

export function listPlaylists(): Promise<Playlist[]> {
  return invoke<Playlist[]>("list_playlists");
}

export function createPlaylist(name: string): Promise<number> {
  return invoke<number>("create_playlist", { name });
}

export function renamePlaylist(id: number, name: string): Promise<void> {
  return invoke<void>("rename_playlist", { id, name });
}

export function deletePlaylist(id: number): Promise<void> {
  return invoke<void>("delete_playlist", { id });
}

/** Tracks in stored order; `null` when the playlist doesn't exist. */
export function getPlaylistTracks(id: number): Promise<Track[] | null> {
  return invoke<Track[] | null>("get_playlist_tracks", { id });
}

export function addToPlaylist(playlistId: number, trackId: number): Promise<void> {
  return invoke<void>("add_to_playlist", { playlistId, trackId });
}

export function removeFromPlaylist(playlistId: number, trackId: number): Promise<void> {
  return invoke<void>("remove_from_playlist", { playlistId, trackId });
}

/** Replace a playlist's order with `trackIds` (also drops unlisted entries). */
export function reorderPlaylist(playlistId: number, trackIds: number[]): Promise<void> {
  return invoke<void>("reorder_playlist", { playlistId, trackIds });
}

/** Import an `.m3u` file as a new playlist; resolves to its id. */
export function importM3u(path: string): Promise<number> {
  return invoke<number>("import_m3u", { path });
}

export function exportM3u(playlistId: number, path: string): Promise<void> {
  return invoke<void>("export_m3u", { playlistId, path });
}

/** Native picker for choosing an `.m3u` file to import; `null` on cancel. */
export async function pickM3uFile(): Promise<string | null> {
  const picked = await open({
    directory: false,
    multiple: false,
    filters: [{ name: "Playlists", extensions: ["m3u"] }],
    title: "Import playlist",
  });
  return typeof picked === "string" ? picked : null;
}

/** Native save dialog for exporting a playlist; `null` on cancel. */
export async function pickM3uSave(defaultPath: string): Promise<string | null> {
  const picked = await save({
    defaultPath,
    filters: [{ name: "Playlists", extensions: ["m3u"] }],
    title: "Export playlist",
  });
  return typeof picked === "string" ? picked : null;
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