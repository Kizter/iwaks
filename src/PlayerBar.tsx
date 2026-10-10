import { useEffect, useState } from "react";
import { Cover } from "./Cover";
import { LyricsPanel } from "./LyricsPanel";
import { formatDuration } from "./format";
import {
  playerNext,
  playerPrev,
  playerReshuffle,
  playerSeek,
  playerSetEq,
  playerSetReplayGain,
  playerSetRepeat,
  playerSetShuffle,
  playerSetSleepTimer,
  playerSetSpeed,
  playerSetVolume,
  playerToggleMute,
  playerTogglePlay,
  toggleMiniPlayer,
} from "./api";
import type { PlayerState, ReplayGainMode, RepeatMode } from "./types";
import {
  DiceFour,
  Moon,
  MusicNotes,
  Pause,
  PictureInPicture,
  Play,
  Repeat,
  RepeatOnce,
  Shuffle,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
  SpeakerHigh,
  SpeakerSlash,
} from "@phosphor-icons/react";

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

/** Labels for the 10 EQ bands (ISO frequencies, matches EQ_BANDS in Rust). */
const EQ_LABELS = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
const EQ_ZERO = Array(EQ_LABELS.length).fill(0) as number[];

/** Compact dB label: "0", "-3.5", "+6". */
function fmtDb(v: number): string {
  const s = Math.round(v * 10) / 10;
  return `${s > 0 ? "+" : ""}${s}`;
}

/** Always-visible bottom bar: now-playing, transport, seek, repeat, volume. */
export default function PlayerBar({
  state,
  nowPlaying,
  onToggleNowPlaying,
}: {
  state: PlayerState | null;
  nowPlaying?: boolean;
  onToggleNowPlaying?: () => void;
}) {
  const [drag, setDrag] = useState<number | null>(null);
  const [volDraft, setVolDraft] = useState<number | null>(null);
  const [sleepChoice, setSleepChoice] = useState("off");
  const [eqOpen, setEqOpen] = useState(false);
  const [eqDraft, setEqDraft] = useState<{ preamp: number; gains: number[] } | null>(null);
  const [lyricsOpen, setLyricsOpen] = useState(false);
  const [sleepOpen, setSleepOpen] = useState(false);
  const cur = state?.current ?? null;
  const hasTrack = cur !== null;
  const playing = hasTrack && !state!.paused && !state!.stopped;
  const duration = state?.duration ?? 0;
  const position = state?.position ?? 0;
  const shownPos = drag !== null ? drag : Math.min(position, duration || 0);
  const volume = state?.volume ?? 0;
  const shownVolume = volDraft ?? volume;
  const repeat = state?.repeat ?? "off";
  const shuffle = state?.shuffle ?? false;
  const mute = state?.mute ?? false;
  const speed = state?.speed ?? 1;
  const sleepRemaining = state?.sleepRemaining ?? null;
  const eqPreamp = state?.eqPreamp ?? 0;
  const eqGains = state?.eq ?? EQ_ZERO;
  const shownPreamp = eqDraft ? eqDraft.preamp : eqPreamp;
  const shownGain = (i: number) => (eqDraft ? eqDraft.gains[i] : eqGains[i]);

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

  // ---- EQ draft: sliders edit locally, one commit on release ----
  const eqBase = () => (eqDraft ?? { preamp: eqPreamp, gains: [...eqGains] });

  const setEqBand = (i: number, v: number) => {
    const base = eqBase();
    const gains = [...base.gains];
    gains[i] = v;
    setEqDraft({ preamp: base.preamp, gains });
  };

  const setEqPreamp = (v: number) => {
    const base = eqBase();
    setEqDraft({ preamp: v, gains: base.gains });
  };

  const commitEq = () => {
    if (eqDraft) {
      void playerSetEq(eqDraft.preamp, eqDraft.gains);
      setEqDraft(null);
    }
  };

  const resetEq = () => {
    setEqDraft(null);
    void playerSetEq(0, [...EQ_ZERO]);
  };

  /** Fire the pending volume change once, on release (pointer/key) or blur. */
  const commitVolume = () => {
    if (volDraft !== null) {
      void playerSetVolume(volDraft);
      setVolDraft(null);
    }
  };

  return (
    <footer className="player-bar" aria-label="Player">
      <div className="pb-left">
        {cur ? (
          onToggleNowPlaying ? (
            <button
              type="button"
              className="pb-open"
              onClick={onToggleNowPlaying}
              aria-label={nowPlaying ? "Close Now Playing" : "Open Now Playing"}
              aria-pressed={nowPlaying}
              title={nowPlaying ? "Close Now Playing" : "Open Now Playing"}
            >
              <span className="pb-art">
                <Cover track={cur} />
              </span>
              <span className="pb-meta">
                <span className="pb-title">{cur.title}</span>
                <span className="pb-sub">{cur.artist ?? "Unknown artist"}</span>
              </span>
            </button>
          ) : (
            <>
              <span className="pb-art">
                <Cover track={cur} />
              </span>
              <span className="pb-meta">
                <span className="pb-title">{cur.title}</span>
                <span className="pb-sub">{cur.artist ?? "Unknown artist"}</span>
              </span>
            </>
          )
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
            <SkipBack size={18} aria-hidden="true" />
          </button>
          <button
            className="icon-btn play-main"
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
            className="icon-btn"
            onClick={() => void playerNext()}
            disabled={!state}
            aria-label="Next track"
            title="Next track"
          >
            <SkipForward size={18} aria-hidden="true" />
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
            // Commit a pending seek when focus leaves or the pointer is
            // cancelled, instead of silently discarding the dragged value.
            onBlur={commitSeek}
            onPointerCancel={commitSeek}
          />
          <span>{formatDuration(duration * 1000)}</span>
        </div>
      </div>

      <div className="pb-right">
        <button
          className={`icon-btn${shuffle ? " repeat-on" : ""}`}
          onClick={() => void playerSetShuffle(!shuffle)}
          disabled={!state}
          title={shuffle ? "Shuffle on — click to turn off" : "Shuffle — randomize the rest of the list"}
          aria-label="Shuffle"
          aria-pressed={shuffle}
        >
          <Shuffle size={18} aria-hidden="true" />
        </button>
        <button
          className="icon-btn dice-btn"
          onClick={() => void playerReshuffle()}
          disabled={!state}
          title={shuffle ? "Reshuffle the remaining tracks" : "Shuffle (randomizes the rest of the list)"}
          aria-label="Reshuffle remaining tracks"
        >
          <DiceFour size={18} aria-hidden="true" />
        </button>
        <button
          className={`icon-btn${repeat !== "off" ? " repeat-on" : ""}`}
          onClick={cycleRepeat}
          disabled={!state}
          title={REPEAT_TITLE[repeat]}
          aria-label={REPEAT_TITLE[repeat]}
          aria-pressed={repeat !== "off"}
        >
          {repeat === "one" ? (
            <RepeatOnce size={18} aria-hidden="true" />
          ) : (
            <Repeat size={18} aria-hidden="true" />
          )}
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
        <div className="sleep-wrap">
          <button
            className="icon-btn"
            onClick={() => setSleepOpen((o) => !o)}
            disabled={!state}
            title={sleepRemaining !== null ? "Sleep timer active" : "Sleep timer"}
            aria-label="Sleep timer"
            aria-expanded={sleepOpen}
          >
            <Moon size={18} aria-hidden="true" />
          </button>
          {sleepOpen && (
            <div className="sleep-pop" role="menu" aria-label="Sleep timer">
              {SLEEP_OPTIONS.map((o) => (
                <button
                  key={o.value}
                  type="button"
                  role="menuitemradio"
                  aria-checked={sleepChoice === o.value}
                  className={`sleep-opt${sleepChoice === o.value ? " on" : ""}`}
                  onClick={() => {
                    setSleepChoice(o.value);
                    setSleepOpen(false);
                    void playerSetSleepTimer(o.value === "off" ? null : Number(o.value));
                  }}
                >
                  <span>{o.label}</span>
                  {sleepChoice === o.value && (
                    <span className="sleep-opt-dot" aria-hidden="true" />
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
        {sleepRemaining !== null && sleepRemaining > 0 && (
          <span className="sleep-count" aria-label="Sleep timer remaining">
            {formatDuration(Math.round(sleepRemaining * 1000))}
          </span>
        )}
        <div className="eq-wrap">
          <button
            className={`icon-btn${eqDraft ? " repeat-on" : ""}`}
            onClick={() => setEqOpen((o) => !o)}
            disabled={!state}
            title="Equalizer"
            aria-label="Equalizer"
            aria-expanded={eqOpen}
            aria-pressed={eqDraft !== null}
          >
            <SlidersHorizontal size={18} aria-hidden="true" />
          </button>
          {eqOpen && state && (
            <div className="eq-pop" role="group" aria-label="Equalizer panel">
              <div className="eq-head">
                <span>Equalizer</span>
                <button className="eq-reset" onClick={resetEq}>
                  Reset
                </button>
              </div>
              <div className="eq-bands">
                {EQ_LABELS.map((label, i) => (
                  <label key={label} className="eq-band">
                    <span className="eq-val">{fmtDb(shownGain(i))}</span>
                    <input
                      type="range"
                      className="eq-slider"
                      min={-12}
                      max={12}
                      step={0.5}
                      value={shownGain(i)}
                      aria-label={`EQ ${label} Hz`}
                      onChange={(e) => setEqBand(i, Number(e.target.value))}
                      onPointerUp={commitEq}
                      onKeyUp={commitEq}
                      onBlur={commitEq}
                    />
                    <span className="eq-freq">{label}</span>
                  </label>
                ))}
              </div>
              <label className="eq-preamp">
                <span className="eq-val">{fmtDb(shownPreamp)}</span>
                <input
                  type="range"
                  className="eq-slider eq-preamp-slider"
                  min={-12}
                  max={12}
                  step={0.5}
                  value={shownPreamp}
                  aria-label="EQ preamp"
                  onChange={(e) => setEqPreamp(Number(e.target.value))}
                  onPointerUp={commitEq}
                  onKeyUp={commitEq}
                  onBlur={commitEq}
                />
                <span>Preamp</span>
              </label>
              <label className="rg-ctl">
                <span>ReplayGain</span>
                <select
                  value={state.replaygain}
                  aria-label="ReplayGain mode"
                  onChange={(e) =>
                    void playerSetReplayGain(e.target.value as ReplayGainMode)
                  }
                >
                  <option value="off">Off</option>
                  <option value="track">Track</option>
                  <option value="album">Album</option>
                </select>
              </label>
            </div>
          )}
        </div>
        <div className="lyr-wrap">
          <button
            className="icon-btn"
            onClick={() => setLyricsOpen((o) => !o)}
            disabled={!state}
            title="Lyrics"
            aria-label="Lyrics"
            aria-expanded={lyricsOpen}
          >
            <MusicNotes size={18} aria-hidden="true" />
          </button>
          {lyricsOpen && state && (
            <div className="lyr-pop" role="group" aria-label="Lyrics panel">
              <LyricsPanel path={cur?.path ?? null} position={position} />
            </div>
          )}
        </div>
        <button
          className="icon-btn"
          onClick={() => void toggleMiniPlayer()}
          disabled={!state}
          title="Always-on-top mini player window"
          aria-label="Open mini player window"
        >
          <PictureInPicture size={18} aria-hidden="true" />
        </button>
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
          // Draft-then-commit (mirrors the EQ/seek sliders): the thumb follows
          // the local value during a drag, one IPC invoke lands on release.
          onChange={(e) => setVolDraft(Number(e.target.value))}
          onPointerUp={commitVolume}
          onKeyUp={commitVolume}
          onBlur={commitVolume}
        />
      </div>
    </footer>
  );
}