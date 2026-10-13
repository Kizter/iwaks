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
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use iwaks_core::track::Track;
use serde::Serialize;

use crate::engine::Engine;
use crate::ffi::{EndFileReason, Mpv, MpvEvent};
use crate::queue::{Queue, RepeatMode};

/// Seconds of core runtime the pump grants libmpv per `wait_event` call.
/// Too-short slices starve the playback core (position then advances far
/// slower than real time); this value balances that against command latency.
const WAIT_SLICE: f64 = 0.1;
/// Minimum interval between state-refresh ticks (the pump's ~10 Hz clock).
const TICK_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Pump slice while a crossfade ramps: finer than [WAIT_SLICE] so the volume
/// steps are ~50 ms and not audible as a staircase (issue #27).
const FADE_SLICE: f64 = 0.02;

/// Playback behaviour knobs applied at startup.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Null audio output — headless (tests/CI, no device required).
    pub ao_null: bool,
    /// WASAPI exclusive mode (bit-perfect). **Default off**: exclusive mode
    /// silences every other app on the machine sample-accurately for the
    /// whole session, which surprised users ("Discord/YouTube go quiet while
    /// Iwaks runs"). Shared mode keeps other apps audible and is the default.
    /// Deviation from design §4.2 "bit-perfect".
    pub audio_exclusive: bool,
    /// mpv `audio-device` name; empty means "let the OS pick" (issue #23).
    pub audio_device: String,
}

/// One entry of mpv's `audio-device-list` (issue #23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// mpv's identifier — what gets sent back to select it.
    pub name: String,
    /// Human label shown in the picker.
    pub description: String,
}

/// How long a caller waits for the pump to answer a device request. The pump
/// wakes at least every [`WAIT_SLICE`], so this is generous; it only exists so
/// a wedged audio stack cannot hang the UI thread forever.
const DEVICE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Crossfade preferences (issue #27). `enabled` is the separate flag the plan
/// settled on: a duration of 0 must be able to mean "very short fade", not
/// "off".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crossfade {
    pub enabled: bool,
    /// Seconds of overlap requested. Only read when `enabled`.
    pub seconds: f64,
}

impl Default for Crossfade {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        }
    }
}

/// The value the plan's D1 decision settled on.
pub const DEFAULT_CROSSFADE_SECONDS: f64 = 3.0;

/// Never prefetch on a window shorter than this, whatever the user asks for: a
/// fade window has to be long enough to be audible at all.
const MIN_CROSSFADE_SECONDS: f64 = 0.5;

impl Crossfade {
    /// The overlap to actually use, or `None` when crossfade is off or the
    /// request is unusable.
    fn window(&self) -> Option<f64> {
        if !self.enabled || !self.seconds.is_finite() {
            return None;
        }
        Some(self.seconds.max(MIN_CROSSFADE_SECONDS))
    }
}

/// Set libmpv's pause flag, reporting failure.
fn set_pause(core: &Mpv, paused: bool) -> Result<(), i32> {
    core.command(&["set", "pause", if paused { "yes" } else { "no" }])
}

/// Set libmpv's mute flag.
fn set_mute(core: &Mpv, muted: bool) {
    let _ = core.command(&["set", "mute", if muted { "yes" } else { "no" }]);
}

/// Clamp a computed level to what mpv accepts and hand it over.
fn set_volume(core: &Mpv, level: f64) {
    let clamped = level.clamp(0.0, 100.0).round() as i64;
    let _ = core.command(&["set", "volume", &clamped.to_string()]);
}

/// A crossfade in progress (issue #27, stage 3).
///
/// The pump owns this: it advances [`Fade::progress`] on every tick, hands the
/// session over at the midpoint, and tears the outgoing core down at the end.
#[derive(Clone)]
struct Fade {
    started: Instant,
    /// Total overlap in seconds — already clamped to what is left of the track.
    window: f64,
    /// Core the incoming track is playing in.
    incoming: usize,
    /// Core still audible at the start of the ramp.
    outgoing: usize,
    /// Track index inside `tracks` that the incoming core holds.
    incoming_track: usize,
    /// The session already moved to `incoming` (the midpoint).
    swapped: bool,
    /// When the ramp was frozen by a pause. `started` is pushed forward on
    /// resume, so a paused player does not run the fade out under the user.
    paused_at: Option<Instant>,
}

impl Fade {
    /// How far along the ramp we are, 0.0 → 1.0.
    fn progress(&self, now: Instant) -> f64 {
        if self.window <= 0.0 {
            return 1.0;
        }
        (now.duration_since(self.started).as_secs_f64() / self.window).clamp(0.0, 1.0)
    }
}

/// Equal-power gain for one side of the ramp.
///
/// A linear fade dips in the middle (two 0.5 signals sum to 0.7); `sin`/`cos`
/// keeps the perceived loudness flat, which is the whole reason a crossfade
/// sounds like one continuous performance instead of a dip.
fn equal_power(t: f64) -> (f64, f64) {
    let angle = t * std::f64::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
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
    /// Shuffle on: the queue walks a permutation of the list.
    pub shuffle: bool,
    /// Bumped whenever the queue order changes (reshuffle, manual reorder,
    /// shuffle toggle, or a fresh list). `index`/`list_len`/`shuffle` can all
    /// stay the same across a reshuffle, so the Queue view keys on this to
    /// refetch; ordinary ticks leave it untouched (no per-tick churn).
    pub queue_version: u64,
    /// Playback-rate multiplier (mpv `speed` property); clamped to 0.25–4.0.
    pub speed: f64,
    /// Seconds left on the sleep timer; `None` when no timer is armed.
    pub sleep_remaining: Option<f64>,
    /// Master gain (dB) applied after the EQ bands (slider range ±12 dB).
    pub eq_preamp: f64,
    /// 10 band gains (dB) at [`EQ_BANDS`] frequencies; all zeros = flat.
    pub eq: Vec<f64>,
    /// ReplayGain mode applied by libmpv.
    pub replaygain: ReplayGainMode,
    /// Active mpv `audio-device` (empty = the OS default), so Settings can show
    /// which output is in use even after the device is unplugged (issue #23).
    pub audio_device: String,
    /// True while the next track sits loaded and paused in the second core,
    /// waiting for its fade (issue #27). Prefetching never changes what is
    /// audible — only stage 3 turns this into sound.
    pub prefetched: bool,
}

/// ReplayGain mode (libmpv `replaygain` option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplayGainMode {
    #[default]
    Track,
    Album,
    Off,
}

impl ReplayGainMode {
    pub(crate) fn as_mpv(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            // mpv's choice values are `no|track|album` (no `off` alias).
            Self::Off => "no",
        }
    }

    fn from_mpv(value: &str) -> Self {
        match value {
            "album" => Self::Album,
            "off" | "no" => Self::Off,
            _ => Self::Track,
        }
    }
}

/// ISO standard 10-band graphic-EQ center frequencies, in Hz.
pub const EQ_BANDS: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// Last-applied EQ settings (the value the frontend sees in state).
#[derive(Debug, Clone, PartialEq)]
struct EqSettings {
    preamp: f64,
    gains: Vec<f64>,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            preamp: 0.0,
            gains: vec![0.0; EQ_BANDS.len()],
        }
    }
}

/// Clamp a dB value to the EQ range and snap it to 0.1 dB steps.
fn clamp_db(value: f64) -> f64 {
    let v = value.clamp(-12.0, 12.0);
    (v * 10.0).round() / 10.0
}

/// Build the `af` lavfi graph for the EQ; `None` when the net effect is flat.
/// Per-band `equalizer` filters use octave width; the master `preamp` gain is
/// applied last (`volume` filter) so it acts as a post-EQ master volume.
fn eq_af_chain(preamp: f64, gains: &[f64]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (freq, gain) in EQ_BANDS.iter().zip(gains) {
        let g = clamp_db(*gain);
        if g != 0.0 {
            parts.push(format!("equalizer=f={freq}:t=o:w=1:g={g}"));
        }
    }
    let p = clamp_db(preamp);
    if p != 0.0 {
        parts.push(format!("volume=volume={p}dB"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("lavfi=[{}]", parts.join(",")))
    }
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
    SetMute(bool),
    SetSpeed(f64),
    SetEq {
        preamp: f64,
        gains: Vec<f64>,
    },
    SetReplayGain(ReplayGainMode),
    SetShuffle(bool),
    Reshuffle,
    Stop,
    SleepElapsed,
    Refresh,
    /// Ask the pump to read `audio-device-list` and answer on the channel.
    ListDevices(mpsc::Sender<Vec<AudioDevice>>),
    /// Apply a new `audio-device` on the pump thread.
    SetAudioDevice(String),
    /// Drop a prefetched track (queue changed underneath it).
    ClearPrefetch,
    /// Update the crossfade preferences. No libmpv work; the pump reads them.
    SetCrossfade(Crossfade),
}

pub struct Player {
    /// The session's libmpv cores (issue #27 stage 1: one audible core now, a
    /// prefetch slot to come). Created, initialised and driven only on the pump
    /// thread. Stored behind a mutex solely for Arc lifetime sharing with
    /// `shutdown`-adjacent cleanup — no other thread ever issues an mpv call,
    /// so no cross-thread wakeup is needed.
    engine: Mutex<Engine>,
    /// Crossfade preferences; written from any thread, read by the pump.
    crossfade: Mutex<Crossfade>,
    /// The crossfade currently ramping, if any. Pump thread only.
    fade: Mutex<Option<Fade>>,
    /// Whether the "crossfade needs exclusive output" refusal was already logged.
    /// `maybe_prefetch` runs on every pump tick, so an unguarded warning would
    /// fill the log.
    crossfade_refusal_logged: AtomicBool,
    /// What the user asked for, 0–100. Tracked separately from libmpv's
    /// `volume` because during a fade the cores are driven below it, and the
    /// reported volume has to stay the user's choice (issue #27, stage 3).
    user_volume: AtomicI64,
    /// Last state produced by the pump; read by any thread without mpv calls.
    state: Mutex<PlayerState>,
    tracks: Mutex<Vec<Track>>,
    queue: Mutex<Queue>,
    sink: Mutex<Option<StateSink>>,
    commands: Mutex<VecDeque<Cmd>>,
    /// When set, the pump pauses playback once `Instant::now()` passes it.
    sleep_deadline: Mutex<Option<Instant>>,
    /// Last-applied graphic EQ settings (pump-side source of truth).
    eq: Mutex<EqSettings>,
    /// Last-requested ReplayGain mode; fallback when the property read fails.
    replaygain: Mutex<ReplayGainMode>,
    /// Monotonic seed source for shuffle permutations (SplitMix64 steps so
    /// consecutive seeds are unrelated).
    shuffle_seed: AtomicU64,
    /// Monotonic counter bumped on every queue-order mutation; surfaced as
    /// [`PlayerState::queue_version`].
    queue_version: AtomicU64,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Id of the pump thread, recorded at spawn. `shutdown` compares it against
    /// the caller: a `Drop` that runs *on* the pump thread (the last `Arc`
    /// released there after a failed `start`) must detach, not join itself.
    pump_thread: Mutex<Option<ThreadId>>,
    stopped: AtomicBool,
    shut_down: AtomicBool,
    cancelled: AtomicBool,
}

/// Read mpv's `audio-device-list` into pickable devices.
///
/// The property is a list-of-options, so it must be read as an mpv node tree —
/// asking for it as a string yields an empty string, which is what made the
/// picker show nothing but "System default" (issue #23 follow-up). The node
/// shape has moved around between mpv versions (array of maps vs. map of maps),
/// so [`device_from_entry`] looks for the two keys it cares about anywhere in an
/// entry rather than at a fixed position.
///
/// Pure and therefore unit-tested; `read_device_list` only does the I/O.
fn device_from_entry(entry: &[(String, String)]) -> Option<AudioDevice> {
    // The last string for a key wins: a map-of-maps entry starts with the outer
    // option name, and the real field comes after it.
    let field = |key: &str| {
        entry
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let description = field("description").unwrap_or_default();
    // mpv's own "let the OS choose" entry is dropped: the picker already renders
    // a "System default" row whose value is the empty device name, and keeping
    // both would show the same destination twice with two different values.
    let name = field("name").filter(|n| !n.is_empty() && n != crate::ffi::AUTO_DEVICE)?;
    Some(AudioDevice { name, description })
}

/// Ask libmpv which output devices exist.
///
/// An empty result is not an error the user should see as a failure: it means
/// "mpv reported nothing", which is what happens when no audio output has been
/// opened yet. It is logged so the log says which of the two happened.
fn read_device_list(api: &Mpv) -> Vec<AudioDevice> {
    let entries = match api.get_option_list("audio-device-list") {
        Ok(entries) => entries,
        Err(rc) => {
            iwaks_log::line(
                iwaks_log::Level::Warn,
                &format!("mpv refused audio-device-list ({rc})"),
            );
            return Vec::new();
        }
    };
    let devices: Vec<AudioDevice> = entries
        .iter()
        .filter_map(|e| device_from_entry(e))
        .collect();
    iwaks_log::line(
        iwaks_log::Level::Debug,
        &format!(
            "audio-device-list: {} entries, {} pickable",
            entries.len(),
            devices.len()
        ),
    );
    devices
}

impl Player {
    /// Spawn the pump thread, which owns the whole libmpv lifecycle, and wait
    /// until mpv is initialized (or initialization failed).
    pub fn start(options: &Options, sink: StateSink) -> Result<Arc<Self>, String> {
        let player = Arc::new(Self {
            engine: Mutex::new(Engine::empty()),
            crossfade: Mutex::new(Crossfade::default()),
            crossfade_refusal_logged: AtomicBool::new(false),
            fade: Mutex::new(None),
            user_volume: AtomicI64::new(80),
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
                shuffle: false,
                queue_version: 1,
                speed: 1.0,
                sleep_remaining: None,
                eq_preamp: 0.0,
                eq: vec![0.0; EQ_BANDS.len()],
                replaygain: ReplayGainMode::Track,
                audio_device: options.audio_device.clone(),
                prefetched: false,
            }),
            tracks: Mutex::new(Vec::new()),
            queue: Mutex::new(Queue::new(0)),
            sink: Mutex::new(Some(sink)),
            commands: Mutex::new(VecDeque::new()),
            sleep_deadline: Mutex::new(None),
            eq: Mutex::new(EqSettings::default()),
            replaygain: Mutex::new(ReplayGainMode::Track),
            shuffle_seed: AtomicU64::new(0xDEADBEEF),
            queue_version: AtomicU64::new(1),
            thread: Mutex::new(None),
            pump_thread: Mutex::new(None),
            stopped: AtomicBool::new(false),
            shut_down: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        });

        let options = options.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let pump_player = Arc::clone(&player);
        let handle = std::thread::spawn(move || {
            let mut engine = Engine::empty();
            if let Err(err) = engine.init_primary(&options) {
                let _ = ready_tx.send(Err(err));
                return;
            }
            *pump_player.engine.lock().unwrap() = engine;
            let _ = ready_tx.send(Ok(()));
            Self::pump(&pump_player);
        });
        let pump_id = handle.thread().id();
        *player.thread.lock().unwrap() = Some(handle);
        *player.pump_thread.lock().unwrap() = Some(pump_id);

        match ready_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(player),
            Ok(Err(err)) => Err(err),
            Err(_) => {
                player.cancelled.store(true, Ordering::SeqCst);
                Err("player init timed out".into())
            }
        }
    }

    /// Replace the queue and start playing `tracks[index]`.
    pub fn play_tracks(&self, tracks: Vec<Track>, index: usize) {
        if tracks.is_empty() {
            return;
        }
        let index = index.min(tracks.len() - 1);
        let (repeat, shuffle) = {
            let q = self.queue.lock().unwrap();
            (q.repeat, q.shuffle)
        };
        let len = tracks.len();
        let mut queue = Queue::new(len).with_repeat(repeat);
        if shuffle {
            // Keep shuffle on across list swaps; fresh permutation, but the
            // requested track still starts playing.
            queue = queue.with_shuffle(self.next_shuffle_seed());
        }
        *self.tracks.lock().unwrap() = tracks;
        *self.queue.lock().unwrap() = queue.start(index);
        self.bump_queue_version();
        self.push(Cmd::ClearPrefetch);
        self.bump_queue_version();
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

    /// Set the mute state explicitly (used when restoring persisted settings).
    pub fn set_mute(&self, mute: bool) {
        self.push(Cmd::SetMute(mute));
    }

    /// Playback-rate multiplier; values outside 0.25–4.0 are clamped
    /// (mirrors how `set_volume` clamps, and matches mpv's own limits).
    pub fn set_speed(&self, speed: f64) {
        self.push(Cmd::SetSpeed(speed.clamp(0.25, 4.0)));
    }

    /// Arm the sleep timer: playback pauses once `seconds` have elapsed.
    /// `None` (or a non-positive value) cancels an armed timer.
    pub fn set_sleep_timer(&self, seconds: Option<f64>) {
        let deadline = seconds
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| Instant::now() + Duration::from_secs_f64(s));
        *self.sleep_deadline.lock().unwrap() = deadline;
        self.push(Cmd::Refresh);
    }

    /// Graphic EQ: master `preamp` plus 10 band gains (dB, ±12, 0.1 steps).
    /// Gains are clamped to [`EQ_BANDS`] length; missing bands read as 0 dB.
    pub fn set_eq(&self, preamp: f64, gains: Vec<f64>) {
        let clamped: Vec<f64> = gains
            .iter()
            .take(EQ_BANDS.len())
            .map(|g| clamp_db(*g))
            .collect();
        let mut padded = vec![0.0; EQ_BANDS.len()];
        padded[..clamped.len()].copy_from_slice(&clamped);
        self.push(Cmd::SetEq {
            preamp: clamp_db(preamp),
            gains: padded,
        });
    }

    /// ReplayGain mode; applied by libmpv from the next loaded file onward.
    pub fn set_replaygain(&self, mode: ReplayGainMode) {
        self.push(Cmd::SetReplayGain(mode));
    }

    pub fn set_repeat(&self, repeat: RepeatMode) {
        let next = self.queue.lock().unwrap().with_repeat(repeat);
        *self.queue.lock().unwrap() = next;
        self.push(Cmd::Refresh);
    }

    /// Toggle shuffle on/off (`on`: randomize the remaining order, keep the
    /// current track; `off`: return to the linear list order).
    pub fn set_shuffle(&self, shuffle: bool) {
        self.push(Cmd::SetShuffle(shuffle));
    }

    /// Shuffle the remaining tracks again (keeps the current track).
    pub fn reshuffle(&self) {
        self.push(Cmd::Reshuffle);
    }

    /// Ordered tracks in the current session queue (play order — the shuffle
    /// permutation when enabled). Read for the Queue view; never touches mpv.
    pub fn queue_tracks(&self) -> Vec<Track> {
        let indices = self.queue.lock().unwrap().ordered_indices();
        let tracks = self.tracks.lock().unwrap();
        indices
            .iter()
            .filter_map(|&i| tracks.get(i).cloned())
            .collect()
    }

    /// Move the track at queue position `from` to `to`. The currently playing
    /// track follows its new slot, so playback is untouched — only the
    /// upcoming order changes (touch-queue only, no mpv calls).
    pub fn reorder_queue(&self, from: usize, to: usize) {
        let next = self.queue.lock().unwrap().reorder(from, to);
        *self.queue.lock().unwrap() = next;
        self.bump_queue_version();
        self.push(Cmd::ClearPrefetch);
        self.bump_queue_version();
        self.push(Cmd::Refresh);
    }

    /// Audio devices mpv can output to, read on the pump thread.
    ///
    /// Returns an empty list (never blocks) if the audio stack does not answer
    /// within [`DEVICE_TIMEOUT`] — the caller still has "system default".
    pub fn list_devices(&self) -> Vec<AudioDevice> {
        let (tx, rx) = mpsc::channel();
        self.push(Cmd::ListDevices(tx));
        match rx.recv_timeout(DEVICE_TIMEOUT) {
            Ok(devices) => devices,
            Err(_) => {
                iwaks_log::line(
                    iwaks_log::Level::Warn,
                    "audio device list timed out waiting for the playback thread",
                );
                Vec::new()
            }
        }
    }

    /// Switch the output device; an empty `name` restores "let the OS choose".
    ///
    /// Applied by the pump like every other mpv change, so a failed switch leaves
    /// the current output untouched and is visible in the next state tick.
    pub fn set_audio_device(&self, name: &str) {
        self.push(Cmd::SetAudioDevice(name.to_string()));
    }

    /// Crossfade on/off and how long the overlap should be (issue #27).
    ///
    /// Off by default (design §4.2). Enabling it makes the pump prefetch the
    /// next track into the second core during the last seconds of the current
    /// one; stage 3 is what makes that prefetch audible.
    pub fn set_crossfade(&self, crossfade: Crossfade) {
        self.push(Cmd::SetCrossfade(crossfade));
    }

    /// The crossfade preferences currently in force.
    pub fn crossfade(&self) -> Crossfade {
        *self.crossfade.lock().unwrap()
    }

    /// Is a track waiting in the second core? Mirrors
    /// [`PlayerState::prefetched`] for callers that want to know before the next
    /// state tick arrives.
    pub fn has_prefetched(&self) -> bool {
        self.engine.lock().unwrap().has_idle()
    }

    /// Record that the queue order changed; the next emitted state carries a
    /// new [`PlayerState::queue_version`] so the Queue view refetches.
    fn bump_queue_version(&self) {
        self.queue_version.fetch_add(1, Ordering::Relaxed);
    }

    /// Remove the queue track at play-order position `position`. Removing
    /// anything else is pure bookkeeping; removing the playing track hands over
    /// to the track that took its slot, or stops when nothing follows.
    pub fn remove_queue_at(&self, position: usize) {
        // One critical section (queue → tracks, the order used everywhere else)
        // so no reader can observe a queue and a track list from different edits.
        let (was_current, new_current) = {
            let mut q = self.queue.lock().unwrap();
            let mut tracks = self.tracks.lock().unwrap();
            let Some(track) = q.track_at(position) else {
                return;
            };
            let was_current = q.current() == Some(track);
            let next = q.without_position(position);
            tracks.remove(track);
            *q = next;
            (was_current, q.current())
        };

        if was_current {
            match new_current {
                Some(track) => self.advance_to(track),
                None => self.push(Cmd::Stop),
            }
        } else {
            self.push(Cmd::Refresh);
        }
        self.bump_queue_version();
        self.push(Cmd::ClearPrefetch);
        self.bump_queue_version();
    }

    /// Empty the session queue and stop playback. Repeat and shuffle survive,
    /// so clearing the list never silently resets those preferences.
    pub fn clear_queue(&self) {
        {
            let mut q = self.queue.lock().unwrap();
            let mut tracks = self.tracks.lock().unwrap();
            tracks.clear();
            *q = q.cleared();
        }
        self.push(Cmd::ClearPrefetch);
        self.bump_queue_version();
        self.push(Cmd::Stop);
    }

    /// Next unique shuffle seed: monotonic SplitMix64 steps, so consecutive
    /// calls never reuse a permutation.
    fn next_shuffle_seed(&self) -> u64 {
        self.shuffle_seed
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
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
            // If we are *on* the pump thread, the last `Arc` was dropped there
            // (a failed/timed-out `start` racing the caller's drop). Joining
            // would block on ourselves forever; the thread is already exiting,
            // so just detach by dropping the handle.
            let on_pump_thread =
                *self.pump_thread.lock().unwrap() == Some(std::thread::current().id());
            if on_pump_thread {
                drop(handle);
            } else {
                // The pump's wait_event times out ≤100 ms, so join returns fast.
                let _ = handle.join();
            }
        }
        self.stopped.store(true, Ordering::SeqCst);
        // The pump is gone, so releasing the cores that are not playing here is
        // safe: `Mpv::drop` calls `mpv_terminate`, and with no pump left there is
        // no other libmpv work it could race. While the pump is alive this must
        // stay on its thread — hence `Cmd::ClearPrefetch` rather than a direct
        // `take_idle` from a command thread.
        //
        // Both roles are drained because a shutdown can land mid-ramp, where the
        // prefetching core is fading in and a third one is fading out.
        let mut engine = self.engine.lock().unwrap();
        drop(engine.take_idle());
        drop(engine.take_outgoing());
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

    /// Enable/disable shuffle without touching playback (pump thread only).
    /// Next/prev/auto-advance all read the new permutation via the queue.
    fn apply_shuffle(&self, on: bool) {
        let seed = self.next_shuffle_seed();
        let next = {
            let q = self.queue.lock().unwrap();
            if on {
                q.with_shuffle(seed)
            } else {
                q.with_shuffle_off()
            }
        };
        *self.queue.lock().unwrap() = next;
        self.bump_queue_version();
        self.emit();
    }

    /// Re-shuffle the remaining tracks, keeping the current one pinned
    /// (pump thread only).
    fn apply_reshuffle(&self) {
        let seed = self.next_shuffle_seed();
        let next = self.queue.lock().unwrap().reshuffle(seed);
        *self.queue.lock().unwrap() = next;
        self.bump_queue_version();
        self.emit();
    }

    // ---- pump side (only the pump thread runs these) ----

    /// Clone the playing core's handle, releasing the lock. Synchronous mpv
    /// calls must run without holding the engine mutex: a pump thread blocked
    /// inside a call must not stall `push`/`snapshot`/`shutdown` on other
    /// threads.
    fn api(&self) -> Arc<Mpv> {
        self.engine.lock().unwrap().active()
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
                let paused = self.api().get_flag("pause").unwrap_or(false);
                // Not just the active core: pausing during a crossfade has to
                // silence the fading one too, or the user keeps hearing a track
                // they believe is paused (issue #27).
                self.set_pause_everywhere(!paused);
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
                // Remember the user's choice, then apply it to whatever can be
                // heard. During a fade the cores are driven below it.
                self.user_volume
                    .store(volume.clamp(0, 100), Ordering::Relaxed);
                self.apply_volume_to_audible();
                self.emit();
            }
            Cmd::ToggleMute => {
                let muted = self.api().get_flag("mute").unwrap_or(false);
                self.set_mute_everywhere(!muted);
                self.emit();
            }
            Cmd::SetMute(mute) => {
                self.set_mute_everywhere(mute);
                self.emit();
            }
            Cmd::SetSpeed(speed) => {
                let api = self.api();
                let _ = api.command(&["set", "speed", &speed.to_string()]);
                // A track that is already crossfading in was loaded at the old
                // speed; it has to follow the session's (issue #27).
                if let Some(incoming) = self.engine.lock().unwrap().idle() {
                    let _ = incoming.command(&["set", "speed", &speed.to_string()]);
                }
                self.emit();
            }
            Cmd::SetEq { preamp, gains } => {
                // Stores the requested EQ first so state matches even if the
                // `af` reconfiguration fails for some reason.
                *self.eq.lock().unwrap() = EqSettings {
                    preamp,
                    gains: gains.clone(),
                };
                let chain = eq_af_chain(preamp, &gains);
                let api = self.api();
                match &chain {
                    Some(chain) => {
                        let _ = api.command(&["set", "af", chain]);
                    }
                    None => {
                        // Flat EQ → drop the whole filter chain.
                        let _ = api.command(&["set", "af", ""]);
                    }
                }
                // A crossfading track that is already loaded would otherwise
                // keep the chain it was prefetched with, so the first half of it
                // would come in under the old EQ (issue #27).
                {
                    let mut engine = self.engine.lock().unwrap();
                    if engine.idle_af() != chain.as_deref() {
                        if let Some(idle) = engine.idle() {
                            let af = chain.clone().unwrap_or_default();
                            let _ = idle.command(&["set", "af", &af]);
                            engine.set_idle_af(chain.clone());
                        }
                    }
                }
                self.emit();
            }
            Cmd::SetReplayGain(mode) => {
                *self.replaygain.lock().unwrap() = mode;
                let api = self.api();
                let _ = api.command(&["set", "replaygain", mode.as_mpv()]);
                // Same reasoning as the EQ above.
                if let Some(idle) = self.engine.lock().unwrap().idle() {
                    let _ = idle.command(&["set", "replaygain", mode.as_mpv()]);
                }
                self.emit();
            }
            Cmd::SetShuffle(on) => self.apply_shuffle(on),
            Cmd::Reshuffle => self.apply_reshuffle(),
            Cmd::Stop => {
                // A ramp in flight has to be landed first: stopping only the
                // active core would leave the other one audible.
                self.abandon_fade();
                self.stopped.store(true, Ordering::SeqCst);
                let _ = self.api().command(&["stop"]);
                self.emit();
            }
            Cmd::SleepElapsed => {
                // Pause only — the queue stays loaded so the user resumes
                // with one press. Mirrors the pause branch of TogglePlay.
                let api = self.api();
                let _ = api.command(&["set", "pause", "yes"]);
                self.emit();
            }
            Cmd::Refresh => self.emit(),
            Cmd::SetCrossfade(next) => {
                *self.crossfade.lock().unwrap() = next;
                // Turning crossfade off must not leave a prefetched core lying
                // around: nothing would ever consume it.
                if next.window().is_none() {
                    self.abandon_fade();
                    self.clear_prefetch();
                }
            }
            Cmd::ClearPrefetch => {
                self.abandon_fade();
                self.clear_prefetch();
            }
            // Both device commands run here because they read/set mpv
            // properties, and only the pump may touch libmpv.
            Cmd::ListDevices(reply) => {
                let devices = read_device_list(&self.api());
                let _ = reply.send(devices);
            }
            Cmd::SetAudioDevice(name) => {
                // mpv reads an empty `audio-device` as "let the OS choose".
                let value = if name.is_empty() {
                    "auto"
                } else {
                    name.as_str()
                };
                if let Err(e) = self.api().command(&["set", "audio-device", value]) {
                    iwaks_log::line(
                        iwaks_log::Level::Warn,
                        &format!("audio-device '{value}' rejected by mpv ({e})"),
                    );
                }
                self.emit();
            }
        }
    }

    fn play_index(&self, index: usize) {
        // Loading a different track supersedes any ramp: the other core is
        // stopped here, so a fade can never leave two cores audible.
        self.abandon_fade();
        self.clear_prefetch();
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

    fn pump(player: &Arc<Player>) {
        let mut last_tick = std::time::Instant::now();
        loop {
            if player.cancelled.load(Ordering::SeqCst) {
                break;
            }
            // Sleep timer: once the deadline passes, clear it and queue the
            // pause. `Cmd::SleepElapsed` runs in the drain below, so the
            // pause lands within the same loop iteration (≤100 ms late).
            let sleep_fired = {
                let mut deadline = player.sleep_deadline.lock().unwrap();
                match *deadline {
                    Some(until) if Instant::now() >= until => {
                        *deadline = None;
                        true
                    }
                    _ => false,
                }
            };
            if sleep_fired {
                player.commands.lock().unwrap().push_back(Cmd::SleepElapsed);
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
            //
            // Advance any crossfade before waiting, so the ramp keeps stepping
            // at the rate the fade needs (issue #27).
            let fading = player.advance_fade();

            // Poll every core without blocking first: during a crossfade the
            // prefetching and the fading-out cores produce events too, and the
            // outgoing core's EOF is what ends a ramp early.
            //
            // The slot list is collected first: a temporary guard inside a
            // `for` head would live for the whole loop and the lock inside the
            // body would deadlock against it.
            let mut busy = false;
            let slots = player.engine.lock().unwrap().live_slots();
            for slot in slots {
                let Some(core) = player.engine.lock().unwrap().core(slot) else {
                    continue;
                };
                while let Some(event) = core.wait_event(0.0) {
                    busy = true;
                    player.handle_event(slot, event);
                }
            }

            if !busy {
                // Nothing pending anywhere: block on the audible core. A fade
                // needs finer steps than a state tick, so it shortens the slice.
                let slice = if fading { FADE_SLICE } else { WAIT_SLICE };
                let (slot, core) = {
                    let engine = player.engine.lock().unwrap();
                    (engine.active_slot(), engine.active())
                };
                if let Some(event) = core.wait_event(slice) {
                    player.handle_event(slot, event);
                }
            }

            if last_tick.elapsed() >= TICK_EVERY {
                // Emit, then consider prefetching (issue #27): doing it in the
                // same tick means the state the user sees and the prefetch
                // decision come from the same snapshot.
                player.emit();
                player.maybe_prefetch();
                last_tick = std::time::Instant::now();
            }
            if player.cancelled.load(Ordering::SeqCst) {
                break;
            }
        }
    }

    /// Handle one event, knowing which core it came from (pump thread only).
    ///
    /// The core matters: an EOF from the fading-out core ends the ramp early,
    /// and a FileLoaded from a prefetching core is not a track change.
    fn handle_event(&self, slot: usize, event: MpvEvent) {
        match event {
            MpvEvent::LogMessage(text) => {
                // Only produced when IWAKS_MPV_LOG is set at startup.
                iwaks_log::line(iwaks_log::Level::Debug, &format!("mpv: {text}"));
            }
            MpvEvent::EndFile(reason) => {
                // The fading-out core is the one the ramp is draining, and it
                // is asked for first: before the handover it *is* the active
                // slot, but its EOF must land the fade rather than advance the
                // queue, which the handover has already done.
                let fading = self.fade.lock().unwrap().clone();
                if let Some(fade) = fading {
                    if slot == fade.outgoing {
                        // The fading track reached its end before the ramp did
                        // (the window is clamped to what was left of it).
                        self.finish_fade();
                        return;
                    }
                }
                if slot == self.engine.lock().unwrap().active_slot() {
                    self.on_end_file(reason);
                }
            }
            MpvEvent::FileLoaded => {
                if slot == self.engine.lock().unwrap().active_slot() {
                    self.emit();
                }
            }
            // EVENT_NONE never reaches here: the pump treats it as "nothing".
            MpvEvent::Other(_) | MpvEvent::Shutdown => {}
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
    ///
    /// A prefetched track is dropped here: the track that was loaded into the
    /// second core is no longer the one that plays next, and a stale prefetch
    /// is exactly what stage 3 must never fade into.
    fn advance_to(&self, index: usize) {
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        self.commands.lock().unwrap().push_back(Cmd::Load(index));
    }

    /// Push the user's volume to the cores that can be heard right now.
    ///
    /// Outside a fade that is just the active core at full volume. During one it
    /// is the active core scaled by the incoming side of the ramp — the outgoing
    /// core keeps its own level, written by [`Player::advance_fade`].
    fn apply_volume_to_audible(&self) {
        let user = self.user_volume.load(Ordering::Relaxed);
        let level = match self.fade.lock().unwrap().clone() {
            Some(fade) if fade.swapped => {
                user as f64 * equal_power(fade.progress(Instant::now())).1
            }
            _ => user as f64,
        };
        let api = self.api();
        set_volume(&api, level);
    }

    /// Pause or resume every core that can be heard right now.
    ///
    /// Normally that is just the active core, but during a crossfade two are
    /// audible and both have to follow — pausing one leaves the other playing
    /// something the user believes is stopped. The ramp is frozen while paused
    /// and resumes from the same point (issue #27).
    fn set_pause_everywhere(&self, paused: bool) {
        let fade = self.fade.lock().unwrap().clone();
        let audible = {
            let engine = self.engine.lock().unwrap();
            let mut slots = vec![engine.active_slot()];
            if let Some(fade) = fade.as_ref() {
                for slot in [fade.incoming, fade.outgoing] {
                    if !slots.contains(&slot) && engine.core(slot).is_some() {
                        slots.push(slot);
                    }
                }
            }
            slots
        };
        for slot in audible {
            if let Some(core) = self.engine.lock().unwrap().core(slot) {
                let _ = set_pause(&core, paused);
            }
        }
        let mut slot = self.fade.lock().unwrap();
        if let Some(fade) = slot.as_mut() {
            match (paused, fade.paused_at) {
                (true, None) => fade.paused_at = Some(Instant::now()),
                (false, Some(since)) => {
                    fade.started += since.elapsed();
                    fade.paused_at = None;
                }
                _ => {}
            }
        }
    }

    /// Mute (or unmute) every core. During a fade two of them are audible, and
    /// a prefetching one has to be in the same state or it would blast at the
    /// moment the ramp gives it volume.
    fn set_mute_everywhere(&self, mute: bool) {
        let flag = if mute { "yes" } else { "no" };
        let slots = self.engine.lock().unwrap().live_slots();
        for slot in slots {
            let Some(core) = self.engine.lock().unwrap().core(slot) else {
                continue;
            };
            let _ = core.command(&["set", "mute", flag]);
        }
    }

    /// The next track is loaded and ready; cross it with the current one
    /// (pump thread only). A no-op when crossfade is off, which is what makes
    /// the prefetch in stage 2 silent.
    fn start_fade(&self, incoming: &Arc<Mpv>, incoming_slot: usize, track: usize, window: f64) {
        if self.fade.lock().unwrap().is_some() {
            return; // one ramp at a time; the free slot is spoken for
        }
        let outgoing_slot = self.engine.lock().unwrap().active_slot();

        // The incoming core starts silent, then ramps up; the outgoing one is
        // dragged down from the user's volume. Note that the engine is *not*
        // told about a fading-out slot yet: until the midpoint that core is
        // still the one playing.
        set_volume(incoming, 0.0);
        if set_pause(incoming, false).is_err() {
            return;
        }

        *self.fade.lock().unwrap() = Some(Fade {
            started: Instant::now(),
            window,
            incoming: incoming_slot,
            outgoing: outgoing_slot,
            incoming_track: track,
            swapped: false,
            paused_at: None,
        });

        // Both sides are audible now, so both need the current mute state.
        let muted = self.api().get_flag("mute").unwrap_or(false);
        set_mute(incoming, muted);
        iwaks_log::line(
            iwaks_log::Level::Debug,
            &format!("crossfade started ({window:.1}s)"),
        );
        self.emit();
    }

    /// One step of the ramp. Returns true while a fade is still running, so the
    /// pump can shorten its wait slice and keep the steps smooth.
    fn advance_fade(&self) -> bool {
        let now = Instant::now();
        let Some(fade) = self.fade.lock().unwrap().clone() else {
            return false;
        };
        let t = fade.progress(now);
        if fade.paused_at.is_some() {
            // Nothing to ramp while the player is paused. Report "not fading" so
            // the pump goes back to its normal wait slice instead of spinning.
            return false;
        }
        let (out_gain, in_gain) = equal_power(t);
        let user = self.user_volume.load(Ordering::Relaxed) as f64;

        let outgoing = self.engine.lock().unwrap().core(fade.outgoing);
        let incoming = self.engine.lock().unwrap().core(fade.incoming);
        if let Some(core) = outgoing.as_ref() {
            set_volume(core, user * out_gain);
        }
        if let Some(core) = incoming.as_ref() {
            set_volume(core, user * in_gain);
        }

        // Midpoint: the incoming track is the louder one, so it takes over the
        // transport. The outgoing core keeps fading out — it is no longer active,
        // but it is still audible, which is why the engine has three slots.
        if !fade.swapped && t >= 0.5 && self.hand_over(&fade) {
            self.emit();
        }

        if t >= 1.0 {
            self.finish_fade();
            return false;
        }
        true
    }

    /// Hand the session to the incoming core (pump thread only).
    ///
    /// Idempotent: a fade that already handed over returns `false` and changes
    /// nothing. Reports whether this call was the one that did it, so the caller
    /// knows whether the published state moved.
    fn hand_over(&self, fade: &Fade) -> bool {
        if fade.swapped {
            return false;
        }
        {
            let mut engine = self.engine.lock().unwrap();
            engine.make_active(fade.incoming);
            // Only now: before the midpoint the fading core was the active one.
            engine.set_outgoing(fade.outgoing);
        }
        let adopted = self.queue.lock().unwrap().adopt_track(fade.incoming_track);
        *self.queue.lock().unwrap() = adopted;
        let mut slot = self.fade.lock().unwrap();
        let Some(current) = slot.as_mut() else {
            return false;
        };
        current.swapped = true;
        iwaks_log::line(iwaks_log::Level::Debug, "crossfade handed over");
        true
    }

    /// Land a ramp: hand over if it has not happened yet, then stop the core
    /// that was fading out and restore the session's volume.
    ///
    /// The handover runs even when the ramp was cut short by the fading track
    /// reaching its end — the incoming track is already audible at that point, so
    /// leaving it without a session would play audio nothing can control.
    fn finish_fade(&self) {
        let Some(fade) = self.fade.lock().unwrap().take() else {
            return;
        };
        self.hand_over(&fade);
        if let Some(core) = self.engine.lock().unwrap().take_outgoing() {
            let _ = core.command(&["stop"]);
        }
        self.apply_volume_to_audible();
        self.emit();
    }

    /// Abandon a fade without a handover — anything that changes what is playing
    /// while one is in flight. Without this, an interrupted ramp would leave a
    /// second core audible.
    fn abandon_fade(&self) {
        let Some(fade) = self.fade.lock().unwrap().take() else {
            return;
        };
        // After a handover the fading core is the engine's outgoing one; before
        // it, the incoming core is still prefetched and has to be dropped too.
        if let Some(core) = self.engine.lock().unwrap().take_outgoing() {
            let _ = core.command(&["stop"]);
        }
        if !fade.swapped {
            let taken = self.engine.lock().unwrap().take_idle();
            if let Some(core) = taken {
                let _ = core.command(&["stop"]);
            }
        }
        self.apply_volume_to_audible();
    }

    /// Drop the prefetched track and stop its core. Pump thread only (libmpv).
    fn clear_prefetch(&self) {
        let taken = self.engine.lock().unwrap().take_idle();
        if let Some(core) = taken {
            let _ = core.command(&["stop"]);
        }
    }

    /// Load the next track into the second core, paused, when the current one is
    /// close enough to its end for a fade to start (pump thread only).
    ///
    /// Deliberately makes **no sound**: the file is loaded paused, so this stage
    /// is invisible apart from a second core existing. Everything that makes a
    /// prefetch audible is stage 3.
    fn maybe_prefetch(&self) {
        let Some(window) = self.crossfade.lock().unwrap().window() else {
            return; // crossfade off
        };
        if self.engine.lock().unwrap().has_idle() {
            return; // already waiting
        }
        // Exclusive output owns the device for the whole session, so a second
        // core cannot open it. Refusing the prefetch degrades crossfade to
        // ordinary gapless playback, which beats failing exclusive mode — and
        // logged once, because it would otherwise repeat on every pump tick
        // (issue #27, risk table).
        if self.engine.lock().unwrap().is_exclusive() {
            if !self.crossfade_refusal_logged.swap(true, Ordering::SeqCst) {
                iwaks_log::line(
                    iwaks_log::Level::Warn,
                    "crossfade is unavailable while exclusive output is on",
                );
            }
            return;
        }
        // Before a handover the "free" slot is the core the ramp is fading *in*,
        // and reloading it would throw the incoming track away. After it, the
        // third core is genuinely free and a short next track can still be
        // prefetched in time (issue #27).
        if let Some(fade) = self.fade.lock().unwrap().clone() {
            if !fade.swapped {
                return;
            }
        }

        // One critical section (queue → tracks, the order used elsewhere) so the
        // decision and the path cannot come from two different queue states.
        let (after, path) = {
            let q = self.queue.lock().unwrap();
            let tracks = self.tracks.lock().unwrap();
            let Some(after) = q.after_end() else {
                return; // nothing follows
            };
            if Some(after) == q.current() {
                return; // repeat-one: the same track, nothing to prefetch
            }
            match tracks.get(after) {
                Some(t) => (after, t.path.clone()),
                None => return,
            }
        };

        // Only prefetch when the current track really is about to end.
        let s = self.snapshot();
        if s.duration <= 0.0 {
            return; // unknown length (a stream): no fade window to aim at
        }
        let remaining = s.duration - s.position;
        if remaining > window {
            return;
        }
        // Never ramp for longer than the track has left: the overlap should end
        // with the track, not run into whatever follows it.
        let window = window.min(remaining.max(MIN_CROSSFADE_SECONDS));

        let Some(incoming_slot) = self.engine.lock().unwrap().idle_slot() else {
            return;
        };
        let Some(core) = self.engine.lock().unwrap().ensure_idle() else {
            return; // no second core available
        };

        // The incoming track must sound like the rest of the album: same EQ
        // chain, same ReplayGain mode. Both are applied before the file loads,
        // so the very first frame is already filtered (stage 3 un-pauses it).
        let chain = {
            let eq = self.eq.lock().unwrap();
            let gains = eq.gains.clone();
            eq_af_chain(eq.preamp, &gains)
        };
        match &chain {
            Some(af) => {
                let _ = core.command(&["set", "af", af]);
            }
            None => {
                let _ = core.command(&["set", "af", ""]);
            }
        }
        let replaygain = *self.replaygain.lock().unwrap();
        let _ = core.command(&["set", "replaygain", replaygain.as_mpv()]);

        if core.command(&["loadfile", &path]).is_err() {
            self.clear_prefetch();
            return;
        }
        // Paused first: this is the stage-2 guarantee, and the fade below is the
        // only thing allowed to un-pause it.
        let _ = set_pause(&core, true);

        self.engine.lock().unwrap().set_idle_af(chain);
        iwaks_log::line(
            iwaks_log::Level::Debug,
            &format!("prefetched the next track ({window:.1}s window)"),
        );
        self.emit();

        // Stage 3: cross it with what is playing right now.
        self.start_fade(&core, incoming_slot, after, window);
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
        // The user's volume, not libmpv's: during a crossfade the cores are driven
        // below it by the ramp, and the slider must not follow them down
        // (issue #27).
        let volume = self.user_volume.load(Ordering::Relaxed).clamp(0, 100);
        let mute = api.get_flag("mute").unwrap_or(false);
        let speed = api.get_double("speed").unwrap_or(1.0);
        let sleep_remaining = self
            .sleep_deadline
            .lock()
            .unwrap()
            .map(|until| (until - Instant::now()).as_secs_f64().max(0.0));
        let (eq_preamp, eq) = {
            let e = self.eq.lock().unwrap();
            (e.preamp, e.gains.clone())
        };
        let stored_rg = *self.replaygain.lock().unwrap();
        let replaygain = api
            .get_string("replaygain")
            .map(|value| ReplayGainMode::from_mpv(&value))
            .unwrap_or(stored_rg);
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
            shuffle: queue.shuffle,
            queue_version: self.queue_version.load(Ordering::Relaxed),
            speed,
            sleep_remaining,
            eq_preamp,
            eq,
            replaygain,
            audio_device: api.get_string("audio-device").unwrap_or_default(),
            prefetched: self.engine.lock().unwrap().has_idle(),
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

    /// A `Player` with no libmpv handle, for exercising `shutdown`/`Drop`
    /// without a working audio backend.
    fn bare_player(thread: Option<JoinHandle<()>>, pump_thread: Option<ThreadId>) -> Player {
        Player {
            engine: Mutex::new(Engine::empty()),
            crossfade: Mutex::new(Crossfade::default()),
            crossfade_refusal_logged: AtomicBool::new(false),
            fade: Mutex::new(None),
            user_volume: AtomicI64::new(80),
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
                shuffle: false,
                queue_version: 1,
                speed: 1.0,
                sleep_remaining: None,
                eq_preamp: 0.0,
                eq: vec![0.0; EQ_BANDS.len()],
                replaygain: ReplayGainMode::Track,
                audio_device: String::new(),
                prefetched: false,
            }),

            tracks: Mutex::new(Vec::new()),
            queue: Mutex::new(Queue::new(0)),
            sink: Mutex::new(None),
            commands: Mutex::new(VecDeque::new()),
            sleep_deadline: Mutex::new(None),
            eq: Mutex::new(EqSettings::default()),
            replaygain: Mutex::new(ReplayGainMode::Track),
            shuffle_seed: AtomicU64::new(1),
            queue_version: AtomicU64::new(1),
            thread: Mutex::new(thread),
            pump_thread: Mutex::new(pump_thread),
            stopped: AtomicBool::new(false),
            shut_down: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }

    /// The last `Arc<Player>` can be released *on the pump thread* when `start`
    /// fails and the caller drops first. `Drop` → `shutdown` must then detach,
    /// not join the thread it is running on (which would hang forever).
    #[test]
    fn shutdown_on_pump_thread_detaches_instead_of_self_joining() {
        // Stand-in pump thread that stays alive until released; a buggy
        // `shutdown` would `join` it and block here.
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let pump = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });

        // Record *this* thread as the pump thread, so `shutdown` sees the
        // self-join condition.
        let player = bare_player(Some(pump), Some(std::thread::current().id()));

        let started = Instant::now();
        player.shutdown();
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "shutdown must detach, not join the still-running pump thread from itself"
        );

        let _ = release_tx.send(()); // let the detached thread exit cleanly
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
                audio_device: String::new(),
            },
            sink,
        )
        .unwrap()
    }

    /// A player on the **real** audio output.
    ///
    /// `ao=null` never enumerates anything, so a device test built on
    /// [`start_player`] cannot fail — its assertions run over an empty list. That
    /// is how the picker shipped showing nothing but "System default".
    fn start_live_audio_player() -> Result<Arc<Player>, String> {
        let sink: StateSink = Box::new(|_| {});
        Player::start(
            &Options {
                ao_null: false,
                audio_exclusive: false,
                audio_device: String::new(),
            },
            sink,
        )
    }

    /// Whether this machine may be asked to open real audio at all.
    ///
    /// Enumerating devices where no endpoint exists crashes libmpv
    /// (STATUS_ACCESS_VIOLATION), and a native crash cannot be caught from Rust,
    /// so the tests that need it opt in with `IWAKS_LIVE_AUDIO=1`. CI sets
    /// nothing and skips them.
    fn live_audio_requested() -> bool {
        std::env::var("IWAKS_LIVE_AUDIO").is_ok()
    }

    #[test]
    fn defaults_use_shared_wasapi_audio() {
        // Exclusive mode silences other apps for the whole session; shared is
        // the default so Discord/YouTube stay audible while Iwaks plays.
        assert!(
            !Options::default().audio_exclusive,
            "shared mode by default"
        );
        assert!(!Options::default().ao_null, "real audio by default");
    }

    #[test]
    fn shuffle_roundtrip_pins_current_and_persists() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("shuffle");
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        let c = dir.join("c.wav");
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
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("starts playing A");
        assert!(!player.snapshot().shuffle);

        player.set_shuffle(true);
        let s = wait_for(&player, Duration::from_secs(3), |s| s.shuffle).expect("shuffle enabled");
        assert_eq!(
            s.current.as_ref().map(|t| t.title.as_str()),
            Some("A"),
            "current track stays selected when shuffle turns on"
        );

        // Reshuffle keeps the current track playing.
        player.reshuffle();
        wait_for(&player, Duration::from_secs(3), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("reshuffle keeps current");

        // Playing a new list keeps shuffle on (fresh permutation), and the
        // requested track still starts.
        player.play_tracks(
            vec![
                track(b.to_str().unwrap(), "B", 2.0),
                track(c.to_str().unwrap(), "C", 2.0),
                track(a.to_str().unwrap(), "A", 2.0),
            ],
            1,
        );
        wait_for(&player, Duration::from_secs(5), |s| {
            s.shuffle && s.current.as_ref().map(|t| t.title.as_str()) == Some("C")
        })
        .expect("shuffle persists and requested track plays");

        player.set_shuffle(false);
        let s =
            wait_for(&player, Duration::from_secs(3), |s| !s.shuffle).expect("shuffle disabled");
        assert_eq!(
            s.current.as_ref().map(|t| t.title.as_str()),
            Some("C"),
            "current track stays when shuffle turns off"
        );
        player.shutdown();
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

    #[test]
    fn speed_property_roundtrip_and_clamp() {
        if !have_libmpv() {
            return;
        }
        let a = test_dir("speed").join("a.wav");
        write_wav(&a, 2.0);
        let tracks = vec![track(a.to_str().unwrap(), "A", 2.0)];

        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            !s.paused && s.duration > 0.2
        })
        .expect("starts playing");
        assert_eq!(player.snapshot().speed, 1.0, "default speed");

        player.set_speed(1.5);
        wait_for(&player, Duration::from_secs(2), |s| s.speed == 1.5)
            .expect("speed property applied");
        assert_eq!(player.snapshot().speed, 1.5);

        // Out-of-range values are clamped, mirroring set_volume.
        player.set_speed(-3.0);
        wait_for(&player, Duration::from_secs(2), |s| s.speed == 0.25)
            .expect("clamped to the floor");
        player.set_speed(99.0);
        wait_for(&player, Duration::from_secs(2), |s| s.speed == 4.0)
            .expect("clamped to the ceiling");
        player.shutdown();
    }

    #[test]
    fn sleep_timer_pauses_playback() {
        if !have_libmpv() {
            return;
        }
        let a = test_dir("sleep").join("a.wav");
        write_wav(&a, 2.0);
        let tracks = vec![track(a.to_str().unwrap(), "A", 2.0)];

        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            !s.paused && s.duration > 0.2
        })
        .expect("starts playing");

        player.set_sleep_timer(Some(0.3));
        let s = wait_for(&player, Duration::from_secs(3), |s| {
            s.paused && !s.stopped && s.current.is_some()
        })
        .expect("sleep timer pauses playback");
        assert_eq!(s.sleep_remaining, None, "timer consumed when it fires");
        player.shutdown();
    }

    #[test]
    fn sleep_timer_cancelled_keeps_playing() {
        if !have_libmpv() {
            return;
        }
        let a = test_dir("sleep-cancel").join("a.wav");
        write_wav(&a, 2.0);
        let tracks = vec![track(a.to_str().unwrap(), "A", 2.0)];

        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            !s.paused && s.duration > 0.2
        })
        .expect("starts playing");

        player.set_sleep_timer(Some(0.3));
        wait_for(&player, Duration::from_secs(2), |s| {
            s.sleep_remaining.is_some()
        })
        .expect("timer armed");
        player.set_sleep_timer(None);
        wait_for(&player, Duration::from_secs(2), |s| {
            s.sleep_remaining.is_none()
        })
        .expect("timer cancelled");

        // Wait past the original deadline: playback must keep going.
        std::thread::sleep(Duration::from_millis(600));
        assert!(!player.snapshot().paused, "cancelled timer must not pause");
        player.shutdown();
    }

    // ---- EQ + ReplayGain (slice 2) ----

    #[test]
    fn eq_af_chain_flat_is_none() {
        assert_eq!(eq_af_chain(0.0, &[0.0; EQ_BANDS.len()]), None);
        // Sub-0.05 dB values round to flat.
        assert_eq!(eq_af_chain(0.04, &[0.01; EQ_BANDS.len()]), None);
    }

    #[test]
    fn eq_af_chain_builds_lavfi_graph() {
        let mut gains = [0.0; EQ_BANDS.len()];
        gains[5] = 2.0; // 1 kHz band
        let chain = eq_af_chain(3.0, &gains).expect("non-flat gains produce a chain");
        assert!(chain.starts_with("lavfi=["));
        assert!(chain.contains("equalizer=f=1000:t=o:w=1:g=2"));
        assert!(chain.ends_with("volume=volume=3dB]"), "preamp applied last");
        assert_eq!(
            chain.matches("equalizer=").count(),
            1,
            "only the boosted band is included"
        );
    }

    #[test]
    fn eq_af_chain_clamps_and_rounds() {
        let mut gains = [0.0; EQ_BANDS.len()];
        gains[0] = 99.0;
        gains[1] = -0.04; // rounds to 0 dB → excluded
        let chain = eq_af_chain(-99.0, &gains).expect("chain for out-of-range preamp");
        assert!(chain.contains("equalizer=f=31:t=o:w=1:g=12"));
        assert!(chain.contains("volume=volume=-12dB"));
        assert!(!chain.contains("f=62"), "sub-0.05 band is dropped");
    }

    #[test]
    fn replaygain_mode_roundtrip() {
        if !have_libmpv() {
            return;
        }
        let player = start_player();
        // Startup option `replaygain=track` (design §4.2) is the default.
        wait_for(&player, Duration::from_secs(2), |s| {
            s.replaygain == ReplayGainMode::Track
        })
        .expect("default mode is Track");

        player.set_replaygain(ReplayGainMode::Album);
        wait_for(&player, Duration::from_secs(2), |s| {
            s.replaygain == ReplayGainMode::Album
        })
        .expect("switches to Album");
        player.set_replaygain(ReplayGainMode::Off);
        wait_for(&player, Duration::from_secs(2), |s| {
            s.replaygain == ReplayGainMode::Off
        })
        .expect("switches to Off");
        player.shutdown();
    }

    #[test]
    fn eq_applies_filter_and_clears_without_breaking_playback() {
        if !have_libmpv() {
            return;
        }
        let a = test_dir("eq").join("a.wav");
        write_wav(&a, 2.0);
        let tracks = vec![track(a.to_str().unwrap(), "A", 2.0)];

        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            !s.paused && s.duration > 0.2
        })
        .expect("starts playing");

        // Boost band 5 (1 kHz) + preamp, plus out-of-range values to prove
        // clamping and padding happen.
        let mut gains = vec![0.0; EQ_BANDS.len()];
        gains[5] = 2.5;
        player.set_eq(99.0, gains);
        wait_for(&player, Duration::from_secs(2), |s| {
            s.eq_preamp == 12.0 && s.eq[5] == 2.5
        })
        .expect("EQ stored with clamped preamp");

        // A short (2 s) gain array is padded with zeros to 10 bands.
        player.set_eq(0.0, vec![1.0, -2.0]);
        wait_for(&player, Duration::from_secs(2), |s| {
            s.eq_preamp == 0.0 && s.eq.len() == EQ_BANDS.len() && s.eq[1] == -2.0 && s.eq[9] == 0.0
        })
        .expect("padded to 10 bands");

        // Playback keeps advancing while the filter chain is active…
        let base = player.snapshot().position;
        wait_for(&player, Duration::from_secs(2), |s| {
            s.position > base + 0.25
        })
        .expect("position advances with the lavfi chain");

        // …and survives clearing back to flat (`set af ""`).
        player.set_eq(0.0, vec![0.0; EQ_BANDS.len()]);
        wait_for(&player, Duration::from_secs(2), |s| {
            s.eq.iter().all(|g| *g == 0.0) && s.eq_preamp == 0.0
        })
        .expect("EQ reset to flat");
        let base = player.snapshot().position;
        wait_for(&player, Duration::from_secs(2), |s| {
            !s.paused && s.position > base + 0.25
        })
        .expect("playback intact after clearing the chain");
        player.shutdown();
    }

    // ---- session queue: order read + manual reorder (M4 slice 2) ----

    #[test]
    fn reorder_queue_changes_upcoming_order_without_touching_playback() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("reorder");
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        let c = dir.join("c.wav");
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
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("starts playing A");

        // Move C (queue position 2) up to position 1 → order A, C, B.
        player.reorder_queue(2, 1);
        let titles: Vec<String> = player
            .queue_tracks()
            .iter()
            .map(|t| t.title.clone())
            .collect();
        assert_eq!(titles, ["A", "C", "B"]);
        assert_eq!(
            player.snapshot().current.as_ref().map(|t| t.title.as_str()),
            Some("A"),
            "reorder never changes the playing track"
        );

        // The queue walks the new order: next goes to C, not B.
        player.next();
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("C")
        })
        .expect("next follows the reordered queue");
        player.shutdown();
    }

    #[test]
    fn queue_tracks_reflects_reorder_with_shuffle_on() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("reorder-shuffle");
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        let c = dir.join("c.wav");
        let d = dir.join("d.wav");
        for p in [&a, &b, &c, &d] {
            write_wav(p, 2.0);
        }
        let tracks = vec![
            track(a.to_str().unwrap(), "A", 2.0),
            track(b.to_str().unwrap(), "B", 2.0),
            track(c.to_str().unwrap(), "C", 2.0),
            track(d.to_str().unwrap(), "D", 2.0),
        ];

        let player = start_player();
        player.play_tracks(tracks, 0);
        player.set_shuffle(true);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.shuffle && s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("shuffle on, current pinned to the front");

        let before = player.queue_tracks();
        assert_eq!(before.len(), 4);

        // Move the element at position 3 to position 1: [b0, b3, b1, b2].
        player.reorder_queue(3, 1);
        let after = player.queue_tracks();
        assert_eq!(after.len(), 4);
        assert_eq!(after[1].path, before[3].path, "moved element lands at 1");
        assert_eq!(after[2].path, before[1].path, "shifted up one");
        assert_eq!(after[3].path, before[2].path, "shifted up one");
        assert_eq!(
            player.snapshot().current.as_ref().map(|t| t.title.as_str()),
            Some("A"),
            "current stays pinned at the front"
        );
        player.shutdown();
    }

    // ---- session queue: manual remove (issue #19) ----

    /// Three 2-second tracks in a temp dir, for the remove/clear tests.
    fn remove_fixture(name: &str) -> (Vec<Track>, TempDirLike) {
        let dir = test_dir(name);
        let paths: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|n| dir.join(format!("{n}.wav")))
            .collect();
        for p in &paths {
            write_wav(p, 2.0);
        }
        let titles = ["A", "B", "C"];
        let tracks = paths
            .iter()
            .zip(titles)
            .map(|(p, t)| track(p.to_str().unwrap(), t, 2.0))
            .collect();
        (tracks, TempDirLike(dir))
    }

    /// Keeps the temp dir alive for the duration of a test.
    struct TempDirLike(std::path::PathBuf);
    impl Drop for TempDirLike {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn titles_of(player: &Player) -> Vec<String> {
        player
            .queue_tracks()
            .iter()
            .map(|t| t.title.clone())
            .collect()
    }

    #[test]
    fn removing_a_queued_track_keeps_playing_and_drops_it_from_the_order() {
        if !have_libmpv() {
            return;
        }
        let (tracks, _dir) = remove_fixture("remove-other");
        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("starts playing A");

        // Drop the upcoming B; A keeps playing and C takes its slot.
        player.remove_queue_at(1);
        assert_eq!(titles_of(&player), ["A", "C"]);
        assert_eq!(
            player.snapshot().current.as_ref().map(|t| t.title.as_str()),
            Some("A"),
            "removing another track never interrupts playback"
        );
        // The published state catches up once the pump drains the refresh.
        wait_for(&player, Duration::from_secs(5), |s| s.list_len == 2)
            .expect("state reports the shorter queue");

        player.next();
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("C")
        })
        .expect("next skips the removed track");
        player.shutdown();
    }

    #[test]
    fn removing_the_playing_track_hands_over_to_the_following_one() {
        if !have_libmpv() {
            return;
        }
        let (tracks, _dir) = remove_fixture("remove-current");
        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("A")
        })
        .expect("starts playing A");

        // Removing the current track advances to B, which slid into slot 0.
        player.remove_queue_at(0);
        assert_eq!(titles_of(&player), ["B", "C"]);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("B")
        })
        .expect("playback hands over to the next track");
        player.shutdown();
    }

    #[test]
    fn removing_the_last_playing_track_stops_and_leaves_no_current() {
        if !have_libmpv() {
            return;
        }
        let (tracks, _dir) = remove_fixture("remove-last");
        let player = start_player();
        player.play_tracks(tracks, 2);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("C")
        })
        .expect("starts playing C");

        player.remove_queue_at(2);
        // Only the playing track is gone; the earlier two stay in the queue.
        assert_eq!(titles_of(&player), ["A", "B"]);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.list_len == 2 && s.current.is_none()
        })
        .expect("queue keeps its other tracks but nothing is selected");
        player.shutdown();
    }

    #[test]
    fn clearing_the_queue_stops_playback_but_keeps_repeat_and_shuffle() {
        if !have_libmpv() {
            return;
        }
        let (tracks, _dir) = remove_fixture("clear-queue");
        let player = start_player();
        player.play_tracks(tracks, 0);
        player.set_shuffle(true);
        player.set_repeat(RepeatMode::All);
        wait_for(&player, Duration::from_secs(5), |s| {
            s.shuffle && s.current.is_some()
        })
        .expect("playing with shuffle on");

        player.clear_queue();
        assert!(player.queue_tracks().is_empty());
        wait_for(&player, Duration::from_secs(5), |s| {
            s.list_len == 0 && s.current.is_none() && s.shuffle && s.repeat == RepeatMode::All
        })
        .expect("queue emptied and stopped, with repeat + shuffle kept");

        // A new list still comes up shuffled, proving the preference survived.
        let (more, _d2) = remove_fixture("clear-queue-replay");
        player.play_tracks(more, 0);
        wait_for(&player, Duration::from_secs(5), |s| s.shuffle)
            .expect("shuffle survived the clear");
        player.shutdown();
    }

    #[test]
    fn removing_out_of_range_positions_is_a_no_op() {
        let queue = Queue::new(3).start(1);
        assert_eq!(queue.without_position(9), queue);
        assert_eq!(queue.without_position(3), queue);
        assert_eq!(Queue::new(0).without_position(0).len, 0);
        assert_eq!(queue.track_at(9), None);
    }

    // ---- audio output (#23, #24) ----

    #[test]
    fn listing_devices_without_a_pump_answers_instead_of_hanging() {
        // The timeout guard, tested without libmpv: a player that never started
        // has no pump to answer, so `list_devices` must give up and report an
        // empty list rather than blocking the UI thread forever.
        let player = bare_player(None, None);
        let before = Instant::now();
        let devices = player.list_devices();
        let waited = before.elapsed();
        assert!(devices.is_empty(), "nothing answered");
        assert!(
            waited >= DEVICE_TIMEOUT,
            "it waited for the answer rather than returning early: {waited:?}"
        );
        assert!(
            waited < DEVICE_TIMEOUT * 3,
            "and it gave up instead of waiting forever: {waited:?}"
        );
        // Queuing a switch on a player with no pump is equally harmless.
        player.set_audio_device("anything");
        player.shutdown();
    }

    #[test]
    fn the_device_list_is_read_from_mpvs_node_tree() {
        if !have_libmpv() || !live_audio_requested() {
            return;
        }
        let player = start_live_audio_player().expect("a real output opens here");

        // Before anything plays. This is the state the picker is in when the user
        // opens Settings on a fresh launch, and the state in which it showed
        // nothing but "System default".
        let before = player.list_devices();
        eprintln!("before playback: {} devices", before.len());
        assert!(
            !before.is_empty(),
            "mpv enumerates devices before anything is played — Settings is read before playback"
        );

        let dir = test_dir("device-list");
        let a = dir.join("a.wav");
        write_wav(&a, 2.0);
        let tracks = vec![track(a.to_str().unwrap(), "A", 2.0)];
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| {
            !s.paused && s.duration > 0.2
        })
        .expect("the track plays, so an audio output exists now");

        let devices = player.list_devices();
        eprintln!("after playback: {devices:?}");
        assert!(
            !devices.is_empty(),
            "mpv offers the devices of a machine that is playing audio"
        );
        assert!(
            devices.iter().any(|d| !d.name.is_empty()),
            "and at least one of them is selectable: {devices:?}"
        );
        // The default entry has no name; every listed entry must still say
        // something, or the picker would render a blank row.
        assert!(
            devices
                .iter()
                .all(|d| !d.name.is_empty() || !d.description.is_empty()),
            "every entry carries something to show: {devices:?}"
        );

        // And the picker has to be able to do anything with those names. Only
        // names mpv itself listed are ever sent back to it — a fabricated one
        // makes libmpv open an output that cannot exist, which crashed the whole
        // test binary before (STATUS_ACCESS_VIOLATION).
        let Some(target) = first_selectable_device(&player) else {
            panic!("mpv listed devices but none is selectable: {devices:?}");
        };
        player.set_audio_device(&target);
        let switched = wait_for(&player, Duration::from_secs(5), |s| {
            s.audio_device == target
        })
        .expect("the chosen device is the one mpv reports");
        assert_eq!(
            switched.audio_device, target,
            "and the choice survives into the published state"
        );
        player.shutdown();
    }

    #[test]
    fn device_list_is_read_from_the_pump_and_answers_the_caller() {
        // Enumerating needs a machine with a real audio endpoint: libmpv itself
        // crashes (STATUS_ACCESS_VIOLATION) where none exists, and a native
        // crash cannot be caught from Rust. CI has no endpoint, so this opts in
        // via IWAKS_LIVE_AUDIO, exactly like the node-read test above.
        if !have_libmpv() || !live_audio_requested() {
            return;
        }
        let player = start_player();
        // A machine with no audio endpoint reports an empty list; what must
        // hold everywhere is that the pump answers rather than hangs.
        let devices = player.list_devices();
        assert!(
            devices
                .iter()
                .all(|d| !d.name.is_empty() || !d.description.is_empty()),
            "every entry carries something to show: {devices:?}"
        );
        // Second call works too: the pump stays responsive after answering.
        assert_eq!(player.list_devices().len(), devices.len());
        player.shutdown();
    }

    /// The first device mpv actually offers, or `None` when it offers none.
    ///
    /// Tests must never invent a device name. `set audio-device <fabricated>`
    /// makes libmpv try to re-open an audio output that cannot exist, and that
    /// crashed the whole test binary with STATUS_ACCESS_VIOLATION on the CI
    /// runner — a native crash cannot be caught from Rust. Only names mpv
    /// itself listed are safe to send.
    fn first_selectable_device(player: &Player) -> Option<String> {
        let devices = player.list_devices();
        let picked = devices
            .iter()
            .find(|d| !d.name.is_empty())
            .map(|d| d.name.clone());
        // Say which way it went: a test that quietly asserts nothing is worse
        // than one that says why it could not.
        match &picked {
            Some(name) => eprintln!("using audio device {name:?} from {devices:?}"),
            None => eprintln!("SKIP: mpv reported no selectable device ({devices:?})"),
        }
        picked
    }

    #[test]
    fn choosing_a_device_is_reflected_in_the_published_state() {
        if !have_libmpv() || !live_audio_requested() {
            return;
        }
        let player = start_player();
        assert_eq!(
            player.snapshot().audio_device,
            "",
            "no device stored means the OS picks"
        );

        // Nothing to select on a machine with no audio endpoint — the state
        // assertion below is still worth making, but there is no device to
        // ask mpv to switch to.
        let Some(name) = first_selectable_device(&player) else {
            assert_eq!(player.snapshot().audio_device, "");
            player.shutdown();
            return;
        };

        player.set_audio_device(&name);
        wait_for(&player, Duration::from_secs(5), |s| s.audio_device == name)
            .expect("state reports the selected device");
        player.shutdown();
    }

    #[test]
    fn the_system_default_choice_does_not_wedge_the_playback_thread() {
        if !have_libmpv() || !live_audio_requested() {
            return;
        }
        let player = start_player();
        if let Some(name) = first_selectable_device(&player) {
            player.set_audio_device(&name);
            wait_for(&player, Duration::from_secs(5), |s| s.audio_device == name)
                .expect("device selected");
            // Empty means "auto" on the mpv side. mpv resolves that to whatever
            // it picked and need not report an empty name back.
            player.set_audio_device("");
        }
        // With no audio endpoint the two calls above are skipped entirely —
        // re-opening an audio output that cannot exist is what crashed CI.
        // What must hold either way: the pump still serves commands afterwards.
        player.set_volume(42);
        wait_for(&player, Duration::from_secs(5), |s| s.volume == 42)
            .expect("pump still serving commands after a device switch");
        player.shutdown();
    }

    #[test]
    fn a_device_entry_becomes_a_pickable_device() {
        // The shape mpv actually returns for `audio-device-list`: an array whose
        // elements are maps of `name` / `description` strings. Pure, so it runs
        // everywhere — the bug it guards was that the whole node tree was never
        // read at all, which no assertion here could have seen.
        let entry = vec![
            ("name".to_string(), "wasapi/{04c02c64}".to_string()),
            ("description".to_string(), "Speakers".to_string()),
        ];
        assert_eq!(
            device_from_entry(&entry),
            Some(AudioDevice {
                name: "wasapi/{04c02c64}".to_string(),
                description: "Speakers".to_string()
            })
        );
    }

    #[test]
    fn a_map_of_maps_entry_is_read_through_its_outer_key() {
        // mpv has shipped this property keyed by its own option name. The real
        // fields sit after it, which is why the lookup takes the *last* match.
        let entry = vec![
            ("wasapi/0".to_string(), String::new()),
            ("name".to_string(), "wasapi/a".to_string()),
            ("description".to_string(), "Headphones".to_string()),
        ];
        assert_eq!(
            device_from_entry(&entry),
            Some(AudioDevice {
                name: "wasapi/a".to_string(),
                description: "Headphones".to_string()
            })
        );
    }

    #[test]
    fn the_system_default_entry_is_left_to_the_picker() {
        // mpv lists its own automatic device first. The picker already renders a
        // "System default" row with an empty value; keeping this one would show
        // the same destination twice, and both as different select values.
        let auto = vec![
            ("name".to_string(), "auto".to_string()),
            ("description".to_string(), "Autoselect device".to_string()),
        ];
        assert_eq!(device_from_entry(&auto), None, "auto is the empty choice");
        // An entry with no name at all is the same case, spelled differently.
        let unnamed = vec![
            ("name".to_string(), String::new()),
            ("description".to_string(), "System default".to_string()),
        ];
        assert_eq!(device_from_entry(&unnamed), None);
        // Nothing usable: not a device, and must not become a blank row.
        assert_eq!(device_from_entry(&[]), None);
        assert_eq!(
            device_from_entry(&[("flags".to_string(), "default".to_string())]),
            None
        );
    }

    // ---- crossfade prefetch (issue #27, stage 2) ----

    /// Two tracks: A of `a_secs`, B a short 2 s follow-up. A prefetch is due once A
    /// has `3 - window` seconds left, so a *long* A keeps the tests that only
    /// care about the prefetch clear of the ramp's midpoint — that handover lands
    /// `window / 2` after the prefetch, and a test that misses it would see the
    /// new track instead of the old one. The dir is returned so it outlives the
    /// test.
    fn crossfade_fixture(name: &str, a_secs: f64) -> (PathBuf, Vec<Track>) {
        let dir = test_dir(name);
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        write_wav(&a, a_secs);
        write_wav(&b, 2.0);
        let tracks = vec![
            track(a.to_str().unwrap(), "A", a_secs),
            track(b.to_str().unwrap(), "B", 2.0),
        ];
        (dir, tracks)
    }

    #[test]
    fn crossfade_is_off_by_default_so_nothing_is_prefetched() {
        if !have_libmpv() {
            return;
        }
        assert!(
            !Crossfade::default().enabled,
            "design 4.2: crossfade is opt-in"
        );
        let (_dirs, tracks) = crossfade_fixture("xf-off", 2.0);
        let player = start_player();
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(5), |s| s.current.is_some()).expect("starts playing");

        // The fixture is short enough that A is already inside a 3 s window; with
        // crossfade off that must still never prefetch. A negative assertion
        // needs a bounded wait, so give the pump several of its ticks.
        std::thread::sleep(Duration::from_millis(700));
        assert!(!player.has_prefetched(), "nothing prefetched while off");
        assert!(!player.snapshot().prefetched);
        player.shutdown();
    }

    #[test]
    fn enabling_crossfade_prefetches_the_next_track_without_playing_it() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-prefetch", 6.0);
        let player = start_player();
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks, 0);

        let state = wait_for(&player, Duration::from_secs(10), |s| s.prefetched)
            .expect("the next track gets prefetched inside the window");
        // The whole point of this stage: prefetching is silent.
        assert_eq!(
            state.current.as_ref().map(|t| t.title.as_str()),
            Some("A"),
            "prefetching must not change what is playing"
        );
        assert_eq!(state.index, Some(0), "and must not advance the queue");
        player.shutdown();
    }

    #[test]
    fn a_prefetched_core_is_given_the_current_eq() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-eq", 6.0);
        let player = start_player();
        // A real (non-flat) curve, so "same EQ as the audible core" is a
        // statement about content rather than two empty strings.
        let gains = vec![4.0, 3.0, 1.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 4.0];
        player.set_eq(-2.0, gains.clone());
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks, 0);

        wait_for(&player, Duration::from_secs(10), |s| s.prefetched).expect("prefetched");
        let applied = player
            .engine
            .lock()
            .unwrap()
            .idle_af()
            .expect("a prefetched core has an af chain")
            .to_string();
        assert_eq!(
            applied,
            eq_af_chain(-2.0, &gains).expect("the chain we asked for"),
            "the incoming track is filtered exactly like the current one"
        );
        player.shutdown();
    }

    #[test]
    fn changing_the_queue_drops_a_prefetched_track() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-drop", 6.0);
        let player = start_player();
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks.clone(), 0);
        wait_for(&player, Duration::from_secs(10), |s| s.prefetched).expect("prefetched");

        // A stale prefetch is the one thing stage 3 must never fade into.
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(10), |s| !s.prefetched)
            .expect("a new list drops the prefetched track");
        player.shutdown();
    }

    #[test]
    fn turning_crossfade_off_releases_the_prefetched_core() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-turn-off", 6.0);
        let player = start_player();
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(10), |s| s.prefetched).expect("prefetched");

        player.set_crossfade(Crossfade {
            enabled: false,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        wait_for(&player, Duration::from_secs(10), |s| !s.prefetched)
            .expect("switching crossfade off leaves no core behind");
        player.shutdown();
    }

    // ---- crossfade ramp (issue #27, stage 3) ----

    #[test]
    fn the_ramp_hands_the_session_to_the_incoming_track() {
        if !have_libmpv() {
            return;
        }
        // A 2 s A with a 1 s crossfade: the prefetch lands 1 s in, the handover
        // half a second later, the ramp ends as A does.
        let (_dirs, tracks) = crossfade_fixture("xf-ramp", 2.0);
        let player = start_player();
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: 1.0,
        });
        player.play_tracks(tracks, 0);

        let state = wait_for(&player, Duration::from_secs(10), |s| {
            s.current.as_ref().map(|t| t.title.as_str()) == Some("B") && s.index == Some(1)
        })
        .expect("the incoming track takes over at the midpoint");
        assert!(!state.paused, "the incoming track is playing, not waiting");
        assert!(
            state.position > 0.0 && state.position < 1.0,
            "the new track is heard from its start, got {:?}",
            state.position
        );

        // Once the ramp is over the session is back to a single core: the fading
        // one was stopped and released, so nothing is left playing behind the
        // track the user is looking at.
        wait_for(&player, Duration::from_secs(10), |_| {
            player.fade.lock().unwrap().is_none()
        })
        .expect("the ramp lands");
        assert_eq!(
            player.engine.lock().unwrap().live_slots().len(),
            1,
            "no core is left over once the fade is done"
        );
        player.shutdown();
    }

    #[test]
    fn interrupting_a_crossfade_leaves_one_core_playing() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-interrupt", 6.0);
        let player = start_player();
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(10), |s| s.prefetched).expect("prefetched");

        // Switching crossfade off mid-ramp has to silence the fading core too:
        // leaving it running would be an invisible track the user cannot stop.
        player.set_crossfade(Crossfade {
            enabled: false,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        wait_for(&player, Duration::from_secs(10), |_| {
            !player.has_prefetched() && player.fade.lock().unwrap().is_none()
        })
        .expect("the ramp is abandoned");
        assert_eq!(
            player.engine.lock().unwrap().live_slots().len(),
            1,
            "only the core the user is listening to survives"
        );
        player.shutdown();
    }

    #[test]
    fn pausing_during_a_crossfade_freezes_the_ramp() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-pause", 6.0);
        let player = start_player();
        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(10), |s| s.prefetched).expect("prefetched");

        player.toggle_play();
        wait_for(&player, Duration::from_secs(5), |s| s.paused).expect("paused");
        // Well past the half of the window still to run. A ramp on wall-clock
        // time would have finished, swapped the queue, and left the incoming
        // track audible under a player the user believes is paused.
        std::thread::sleep(Duration::from_millis(1800));
        assert!(
            player.fade.lock().unwrap().is_some(),
            "a paused player does not run the fade out"
        );
        assert_eq!(
            player.snapshot().current.as_ref().map(|t| t.title.as_str()),
            Some("A"),
            "and the queue has not moved on"
        );
        player.shutdown();
    }

    #[test]
    fn the_volume_ramp_is_equal_power() {
        // Pure maths: two signals that sum linearly dip audibly in the middle,
        // which is exactly what a crossfade must not do.
        let (out_start, in_start) = equal_power(0.0);
        assert!((out_start - 1.0).abs() < 1e-9 && in_start.abs() < 1e-9);
        let (out_mid, in_mid) = equal_power(0.5);
        assert!(
            (out_mid * out_mid + in_mid * in_mid - 1.0).abs() < 1e-9,
            "the pair holds constant power at the midpoint"
        );
        let (out_end, in_end) = equal_power(1.0);
        assert!(out_end.abs() < 1e-9 && (in_end - 1.0).abs() < 1e-9);
        // Monotonic: the outgoing side only ever gets quieter.
        let mut previous = 1.0;
        for step in 0..=20 {
            let (out, _) = equal_power(step as f64 / 20.0);
            assert!(out <= previous + 1e-9, "step {step} went backwards");
            previous = out;
        }
    }

    #[test]
    fn exclusive_output_refuses_to_prefetch() {
        if !have_libmpv() {
            return;
        }
        let (_dirs, tracks) = crossfade_fixture("xf-exclusive", 6.0);
        let sink: StateSink = Box::new(|_| {});
        // `ao=null` keeps this off any real device — CI has no audio endpoint at
        // all — so the only thing under test is that the session refuses to
        // open a second core when it is exclusive.
        let player = Player::start(
            &Options {
                ao_null: true,
                audio_exclusive: true,
                audio_device: String::new(),
            },
            sink,
        )
        .expect("an exclusive session still starts (ao=null ignores it)");
        assert!(
            player.engine.lock().unwrap().is_exclusive(),
            "the session knows it is exclusive"
        );

        player.set_crossfade(Crossfade {
            enabled: true,
            seconds: DEFAULT_CROSSFADE_SECONDS,
        });
        player.play_tracks(tracks, 0);
        wait_for(&player, Duration::from_secs(10), |s| {
            !s.paused && s.duration > 0.2
        })
        .expect("the track still plays");
        // A negative assertion, so it needs a bounded wait: several pump ticks
        // with the track well inside the fade window.
        std::thread::sleep(Duration::from_millis(1500));
        assert!(
            !player.has_prefetched(),
            "a second core cannot open an exclusive device"
        );
        assert_eq!(
            player.engine.lock().unwrap().live_slots().len(),
            1,
            "and none was created"
        );
        player.shutdown();
    }

    #[test]
    fn crossfade_preferences_are_validated() {
        // Pure logic, no libmpv.
        assert_eq!(Crossfade::default().window(), None, "off means no window");
        let on = Crossfade {
            enabled: true,
            seconds: 3.0,
        };
        assert_eq!(on.window(), Some(3.0));
        // A silly or broken duration still produces a usable window rather than
        // a negative one (which would prefetch after the track had ended).
        assert_eq!(
            Crossfade {
                enabled: true,
                seconds: -5.0
            }
            .window(),
            Some(MIN_CROSSFADE_SECONDS)
        );
        assert_eq!(
            Crossfade {
                enabled: true,
                seconds: f64::NAN
            }
            .window(),
            None,
            "NaN is not a duration"
        );
        assert_eq!(DEFAULT_CROSSFADE_SECONDS, 3.0, "the plan's D1 value");
    }

    #[test]
    fn reshuffle_bumps_queue_version_so_the_view_can_refetch() {
        if !have_libmpv() {
            return;
        }
        let dir = test_dir("reshuffle-version");
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        let c = dir.join("c.wav");
        let d = dir.join("d.wav");
        for p in [&a, &b, &c, &d] {
            write_wav(p, 2.0);
        }
        let tracks = vec![
            track(a.to_str().unwrap(), "A", 2.0),
            track(b.to_str().unwrap(), "B", 2.0),
            track(c.to_str().unwrap(), "C", 2.0),
            track(d.to_str().unwrap(), "D", 2.0),
        ];

        let player = start_player();
        player.play_tracks(tracks, 0);
        player.set_shuffle(true);
        wait_for(&player, Duration::from_secs(5), |s| s.shuffle).expect("shuffle on");

        let before = player.snapshot();
        player.reshuffle();
        // A reshuffle leaves index/listLen/shuffle untouched — `queue_version`
        // is the Queue view's only cue to refetch.
        let after = wait_for(&player, Duration::from_secs(5), |s| {
            s.queue_version > before.queue_version
        })
        .expect("reshuffle advances queue_version");
        assert_eq!(after.index, before.index, "current position is unchanged");
        assert_eq!(after.list_len, before.list_len);
        assert!(after.shuffle);
        player.shutdown();
    }
}
