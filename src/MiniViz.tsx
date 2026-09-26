// Layout for the always-on-top mini visualizer window (`#mini` in the URL).
// Player state arrives through the same app-wide `player-state` events, so
// this window animates exactly like the main one.

import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getPlayerState, listenPlayer } from "./api";
import type { PlayerState } from "./types";
import { Visualizer } from "./Visualizer";

function CloseIcon() {
  return (
    <svg viewBox="0 0 14 14" width="13" height="13" aria-hidden="true">
      <path fill="currentColor" d="M2.5 2.5l9 9M11.5 2.5l-9 9" stroke="currentColor" />
    </svg>
  );
}

export function MiniViz() {
  const [state, setState] = useState<PlayerState | null>(null);
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

  return (
    <div className="mini-viz" data-playing={playing}>
      <div className="mini-viz-head" data-tauri-drag-region>
        <span className="mini-viz-title" title={cur?.title ?? ""}>
          {playing && cur ? `${cur.title} — ${cur.artist ?? "Unknown artist"}` : "Iwaks — idle"}
        </span>
        <button
          type="button"
          className="mini-viz-close"
          onClick={() => void getCurrentWindow().close()}
          aria-label="Close mini visualizer"
          title="Close"
        >
          <CloseIcon />
        </button>
      </div>
      <div className="mini-viz-stage">
        <Visualizer state={state} seed={cur?.path ?? null} variant="mini" className="viz-canvas" />
      </div>
    </div>
  );
}