import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { ScanEvent, ScanProgress, Track } from "./types";

export function getTracks(): Promise<Track[]> {
  return invoke<Track[]>("get_tracks");
}

export function searchTracks(query: string): Promise<Track[]> {
  return invoke<Track[]>("search_tracks", { query });
}

export function scanFolder(path: string): Promise<void> {
  return invoke<void>("scan_folder", { path });
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