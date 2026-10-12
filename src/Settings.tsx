import { useEffect, useRef, useState } from "react";
import { DiscordLogo, Keyboard, NotePencil, PaintBrushBroad, SpeakerHigh } from "@phosphor-icons/react";
import {
  getAudioDevice,
  getAudioExclusive,
  getLogPath,
  getOnlineCover,
  getTheme,
  listAudioDevices,
  setAudioDevice,
  setAudioExclusive,
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
  // Audio output (issues #23, #24). `devices` is fetched when the picker opens
  // so it can never be a stale hot-plug snapshot.
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [devicesLoading, setDevicesLoading] = useState(false);
  const [device, setDevice] = useState<string | null>(null);
  const [exclusive, setExclusive] = useState<boolean | null>(null);
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
      .catch(() => {});
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

  // Enumerated on demand: mpv has no change event we consume, and opening the
  // picker is the moment a stale list would actually be noticed.
  const loadDevices = () => {
    setDevicesLoading(true);
    listAudioDevices()
      .then((list) => setDevices(list))
      .catch((e: unknown) => setError(`Couldn't list audio devices — ${friendlyError(e)}`))
      .finally(() => setDevicesLoading(false));
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
            <select
              className="set-select"
              aria-labelledby="set-device"
              value={device ?? ""}
              disabled={devicesLoading || device === null}
              onFocus={loadDevices}
              onClick={loadDevices}
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
                of the device, which silences other apps for as long as Iwaks
                plays — off is the default, and the switch takes effect the next
                time Iwaks starts.
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