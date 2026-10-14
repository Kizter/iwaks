
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
    let s = wait_for(&player, Duration::from_secs(3), |s| !s.shuffle).expect("shuffle disabled");
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
        s.current.as_ref().map(|t| t.title.as_str()) == Some("A") && !s.paused && s.duration > 0.2
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

    let s =
        wait_for(&player, Duration::from_secs(6), |s| s.stopped).expect("reaches end and stops");
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
    wait_for(&player, Duration::from_secs(2), |s| s.speed == 1.5).expect("speed property applied");
    assert_eq!(player.snapshot().speed, 1.5);

    // Out-of-range values are clamped, mirroring set_volume.
    player.set_speed(-3.0);
    wait_for(&player, Duration::from_secs(2), |s| s.speed == 0.25).expect("clamped to the floor");
    player.set_speed(99.0);
    wait_for(&player, Duration::from_secs(2), |s| s.speed == 4.0).expect("clamped to the ceiling");
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
    wait_for(&player, Duration::from_secs(5), |s| s.shuffle).expect("shuffle survived the clear");
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
fn a_short_next_track_never_plays_silence() {
    // Regression for code-review #30: the fade window is clamped against the
    // incoming track's own length, so a next track shorter than half the
    // window is a plain cut, not a ramp into a dead core. Before the clamp
    // the ramp un-paused it, its EOF landed before the midpoint and was
    // dropped, and the handover adopted a finished core: index 1, position
    // 0, no auto-advance — silence.
    if !have_libmpv() {
        return;
    }
    let dir = test_dir("xf-short-incoming");
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_wav(&a, 6.0);
    write_wav(&b, 0.9); // shorter than half the 3 s window
    let tracks = vec![
        track(a.to_str().unwrap(), "A", 6.0),
        track(b.to_str().unwrap(), "B", 0.9),
    ];
    let player = start_player();
    player.set_crossfade(Crossfade {
        enabled: true,
        seconds: DEFAULT_CROSSFADE_SECONDS,
    });
    player.play_tracks(tracks, 0);

    // The sequencer has two legitimate paths here — when mpv already knows
    // the incoming length, `start_fade` refuses a window under the floor
    // and this is a plain cut; when it does not yet, the ramp starts and
    // the EOF safety net abandons it. Both must end the same way: the queue
    // plays B and finishes. The pre-fix bug hung instead — the dropped
    // incoming EOF left a dead adopted core, so playback never stopped.
    wait_for(&player, Duration::from_secs(20), |s| s.stopped)
        .expect("the queue finishes instead of hanging on a dead core");
    player.shutdown();
}

#[test]
fn a_short_middle_track_rearms_the_next_crossfade() {
    // Regression for code-review #30: the next-next track is prefetched
    // while the previous ramp is still draining (issue #27's third core),
    // which `start_fade` refuses ("one ramp at a time"), leaving that core
    // parked. `finish_fade` now drops the parked core and re-decides, so
    // the short middle track still fades into what follows instead of a
    // hard cut with crossfade left wedged.
    if !have_libmpv() {
        return;
    }
    let dir = test_dir("xf-short-middle");
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    let c = dir.join("c.wav");
    write_wav(&a, 6.0);
    write_wav(&b, 4.0); // short enough that C is prefetched mid-ramp-A→B
    write_wav(&c, 6.0);
    let tracks = vec![
        track(a.to_str().unwrap(), "A", 6.0),
        track(b.to_str().unwrap(), "B", 4.0),
        track(c.to_str().unwrap(), "C", 6.0),
    ];
    let player = start_player();
    player.set_crossfade(Crossfade {
        enabled: true,
        seconds: DEFAULT_CROSSFADE_SECONDS,
    });
    player.play_tracks(tracks, 0);

    // B is current and has played past where A's ramp could possibly still
    // be draining (it ends ≤ 2 s of window after B takes over, clamped to
    // B's own length). If the next ramp did not re-arm, this is the exact
    // pause where the third core sits parked doing nothing.
    wait_for(&player, Duration::from_secs(12), |s| {
        s.index == Some(1) && s.position > 1.6
    })
    .expect("B is past A's ramp");
    assert!(
        player.engine.lock().unwrap().is_fading(),
        "the fade into C is re-armed, not killed by the parked core"
    );
    wait_for(&player, Duration::from_secs(10), |s| {
        s.current.as_ref().map(|t| t.title.as_str()) == Some("C")
    })
    .expect("C takes over from B");
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
