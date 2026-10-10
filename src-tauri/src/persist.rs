//! Persisting player state across restarts (issue #01).
//!
//! Writes are coalesced: a burst of slider events marks the state dirty and a
//! single database write happens `DEBOUNCE` after the last one, so dragging the
//! volume never writes once per pixel. The mapping between the player's
//! [`PlayerState`] and the storage structs lives here, since this is the only
//! layer that knows both crates.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use iwaks_core::track::Track;
use iwaks_library::db::Library;
use iwaks_library::persist::{PlayerSettings, SessionSnapshot};
use iwaks_player::{Player, PlayerState, RepeatMode, ReplayGainMode};

/// How long the writer waits for the event burst to settle before writing.
const DEBOUNCE: Duration = Duration::from_millis(700);

/// Handle that tells the debounce thread player state changed. Cheap and
/// non-blocking; the write happens later and is skipped when nothing plays.
pub struct Persist {
    tx: mpsc::Sender<()>,
}

impl Persist {
    /// Spawn the debounce writer over a shared player handle. The thread ends
    /// when this handle (the only sender) is dropped.
    pub fn spawn(db_path: PathBuf, player: Arc<Mutex<Option<Arc<Player>>>>) -> Self {
        let (tx, rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            // Each `recv` opens a burst; drain until it goes quiet, then write.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                write_now(&db_path, &player);
            }
        });
        Self { tx }
    }

    /// Mark the player state dirty (coalesced into the next write).
    pub fn touch(&self) {
        let _ = self.tx.send(());
    }
}

/// Write the current settings + session synchronously. Called by the debounce
/// worker and by the final flush on exit.
pub fn write_now(db_path: &Path, player: &Arc<Mutex<Option<Arc<Player>>>>) {
    let handle = player.lock().unwrap().clone();
    let Some(p) = handle else { return };
    let Ok(lib) = Library::open(&db_path.to_string_lossy()) else {
        return;
    };
    let state = p.snapshot();
    let _ = lib.save_player_settings(&settings_from(&state));
    if let Some(session) = session_from(&state, p.queue_tracks()) {
        let _ = lib.save_session(&session);
    }
}

/// Map a live player snapshot to the persisted settings struct.
pub fn settings_from(state: &PlayerState) -> PlayerSettings {
    PlayerSettings {
        volume: state.volume,
        mute: state.mute,
        repeat: repeat_name(state.repeat).to_string(),
        shuffle: state.shuffle,
        speed: state.speed,
        replaygain: replaygain_name(state.replaygain).to_string(),
        eq_preamp: state.eq_preamp,
        eq: state.eq.clone(),
        sleep_seconds: state.sleep_remaining,
    }
}

/// Build the session snapshot, or `None` when there is nothing to resume.
pub fn session_from(state: &PlayerState, queue: Vec<Track>) -> Option<SessionSnapshot> {
    let current = state.current.as_ref()?;
    if queue.is_empty() {
        return None;
    }
    // The queue is already in play order; find the current file inside it so
    // the shuffle permutation is preserved.
    let index = queue
        .iter()
        .position(|t| t.path == current.path)
        .unwrap_or(0);
    Some(SessionSnapshot {
        paths: queue.into_iter().map(|t| t.path).collect(),
        index,
        position: state.position,
        shuffle: state.shuffle,
    })
}

/// Apply persisted settings to a freshly started player.
pub fn apply_settings(player: &Player, settings: &PlayerSettings) {
    player.set_volume(settings.volume);
    player.set_mute(settings.mute);
    player.set_speed(settings.speed);
    player.set_repeat(parse_repeat(&settings.repeat));
    player.set_shuffle(settings.shuffle);
    player.set_replaygain(parse_replaygain(&settings.replaygain));
    player.set_eq(settings.eq_preamp, settings.eq.clone());
    player.set_sleep_timer(settings.sleep_seconds);
}

/// Rebuild the queue from `lib` and resume the stored position. Tracks whose
/// files are gone are skipped; the current track follows to its new slot.
pub fn restore_session(player: &Player, lib: &Library, session: &SessionSnapshot) {
    let Ok(all) = lib.all_tracks() else { return };
    let by_path: HashMap<&str, &Track> = all.iter().map(|t| (t.path.as_str(), t)).collect();
    let mut tracks = Vec::new();
    let mut index = 0;
    for (i, path) in session.paths.iter().enumerate() {
        if let Some(track) = by_path.get(path.as_str()) {
            if i == session.index {
                index = tracks.len();
            }
            tracks.push((*track).clone());
        }
    }
    if tracks.is_empty() {
        return;
    }
    player.play_tracks(tracks, index);
    if session.position > 0.0 {
        player.seek(session.position);
    }
}

fn repeat_name(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    }
}

fn replaygain_name(mode: ReplayGainMode) -> &'static str {
    match mode {
        ReplayGainMode::Track => "track",
        ReplayGainMode::Album => "album",
        ReplayGainMode::Off => "off",
    }
}

fn parse_repeat(value: &str) -> RepeatMode {
    match value {
        "all" => RepeatMode::All,
        "one" => RepeatMode::One,
        _ => RepeatMode::Off,
    }
}

fn parse_replaygain(value: &str) -> ReplayGainMode {
    match value {
        "album" => ReplayGainMode::Album,
        "off" | "no" => ReplayGainMode::Off,
        _ => ReplayGainMode::Track,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iwaks_core::track::AudioMetadata;

    fn track(path: &str, title: &str) -> Track {
        let mut t = Track::from_metadata(path, &AudioMetadata::new("flac"), 1, 1);
        t.title = title.to_string();
        t
    }

    fn state() -> PlayerState {
        PlayerState {
            current: Some(track(r"C:\music\b.flac", "B")),
            index: Some(1),
            list_len: 3,
            position: 42.0,
            duration: 200.0,
            paused: false,
            stopped: false,
            volume: 63,
            mute: true,
            repeat: RepeatMode::All,
            shuffle: true,
            queue_version: 7,
            speed: 1.25,
            sleep_remaining: Some(300.0),
            eq_preamp: -2.0,
            eq: vec![1.0; 10],
            replaygain: ReplayGainMode::Album,
        }
    }

    #[test]
    fn settings_map_every_persisted_field() {
        let s = settings_from(&state());
        assert_eq!(s.volume, 63);
        assert!(s.mute);
        assert_eq!(s.repeat, "all");
        assert!(s.shuffle);
        assert_eq!(s.speed, 1.25);
        assert_eq!(s.replaygain, "album");
        assert_eq!(s.eq_preamp, -2.0);
        assert_eq!(s.eq, vec![1.0; 10]);
        assert_eq!(s.sleep_seconds, Some(300.0));
    }

    #[test]
    fn enum_names_roundtrip() {
        for mode in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            assert_eq!(parse_repeat(repeat_name(mode)), mode);
        }
        for mode in [
            ReplayGainMode::Track,
            ReplayGainMode::Album,
            ReplayGainMode::Off,
        ] {
            assert_eq!(parse_replaygain(replaygain_name(mode)), mode);
        }
    }

    #[test]
    fn session_follows_play_order_and_current_slot() {
        let queue = vec![
            track(r"C:\music\a.flac", "A"),
            track(r"C:\music\b.flac", "B"),
            track(r"C:\music\c.flac", "C"),
        ];
        let s = session_from(&state(), queue).expect("session");
        assert_eq!(
            s.paths,
            vec![r"C:\music\a.flac", r"C:\music\b.flac", r"C:\music\c.flac"]
        );
        assert_eq!(s.index, 1, "index into the play-order list");
        assert_eq!(s.position, 42.0);
        assert!(s.shuffle);
    }

    #[test]
    fn session_is_none_without_something_to_resume() {
        let mut idle = state();
        idle.current = None;
        assert!(session_from(&idle, vec![track(r"C:\music\a.flac", "A")]).is_none());
        assert!(session_from(&state(), Vec::new()).is_none());
    }
}
