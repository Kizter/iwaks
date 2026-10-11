import { useEffect, useState } from "react";
import { DiscordLogo, Keyboard } from "@phosphor-icons/react";
import { getOnlineCover, setOnlineCover } from "./api";
import { friendlyError } from "./format";

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