import { useEffect, useMemo, useRef, useState } from "react";
import { Cover } from "./Cover";
import { formatDuration } from "./format";
import {
  getLyrics,
  getSpectrum,
  playerNext,
  playerPrev,
  playerSeek,
  playerSetEq,
  playerSetReplayGain,
  playerSetRepeat,
  playerSetSleepTimer,
  playerSetSpeed,
  playerSetVolume,
  playerToggleMute,
  playerTogglePlay,
} from "./api";
import type { Lyrics, PlayerState, ReplayGainMode, RepeatMode, Spectrum } from "./types";

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

function EqBarsIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        fill="currentColor"
        d="M2.5 3h3v10h-3zM6.5 6h3v7h-3zM10.5 2h3v11h-3z"
      />
    </svg>
  );
}

/** Compact dB label: "0", "-3.5", "+6". */
function fmtDb(v: number): string {
  const s = Math.round(v * 10) / 10;
  return `${s > 0 ? "+" : ""}${s}`;
}

function LyricsIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        fill="currentColor"
        d="M2 3.4h12v1.6H2zM2 7.2h12v1.6H2zM2 11h8v1.6H2z"
      />
    </svg>
  );
}

function VizIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        fill="currentColor"
        d="M1.6 6.4h1.9v3.2H1.6zM4.7 4.4h1.9v7.2H4.7zM7.8 1.9h1.9v12.2H7.8zM10.9 4.4h1.9v7.2h-1.9zM13.9 6.4h1.9v3.2h-1.9z"
      />
    </svg>
  );
}

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
  const [eqOpen, setEqOpen] = useState(false);
  const [eqDraft, setEqDraft] = useState<{ preamp: number; gains: number[] } | null>(null);
  const [lyricsOpen, setLyricsOpen] = useState(false);
  const [lyrics, setLyrics] = useState<Lyrics | null | undefined>(undefined);
  const [vizOpen, setVizOpen] = useState(false);
  const [spectrum, setSpectrum] = useState<Spectrum | null | undefined>(undefined);
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

  // ---- lyrics: fetch for the current track while the panel is open ----
  const lyricPath = cur?.path ?? null;
  useEffect(() => {
    if (!lyricsOpen || !lyricPath) {
      setLyrics(undefined);
      return;
    }
    let cancelled = false;
    setLyrics(undefined); // loading
    void getLyrics(lyricPath).then((l) => {
      if (!cancelled) setLyrics(l);
    });
    return () => {
      cancelled = true;
    };
  }, [lyricsOpen, lyricPath]);

  // Active line = the last one whose timestamp is <= playback position.
  const activeIdx = useMemo(() => {
    const timed = lyrics?.timed ?? [];
    if (timed.length === 0) return -1;
    const t = state?.position ?? 0;
    let idx = -1;
    for (let i = 0; i < timed.length; i++) {
      if (timed[i].time <= t) idx = i;
      else break;
    }
    return idx;
  }, [lyrics, state?.position]);

  const lyricBox = useRef<HTMLDivElement | null>(null);
  const activeLine = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const box = lyricBox.current;
    const el = activeLine.current;
    if (box && el) {
      box.scrollTo({
        top: Math.max(0, el.offsetTop - box.clientHeight / 2),
        behavior: "smooth",
      });
    }
  }, [activeIdx]);

  // ---- visualizer: fetch the spectrum timeline for the current track ----
  const vizPath = cur?.path ?? null;
  useEffect(() => {
    if (!vizOpen || !vizPath) {
      setSpectrum(undefined);
      return;
    }
    let cancelled = false;
    setSpectrum(undefined); // analyzing
    void getSpectrum(vizPath).then((s) => {
      if (!cancelled) setSpectrum(s);
    });
    return () => {
      cancelled = true;
    };
  }, [vizOpen, vizPath]);

  // Smoothed bar magnitudes persist across re-runs so ~10 Hz state ticks
  // don't reset the bars; canvas size is re-applied only when it changes.
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const vizBars = useRef<Float32Array | null>(null);
  const vizSize = useRef({ w: 0, h: 0, dpr: 0 });
  useEffect(() => {
    if (!vizOpen || !state || !spectrum || spectrum.frames.length === 0) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const { frames, fps, bins } = spectrum;
    if (!vizBars.current || vizBars.current.length !== bins) {
      vizBars.current = new Float32Array(bins);
    }
    const current = vizBars.current;
    const target = new Float32Array(bins);

    let lastPos = state.position;
    let lastWall = performance.now();
    let playing = !state.paused && !state.stopped;
    let speed = state.speed;
    let raf = 0;

    const draw = (now: number) => {
      // Extrapolate position between ~10 Hz state ticks while playing.
      if (playing && now > lastWall) {
        lastPos += ((now - lastWall) / 1000) * speed;
      }
      lastWall = now;

      const fi = Math.min(frames.length - 1, Math.max(0, Math.floor(lastPos * fps)));
      const frame = frames[fi];
      if (playing) {
        for (let i = 0; i < bins; i++) target[i] = frame[i] ?? 0;
      } else {
        target.fill(0);
      }
      // Fast attack when a bar rises, slower release when it falls.
      for (let i = 0; i < bins; i++) {
        current[i] += (target[i] - current[i]) * (target[i] > current[i] ? 0.55 : 0.12);
      }

      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      const dpr = window.devicePixelRatio || 1;
      const size = vizSize.current;
      if (size.w !== w || size.h !== h || size.dpr !== dpr) {
        vizSize.current = { w, h, dpr };
        canvas.width = Math.max(1, Math.floor(w * dpr));
        canvas.height = Math.max(1, Math.floor(h * dpr));
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      const slot = w / bins;
      const bw = Math.max(2, slot * 0.62);
      const grad = ctx.createLinearGradient(0, h, 0, 0);
      grad.addColorStop(0, "#8fd6e8");
      grad.addColorStop(0.55, "#3f9fc9");
      grad.addColorStop(1, "#026aa7");
      ctx.fillStyle = grad;
      for (let i = 0; i < bins; i++) {
        const bh = 2 + current[i] * (h - 8);
        ctx.fillRect(i * slot + (slot - bw) / 2, h - bh, bw, bh);
      }
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [vizOpen, state, spectrum]);

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
            <EqBarsIcon />
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
            <LyricsIcon />
          </button>
          {lyricsOpen && state && (
            <div className="lyr-pop" role="group" aria-label="Lyrics panel">
              {lyrics === undefined ? (
                <p className="lyr-empty">Loading…</p>
              ) : lyrics === null || (lyrics.timed.length === 0 && !lyrics.plain) ? (
                <p className="lyr-empty">No lyrics for this track</p>
              ) : lyrics.timed.length > 0 ? (
                <div className="lyr-list" ref={lyricBox}>
                  {lyrics.timed.map((line, i) => (
                    <div
                      key={i}
                      ref={i === activeIdx ? activeLine : undefined}
                      className={`lyr-line${i === activeIdx ? " on" : ""}`}
                    >
                      {line.text}
                    </div>
                  ))}
                </div>
              ) : (
                <pre className="lyr-plain">{lyrics.plain}</pre>
              )}
            </div>
          )}
        </div>
        <div className="viz-wrap">
          <button
            className="icon-btn"
            onClick={() => setVizOpen((o) => !o)}
            disabled={!state}
            title="Visualizer"
            aria-label="Visualizer"
            aria-expanded={vizOpen}
          >
            <VizIcon />
          </button>
          {vizOpen && state && (
            <div className="viz-pop" role="group" aria-label="Visualizer panel">
              {spectrum === undefined ? (
                <p className="viz-empty">Analyzing…</p>
              ) : spectrum === null || spectrum.frames.length === 0 ? (
                <p className="viz-empty">No spectrum for this track</p>
              ) : (
                <canvas
                  ref={canvasRef}
                  className="viz-canvas"
                  aria-label="Spectrum visualizer"
                />
              )}
            </div>
          )}
        </div>
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