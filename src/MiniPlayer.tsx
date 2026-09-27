// Layout for the always-on-top mini player window (`#mini` in the URL).
// Player state arrives through the same app-wide `player-state` events, so
// the mini window mirrors the main player: play/pause, prev/next, shuffle,
// plus a playlist/queue picker. The whole surface is a drag region — grab
// anywhere to move the frameless window. The picker is an OVERLAY inside the
// fixed-size window (the window never resizes; the list scrolls within it).

import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ListBullets,
  Pause,
  Play,
  Queue as QueueIcon,
  Shuffle,
  SkipBack,
  SkipForward,
  X,
} from "@phosphor-icons/react";
import {
  getPlaylistTracks,
  getPlayerState,
  getQueue,
  listenPlayer,
  listPlaylists,
  playerNext,
  playerPrev,
  playerSetShuffle,
  playerTogglePlay,
  playTracks,
} from "./api";
import type { Playlist, PlayerState, Track } from "./types";
import { Cover, NoteIcon } from "./Cover";

export function MiniPlayer() {
  const [state, setState] = useState<PlayerState | null>(null);
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  // Label of the last source chosen here (playlist name or "Queue").
  const [source, setSource] = useState("Playlist");
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let alive = true;
    void getPlayerState().then((s) => {
      if (alive) setState(s);
    });
    void listenPlayer(
      (s) => {
        if (alive) setState(s);
      },
      () => {},
    ).then((un) => {
      if (!alive) un();
      else unlistenRef.current = un;
    });
    return () => {
      alive = false;
      unlistenRef.current?.();
    };
  }, []);

  const cur = state?.current ?? null;
  const playing = state ? !state.paused && !state.stopped : false;
  const shuffle = state?.shuffle ?? false;

  const openPicker = async () => {
    if (!pickerOpen) {
      let list: Playlist[] = [];
      try {
        list = await listPlaylists();
        setPlaylists(list);
      } catch {
        setPlaylists([]);
      }
      setPickerOpen(true);
    } else {
      setPickerOpen(false);
    }
  };

  const playSource = async (tracks: Track[], label: string) => {
    if (tracks.length === 0) return;
    setPickerOpen(false);
    await playTracks(tracks, 0);
    setSource(label);
  };

  const playQueue = async () => {
    try {
      await playSource(await getQueue(), "Queue");
    } catch {
      // empty/stale session queue — ignore
    }
  };

  const playPlaylist = async (id: number, name: string) => {
    try {
      const tracks = await getPlaylistTracks(id);
      if (tracks) await playSource(tracks, name);
    } catch {
      // unreadable playlist — ignore
    }
  };

  // Frameless window: start an OS window drag when pressing anywhere that is
  // not an interactive element (buttons keep their clicks).
  const startDrag = (e: React.MouseEvent<HTMLDivElement>) => {
    const t = e.target as HTMLElement;
    if (t.closest("button, input, select, textarea, a, [role='option']")) return;
    void getCurrentWindow().startDragging();
  };

  return (
    <div className="mini-player" onMouseDown={startDrag} data-playing={playing}>
      <div className="mp-head">
        <span className="mp-art">{cur ? <Cover track={cur} /> : <NoteIcon />}</span>
        <span className="mp-meta">
          <span className="mp-title">{playing && cur ? cur.title : "Iwaks"}</span>
          <span className="mp-sub">
            {cur ? (cur.artist ?? "Unknown artist") : "Nothing playing"}
          </span>
        </span>
        <button
          type="button"
          className="mp-close"
          onClick={() => void getCurrentWindow().close()}
          aria-label="Close mini player"
          title="Close"
        >
          <X size={14} aria-hidden="true" />
        </button>
      </div>

      <div className="mp-controls">
        <button
          type="button"
          className={`mp-btn${shuffle ? " on" : ""}`}
          onClick={() => void playerSetShuffle(!shuffle)}
          disabled={!state}
          aria-label={shuffle ? "Shuffle on — click to turn off" : "Shuffle"}
          aria-pressed={shuffle}
          title={shuffle ? "Shuffle on" : "Shuffle"}
        >
          <Shuffle size={16} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="mp-btn"
          onClick={() => void playerPrev()}
          disabled={!state}
          aria-label="Previous track"
          title="Previous track"
        >
          <SkipBack size={16} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="mp-btn mp-play"
          onClick={() => void playerTogglePlay()}
          disabled={!state}
          aria-label={playing ? "Pause" : "Play"}
          title={playing ? "Pause" : "Play"}
        >
          {playing ? (
            <Pause size={20} aria-hidden="true" />
          ) : (
            <Play size={20} aria-hidden="true" />
          )}
        </button>
        <button
          type="button"
          className="mp-btn"
          onClick={() => void playerNext()}
          disabled={!state}
          aria-label="Next track"
          title="Next track"
        >
          <SkipForward size={16} aria-hidden="true" />
        </button>

        <div className="mp-picker">
          <button
            type="button"
            className="mp-btn mp-source"
            onClick={() => void openPicker()}
            disabled={!state}
            aria-haspopup="listbox"
            aria-expanded={pickerOpen}
            title="Change what's playing"
          >
            {source === "Queue" ? (
              <QueueIcon size={15} aria-hidden="true" />
            ) : (
              <ListBullets size={15} aria-hidden="true" />
            )}
            <span className="mp-source-name">{source}</span>
          </button>
        </div>
      </div>

      {pickerOpen && (
        <div className="mp-panel" aria-label="Choose what to play">
          <div className="mp-panel-head">
            <span className="mp-panel-title">Choose what to play</span>
            <button
              type="button"
              className="mp-panel-close"
              onClick={() => setPickerOpen(false)}
              aria-label="Close picker"
              title="Close"
            >
              <X size={14} aria-hidden="true" />
            </button>
          </div>
          <div className="mp-panel-list" role="listbox" aria-label="Pick a source">
            <button
              type="button"
              role="option"
              aria-selected={source === "Queue"}
              className={`mp-panel-item${source === "Queue" ? " on" : ""}`}
              onClick={() => void playQueue()}
            >
              <QueueIcon size={16} className="mp-panel-ico" aria-hidden="true" />
              <span className="mp-panel-name">Queue (session)</span>
            </button>
            {(playlists ?? []).map((p) => (
              <button
                key={p.id}
                type="button"
                role="option"
                aria-selected={source === p.name}
                className={`mp-panel-item${source === p.name ? " on" : ""}`}
                onClick={() => void playPlaylist(p.id, p.name)}
              >
                <ListBullets size={16} className="mp-panel-ico" aria-hidden="true" />
                <span className="mp-panel-name">{p.name}</span>
                <span className="mp-panel-count">{p.trackCount}</span>
              </button>
            ))}
            {(playlists?.length ?? 0) === 0 && (
              <p className="mp-panel-empty">
                No playlists yet — create one in the main window.
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}