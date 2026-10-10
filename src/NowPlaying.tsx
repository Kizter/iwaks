import { useState } from "react";
import {
  CaretLeft,
  Pause,
  Play,
  SpeakerHigh,
  SpeakerSlash,
  SkipBack,
  SkipForward,
} from "@phosphor-icons/react";
import { Cover } from "./Cover";
import { LyricsPanel } from "./LyricsPanel";
import { formatDuration } from "./format";
import {
  playerNext,
  playerPrev,
  playerSeek,
  playerSetVolume,
  playerToggleMute,
  playerTogglePlay,
} from "./api";
import type { PlayerState } from "./types";

/**
 * Album-art-first Now Playing surface — the "big cover" hero from the design
 * brief. Rendered inside the app shell, so the sidebar and player bar stay
 * visible; `onClose` returns to the library view underneath.
 */
export function NowPlaying({
  state,
  onClose,
}: {
  state: PlayerState | null;
  onClose: () => void;
}) {
  const [drag, setDrag] = useState<number | null>(null);
  const [volDraft, setVolDraft] = useState<number | null>(null);

  const cur = state?.current ?? null;
  const hasTrack = cur !== null;
  const playing = hasTrack && !state!.paused && !state!.stopped;
  const duration = state?.duration ?? 0;
  const position = state?.position ?? 0;
  const shownPos = drag !== null ? drag : Math.min(position, duration || 0);
  const volume = state?.volume ?? 0;
  const shownVolume = volDraft ?? volume;
  const mute = state?.mute ?? false;

  // Draft-then-commit, mirroring the player bar: the thumb follows the local
  // value while dragging and a single IPC invoke lands on release/blur.
  const commitSeek = () => {
    if (drag !== null) {
      void playerSeek(drag);
      setDrag(null);
    }
  };
  const commitVolume = () => {
    if (volDraft !== null) {
      void playerSetVolume(volDraft);
      setVolDraft(null);
    }
  };

  return (
    <div className="now-page">
      <div className="now-head">
        <button type="button" className="back-btn" onClick={onClose}>
          <CaretLeft size={16} aria-hidden="true" />
          <span>Library</span>
        </button>
      </div>

      <div className="now-grid">
        <section className="now-left" aria-label="Now playing">
          <div className="now-art">
            {cur ? <Cover track={cur} size={96} /> : <span className="now-art-empty" />}
          </div>
          {cur ? (
            <>
              <div className="now-meta">
                <h1 className="now-title">{cur.title}</h1>
                <p className="now-sub">
                  {cur.artist ?? "Unknown artist"}
                  {cur.album ? ` — ${cur.album}` : ""}
                </p>
              </div>
              <div className="now-controls">
                <button
                  className="icon-btn now-btn"
                  onClick={() => void playerPrev()}
                  disabled={!state}
                  aria-label="Previous track"
                  title="Previous track"
                >
                  <SkipBack size={24} aria-hidden="true" />
                </button>
                <button
                  className="icon-btn play-main now-play"
                  onClick={() => void playerTogglePlay()}
                  disabled={!state}
                  aria-label={playing ? "Pause" : "Play"}
                  title={playing ? "Pause" : "Play"}
                >
                  {playing ? (
                    <Pause size={26} aria-hidden="true" />
                  ) : (
                    <Play size={26} aria-hidden="true" />
                  )}
                </button>
                <button
                  className="icon-btn now-btn"
                  onClick={() => void playerNext()}
                  disabled={!state}
                  aria-label="Next track"
                  title="Next track"
                >
                  <SkipForward size={24} aria-hidden="true" />
                </button>
              </div>
              <div className="now-seek">
                <span>{formatDuration(shownPos * 1000)}</span>
                <input
                  type="range"
                  className="seek"
                  min={0}
                  max={Math.max(1, Math.floor(duration))}
                  step={1}
                  value={shownPos}
                  disabled={!hasTrack}
                  aria-label="Seek"
                  onChange={(e) => setDrag(Number(e.target.value))}
                  onPointerUp={commitSeek}
                  onKeyUp={commitSeek}
                  onBlur={commitSeek}
                  onPointerCancel={commitSeek}
                />
                <span>{formatDuration(duration * 1000)}</span>
              </div>
              <div className="now-vol">
                <button
                  className="icon-btn"
                  onClick={() => void playerToggleMute()}
                  disabled={!state}
                  aria-label={mute ? "Unmute" : "Mute"}
                  title={mute ? "Unmute" : "Mute"}
                >
                  {mute ? (
                    <SpeakerSlash size={18} aria-hidden="true" />
                  ) : (
                    <SpeakerHigh size={18} aria-hidden="true" />
                  )}
                </button>
                <input
                  type="range"
                  className="vol"
                  min={0}
                  max={100}
                  step={1}
                  value={shownVolume}
                  disabled={!state}
                  aria-label="Volume"
                  onChange={(e) => setVolDraft(Number(e.target.value))}
                  onPointerUp={commitVolume}
                  onKeyUp={commitVolume}
                  onBlur={commitVolume}
                />
              </div>
            </>
          ) : (
            <p className="now-empty">{state ? "Nothing playing" : "Playback unavailable"}</p>
          )}
        </section>

        <section className="now-lyrics" aria-label="Lyrics">
          <h2 className="now-lyrics-title">Lyrics</h2>
          <div className="now-lyrics-body">
            <LyricsPanel path={cur?.path ?? null} position={position} />
          </div>
        </section>
      </div>
    </div>
  );
}
