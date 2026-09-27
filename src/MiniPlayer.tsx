// Layout for the always-on-top mini player window (`#mini` in the URL).
// Player state arrives through the same app-wide `player-state` events, so
// the mini window mirrors the main player: play/pause, prev/next, shuffle,
// plus a playlist/queue picker. The whole surface is a drag region — grab
// anywhere to move the frameless window.

import { useEffect, useRef, useState } from "react";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
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

function CloseIcon() {
  return (
    <svg viewBox="0 0 14 14" width="13" height="13" aria-hidden="true">
      <path fill="currentColor" d="M2.5 2.5l9 9M11.5 2.5l-9 9" stroke="currentColor" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path fill="currentColor" d="M4 2.6v10.8l8.6-5.4L4 2.6z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path fill="currentColor" d="M3.5 2.5h3.2v11H3.5zM9.3 2.5h3.2v11H9.3z" />
    </svg>
  );
}

function PrevIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path fill="currentColor" d="M3 2.5h1.9v11H3zM13.4 3.1v9.8L6.9 8l6.5-4.9z" />
    </svg>
  );
}

function NextIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path fill="currentColor" d="M11.1 2.5H13v11h-1.9zM2.6 3.1v9.8L9.1 8 2.6 3.1z" />
    </svg>
  );
}

function ShuffleIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path
        fill="currentColor"
        d="M1.8 4.4h2.3c1.1 0 2.1.5 2.7 1.4l.5.7-.9 1.2-.5-.7c-.4-.5-.9-.8-1.5-.8H1.8zm9.9 0h2l-2.3 2.3-2.3-2.3-.9.9 3.2 3.2 3.2-3.2-.9-.9zM2.2 11.6h.9c1.1 0 2.1-.5 2.7-1.4l3.1-4.5c.4-.5.9-.8 1.5-.8h.9l-2.3 2.3-2.3-2.3-.9.9 3.2 3.2 3.2-3.2-.9-.9h-2c-1.1 0-2.1.5-2.7 1.4l-3.1 4.5c-.2.3-.3.6-.3.9v.7h.9z"
      />
    </svg>
  );
}

function PlaylistIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        fill="currentColor"
        d="M2 3.2h12v1.6H2zM2 7h12v1.6H2zM2 10.8h7.2v1.6H2z"
      />
    </svg>
  );
}

export function MiniPlayer() {
  const [state, setState] = useState<PlayerState | null>(null);
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  // Label of the last source chosen here (playlist name or "Queue").
  const [source, setSource] = useState("Playlist");
  const unlistenRef = useRef<(() => void) | null>(null);

  // Base window height (opened via `open_mini_window`). The playlist picker
  // is a panel at the BOTTOM of the window, so the window grows while it is
  // open (height measured from the panel's actual rendered position) and
  // shrinks back when it closes. Width is left untouched.
  const BASE_HEIGHT = 160;

  const fitPicker = async (visible: boolean) => {
    if (visible) {
      // Let React commit the panel first so its height can be measured.
      await new Promise((r) => setTimeout(r, 0));
    }
    const win = getCurrentWindow();
    const sf = await win.scaleFactor();
    const size = await win.outerSize();
    const panel = document.querySelector(".mp-panel");
    const target =
      visible && panel
        ? Math.min(Math.ceil(panel.getBoundingClientRect().bottom + 10), 560)
        : BASE_HEIGHT;
    if (target !== size.height) {
      await win.setSize(new LogicalSize(size.width / sf, target));
    }
  };

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
      void fitPicker(true);
    } else {
      setPickerOpen(false);
      void fitPicker(false);
    }
  };

  const playSource = async (tracks: Track[], label: string) => {
    if (tracks.length === 0) return;
    setPickerOpen(false);
    void fitPicker(false);
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
          <CloseIcon />
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
          <ShuffleIcon />
        </button>
        <button
          type="button"
          className="mp-btn"
          onClick={() => void playerPrev()}
          disabled={!state}
          aria-label="Previous track"
          title="Previous track"
        >
          <PrevIcon />
        </button>
        <button
          type="button"
          className="mp-btn mp-play"
          onClick={() => void playerTogglePlay()}
          disabled={!state}
          aria-label={playing ? "Pause" : "Play"}
          title={playing ? "Pause" : "Play"}
        >
          {playing ? <PauseIcon /> : <PlayIcon />}
        </button>
        <button
          type="button"
          className="mp-btn"
          onClick={() => void playerNext()}
          disabled={!state}
          aria-label="Next track"
          title="Next track"
        >
          <NextIcon />
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
            <PlaylistIcon />
            <span className="mp-source-name">{source}</span>
          </button>
        </div>
      </div>

      {pickerOpen && (
        <div className="mp-panel" role="listbox" aria-label="Choose what to play">
          <button
            type="button"
            role="option"
            aria-selected={source === "Queue"}
            className={`mp-panel-item${source === "Queue" ? " on" : ""}`}
            onClick={() => void playQueue()}
          >
            <span>Queue (session)</span>
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
              <span>{p.name}</span>
              <span className="mp-panel-count">{p.trackCount}</span>
            </button>
          ))}
          {(playlists?.length ?? 0) === 0 && (
            <p className="mp-panel-empty">
              No playlists yet — create one in the main window.
            </p>
          )}
        </div>
      )}
    </div>
  );
}