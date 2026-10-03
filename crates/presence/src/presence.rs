//! Presence facade — ties what's playing to the RPC client.
//!
//! [`start`] spawns a single driver thread that owns a [`Client`] state
//! machine and a [`Transport`] (the Windows named pipe). It consumes
//! [`NowPlaying`] events from a channel, dedupes by track *content* (position
//! ticks do not re-send), and reconnects with backoff — all behaviours that
//! are unit-tested at the [`Client`] level and end-to-end below with a
//! scripted transport.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::client::{Client, Effect, Event, Phase, Transport};
use crate::pipe::NamedPipe;
use crate::protocol::{Activity, Timestamps, FIELD_LIMIT, OP_CLOSE};

/// What the user is listening to right now (player-agnostic — the Tauri
/// layer converts its own state into this).
#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Position in the track, in seconds (used to anchor the elapsed timer).
    pub position_secs: f64,
    pub playing: bool,
}

/// App name shown next to the track title.
const APP_NAME: &str = "Iwaks";

/// Appended to the track title so the member-list line reads
/// "<judul> - Iwaks".
const TITLE_SUFFIX: &str = " - Iwaks";

/// Portal asset key used when `IWAKS_DISCORD_ASSET_KEY` is unset.
const DEFAULT_ASSET_KEY: &str = "logo";

/// Image asset keys, resolved by Discord against the Rich Presence assets
/// uploaded in the Developer Portal.
///
/// Only uploaded keys render. Verified against the live client: `file://`
/// values are rejected outright, and `http(s)://` values (loopback *and*
/// public) are accepted by the RPC server but never displayed — Discord
/// proxies them server-side and drops them from the activity. So a per-track
/// album cover cannot be shipped from a local file; only portal assets show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetKeys {
    /// Card image (Iwaks cover). `None` sends no large image at all.
    pub large: Option<String>,
    /// Small overlay image, drawn on top of the large one.
    pub small: Option<String>,
}

impl AssetKeys {
    /// Read the keys from the environment. Overrides exist because the portal
    /// is the only place assets can be registered: renaming an asset there
    /// must not require a rebuild.
    pub fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self {
            large: var("IWAKS_DISCORD_ASSET_KEY").or_else(|| Some(DEFAULT_ASSET_KEY.to_string())),
            small: var("IWAKS_DISCORD_SMALL_ASSET_KEY"),
        }
    }
}

impl Default for AssetKeys {
    fn default() -> Self {
        Self {
            large: Some(DEFAULT_ASSET_KEY.to_string()),
            small: None,
        }
    }
}

/// We poll the pipe for frames roughly this often while idle.
const MAX_IDLE: Duration = Duration::from_millis(50);

/// Spawn the presence driver. The thread keeps running until the sender of
/// `rx` is dropped (app shutdown); a dropped [`JoinHandle`] detaches it.
pub fn start(app_id: &str, rx: Receiver<Option<NowPlaying>>) -> JoinHandle<()> {
    let app_id = app_id.to_string();
    std::thread::spawn(move || {
        run_with_transport(
            app_id,
            rx,
            Box::new(NamedPipe::new()),
            AssetKeys::from_env(),
        )
    })
}

/// The driver loop; also the entry point for integration tests with a
/// scripted [`Transport`].
fn run_with_transport(
    app_id: String,
    rx: Receiver<Option<NowPlaying>>,
    mut transport: Box<dyn Transport>,
    assets: AssetKeys,
) {
    if cfg!(debug_assertions) {
        eprintln!("[debug] discord presence: app id = {app_id}");
    }
    let pid = std::process::id();
    let mut client = Client::new(&app_id, pid);
    // Try to connect right away, before the user even plays anything, so the
    // presence registers as soon as Discord is online.
    let mut effects: VecDeque<Effect> = client.on_event(Event::Timer).into();
    let mut wait_until: Option<Instant> = None;
    let mut last_phase = client.phase();
    // Content identity of the activity last sent — position ticks are
    // ignored so a playing track does not re-send 10×/s.
    let mut last_key: Option<String> = None;

    loop {
        // (1) Carry out effects; they may schedule more (connect → handshake
        // write, READY → SET_ACTIVITY, …).
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::OpenPipe => {
                    let event = match transport.connect() {
                        Ok(()) => Event::Opened,
                        Err(cause) => {
                            if cfg!(debug_assertions) {
                                eprintln!(
                                    "[debug] discord presence: Discord not reachable — {cause}"
                                );
                            }
                            Event::ConnectFailed
                        }
                    };
                    effects.extend(client.on_event(event));
                }
                Effect::Write(bytes) => {
                    if let Err(cause) = transport.write(&bytes) {
                        if cfg!(debug_assertions) {
                            eprintln!("[debug] discord presence: write failed — {cause}");
                        }
                        // Broken pipe — let the client schedule a reconnect.
                        effects.extend(client.on_event(Event::Disconnected));
                    }
                }
                Effect::ClosePipe => transport.close(),
                Effect::Wait(delay) => wait_until = Some(Instant::now() + delay),
            }
        }

        // (2) Poll the pipe for frames (never blocks).
        if transport.is_open() {
            match transport.try_read_frame() {
                Ok(Some(frame)) => {
                    if cfg!(debug_assertions) && frame.opcode == OP_CLOSE {
                        eprintln!(
                            "[debug] discord presence: Discord closed the connection: {}",
                            frame.payload
                        );
                    }
                    effects.extend(client.on_event(Event::Frame(frame)));
                }
                Ok(None) => {}
                Err(cause) => {
                    if cfg!(debug_assertions) {
                        eprintln!("[debug] discord presence: read error — {cause}");
                    }
                    effects.extend(client.on_event(Event::Disconnected));
                }
            }
        }

        // (3) Reconnect timer matured?
        if let Some(until) = wait_until {
            if Instant::now() >= until {
                wait_until = None;
                effects.extend(client.on_event(Event::Timer));
            }
        }

        // (4) Wait for the next user event (this doubles as the poll sleep).
        let idle = match wait_until {
            Some(until) => until
                .saturating_duration_since(Instant::now())
                .min(MAX_IDLE),
            None => MAX_IDLE,
        };
        match rx.recv_timeout(idle) {
            Ok(None) => {
                if last_key.is_some() {
                    effects.extend(client.set_activity(None));
                    last_key = None;
                }
            }
            Ok(Some(np)) => {
                let key = content_key(&np);
                if last_key.as_deref() != Some(key.as_str()) {
                    let activity = activity_for(&np, unix_now(), &assets);
                    effects.extend(client.set_activity(Some(activity)));
                    last_key = Some(key);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return, // app shutting down
        }

        // (5) Diagnostics: log connection-phase transitions once each.
        if cfg!(debug_assertions) && client.phase() != last_phase {
            log_phase_change(last_phase, client.phase());
            last_phase = client.phase();
        }
    }
}

/// Debug-only log of a connection-phase transition (helps diagnose why the
/// presence does not appear: unreachable pipe, rejected handshake, drop).
fn log_phase_change(before: Phase, after: Phase) {
    let message = match (before, after) {
        (Phase::Handshaking, Phase::Ready) => "connected to Discord RPC (READY)".to_string(),
        (Phase::Handshaking, Phase::Disconnected) => {
            "handshake failed — Discord closed the connection (check the Application ID)"
                .to_string()
        }
        (Phase::Ready, Phase::Disconnected) => "RPC connection lost — reconnecting".to_string(),
        _ => return,
    };
    eprintln!("[debug] discord presence: {message}");
}

/// Content identity: position and timestamps are deliberately excluded so
/// steady playback does not resend the activity for every state tick.
fn content_key(np: &NowPlaying) -> String {
    format!(
        "{}|{}|{}|{}",
        np.title,
        np.artist.as_deref().unwrap_or(""),
        np.album.as_deref().unwrap_or(""),
        np.playing
    )
}

/// Build the activity for what is playing. The elapsed timer (Discord renders
/// it automatically from `start`) is anchored at `now − position`.
///
/// Field placement was determined by probing the live client, not by guessing:
/// Discord shows `name` next to the user in the member list ("Listening to
/// &lt;title&gt; - Iwaks") and `details` / `state` only on the profile card. So
/// the track title goes in `name` and the artist/album line in `details`.
fn activity_for(np: &NowPlaying, now: u64, assets: &AssetKeys) -> Activity {
    // Reserve room for the suffix first: clamping the finished string would
    // cut the app name off whenever the title is long.
    let room = FIELD_LIMIT - TITLE_SUFFIX.chars().count();
    let title: String = np.title.chars().take(room).collect();
    let mut activity = Activity::listening().name(&format!("{title}{TITLE_SUFFIX}"));
    match (&np.artist, &np.album) {
        (Some(artist), Some(album)) => {
            activity = activity.details(&format!("{artist} — {album}"));
        }
        (Some(artist), None) => activity = activity.details(artist),
        (None, Some(album)) => activity = activity.details(album),
        (None, None) => {}
    }
    if let Some(key) = &assets.large {
        activity = activity.large_image(key).large_text(APP_NAME);
    }
    if let Some(key) = &assets.small {
        activity = activity.small_image(key);
    }
    if np.playing {
        let start = now.saturating_sub(np.position_secs.max(0.0) as u64);
        activity = activity.timestamps(Timestamps {
            start: Some(start),
            end: None,
        });
    }
    activity
}

/// Current Unix seconds.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{decode_frame, encode_frame, OP_FRAME, OP_HANDSHAKE};
    use std::sync::mpsc;

    fn np(title: &str, position: f64, playing: bool) -> NowPlaying {
        NowPlaying {
            title: title.to_string(),
            artist: Some("Artis".to_string()),
            album: Some("Album".to_string()),
            position_secs: position,
            playing,
        }
    }

    #[test]
    fn content_key_ignores_position() {
        assert_eq!(
            content_key(&np("Lagu", 10.0, true)),
            content_key(&np("Lagu", 99.0, true))
        );
        assert_ne!(
            content_key(&np("Lagu", 10.0, true)),
            content_key(&np("Lagu", 99.0, false))
        );
        assert_ne!(
            content_key(&np("Lagu", 10.0, true)),
            content_key(&np("Lain", 10.0, true))
        );
    }

    #[test]
    fn activity_for_playing_is_listening_with_timestamps() {
        let a = activity_for(&np("Lagu A", 12.0, true), 1_000_000, &AssetKeys::default());
        let payload = crate::protocol::set_activity_json(1, "x", Some(&a));
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let act = &v["args"]["activity"];
        assert_eq!(act["type"], 2);
        assert_eq!(act["name"], "Lagu A - Iwaks");
        assert_eq!(act["details"], "Artis — Album");
        assert_eq!(act["assets"]["large_image"], "logo");
        assert_eq!(act["timestamps"]["start"], 1_000_000 - 12);
    }

    /// Discord's member list renders `name` (probed against the live client),
    /// so the track title must never end up only in `details`.
    #[test]
    fn name_carries_the_track_title_and_the_app() {
        let a = activity_for(&np("Surabaya", 3.0, true), 0, &AssetKeys::default());
        assert_eq!(a.name.as_deref(), Some("Surabaya - Iwaks"));
        assert_eq!(a.details.as_deref(), Some("Artis — Album"));
        assert_eq!(a.state, None);
    }

    /// Titles longer than Discord's 128-char field limit are clamped, and the
    /// clamp must not eat the app suffix that identifies the player.
    #[test]
    fn long_titles_are_clamped_but_keep_the_app_suffix() {
        let long = "x".repeat(200);
        let a = activity_for(&np(&long, 0.0, true), 0, &AssetKeys::default());
        let name = a.name.unwrap();
        assert_eq!(name.chars().count(), 128);
        assert!(
            name.ends_with(TITLE_SUFFIX),
            "clamped name should still name the app"
        );
    }

    #[test]
    fn asset_keys_choose_which_images_are_sent() {
        let keys = AssetKeys {
            large: Some("cover".to_string()),
            small: Some("note".to_string()),
        };
        let a = activity_for(&np("Lagu", 0.0, true), 0, &keys);
        let payload = crate::protocol::set_activity_json(1, "x", Some(&a));
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["args"]["activity"]["assets"]["large_image"], "cover");
        assert_eq!(v["args"]["activity"]["assets"]["small_image"], "note");
    }

    /// Discord drops unresolved keys, so a misconfigured key must produce no
    /// assets at all rather than a broken image reference.
    #[test]
    fn missing_asset_keys_send_no_images() {
        let a = activity_for(
            &np("Lagu", 0.0, true),
            0,
            &AssetKeys {
                large: None,
                small: None,
            },
        );
        let payload = crate::protocol::set_activity_json(1, "x", Some(&a));
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v["args"]["activity"].get("assets").is_none());
    }

    #[test]
    fn activity_for_paused_has_no_timestamps() {
        let a = activity_for(&np("Lagu A", 12.0, false), 1_000_000, &AssetKeys::default());
        let payload = crate::protocol::set_activity_json(1, "x", Some(&a));
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v["args"]["activity"].get("timestamps").is_none());
    }

    #[test]
    fn details_falls_back_to_artist_only() {
        let mut n = np("Lagu", 0.0, true);
        n.album = None;
        let a = activity_for(&n, 0, &AssetKeys::default());
        let payload = crate::protocol::set_activity_json(1, "x", Some(&a));
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["args"]["activity"]["details"], "Artis");
    }

    /// A scripted transport: writes go to a channel the test asserts on,
    /// frames arrive from a channel the test feeds (last one wins per poll).
    struct ScriptedTransport {
        writes: mpsc::Sender<Vec<u8>>,
        script: mpsc::Receiver<Script>,
        open: bool,
    }

    enum Script {
        Frame(Vec<u8>),
    }

    impl Transport for ScriptedTransport {
        fn connect(&mut self) -> Result<(), String> {
            self.open = true;
            Ok(())
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            let _ = self.writes.send(bytes.to_vec());
            Ok(())
        }
        fn try_read_frame(&mut self) -> Result<Option<crate::protocol::Frame>, String> {
            match self.script.try_recv() {
                Ok(Script::Frame(bytes)) => {
                    decode_frame(&bytes).map(Some).map_err(|e| e.to_string())
                }
                Err(_) => Ok(None),
            }
        }
        fn close(&mut self) {
            self.open = false;
        }
        fn is_open(&self) -> bool {
            self.open
        }
    }

    #[test]
    fn e2e_pushes_activity_dedupes_and_clears() {
        let (tx_user, rx_user) = mpsc::channel::<Option<NowPlaying>>();
        let (tx_write, rx_write) = mpsc::channel::<Vec<u8>>();
        let (tx_script, rx_script) = mpsc::channel::<Script>();

        let handle = std::thread::spawn(move || {
            run_with_transport(
                "1553".to_string(),
                rx_user,
                Box::new(ScriptedTransport {
                    writes: tx_write,
                    script: rx_script,
                    open: false,
                }),
                AssetKeys::default(),
            )
        });

        // 1) Driver connects and sends the handshake on its own.
        let handshake = rx_write
            .recv_timeout(Duration::from_secs(3))
            .expect("handshake frame");
        let f = decode_frame(&handshake).unwrap();
        assert_eq!(f.opcode, OP_HANDSHAKE);

        // 2) Discord answers READY.
        tx_script
            .send(Script::Frame(encode_frame(OP_FRAME, r#"{"cmd":"READY"}"#)))
            .unwrap();

        // 3) Play a track → SET_ACTIVITY with details + start timestamp.
        tx_user.send(Some(np("Lagu A", 12.0, true))).unwrap();
        let activity = rx_write
            .recv_timeout(Duration::from_secs(3))
            .expect("SET_ACTIVITY frame");
        let f = decode_frame(&activity).unwrap();
        let v: serde_json::Value = serde_json::from_str(&f.payload).unwrap();
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        assert_eq!(v["args"]["activity"]["name"], "Lagu A - Iwaks");
        assert!(v["args"]["activity"]["timestamps"]["start"].is_u64());

        // 4) Position tick with SAME content → no re-send.
        tx_user.send(Some(np("Lagu A", 42.0, true))).unwrap();
        assert!(
            rx_write.recv_timeout(Duration::from_millis(400)).is_err(),
            "position ticks must not re-send the activity"
        );

        // 5) Pause (same track) → clear timestamps via a new activity.
        tx_user.send(Some(np("Lagu A", 42.0, false))).unwrap();
        let paused = rx_write
            .recv_timeout(Duration::from_secs(3))
            .expect("paused activity");
        let f = decode_frame(&paused).unwrap();
        let v: serde_json::Value = serde_json::from_str(&f.payload).unwrap();
        assert!(v["args"]["activity"].get("timestamps").is_none());

        // 6) Stop → clear activity.
        tx_user.send(None).unwrap();
        let clear = rx_write
            .recv_timeout(Duration::from_secs(3))
            .expect("clear activity");
        let f = decode_frame(&clear).unwrap();
        assert!(f.payload.contains("\"activity\":{}"));

        // 7) Shutdown: dropping the sender ends the loop cleanly.
        drop(tx_user);
        handle.join().expect("driver exits on channel close");
    }
}
