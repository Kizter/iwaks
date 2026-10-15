import { useEffect, useRef, useState } from "react";
import { DiscordLogo, Keyboard, NotePencil, PaintBrushBroad, SpeakerHigh } from "@phosphor-icons/react";
import {
  getAudioDevice,
  getAudioExclusive,
  getCrossfade,
  getLogPath,
  getOnlineCover,
  getTheme,
  listAudioDevices,
  setAudioDevice,
  setAudioExclusive,
  setCrossfade,
  setOnlineCover,
  setTheme,
  type AudioDevice,
} from "./api";
import { friendlyError } from "./format";
import {
  applyTheme,
  isThemeMode,
  THEME_MODES,
  watchSystemTheme,
  type ThemeMode,
} from "./theme";
import type { CrossfadeSettings } from "./types";

/** Bounds of the overlap control; the Rust side clamps to the same numbers. */
const CROSSFADE_MIN = 1;
const CROSSFADE_MAX = 10;
/** Half a second is fine enough to tune a fade by ear, coarse enough to hit. */
const CROSSFADE_STEP = 0.5;

/** Every global shortcut, so the keys are discoverable without a manual. */
const SHORTCUTS: Array<{ keys: string; what: string }> = [
  { keys: "Space", what: "Play / pause" },
  { keys: "← / →", what: "Seek 10 seconds back / forward" },
  { keys: "Shift + ← / →", what: "Previous / next track" },
  { keys: "N", what: "Open Now Playing" },
  { keys: "Escape", what: "Close Now Playing, pickers, and modals" },
  { keys: "/", what: "Focus the search box" },
  { keys: "Alt + ↑ / ↓", what: "Move the queue item up / down" },
];

/** Settings surface: one row per persisted flag, nothing that scans or plays. */
export default function Settings() {
  // `null` until the stored value arrives, so the switch never flashes the
  // wrong state (and stays disabled while the write is in flight).
  const [onlineCover, setOnline] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [logFile, setLogFile] = useState<string | null>(null);
  // Audio output (issues #23, #24, #33). `devices` is read once the page
  // mounts — not when the select is clicked. Fetching on the open gesture
  // disabled the very control being clicked, so the native popup was dismissed
  // before it could show anything but "System default".
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [device, setDevice] = useState<string | null>(null);
  const [exclusive, setExclusive] = useState<boolean | null>(null);
  // Crossfade (issue #27). `null` until the stored pair arrives. `draft` holds
  // the slider position while it is being dragged: committing every step would
  // write to the database on each pixel of movement.
  const [crossfade, setCrossfadeState] = useState<CrossfadeSettings | null>(null);
  const [fadeSeconds, setFadeSeconds] = useState<number | null>(null);
  // Appearance (issue #25). `null` until the stored mode arrives.
  const [theme, setThemeMode] = useState<ThemeMode | null>(null);
  // Unsubscribe from the OS theme watcher while this page is mounted.
  const unwatchRef = useRef<(() => void) | null>(null);
  useEffect(() => () => unwatchRef.current?.(), []);

  useEffect(() => {
    let alive = true;
    getOnlineCover()
      .then((on) => {
        if (alive) setOnline(on);
      })
      .catch(() => {
        // Leave the switch off; the flag defaults to off on the Rust side too.
        if (alive) setOnline(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    getLogPath()
      .then((p) => {
        if (alive) setLogFile(p);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    Promise.all([getAudioDevice(), getAudioExclusive()])
      .then(([d, x]) => {
        if (!alive) return;
        setDevice(d);
        setExclusive(x);
      })
      .catch(() => {
        // A failed read must not leave the picker disabled: a select that
        // cannot be opened is exactly what issue #33 looked like from outside.
        if (alive) setDevice((current) => current ?? "");
      });
    // Read the list here, with the rest of the audio row (issue #33).
    listAudioDevices()
      .then((list) => {
        if (alive) setDevices(list);
      })
      .catch((e: unknown) => {
        if (alive) setError(`Couldn't list audio devices — ${friendlyError(e)}`);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    getCrossfade()
      .then((c) => {
        if (!alive) return;
        // Functional updates so a late answer cannot overwrite a choice the
        // user already made while the read was in flight.
        setCrossfadeState((current) => current ?? c);
        setFadeSeconds((current) => current ?? c.seconds);
      })
      .catch(() => {
        // Defaults the same way the Rust side does, so the row still works.
        if (alive) {
          setCrossfadeState({ enabled: false, seconds: 3 });
          setFadeSeconds(3);
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  // Appearance (issue #25): the App already applied the stored mode, so this
  // only reflects it in the control.
  useEffect(() => {
    let alive = true;
    getTheme()
      .then((mode) => {
        if (alive) setThemeMode(isThemeMode(mode) ? mode : "system");
      })
      .catch(() => {
        if (alive) setThemeMode("system");
      });
    return () => {
      alive = false;
    };
  }, []);

  const pickTheme = (mode: ThemeMode) => {
    const previous = theme;
    setThemeMode(mode);
    applyTheme(mode);
    // A pinned theme stops following the OS; `system` starts following again.
    unwatchRef.current?.();
    unwatchRef.current =
      mode === "system" ? watchSystemTheme(() => applyTheme("system")) : null;
    void setTheme(mode).catch((e: unknown) => {
      setThemeMode(previous);
      if (previous) applyTheme(previous);
      setError(`Couldn't save that setting — ${friendlyError(e)}`);
    });
  };

  const pickDevice = (name: string) => {
    const previous = device;
    setDevice(name);
    void setAudioDevice(name).catch((e: unknown) => {
      setDevice(previous);
      setError(`Couldn't switch audio device — ${friendlyError(e)}`);
    });
  };

  const toggleExclusive = (next: boolean) => {
    const previous = exclusive;
    setExclusive(next);
    void setAudioExclusive(next).catch((e: unknown) => {
      setExclusive(previous);
      setError(`Couldn't save that setting — ${friendlyError(e)}`);
    });
  };

  // The crossfade is stored as one pair, so both controls go through here: a
  // failed write has to put both back, or the switch and the slider would
  // disagree about what is saved.
  const commitCrossfade = (next: CrossfadeSettings, previous: CrossfadeSettings | null) => {
    setCrossfadeState(next);
    setFadeSeconds(next.seconds);
    void setCrossfade(next).catch((e: unknown) => {
      setCrossfadeState(previous);
      setFadeSeconds(previous ? previous.seconds : null);
      setError(`Couldn't save that setting — ${friendlyError(e)}`);
    });
  };

  const toggleCrossfade = (next: boolean) => {
    if (!crossfade) return;
    commitCrossfade({ ...crossfade, enabled: next }, crossfade);
  };

  // Fires on release rather than on every step of the drag, so one gesture is
  // one write. Keyboard arrows arrive as change-then-keyup, so they land here
  // too; a click that did not move the slider writes the value back unchanged.
  const commitFadeSeconds = () => {
    if (!crossfade || fadeSeconds === null || fadeSeconds === crossfade.seconds) {
      return;
    }
    commitCrossfade({ ...crossfade, seconds: fadeSeconds }, crossfade);
  };

  const toggle = (next: boolean) => {
    const previous = onlineCover;
    setOnline(next);
    setSaving(true);
    setError(null);
    void setOnlineCover(next)
      .catch((e: unknown) => {
        setOnline(previous);
        setError(`Couldn't save that setting — ${friendlyError(e)}`);
      })
      .finally(() => setSaving(false));
  };

  return (
    <>
      <div className="toolbar">
        <div className="toolbar-title">
          <h1>Settings</h1>
        </div>
      </div>
      <div className="set-scroll">
        <section className="set-group" aria-labelledby="set-discord">
          <h2 className="set-group-title" id="set-discord">
            <DiscordLogo size={18} weight="fill" aria-hidden="true" />
            Discord Rich Presence
          </h2>

          <div className="set-row">
            <div className="set-text">
              <span className="set-title" id="set-online-cover">
                Online album art
              </span>
              <span className="set-note">
                When a track starts, Iwaks looks the album up on iTunes so Discord can show the
                real cover instead of the Iwaks logo. Artist and album tags go to Apple&apos;s
                public search API; the answer is remembered per album, so nothing is asked twice.
                Albums with no cover keep the logo.
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={onlineCover ?? false}
              aria-labelledby="set-online-cover"
              className={`switch${onlineCover ? " on" : ""}`}
              disabled={onlineCover === null || saving}
              onClick={() => toggle(!onlineCover)}
            >
              <span className="switch-knob" aria-hidden="true" />
            </button>
          </div>

          {error && <p className="set-error">{error}</p>}
        </section>

        <section className="set-group" aria-labelledby="set-theme">
          <h2 className="set-group-title" id="set-theme">
            <PaintBrushBroad size={18} aria-hidden="true" />
            Appearance
          </h2>
          <div className="set-row">
            <div className="set-text">
              <span className="set-title" id="set-theme-label">
                Theme
              </span>
              <span className="set-note">
                System follows your desktop&apos;s light/dark setting, even while
                Iwaks is open.
              </span>
            </div>
            <div className="segmented" role="group" aria-labelledby="set-theme-label">
              {THEME_MODES.map((m) => (
                <button
                  key={m.value}
                  type="button"
                  className={`segmented-btn${theme === m.value ? " on" : ""}`}
                  aria-pressed={theme === m.value}
                  disabled={theme === null}
                  onClick={() => pickTheme(m.value)}
                >
                  {m.label}
                </button>
              ))}
            </div>
          </div>
        </section>

        <section className="set-group" aria-labelledby="set-audio">
          <h2 className="set-group-title" id="set-audio">
            <SpeakerHigh size={18} aria-hidden="true" />
            Audio output
          </h2>

          <div className="set-row">
            <div className="set-text">
              <span className="set-title" id="set-device">
                Output device
              </span>
              <span className="set-note">
                Where playback goes. Your choice is remembered, and the list is read
                again each time you open this page — so a device you just plugged
                in is here.
              </span>
            </div>
            {/* Disabled only until the stored choice arrives, so the control
                never flashes "System default" over a saved device. Never
                disabled while the list loads — that is what kept the popup
                from opening (issue #33). */}
            <select
              className="set-select"
              aria-labelledby="set-device"
              value={device ?? ""}
              disabled={device === null}
              onChange={(e) => pickDevice(e.target.value)}
            >
              <option value="">System default</option>
              {devices.map((d) => (
                <option key={d.name || "default"} value={d.name}>
                  {d.description || d.name || "System default"}
                </option>
              ))}
            </select>
          </div>

          <div className="set-row">
            <div className="set-text">
              <span className="set-title" id="set-exclusive">
                Exclusive mode
              </span>
              <span className="set-note">
                Bit-perfect output for hi-res playback. It takes exclusive control
                of the device, which silences other apps (YouTube, Discord) while
                the exclusive stream is open. Off is the default, and the switch
                applies straight away: turning it off hands the device back
                without restarting Iwaks.
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={exclusive ?? false}
              aria-labelledby="set-exclusive"
              className={`switch${exclusive ? " on" : ""}`}
              disabled={exclusive === null}
              onClick={() => toggleExclusive(!exclusive)}
            >
              <span className="switch-knob" aria-hidden="true" />
            </button>
          </div>

          <div className="set-row">
            <div className="set-text">
              <span className="set-title" id="set-crossfade">
                Crossfade
              </span>
              <span className="set-note">
                Overlaps the end of one song with the start of the next. It takes
                over as soon as the overlap reaches its halfway point, so
                pressing pause, skipping or stopping always lands on the track you
                can hear most of. Sample-accurate transitions are given up while
                it's on — tracks stop being gapless — and it can't be combined
                with exclusive output. Off is the default.
                {exclusive
                  ? " Exclusive mode is on, so crossfade stays inactive until you turn it off."
                  : ""}
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={crossfade?.enabled ?? false}
              aria-labelledby="set-crossfade"
              className={`switch${crossfade?.enabled ? " on" : ""}`}
              disabled={crossfade === null}
              onClick={() => toggleCrossfade(!crossfade?.enabled)}
            >
              <span className="switch-knob" aria-hidden="true" />
            </button>
          </div>

          <div className="set-row">
            <div className="set-text">
              <span className="set-title" id="set-fade-seconds">
                Fade length
              </span>
              <span className="set-note">
                How much of each song overlaps. Three seconds is a good starting
                point; longer fades suit intros, shorter ones keep fast tracks
                from being cut into.
              </span>
            </div>
            <div className="set-slider">
              <input
                type="range"
                min={CROSSFADE_MIN}
                max={CROSSFADE_MAX}
                step={CROSSFADE_STEP}
                value={fadeSeconds ?? CROSSFADE_MIN}
                disabled={crossfade === null || !crossfade.enabled}
                aria-labelledby="set-fade-seconds"
                aria-valuetext={`${fadeSeconds ?? CROSSFADE_MIN} seconds`}
                onChange={(e) => setFadeSeconds(Number(e.target.value))}
                onPointerUp={commitFadeSeconds}
                onKeyUp={commitFadeSeconds}
                onBlur={commitFadeSeconds}
                onPointerCancel={commitFadeSeconds}
              />
              {/* The input announces the value through aria-valuetext, so the
                  visible number is decoration for sighted users only. */}
              <span className="set-value" aria-hidden="true">
                {fadeSeconds ?? CROSSFADE_MIN}s
              </span>
            </div>
          </div>
        </section>

        <section className="set-group" aria-labelledby="set-log">
          <h2 className="set-group-title" id="set-log">
            <NotePencil size={18} aria-hidden="true" />
            Diagnostics
          </h2>
          <div className="set-row">
            <div className="set-text">
              <span className="set-title">Log file</span>
              <span className="set-note">
                Iwaks records playback, cover-art and presence events in a daily
                log, and writes down the message and location if it ever panics.
                Attach this file to a bug report.
              </span>
            </div>
            <code className="set-log-path">{logFile ?? "Not available"}</code>
          </div>
        </section>

        <section className="set-group" aria-labelledby="set-keys">
          <h2 className="set-group-title" id="set-keys">
            <Keyboard size={18} aria-hidden="true" />
            Keyboard shortcuts
          </h2>
          <dl className="set-keys">
            {SHORTCUTS.map((s) => (
              <div className="set-key-row" key={s.keys}>
                <dt>
                  {s.keys.split(" / ").map((k) => (
                    <kbd key={k}>{k}</kbd>
                  ))}
                </dt>
                <dd>{s.what}</dd>
              </div>
            ))}
          </dl>
        </section>
      </div>
    </>
  );
}