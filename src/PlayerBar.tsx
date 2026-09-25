import { useEffect, useState } from "react";
import { Cover } from "./Cover";
import { formatDuration } from "./format";
import {
  playerNext,
  playerPrev,
  playerSeek,
  playerSetRepeat,
  playerSetSleepTimer,
  playerSetSpeed,
  playerSetVolume,
  playerToggleMute,
  playerTogglePlay,
} from "./api";
import type { PlayerState, RepeatMode } from "./types";

const REPEAT_ORDER: RepeatMode[] = ["off", "all", "one"];
const REPEAT_TITLE: Record<RepeatMode, string> = {
  off: "Repeat: off",
  all: "Repeat: all",
  one: "Repeat: one",
};

/** Speed cycle offered by the player-bar button, in the same order. */
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];

/** Sleep timer choices in seconds; the select maps "off" to `null`. */
const SLEEP_OPTIONS: Array<{ value: string; label: string }> = [
  { value: "off", label: "Off" },
  { value: "900", label: "15 min" },
  { value: "1800", label: "30 min" },
  { value: "3600", label: "60 min" },
  { value: "5400", label: "90 min" },
];

function PlayIcon() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path fill="currentColor" d="M4 2.6v10.8l8.6-5.4L4 2.6z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path fill="currentColor" d="M3.5 2.5h3.2v11H3.5zM9.3 2.5h3.2v11H9.3z" />
    </svg>
  );
}

function PrevIcon() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path fill="currentColor" d="M3 2.5h1.9v11H3zM13.4 3.1v9.8L6.9 8l6.5-4.9z" />
    </svg>
  );
}

function NextIcon() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path fill="currentColor" d="M11.1 2.5H13v11h-1.9zM2.6 3.1v9.8L9.1 8 2.6 3.1z" />
    </svg>
  );
}

function VolumeIcon({ muted }: { muted: boolean }) {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        fill="currentColor"
        d="M1.5 5.2v5.6h2.9l3.6 2.9V2.3L4.4 5.2H1.5z"
      />
      {muted ? (
        <path fill="currentColor" d="M11.2 5.1l4.5 4.5-.9.9-4.5-4.5.9-.9zm-.9 4.5l.9.9 4.5-4.5-.9-.9-4.5 4.5z" />
      ) : (
        <path
          fill="currentColor"
          d="M11.5 6.3a3.4 3.4 0 0 1 0 3.4l-1.1-.7a2.2 2.2 0 0 0 0-2l1.1-.7z"
        />
      )}
    </svg>
  );
}

function RepeatIcon({ mode }: { mode: RepeatMode }) {
  const glyph = mode === "one" ? "1" : mode === "all" ? "2" : "↻";
  return <span className="repeat-glyph">{glyph}</span>;
}

/** Always-visible bottom bar: now-playing, transport, seek, repeat, volume. */
export default function PlayerBar({ state }: { state: PlayerState | null }) {
  const [drag, setDrag] = useState<number | null>(null);
  const [sleepChoice, setSleepChoice] = useState("off");
  const cur = state?.current ?? null;
  const hasTrack = cur !== null;
  const playing = hasTrack && !state!.paused && !state!.stopped;
  const duration = state?.duration ?? 0;
  const position = state?.position ?? 0;
  const shownPos = drag !== null ? drag : Math.min(position, duration || 0);
  const volume = state?.volume ?? 0;
  const repeat = state?.repeat ?? "off";
  const mute = state?.mute ?? false;
  const speed = state?.speed ?? 1;
  const sleepRemaining = state?.sleepRemaining ?? null;

  // When the timer runs out or is cleared from elsewhere, the select snaps
  // back to "Off" instead of showing a stale countdown value.
  useEffect(() => {
    if (sleepRemaining === null && sleepChoice !== "off") {
      setSleepChoice("off");
    }
  }, [sleepRemaining, sleepChoice]);

  const commitSeek = () => {
    if (drag !== null) {
      void playerSeek(drag);
      setDrag(null);
    }
  };

  const cycleRepeat = () => {
    if (!state) return;
    const i = Math.max(0, REPEAT_ORDER.indexOf(state.repeat));
    void playerSetRepeat(REPEAT_ORDER[(i + 1) % REPEAT_ORDER.length]);
  };

  const cycleSpeed = () => {
    if (!state) return;
    const i = Math.max(0, SPEEDS.indexOf(state.speed));
    void playerSetSpeed(SPEEDS[(i + 1) % SPEEDS.length]);
  };

  return (
    <footer className="player-bar" aria-label="Player">
      <div className="pb-left">
        {cur ? (
          <>
            <span className="pb-art">
              <Cover track={cur} />
            </span>
            <span className="pb-meta">
              <span className="pb-title">{cur.title}</span>
              <span className="pb-sub">{cur.artist ?? "Unknown artist"}</span>
            </span>
          </>
        ) : (
          <span className="pb-empty">{state ? "Nothing playing" : "Playback unavailable"}</span>
        )}
      </div>

      <div className="pb-center">
        <div className="pb-controls">
          <button
            className="icon-btn"
            onClick={() => void playerPrev()}
            disabled={!state}
            aria-label="Previous track"
            title="Previous track"
          >
            <PrevIcon />
          </button>
          <button
            className="icon-btn play-main"
            onClick={() => void playerTogglePlay()}
            disabled={!state}
            aria-label={playing ? "Pause" : "Play"}
            title={playing ? "Pause" : "Play"}
          >
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <button
            className="icon-btn"
            onClick={() => void playerNext()}
            disabled={!state}
            aria-label="Next track"
            title="Next track"
          >
            <NextIcon />
          </button>
        </div>
        <div className="pb-time">
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
            onBlur={() => setDrag(null)}
          />
          <span>{formatDuration(duration * 1000)}</span>
        </div>
      </div>

      <div className="pb-right">
        <button
          className={`icon-btn${repeat !== "off" ? " repeat-on" : ""}`}
          onClick={cycleRepeat}
          disabled={!state}
          title={REPEAT_TITLE[repeat]}
          aria-label={REPEAT_TITLE[repeat]}
          aria-pressed={repeat !== "off"}
        >
          <RepeatIcon mode={repeat} />
        </button>
        <button
          className="icon-btn speed-btn"
          onClick={cycleSpeed}
          disabled={!state}
          title={`Speed: ${speed}×`}
          aria-label="Playback speed"
        >
          {String(speed)}×
        </button>
        <label className="sleep-ctl">
          <span className="sleep-label">Sleep</span>
          <select
            value={sleepChoice}
            disabled={!state}
            aria-label="Sleep timer"
            onChange={(e) => {
              const value = e.target.value;
              setSleepChoice(value);
              void playerSetSleepTimer(value === "off" ? null : Number(value));
            }}
          >
            {SLEEP_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        {sleepRemaining !== null && sleepRemaining > 0 && (
          <span className="sleep-count" aria-label="Sleep timer remaining">
            {formatDuration(Math.round(sleepRemaining * 1000))}
          </span>
        )}
        <button
          className="icon-btn"
          onClick={() => void playerToggleMute()}
          disabled={!state}
          aria-label={mute ? "Unmute" : "Mute"}
          title={mute ? "Unmute" : "Mute"}
        >
          <VolumeIcon muted={mute} />
        </button>
        <input
          type="range"
          className="vol"
          min={0}
          max={100}
          step={1}
          value={volume}
          disabled={!state}
          aria-label="Volume"
          onChange={(e) => void playerSetVolume(Number(e.target.value))}
        />
      </div>
    </footer>
  );
}