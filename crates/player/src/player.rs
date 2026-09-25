//! libmpv-backed player: queue, commands and a background event pump.
//!
//! # Model
//! * **Single-owner mpv access, from birth.** Libmpv 0.39+ runs the playback
//!   core on the thread that created and initialized the `mpv_handle`;
//!   synchronous calls (`command`, property reads) submitted from other
//!   threads are only serviced while that owner thread is inside the client
//!   API — and a cross-thread `mpv_wakeup` racing a synchronous call can
//!   deadlock that call. So the pump thread does *everything*: create →
//!   options → initialize → `wait_event` → commands, and **no other thread
//!   ever touches the handle**. Other threads push a [`Cmd`]; the pump
//!   picks it up at its next loop (≤ the 100 ms `wait_event` timeout).
//! * Queue position + repeat live in [`Queue`] (pure logic, see `queue.rs`).
//!   Values are never mutated; builders return new objects.
//! * Every pump wake (~4 Hz + on each command/event) recomputes the full
//!   [`PlayerState`] and stores it in a shared slot, then pushes it to the
//!   [`StateSink`]. `snapshot()` just clones that slot — no mpv access.
//! * `END_FILE` events drive auto-advance / stop; a file mpv can't demux
//!   surfaces as `EndFileReason::Redirect` and is skipped like an error.
//!
//! # Shutdown
//! `shutdown()` is deterministic: set `cancelled`; the pump's 100 ms
//! `wait_event` timeout lets it notice and exit, then join. libmpv is
//! released from `Mpv::drop` only after the pump exited.
//!
//! Tests run headless (`ao=null`); when no libmpv DLL is present they skip.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use iwaks_core::track::Track;
use serde::Serialize;

use crate::ffi::{EndFileReason, Mpv, MpvEvent};
use crate::queue::{Queue, RepeatMode};

/// Seconds of core runtime the pump grants libmpv per `wait_event` call.
/// Too-short slices starve the playback core (position then advances far
/// slower than real time); this value balances that against command latency.
const WAIT_SLICE: f64 = 0.1;
/// Minimum interval between state-refresh ticks (the pump's ~10 Hz clock).
const TICK_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Playback behaviour knobs applied at startup.
#[derive(Debug, Clone)]
pub struct Options {
    /// Null audio output — headless (tests/CI, no device required).
    pub ao_null: bool,
    /// WASAPI exclusive mode (bit-perfect); only meaningful with a real AO.
    pub audio_exclusive: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ao_null: false,
            audio_exclusive: true,
        }
    }
}

/// Full playback state pushed to the frontend on `player-state`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub current: Option<Track>,
    pub index: Option<usize>,
    pub list_len: usize,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    /// True once the list ended naturally (repeat off) and not restarted.
    pub stopped: bool,
    pub volume: i64,
    pub mute: bool,
    pub repeat: RepeatMode,
}

/// Receives a fresh [`PlayerState`] after every change and each pump tick.
pub type StateSink = Box<dyn Fn(&PlayerState) + Send>;

/// Work items for the pump (the only thread that may call the mpv API).
enum Cmd {
    Load(usize),
    TogglePlay,
    Next,
    Prev,
    Seek(f64),
    SeekRelative(f64),
    SetVolume(i64),
    ToggleMute,
    Stop,
    Refresh,
}

pub struct Player {
    /// Owned by the pump thread exclusively: mpv is created, initialized and
    /// driven only on the pump thread. Stored behind a mutex solely for Arc
    /// lifetime sharing with `shutdown`-adjacent cleanup — no other thread
    /// ever issues an mpv call, so no cross-thread wakeup is needed.
    api: Mutex<Option<Arc<Mpv>>>,
    /// Last state produced by the pump; read by any thread without mpv calls.
    state: Mutex<PlayerState>,
    tracks: Mutex<Vec<Track>>,
    queue: Mutex<Queue>,
    sink: Mutex<Option<StateSink>>,
    commands: Mutex<VecDeque<Cmd>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    stopped: AtomicBool,
    shut_down: AtomicBool,
    cancelled: AtomicBool,
}

impl Player {
    /// Spawn the pump thread, which owns the whole libmpv lifecycle, and wait
    /// until mpv is initialized (or initialization failed).
    pub fn start(options: &Options, sink: StateSink) -> Result<Arc<Self>, String> {
        let player = Arc::new(Self {
            api: Mutex::new(None),
            state: Mutex::new(PlayerState {
                current: None,
                index: None,
                list_len: 0,
                position: 0.0,
                duration: 0.0,
                paused: true,
                stopped: false,
                volume: 80,
                mute: false,
                repeat: RepeatMode::Off,
            }),
            tracks: Mutex::new(Vec::new()),
            queue: Mutex::new(Queue::new(0)),
            sink: Mutex::new(Some(sink)),
            commands: Mutex::new(VecDeque::new()),
            thread: Mutex::new(None),
            stopped: AtomicBool::new(false),
            shut_down: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        });

        let options = options.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let pump_player = Arc::clone(&player);
        let handle = std::thread::spawn(move || {
            let setup = Self::setup_mpv(&options);
            match setup {
                Ok(api) => {
                    *pump_player.api.lock().unwrap() = Some(Arc::clone(&api));
                    let _ = ready_tx.send(Ok(()));
                    Self::pump(&pump_player, api);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            }
        });
        *player.thread.lock().unwrap() = Some(handle);

        match ready_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(player),
            Ok(Err(err)) => Err(err),
            Err(_) => {
                player.cancelled.store(true, Ordering::SeqCst);
                Err("player init timed out".into())
            }
        }
    }

    /// Run on the pump thread: create mpv, apply options, initialize.
    fn setup_mpv(options: &Options) -> Result<Arc<Mpv>, String> {
        let api = Mpv::new()?;
        let apply = |key: &str, value: &str| -> Result<(), String> {
            api.set_option(key, value)
                .map_err(|code| format!("option {key}={value}: {code}"))
        };
        apply("idle", "yes")?; // stay alive between files
        apply("vo", "null")?; // headless app: no window
        apply("ao", if options.ao_null { "null" } else { "wasapi" })?;
        if options.ao_null {
            // Real-time pacing so position/seek assertions are deterministic.
            apply("ao-null-untimed", "no")?;
        }
        if options.audio_exclusive {
            apply("audio-exclusive", "yes")?;
        }
        apply("audio-client-name", "Iwaks")?;
        apply("volume-max", "100")?;
        apply("keep-open", "no")?;
        apply("gapless-audio", "yes")?;
        let _ = api.set_option("audio-resampler", "soxr"); // optional build; harmless
        if let Ok(level) = std::env::var("IWAKS_MPV_LOG") {
            let _ = api.request_log_messages(&level); // debug hook
        }
        api.initialize()
            .map_err(|code| format!("mpv_initialize failed: {code}"))?;
        Ok(Arc::new(api))
    }

    /// Replace the queue and start playing `tracks[index]`.
    pub fn play_tracks(&self, tracks: Vec<Track>, index: usize) {
        if tracks.is_empty() {
            return;
        }
        let index = index.min(tracks.len() - 1);
        let repeat = self.queue.lock().unwrap().repeat;
        let len = tracks.len();
        *self.tracks.lock().unwrap() = tracks;
        *self.queue.lock().unwrap() = Queue::new(len).with_repeat(repeat).start(index);
        self.push(Cmd::Load(index));
    }

    pub fn toggle_play(&self) {
        self.push(Cmd::TogglePlay);
    }

    pub fn next(&self) {
        self.push(Cmd::Next);
    }

    pub fn prev(&self) {
        self.push(Cmd::Prev);
    }

    /// Absolute seek, in seconds.
    pub fn seek(&self, seconds: f64) {
        if seconds.is_finite() && seconds >= 0.0 {
            self.push(Cmd::Seek(seconds));
        }
    }

    /// Relative seek, in seconds (negative allowed).
    pub fn seek_relative(&self, delta: f64) {
        if delta.is_finite() {
            self.push(Cmd::SeekRelative(delta));
        }
    }

    pub fn set_volume(&self, volume: i64) {
        self.push(Cmd::SetVolume(volume.clamp(0, 100)));
    }

    pub fn toggle_mute(&self) {
        self.push(Cmd::ToggleMute);
    }

    pub fn set_repeat(&self, repeat: RepeatMode) {
        let next = self.queue.lock().unwrap().with_repeat(repeat);
        *self.queue.lock().unwrap() = next;
        self.push(Cmd::Refresh);
    }

    /// Stop playback and mark the queue as ended.
    pub fn stop(&self) {
        self.push(Cmd::Stop);
    }

    /// Latest state produced by the pump (never touches libmpv).
    pub fn snapshot(&self) -> PlayerState {
        self.state.lock().unwrap().clone()
    }

    /// Stop the pump and release libmpv. Idempotent and deterministic.
    pub fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().unwrap().take() {
            // The pump's wait_event times out ≤100 ms, so join returns fast.
            let _ = handle.join();
        }
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Queue a command for the pump. Never touches the mpv handle: a
    /// cross-thread call (even `mpv_wakeup`) racing a synchronous mpv
    /// command can deadlock that command, so the pump learns about new
    /// work at the top of its next loop (≤ the 100 ms wait_event timeout).
    fn push(&self, cmd: Cmd) {
        let mut queue = self.commands.lock().unwrap();
        if self.cancelled.load(Ordering::SeqCst) {
            return; // shutting down — ignore new work
        }
        queue.push_back(cmd);
    }

    // ---- pump side (only the pump thread runs these) ----

    /// Clone the libmpv handle, releasing the lock. Synchronous mpv calls
    /// must run without holding the mutex: a pump thread blocked inside a
    /// call must not stall `push`/`snapshot`/`shutdown` on other threads.
    fn api(&self) -> Arc<Mpv> {
        self.api
            .lock()
            .unwrap()
            .as_ref()
            .expect("player not started")
            .clone()
    }

    fn run_cmd(&self, cmd: Cmd) {
        match cmd {
            Cmd::Load(index) => self.play_index(index),
            Cmd::TogglePlay => {
                if self.stopped.load(Ordering::SeqCst) {
                    // Restart the last selected track from the beginning.
                    // Bind the guard first: `if let` on `lock()...` keeps the
                    // guard alive across the body, so calling play_index
                    // (which locks the queue again) would self-deadlock.
                    let current = self.queue.lock().unwrap().current();
                    if let Some(i) = current {
                        self.stopped.store(false, Ordering::SeqCst);
                        self.play_index(i);
                    }
                    return;
                }
                let api = self.api();
                let paused = api.get_flag("pause").unwrap_or(false);
                let _ = api.command(&["set", "pause", if paused { "no" } else { "yes" }]);
                self.emit();
            }
            Cmd::Next => {
                // Bind the lock result: the guard from a `match lock()...`
                // scrutinee stays alive for the whole match, which would
                // self-deadlock when play_index re-locks the queue below.
                let next = self.queue.lock().unwrap().user_next();
                match next {
                    Some(i) => self.play_index(i),
                    None => self.emit(),
                }
            }
            Cmd::Prev => {
                let prev = self.queue.lock().unwrap().user_prev();
                match prev {
                    Some(i) => self.play_index(i),
                    None => self.emit(),
                }
            }
            Cmd::Seek(seconds) => {
                let api = self.api();
                let _ = api.command(&["seek", &seconds.to_string(), "absolute"]);
                self.emit();
            }
            Cmd::SeekRelative(delta) => {
                let api = self.api();
                let _ = api.command(&["seek", &delta.to_string(), "relative"]);
                self.emit();
            }
            Cmd::SetVolume(volume) => {
                let api = self.api();
                let _ = api.command(&["set", "volume", &volume.to_string()]);
                self.emit();
            }
            Cmd::ToggleMute => {
                let api = self.api();
                let muted = api.get_flag("mute").unwrap_or(false);
                let _ = api.command(&["set", "mute", if muted { "no" } else { "yes" }]);
                self.emit();
            }
            Cmd::Stop => {
                self.stopped.store(true, Ordering::SeqCst);
                let _ = self.api().command(&["stop"]);
                self.emit();
            }
            Cmd::Refresh => self.emit(),
        }
    }

    fn play_index(&self, index: usize) {
        let path = {
            let mut q = self.queue.lock().unwrap();
            *q = q.start(index);
            let tracks = self.tracks.lock().unwrap();
            tracks.get(index).map(|t| t.path.clone())
        };
        self.stopped.store(false, Ordering::SeqCst);
        if let Some(path) = path {
            let _ = self.api().command(&["loadfile", path.as_str()]);
        }
        self.emit();
    }

    fn pump(player: &Arc<Player>, api: Arc<Mpv>) {
        let mut last_tick = std::time::Instant::now();
        loop {
            if player.cancelled.load(Ordering::SeqCst) {
                break;
            }
            // Drain commands (lock released before running, so `push` never
            // waits on a command that is executing). A queued command waits
            // at most one `WAIT_SLICE` (100 ms) below.
            loop {
                let cmd = player.commands.lock().unwrap().pop_front();
                match cmd {
                    Some(cmd) => {
                        player.run_cmd(cmd);
                        if player.cancelled.load(Ordering::SeqCst) {
                            return;
                        }
                    }
                    None => break,
                }
            }
            // Grant libmpv's playback core a short slice of runtime.
            // Blocking long in one call (e.g. 250 ms) made libmpv 0.41 hang
            // when the next `loadfile` followed an END_FILE; short slices
            // also keep command latency low. No other thread ever calls
            // into mpv (`mpv_wakeup` included) — a cross-thread wakeup
            // racing a synchronous command deadlocks it.
            let event = api.wait_event(WAIT_SLICE);
            if player.cancelled.load(Ordering::SeqCst) {
                break;
            }
            match event {
                Some(MpvEvent::LogMessage(text)) => {
                    // Only produced when `IWAKS_MPV_LOG` is set at startup.
                    eprintln!("[mpv] {text}");
                }
                Some(MpvEvent::EndFile(reason)) => player.on_end_file(reason),
                Some(MpvEvent::FileLoaded) => player.emit(),
                // EVENT_NONE = timeout: refresh state, but at most ~10 Hz.
                None if last_tick.elapsed() >= TICK_EVERY => {
                    player.emit();
                    last_tick = std::time::Instant::now();
                }
                _ => {}
            }
        }
    }

    fn on_end_file(&self, reason: EndFileReason) {
        // `Redirect` = the file could not be demuxed (mpv reports unreadable
        // input that way) — treat it like an error so the queue moves on.
        // `Stop` (manual stop/next) is deliberately ignored: no auto-advance.
        //
        // This handler never touches libmpv: right after an END_FILE the
        // core is unwinding (to the next file or into idle) on this same
        // thread, and a synchronous property read at that moment can
        // self-deadlock it. The advance path queues a `Cmd::Load` for the
        // pump's command drain; the stop path only updates the stored state.
        match reason {
            EndFileReason::Eof | EndFileReason::Error | EndFileReason::Redirect => {
                match self.queue.lock().unwrap().after_end() {
                    Some(i) => self.advance_to(i),
                    None => {
                        self.stopped.store(true, Ordering::SeqCst);
                        let mut s = self.state.lock().unwrap();
                        s.stopped = true;
                        s.paused = true;
                        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
                            sink(&s);
                        }
                    }
                }
            }
            // Manual `Stop` was already emitted from `run_cmd(Cmd::Stop)`.
            _ => {}
        }
    }

    /// Queue the next track; the pump loads it once back in its command
    /// drain (the only place safe for synchronous mpv calls).
    fn advance_to(&self, index: usize) {
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        self.commands.lock().unwrap().push_back(Cmd::Load(index));
    }

    fn emit(&self) {
        let state = self.read_state();
        *self.state.lock().unwrap() = state.clone();
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink(&state);
        }
    }

    /// Recompute the full state from libmpv properties (pump thread only).
    fn read_state(&self) -> PlayerState {
        let api = self.api();
        let position = api.get_double("playback-time").unwrap_or(0.0).max(0.0);
        let duration = api.get_double("duration").unwrap_or(0.0).max(0.0);
        let paused = api.get_flag("pause").unwrap_or(true);
        let volume = api.get_i64("volume").unwrap_or(80).clamp(0, 100);
        let mute = api.get_flag("mute").unwrap_or(false);
        let (queue, tracks) = {
            // Same order as `play_index` (queue → tracks) to avoid ABBA.
            let q = self.queue.lock().unwrap();
            let t = self.tracks.lock().unwrap();
            (q.clone(), t.clone())
        };
        let current = queue.current().and_then(|i| tracks.get(i).cloned());
        PlayerState {
            current,
            index: queue.current(),
            list_len: tracks.len(),
            position,
            duration,
            paused,
            stopped: self.stopped.load(Ordering::SeqCst),
            volume,
            mute,
            repeat: queue.repeat,
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn have_libmpv() -> bool {
        if Mpv::load_library().is_ok() {
            return true;
        }
        eprintln!("SKIP: no libmpv DLL on this machine");
        false
    }

    fn write_wav(path: &Path, seconds: f64) {
        const RATE: u32 = 44_100;
        let samples = (RATE as f64 * seconds) as u32;
        let data_len = samples * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, data_len as usize));
        std::fs::write(path, wav).unwrap();
    }

    fn track(path: &str, title: &str, seconds: f64) -> Track {
        Track {
            id: 0,
            path: path.to_string(),
            title: title.to_string(),
            artist: None,
            album: None,
            album_artist: None,
            genre: None,
            year: None,
            track_no: None,
            disc_no: None,
            duration_ms: (seconds * 1000.0) as i64,
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            bitrate: Some(1_411),
            format: "wav".into(),
            file_size: 0,
            modified_at: 0,
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iwaks-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for(
        player: &Player,
        timeout: Duration,
        mut pred: impl FnMut(&PlayerState) -> bool,
    ) -> Option<PlayerState> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            let s = player.snapshot();
            if pred(&s) {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        None
    }

    fn start_player() -> Arc<Player> {
        let sink: StateSink = Box::new(|_| {});
        Player::start(
            &Options {
                ao_null: true,
                audio_exclusive: false,
            },
            sink,
        )
        .unwrap()
    }

    #[test]
    fn headless_playback_roundtrip() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("roundtrip");
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        let c = dir.join("c.wav");
        // Tracks must be long enough (>= 1s) for `playback-time` to advance
        // under the headless `ao=null` config: a fully-buffered sub-second
        // file keeps time-pos pinned at its start value.
        for p in [&a, &b, &c] {
            write_wav(p, 2.0);
        }
        let tracks = vec![
            track(a.to_str().unwrap(), "A", 2.0),
            track(b.to_str().unwrap(), "B", 2.0),
            track(c.to_str().unwrap(), "C", 2.0),
        ];

        let player = start_player();
        player.play_tracks(tracks, 0);

        let s = wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
                && !s.paused
                && s.duration > 0.2
        })
        .expect("starts playing A");
        assert_eq!(s.index, Some(0));
        assert!(s.duration > 0.2, "duration decoded from the file");

        player.toggle_play();
        wait_for(&player, Duration::from_secs(2), |s| s.paused).expect("pauses");
        player.toggle_play();
        wait_for(&player, Duration::from_secs(2), |s| !s.paused).expect("resumes");

        // Seek ahead on the 2s track: position 0.9+ is unreachable by natural
        // playback at this point (~0.3s in), so it can only come from the seek.
        player.seek(1.0);
        wait_for(&player, Duration::from_secs(2), |s| s.position >= 0.9).expect("seek moves");
        player.set_volume(42);
        wait_for(&player, Duration::from_secs(2), |s| s.volume == 42).expect("volume applied");

        player.set_repeat(RepeatMode::All);
        player.next();
        wait_for(&player, Duration::from_secs(5), |s| s.index == Some(1)).expect("next -> B");

        // A -> B -> C -> wrap to A (repeat all), without stopping.
        let s = wait_for(&player, Duration::from_secs(12), |s| {
            s.index == Some(0) && s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("auto-advances and wraps A->B->C->A");
        assert!(!s.stopped);

        player.shutdown();
    }

    #[test]
    fn stops_at_end_of_short_list() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("stop");
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        write_wav(&a, 0.3);
        write_wav(&b, 0.3);
        let tracks = vec![
            track(a.to_str().unwrap(), "A", 0.3),
            track(b.to_str().unwrap(), "B", 0.3),
        ];

        let player = start_player();
        player.play_tracks(tracks, 1); // start on the LAST track, repeat off

        let s = wait_for(&player, Duration::from_secs(6), |s| s.stopped)
            .expect("reaches end and stops");
        assert_eq!(s.index, Some(1), "stops on the last track");
        assert_eq!(s.repeat, RepeatMode::Off);
        player.shutdown();
    }

    #[test]
    fn corrupt_file_is_skipped_automatically() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("skip");
        let bad = dir.join("bad.wav");
        let good = dir.join("good.wav");
        std::fs::write(&bad, b"this is definitely not a wave file").unwrap();
        write_wav(&good, 0.3);
        let tracks = vec![
            track(bad.to_str().unwrap(), "BAD", 0.3),
            track(good.to_str().unwrap(), "GOOD", 0.3),
        ];

        let player = start_player();
        player.play_tracks(tracks, 0);

        let s = wait_for(&player, Duration::from_secs(8), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("GOOD")
        })
        .expect("skips the corrupt file and plays the next");
        assert_eq!(s.index, Some(1));
        player.shutdown();
    }
}
