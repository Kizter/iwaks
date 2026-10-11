import { useEffect, useState } from "react";
import { Cover } from "./Cover";
import { LyricsPanel } from "./LyricsPanel";
import { BUILTIN_PRESETS, EQ_LABELS, EQ_ZERO, matchPreset } from "./eq";
import { formatDuration } from "./format";
import {
  getEqPresets,
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
  setEqPresets,
  toggleMiniPlayer,
} from "./api";
import type { EqPreset, PlayerState, ReplayGainMode, RepeatMode } from "./types";
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
  Trash,
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
  const [customPresets, setCustomPresets] = useState<EqPreset[]>([]);
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
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

  // The dropdown highlights the preset whose curve matches the live one.
  const activePreset = matchPreset(eqPreamp, eqGains, [...BUILTIN_PRESETS, ...customPresets]);
  const presetsMatch = (p: EqPreset) => p.name === activePreset?.name;
  const activeCustom = activePreset && customPresets.some(presetsMatch) ? activePreset.name : null;
  const presetValue = activePreset
    ? `${customPresets.some(presetsMatch) ? "custom" : "builtin"}:${activePreset.name}`
    : "";

  // When the timer runs out or is cleared from elsewhere, the select snaps
  // back to "Off" instead of showing a stale countdown value.
  useEffect(() => {
    if (sleepRemaining === null && sleepChoice !== "off") {
      setSleepChoice("off");
    }
  }, [sleepRemaining, sleepChoice]);

  // Load the user's saved EQ presets once; failures just leave the menu with
  // the built-in templates (the dropdown degrades gracefully offline).
  useEffect(() => {
    let alive = true;
    getEqPresets()
      .then((presets) => {
        if (alive) setCustomPresets(presets);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

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

  /** Apply a preset from the dropdown (built-in or saved custom one). */
  const onPickPreset = (value: string) => {
    if (!value) return;
    const preset = value.startsWith("custom:")
      ? customPresets.find((p) => p.name === value.slice("custom:".length))
      : BUILTIN_PRESETS.find((p) => p.name === value.slice("builtin:".length));
    if (!preset) return;
    setEqDraft(null);
    void playerSetEq(preset.preamp, [...preset.gains]);
  };

  /** Persist the current curve (draft included) under a new or reused name. */
  const savePreset = (e: React.FormEvent) => {
    e.preventDefault();
    const name = saveName.trim();
    if (!name) return;
    const base = eqBase();
    const preset: EqPreset = { name, preamp: base.preamp, gains: [...base.gains] };
    const i = customPresets.findIndex((p) => p.name.toLowerCase() === name.toLowerCase());
    const next =
      i >= 0
        ? customPresets.map((p, idx) => (idx === i ? preset : p))
        : [...customPresets, preset];
    setCustomPresets(next);
    setSaveOpen(false);
    setSaveName("");
    void setEqPresets(next);
  };

  /** Remove one saved preset and persist the remaining list. */
  const deleteCustomPreset = (name: string) => {
    const next = customPresets.filter((p) => p.name !== name);
    setCustomPresets(next);
    void setEqPresets(next);
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
              <div className="eq-presets">
                <label className="eq-preset">
                  <span>Preset</span>
                  <select
                    value={presetValue}
                    aria-label="Equalizer preset"
                    onChange={(e) => onPickPreset(e.target.value)}
                  >
                    <option value="">Custom</option>
                    <optgroup label="Presets">
                      {BUILTIN_PRESETS.map((p) => (
                        <option key={p.name} value={`builtin:${p.name}`}>
                          {p.name}
                        </option>
                      ))}
                    </optgroup>
                    {customPresets.length > 0 && (
                      <optgroup label="My presets">
                        {customPresets.map((p) => (
                          <option key={p.name} value={`custom:${p.name}`}>
                            {p.name}
                          </option>
                        ))}
                      </optgroup>
                    )}
                  </select>
                </label>
                {saveOpen ? (
                  <form className="eq-save" onSubmit={savePreset}>
                    <input
                      autoFocus
                      className="eq-save-input"
                      value={saveName}
                      placeholder="Preset name…"
                      aria-label="Preset name"
                      onChange={(e) => setSaveName(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") {
                          setSaveOpen(false);
                          setSaveName("");
                        }
                      }}
                    />
                    <button type="submit" className="eq-save-btn" disabled={!saveName.trim()}>
                      Save
                    </button>
                    <button
                      type="button"
                      className="eq-save-btn"
                      onClick={() => {
                        setSaveOpen(false);
                        setSaveName("");
                      }}
                    >
                      Cancel
                    </button>
                  </form>
                ) : (
                  <div className="eq-save-row">
                    <button
                      type="button"
                      className="eq-save-btn"
                      onClick={() => {
                        setSaveName(activeCustom ?? "");
                        setSaveOpen(true);
                      }}
                    >
                      Save as…
                    </button>
                    {activeCustom && (
                      <button
                        type="button"
                        className="eq-delete"
                        onClick={() => deleteCustomPreset(activeCustom)}
                        aria-label={`Delete preset ${activeCustom}`}
                        title="Delete preset"
                      >
                        <Trash size={14} aria-hidden="true" />
                      </button>
                    )}
                  </div>
                )}
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