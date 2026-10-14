use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use iwaks_core::track::{TagEdits, Track};
use iwaks_library::db::{Library, TrackFilter, TrackSort};
use iwaks_library::persist::EqPreset;
use iwaks_library::playlists::Playlist;
use iwaks_library::scan::{scan, scan_files, ScanOptions, ScanProgress};
use iwaks_player::{
    AudioDevice, Crossfade, Options as PlayerOptions, Player, PlayerState, RepeatMode,
    ReplayGainMode, DEFAULT_CROSSFADE_SECONDS,
};
use iwaks_presence::NowPlaying;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

pub mod cover;
pub mod persist;

/// Shared application state. `Library` connections are opened per call,
/// so this stays `Send + Sync`. The player is `None` when libmpv failed to
/// initialize (e.g. the DLL is missing) — the app still runs, without audio.
pub struct AppState {
    db_path: PathBuf,
    scanning: Arc<AtomicBool>,
    player: Arc<Mutex<Option<Arc<Player>>>>,
    /// Live opt-in flag for online album art. Cached here because the playback
    /// sink reads it ~10×/second and must not open the database to do so.
    online_cover: Arc<AtomicBool>,
    /// Last resolved album art, shared with the lookup threads.
    cover_memo: cover::SharedMemo,
    /// Debounced writer for player settings + last session (issue #01).
    persist: persist::Persist,
}

/// Payload pushed to the frontend during / after a scan.
#[derive(Clone, Serialize)]
struct ScanEvent {
    progress: ScanProgress,
    finished: bool,
}

// User-facing copy only: the specific cause (missing DLL, init failure) is
// logged at startup, never shown in a banner.
const PLAYER_UNAVAILABLE: &str = "Playback is unavailable right now.";

/// Iwaks' Discord application id (public — not a secret). The Rich Presence
/// session and the `discord-<id>://` protocol registration use this.
/// Override at runtime with `IWAKS_DISCORD_APP_ID` (used by tests/CI).
const DISCORD_APP_ID: &str = "1553676698900365344";

/// Map the player state to what Discord should show. `None` clears the
/// presence — no track loaded, or the queue ended naturally.
fn to_now_playing(state: &PlayerState, cover_url: Option<String>) -> Option<NowPlaying> {
    if state.stopped {
        return None;
    }
    let track = state.current.as_ref()?;
    Some(NowPlaying {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        position_secs: state.position,
        playing: !state.paused,
        cover_url,
    })
}

/// Decide what cover art to show for this tick, kicking off a lookup when the
/// track changed. Cheap by construction: one mutex lock, and no disk or network
/// work unless a new album needs resolving.
fn cover_for_tick(
    state: &PlayerState,
    enabled: bool,
    memo: &cover::SharedMemo,
    db_path: &Path,
) -> Option<String> {
    let track = state.current.as_ref()?;
    let (artist, album) = (
        track.artist.clone().unwrap_or_default(),
        track.album.clone().unwrap_or_default(),
    );
    let key = if enabled {
        iwaks_cover::album_key(Some(&artist), Some(&album))
    } else {
        String::new()
    };
    match memo.lock().expect("cover memo").on_track(&key) {
        cover::CoverAction::Known(url) => url,
        cover::CoverAction::Waiting => None,
        cover::CoverAction::Start => {
            cover::resolve_in_background(db_path.to_path_buf(), memo.clone(), key, artist, album);
            None
        }
    }
}

/// Whether album art is fetched from the internet for the Rich Presence.
///
/// Stored as text because the settings table is a plain key/value store; an
/// absent or unreadable key means "off", which keeps the network-free default
/// even if the database is unavailable.
fn read_online_cover(db_path: &Path) -> bool {
    Library::open(&db_path.to_string_lossy())
        .ok()
        .and_then(|lib| lib.setting(cover::SETTING_ONLINE_COVER).ok().flatten())
        .is_some_and(|value| value == "true")
}

fn open_lib(state: &AppState) -> Result<Library, String> {
    Library::open(&state.db_path.to_string_lossy()).map_err(|e| e.to_string())
}

/// Keys for the audio-output settings (issues #23, #24). Plain text, like every
/// other entry in the `settings` table.
pub const SETTING_AUDIO_DEVICE: &str = "audio.device";
pub const SETTING_AUDIO_EXCLUSIVE: &str = "audio.exclusive";
/// UI theme: `system`, `light` or `dark` (issue #25).
pub const SETTING_THEME: &str = "ui.theme";
/// Crossfade on/off (issue #27). Off by default, per design §4.2.
pub const SETTING_CROSSFADE: &str = "audio.crossfade";
/// Crossfade overlap in seconds (issue #27). Kept separate from the on/off flag
/// so a short fade is expressible.
pub const SETTING_CROSSFADE_SECONDS: &str = "audio.crossfade_seconds";
/// Bounds the UI offers for the overlap, per D1. The player floor is lower
/// (defensive, for a hand-written value); this is what a person can pick.
pub const MIN_CROSSFADE_SECONDS: f64 = 1.0;
pub const MAX_CROSSFADE_SECONDS: f64 = 10.0;

/// The stored crossfade preferences, falling back to the plan's defaults.
///
/// Two settings rather than one sentinel: `seconds = 0` has to stay a legal
/// (if silly) duration, so "off" needs its own flag.
///
/// The overlap is clamped the same way every other reader does it: the row is
/// plain text a user can hand-edit, and an unclamped value would reach libmpv
/// as a fade window (code-review #30).
fn stored_crossfade(db_path: &Path) -> Crossfade {
    let seconds = read_setting(db_path, SETTING_CROSSFADE_SECONDS)
        .and_then(|raw| raw.parse::<f64>().ok())
        .map(clamp_crossfade_seconds)
        .unwrap_or(DEFAULT_CROSSFADE_SECONDS);
    Crossfade {
        enabled: read_setting(db_path, SETTING_CROSSFADE).as_deref() == Some("true"),
        seconds,
    }
}

fn read_setting(db_path: &Path, key: &str) -> Option<String> {
    Library::open(&db_path.to_string_lossy())
        .ok()
        .and_then(|lib| lib.setting(key).ok().flatten())
}

/// Playback options from the stored audio settings.
///
/// Read before the player exists, so a missing or unreadable database simply
/// yields the defaults — playback is never blocked by a settings problem.
fn startup_options(db_path: &Path) -> PlayerOptions {
    PlayerOptions {
        audio_device: read_setting(db_path, SETTING_AUDIO_DEVICE).unwrap_or_default(),
        audio_exclusive: read_setting(db_path, SETTING_AUDIO_EXCLUSIVE).as_deref() == Some("true"),
        ..PlayerOptions::default()
    }
}

/// Whether the Rich Presence looks album art up online (opt-in).
#[tauri::command]
fn get_online_cover(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.online_cover.load(Ordering::Relaxed))
}

/// Turn online album art on or off. Clearing the memo makes the change visible
/// on the very next playback tick: on, the current track is looked up; off, any
/// cover already resolved is dropped from the card.
#[tauri::command]
fn set_online_cover(enabled: bool, state: State<'_, AppState>) -> Result<(), String> {
    open_lib(&state)?
        .set_setting(
            cover::SETTING_ONLINE_COVER,
            if enabled { "true" } else { "false" },
        )
        .map_err(|e| e.to_string())?;
    state.online_cover.store(enabled, Ordering::Relaxed);
    state.cover_memo.lock().expect("cover memo").clear();
    Ok(())
}

/// The user's saved equalizer presets (empty when none have been saved).
#[tauri::command]
fn get_eq_presets(state: State<'_, AppState>) -> Result<Vec<EqPreset>, String> {
    open_lib(&state)?
        .load_eq_presets()
        .map_err(|e| e.to_string())
}

/// The three modes the UI offers. Guarded here too because the value is
/// persisted: a stray IPC string must not become the stored theme and then
/// come back out of `get_theme` (code-review #30).
fn is_theme_mode(mode: &str) -> bool {
    matches!(mode, "system" | "light" | "dark")
}

/// Stored UI theme (`system` | `light` | `dark`); `system` when never set or
/// when a hand-edited row holds something unusable — the frontend only ever
/// applies a known mode, so it falls back the same way.
#[tauri::command]
fn get_theme(state: State<'_, AppState>) -> Result<String, String> {
    Ok(read_setting(&state.db_path, SETTING_THEME)
        .filter(|mode| is_theme_mode(mode))
        .unwrap_or_else(|| "system".to_string()))
}

/// Remember the UI theme choice.
#[tauri::command]
fn set_theme(mode: String, state: State<'_, AppState>) -> Result<(), String> {
    if !is_theme_mode(&mode) {
        return Err(format!("unknown theme mode: {mode}"));
    }
    open_lib(&state)?
        .set_setting(SETTING_THEME, &mode)
        .map_err(|e| e.to_string())
}

/// Audio devices mpv can output to. Read from the playback thread on demand,
/// so the list is never stale — no change event is needed (issue #23).
#[tauri::command]
fn list_audio_devices(state: State<'_, AppState>) -> Result<Vec<AudioDevice>, String> {
    Ok(player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .list_devices())
}

/// Pick the output device; an empty `name` means "system default".
///
/// Stored so the choice survives a restart, and applied immediately so the user
/// hears the switch without replaying anything. Stored first, applied second —
/// the same order `set_crossfade` uses: a write that fails must not leave the
/// session and the UI disagreeing about the device that is actually playing,
/// and the frontend rolls the switch back when the command rejects.
#[tauri::command]
fn set_audio_device(name: String, state: State<'_, AppState>) -> Result<(), String> {
    open_lib(&state)?
        .set_setting(SETTING_AUDIO_DEVICE, &name)
        .map_err(|e| e.to_string())?;
    if let Some(p) = player(&state) {
        p.set_audio_device(&name);
    }
    Ok(())
}

/// The stored device name (`""` = system default).
#[tauri::command]
fn get_audio_device(state: State<'_, AppState>) -> Result<String, String> {
    Ok(read_setting(&state.db_path, SETTING_AUDIO_DEVICE).unwrap_or_default())
}

/// Whether exclusive (bit-perfect) output is on. Applied at startup, since
/// libmpv only accepts the option before initialisation (issue #24).
#[tauri::command]
fn get_audio_exclusive(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(read_setting(&state.db_path, SETTING_AUDIO_EXCLUSIVE).as_deref() == Some("true"))
}

/// Turn exclusive output on/off; takes effect the next time Iwaks starts.
#[tauri::command]
fn set_audio_exclusive(enabled: bool, state: State<'_, AppState>) -> Result<(), String> {
    open_lib(&state)?
        .set_setting(
            SETTING_AUDIO_EXCLUSIVE,
            if enabled { "true" } else { "false" },
        )
        .map_err(|e| e.to_string())
}

/// The stored crossfade preferences, clamped to what the UI offers (issue #27).
#[tauri::command]
fn get_crossfade(state: State<'_, AppState>) -> Crossfade {
    let stored = stored_crossfade(&state.db_path);
    Crossfade {
        enabled: stored.enabled,
        seconds: clamp_crossfade_seconds(stored.seconds),
    }
}

/// Remember the crossfade choice and apply it to the running session.
///
/// Unlike exclusive output this takes effect immediately: the pump re-reads the
/// preferences on every tick, so the next prefetch uses the new window. Stored
/// first — a write that fails must not leave the UI and the session disagreeing,
/// and the frontend rolls the switch back when the command rejects.
#[tauri::command]
fn set_crossfade(crossfade: Crossfade, state: State<'_, AppState>) -> Result<(), String> {
    let wanted = Crossfade {
        enabled: crossfade.enabled,
        seconds: clamp_crossfade_seconds(crossfade.seconds),
    };
    let lib = open_lib(&state)?;
    lib.set_setting(
        SETTING_CROSSFADE,
        if wanted.enabled { "true" } else { "false" },
    )
    .map_err(|e| e.to_string())?;
    lib.set_setting(SETTING_CROSSFADE_SECONDS, &wanted.seconds.to_string())
        .map_err(|e| e.to_string())?;
    if let Some(p) = player(&state) {
        p.set_crossfade(wanted);
    }
    Ok(())
}

/// Keep a stored duration inside the range D1 settled on: 1–10 s.
///
/// Below 1 s it sounds like a glitch rather than a fade, and above 10 s a fade
/// starts swallowing whole tracks whose intros are short. Non-finite input (a
/// hand-edited settings table) falls back to the default instead of reaching
/// libmpv.
fn clamp_crossfade_seconds(seconds: f64) -> f64 {
    if !seconds.is_finite() {
        return DEFAULT_CROSSFADE_SECONDS;
    }
    seconds.clamp(MIN_CROSSFADE_SECONDS, MAX_CROSSFADE_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stored row is the one untrusted input left in the crossfade path: it
    /// is plain text a user can edit, and it reaches libmpv as a fade window.
    #[test]
    fn a_stored_overlap_is_clamped_to_what_the_ui_offers() {
        assert_eq!(clamp_crossfade_seconds(5.0), 5.0, "a normal value passes");
        assert_eq!(clamp_crossfade_seconds(1.0), 1.0, "the ends are reachable");
        assert_eq!(clamp_crossfade_seconds(10.0), 10.0);
        assert_eq!(clamp_crossfade_seconds(0.2), MIN_CROSSFADE_SECONDS);
        assert_eq!(clamp_crossfade_seconds(-40.0), MIN_CROSSFADE_SECONDS);
        assert_eq!(clamp_crossfade_seconds(600.0), MAX_CROSSFADE_SECONDS);
    }

    #[test]
    fn an_unusable_stored_overlap_falls_back_to_the_default() {
        // A NaN would survive `clamp` untouched and poison every comparison the
        // ramp makes against it. An infinity is not a meaningful window either —
        // it would make every track "about to end" — so it defaults too.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                clamp_crossfade_seconds(bad),
                DEFAULT_CROSSFADE_SECONDS,
                "{bad} is not a duration anyone can mean"
            );
        }
    }

    /// The startup reader is the third path a stored overlap can take (after
    /// `get_crossfade` and `set_crossfade`) — without the clamp here a
    /// hand-edited 600s row would reach the pump as a fade window (issue #27
    /// says the read is clamped; code-review #30 found it was not).
    #[test]
    fn the_startup_crossfade_reader_clamps_a_hand_edited_row() {
        let dir = std::env::temp_dir().join(format!("iwaks-lib-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("settings.sqlite");
        let _ = std::fs::remove_file(&db);
        let lib = Library::open(&db.to_string_lossy()).expect("a fresh library opens");
        lib.set_setting(SETTING_CROSSFADE_SECONDS, "600")
            .expect("hand-edited style row writes");
        lib.set_setting(SETTING_CROSSFADE, "true").unwrap();
        drop(lib);

        let crossfade = stored_crossfade(&db);
        assert!(crossfade.enabled, "the on/off flag passes through");
        assert_eq!(
            crossfade.seconds, MAX_CROSSFADE_SECONDS,
            "600s clamps down to the UI maximum"
        );

        let lib = Library::open(&db.to_string_lossy()).unwrap();
        lib.set_setting(SETTING_CROSSFADE_SECONDS, "0.2").unwrap();
        drop(lib);
        assert_eq!(
            stored_crossfade(&db).seconds,
            MIN_CROSSFADE_SECONDS,
            "0.2s clamps up to the UI minimum"
        );

        let lib = Library::open(&db.to_string_lossy()).unwrap();
        lib.set_setting(SETTING_CROSSFADE_SECONDS, "not-a-duration")
            .unwrap();
        drop(lib);
        assert_eq!(
            stored_crossfade(&db).seconds,
            DEFAULT_CROSSFADE_SECONDS,
            "an unreadable row falls back to the default"
        );
    }

    /// `set_theme` persists whatever IPC hands it, and `get_theme` reads it
    /// back — so the guard is the only thing keeping a stray string out of the
    /// stored theme (code-review #30).
    #[test]
    fn only_the_three_offered_theme_modes_are_stored() {
        for good in ["system", "light", "dark"] {
            assert!(is_theme_mode(good), "{good} is a mode the UI offers");
        }
        for bad in ["", "System", "auto", "dark ", "systemx", "light\n"] {
            assert!(!is_theme_mode(bad), "{bad} is not a mode the UI offers");
        }
    }
}

/// Path of the log file this session writes to (`null` when logging to disk
/// could not be opened); shown in Settings so a bug report can point at it.
#[tauri::command]
fn log_path() -> Option<String> {
    iwaks_log::path().map(|p| p.to_string_lossy().to_string())
}

/// Replace the user's saved equalizer presets. Built-in templates are not
/// stored; the frontend sends only the custom list.
#[tauri::command]
fn set_eq_presets(presets: Vec<EqPreset>, state: State<'_, AppState>) -> Result<(), String> {
    open_lib(&state)?
        .save_eq_presets(&presets)
        .map_err(|e| e.to_string())
}

/// Clone of the live player handle, or `None` when playback is unavailable.
fn player(state: &AppState) -> Option<Arc<Player>> {
    state.player.lock().unwrap().clone()
}

/// All library tracks, sorted artist → album → title.
#[tauri::command]
fn get_tracks(
    filter: TrackFilter,
    sort: TrackSort,
    state: State<'_, AppState>,
) -> Result<Vec<Track>, String> {
    open_lib(&state)?
        .tracks_filtered(None, &filter, sort)
        .map_err(|e| e.to_string())
}

/// Full-text search narrowed by `filter` and ordered by `sort`; a blank query
/// returns the (filtered) library.
#[tauri::command]
fn search_tracks(
    query: String,
    filter: TrackFilter,
    sort: TrackSort,
    state: State<'_, AppState>,
) -> Result<Vec<Track>, String> {
    let lib = open_lib(&state)?;
    let q = (!query.trim().is_empty()).then_some(query);
    lib.tracks_filtered(q.as_deref(), &filter, sort)
        .map_err(|e| e.to_string())
}

/// Every genre in the library, for the filter chips.
#[tauri::command]
fn list_genres(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    open_lib(&state)?.genres().map_err(|e| e.to_string())
}

/// Decades holding at least one dated track, newest first.
#[tauri::command]
fn list_decades(state: State<'_, AppState>) -> Result<Vec<i64>, String> {
    open_lib(&state)?.decades().map_err(|e| e.to_string())
}

/// Album art for a track file as a `data:` URL (`None` when the file has no
/// embedded picture or cannot be read). Loaded on demand for visible rows.
#[tauri::command]
fn read_cover(path: String) -> Option<String> {
    use base64::Engine as _;
    let cover = iwaks_tags::cover::read_cover(std::path::Path::new(&path))
        .ok()
        .flatten()?;
    Some(format!(
        "data:{};base64,{}",
        cover.mime,
        base64::engine::general_purpose::STANDARD.encode(&cover.data)
    ))
}

/// Lyrics for a track: `.lrc`/embedded LRC timed lines plus embedded plain
/// text (USLT / Vorbis `LYRICS`). `null` when the file has none.
#[tauri::command]
fn get_lyrics(path: String) -> Option<iwaks_tags::lyrics::Lyrics> {
    iwaks_tags::lyrics::read_lyrics(std::path::Path::new(&path))
        .ok()
        .flatten()
}

/// Write the 8 editable tag fields of a track file (`.bak` backup first, via
/// `iwaks-tags`), then refresh the library row from the tags that landed on
/// disk. Returns the updated track. Playback is never touched.
#[tauri::command]
fn write_tags(path: String, edits: TagEdits, state: State<'_, AppState>) -> Result<Track, String> {
    let file_path = std::path::Path::new(&path);
    let meta = iwaks_tags::write::apply_edits(file_path, &edits).map_err(|e| e.to_string())?;

    let stat = std::fs::metadata(file_path).map_err(|e| e.to_string())?;
    let modified = stat
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let track = Track::from_metadata(&path, &meta, stat.len() as i64, modified);
    let id = open_lib(&state)?
        .upsert_track(&track)
        .map_err(|e| e.to_string())?;
    let mut updated = track;
    updated.id = id;
    Ok(updated)
}

/// Kick off a background incremental scan of `path`. Emits `scan-started`,
/// `scan-progress` (with `finished: false`), then a final `scan-progress`
/// with `finished: true` or a `scan-error` event.
#[tauri::command]
fn scan_folder(app: AppHandle, path: String, state: State<'_, AppState>) -> Result<(), String> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("a scan is already running".to_string());
    }
    let db = state.db_path.clone();
    let flag = Arc::clone(&state.scanning);
    let _ = app.emit("scan-started", ());

    std::thread::spawn(move || {
        let _guard = ScanGuard(flag);
        let mut lib = match Library::open(&db.to_string_lossy()) {
            Ok(l) => l,
            Err(e) => {
                let _ = app.emit("scan-error", e.to_string());
                return;
            }
        };
        let opts = ScanOptions {
            root: PathBuf::from(path),
            clean_missing: true,
        };
        let result = scan(&mut lib, &opts, &mut |p| {
            let _ = app.emit(
                "scan-progress",
                ScanEvent {
                    progress: p.clone(),
                    finished: false,
                },
            );
        });
        match result {
            Ok(progress) => {
                let _ = app.emit(
                    "scan-progress",
                    ScanEvent {
                        progress,
                        finished: true,
                    },
                );
            }
            Err(e) => {
                let _ = app.emit("scan-error", e.to_string());
            }
        }
    });
    Ok(())
}

/// Add individually-picked music files (multi-select dialog) to the library.
/// Sync — the picker returns a handful of files, so this is fast enough to
/// run inline and the frontend refreshes its list when it resolves.
#[tauri::command]
fn add_files(paths: Vec<String>, state: State<'_, AppState>) -> Result<ScanProgress, String> {
    let mut lib = open_lib(&state)?;
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    scan_files(&mut lib, &files, &mut |_| {}).map_err(|e| e.to_string())
}

// ---------- playback (libmpv via iwaks-player) ----------

/// Play `tracks` starting at `index`; the queue is rebuilt from this list.
#[tauri::command]
fn play_tracks(tracks: Vec<Track>, index: usize, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .play_tracks(tracks, index);
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn toggle_play(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .toggle_play();
    Ok(())
}

#[tauri::command]
fn next_track(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .next();
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn prev_track(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .prev();
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn seek(position: f64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .seek(position);
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn seek_relative(delta: f64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .seek_relative(delta);
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn set_volume(volume: i64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_volume(volume);
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn toggle_mute(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .toggle_mute();
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn set_speed(speed: f64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_speed(speed);
    state.persist.touch();
    Ok(())
}

/// Arm the sleep timer (`seconds` seconds) or cancel it with `null`.
#[tauri::command]
fn set_sleep_timer(seconds: Option<f64>, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_sleep_timer(seconds);
    state.persist.touch();
    Ok(())
}

/// Graphic EQ: `preamp` dB master gain + 10 band gains (`eq`, dB).
#[tauri::command]
fn set_eq(preamp: f64, eq: Vec<f64>, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_eq(preamp, eq);
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn set_replaygain(mode: ReplayGainMode, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_replaygain(mode);
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn set_repeat(repeat: RepeatMode, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_repeat(repeat);
    state.persist.touch();
    Ok(())
}

/// Toggle shuffle on/off (current track stays; remaining order re-randomized).
#[tauri::command]
fn set_shuffle(shuffle: bool, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_shuffle(shuffle);
    state.persist.touch();
    Ok(())
}

/// Re-shuffle the remaining tracks (current track stays selected).
#[tauri::command]
fn reshuffle_tracks(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .reshuffle();
    state.persist.touch();
    Ok(())
}

#[tauri::command]
fn stop_playback(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .stop();
    state.persist.touch();
    Ok(())
}

/// Initial `player-state`: `null` when playback is unavailable.
#[tauri::command]
fn get_player_state(state: State<'_, AppState>) -> Option<PlayerState> {
    player(&state).map(|p| p.snapshot())
}

struct ScanGuard(Arc<AtomicBool>);
impl Drop for ScanGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Label of the always-on-top mini player window.
const MINI_LABEL: &str = "mini";

/// Create the mini player window if it doesn't exist yet. It loads the same
/// frontend with `#mini` in the URL; `App` switches to the mini layout. The
/// whole window surface is a drag region (CSS `data-tauri-drag-region`), so
/// it can be dragged anywhere even though it is frameless.
fn open_mini_window(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(MINI_LABEL).is_some() {
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        app,
        MINI_LABEL,
        tauri::WebviewUrl::App("index.html#mini".into()),
    )
    .title("Iwaks Mini Player")
    .inner_size(320.0, 160.0)
    .min_inner_size(280.0, 145.0)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .decorations(false)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open the mini player when the main window is minimized, close it when
/// the main window comes back.
///
/// The open/close is **deferred off the window-event callback**: creating a
/// WebView2 window synchronously inside `Resized` (which Windows fires mid
/// minimize-transition) blocks the main thread's loop → the main app becomes
/// unclickable and the mini window arrives as a white, not-responding screen.
/// We wait for the transition to settle, re-check `is_minimized()`, then run
/// the create/close on the main loop via `run_on_main_thread`.
fn sync_mini(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let inner = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let minimized = inner
                .get_webview_window("main")
                .map(|w| w.is_minimized().unwrap_or(false))
                .unwrap_or(false);
            if minimized {
                let _ = open_mini_window(&inner);
            } else if let Some(mini) = inner.get_webview_window(MINI_LABEL) {
                let _ = mini.close();
            }
        });
    });
}

/// Manual toggle from the player bar (independent of the minimized state).
#[tauri::command]
fn toggle_mini_player(app: AppHandle) -> Result<(), String> {
    if let Some(mini) = app.get_webview_window(MINI_LABEL) {
        let _ = mini.close();
    } else {
        open_mini_window(&app)?;
    }
    Ok(())
}

// ---------- playlists (M4 slice 1) ----------

#[tauri::command]
fn list_playlists(state: State<'_, AppState>) -> Result<Vec<Playlist>, String> {
    open_lib(&state)?
        .list_playlists()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_playlist(name: String, state: State<'_, AppState>) -> Result<i64, String> {
    open_lib(&state)?
        .create_playlist(&name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_playlist(id: i64, name: String, state: State<'_, AppState>) -> Result<(), String> {
    open_lib(&state)?
        .rename_playlist(id, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_playlist(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    open_lib(&state)?
        .delete_playlist(id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_playlist_tracks(id: i64, state: State<'_, AppState>) -> Result<Option<Vec<Track>>, String> {
    open_lib(&state)?
        .get_playlist_tracks(id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn add_to_playlist(
    playlist_id: i64,
    track_id: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    open_lib(&state)?
        .add_track_to_playlist(playlist_id, track_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_from_playlist(
    playlist_id: i64,
    track_id: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    open_lib(&state)?
        .remove_track_from_playlist(playlist_id, track_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_playlist(
    playlist_id: i64,
    track_ids: Vec<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    open_lib(&state)?
        .reorder_playlist(playlist_id, &track_ids)
        .map_err(|e| e.to_string())
}

/// Import an `.m3u` file as a new playlist; returns its id.
#[tauri::command]
fn import_m3u(path: String, state: State<'_, AppState>) -> Result<i64, String> {
    open_lib(&state)?
        .import_m3u(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

/// Export a playlist to an `.m3u` file at `path`.
#[tauri::command]
fn export_m3u(playlist_id: i64, path: String, state: State<'_, AppState>) -> Result<(), String> {
    let lib = open_lib(&state)?;
    let content = lib
        .export_m3u(playlist_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "playlist not found".to_string())?;
    std::fs::write(std::path::Path::new(&path), content).map_err(|e| e.to_string())
}

// ---------- session queue (M4 slice 2) ----------

/// Tracks of the current session queue in play order (the shuffle
/// permutation when enabled) — for the Queue view.
#[tauri::command]
fn get_queue(state: State<'_, AppState>) -> Result<Vec<Track>, String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())
        .map(|p| p.queue_tracks())
}

/// Move the queue track at position `from` to `to` (current track follows;
/// playback is untouched).
#[tauri::command]
fn reorder_queue(from: usize, to: usize, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .reorder_queue(from, to);
    state.persist.touch();
    Ok(())
}

/// Remove the queue track at play-order `position`. Removing the playing track
/// moves on to the next one, or stops when the queue just ran out.
#[tauri::command]
fn remove_from_queue(position: usize, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .remove_queue_at(position);
    state.persist.touch();
    Ok(())
}

/// Empty the session queue and stop playback (repeat/shuffle are kept).
#[tauri::command]
fn clear_queue(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .clear_queue();
    state.persist.touch();
    Ok(())
}

/// Save the current session queue as a playlist (trimmed non-empty name).
/// Returns the new playlist id.
#[tauri::command]
fn save_queue_as_playlist(name: String, state: State<'_, AppState>) -> Result<i64, String> {
    let tracks = player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .queue_tracks();
    if tracks.is_empty() {
        return Err("Queue is empty — nothing to save".to_string());
    }
    let mut lib = open_lib(&state)?;
    let playlist_id = lib.create_playlist(&name).map_err(|e| e.to_string())?;
    let track_ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
    lib.add_tracks_to_playlist(playlist_id, &track_ids)
        .map_err(|e| e.to_string())?;
    Ok(playlist_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Log file + panic hook first: everything below is exactly the kind
            // of startup that goes wrong silently otherwise (issue #22).
            match app.path().app_log_dir() {
                Ok(dir) => match iwaks_log::init(&dir) {
                    Some(path) => iwaks_log::line(
                        iwaks_log::Level::Info,
                        &format!("iwaks starting, logging to {}", path.display()),
                    ),
                    None => iwaks_log::line(
                        iwaks_log::Level::Warn,
                        &format!("could not open a log file in {}", dir.display()),
                    ),
                },
                Err(e) => iwaks_log::line(
                    iwaks_log::Level::Warn,
                    &format!("no app log directory ({e})"),
                ),
            }
            iwaks_log::install_panic_hook();

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("iwaks.db");

            // Album art for the presence card. Read once here so the playback
            // sink never opens the database; the opt-in lives in settings and
            // defaults to off, i.e. no request leaves the machine.
            let online_cover = Arc::new(AtomicBool::new(read_online_cover(&db_path)));
            let cover_memo: cover::SharedMemo = Arc::new(Mutex::new(cover::CoverMemo::new()));
            // The sink closure needs its own handles; `AppState` keeps the
            // originals so the settings commands can reach them.
            let sink_db_path = db_path.clone();
            let sink_flag = Arc::clone(&online_cover);
            let sink_memo = Arc::clone(&cover_memo);

            let handle = app.handle().clone();

            // Discord Rich Presence: spawn the driver + register the
            // `discord-<app_id>://` protocol so Discord treats Iwaks as an
            // app (needed for the presence session and the in-app overlay).
            let (presence_tx, presence_rx) = mpsc::channel::<Option<NowPlaying>>();
            let app_id = std::env::var("IWAKS_DISCORD_APP_ID")
                .unwrap_or_else(|_| DISCORD_APP_ID.to_string());
            let _presence = iwaks_presence::start(&app_id, presence_rx);
            if let Ok(exe) = std::env::current_exe() {
                let _ = iwaks_presence::register::register_protocol(&app_id, &exe);
            }

            let player = match Player::start(
                &startup_options(&db_path),
                Box::new(move |s: &PlayerState| {
                    let _ = handle.emit("player-state", s.clone());
                    let url = cover_for_tick(
                        s,
                        sink_flag.load(Ordering::Relaxed),
                        &sink_memo,
                        &sink_db_path,
                    );
                    let _ = presence_tx.send(to_now_playing(s, url));
                }),
            ) {
                Ok(p) => Some(p),
                Err(e) => {
                    iwaks_log::line(iwaks_log::Level::Warn, &format!("player unavailable: {e}"));
                    let _ = app.emit("player-error", PLAYER_UNAVAILABLE);
                    None
                }
            };

            let player_cell = Arc::new(Mutex::new(player));

            // Restore what the last run left behind: player preferences first,
            // then the queue + position so playback resumes where it stopped.
            if let Some(p) = player_cell.lock().unwrap().clone() {
                if let Ok(lib) = Library::open(&db_path.to_string_lossy()) {
                    if let Ok(Some(settings)) = lib.load_player_settings() {
                        persist::apply_settings(&p, &settings);
                    }
                    if let Ok(Some(session)) = lib.load_session() {
                        persist::restore_session(&p, &lib, &session);
                    }
                }
                p.set_crossfade(stored_crossfade(&db_path));
            }

            let persist = persist::Persist::spawn(db_path.clone(), Arc::clone(&player_cell));

            app.manage(AppState {
                db_path,
                scanning: Arc::new(AtomicBool::new(false)),
                player: player_cell,
                online_cover,
                cover_memo,
                persist,
            });

            // Auto-open the mini player while the main window is minimized,
            // close it on restore, and never let it outlive the main window.
            let main_handle = app.handle().clone();
            if let Some(main) = app.get_webview_window("main") {
                main.on_window_event(move |event| match event {
                    tauri::WindowEvent::Resized(_) => {
                        // Re-check the actual minimize state after the
                        // transition settles — see `sync_mini`.
                        sync_mini(&main_handle);
                    }
                    tauri::WindowEvent::Destroyed => {
                        if let Some(mini) = main_handle.get_webview_window(MINI_LABEL) {
                            let _ = mini.close();
                        }
                        main_handle.exit(0);
                    }
                    _ => {}
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_tracks,
            search_tracks,
            list_genres,
            list_decades,
            scan_folder,
            add_files,
            read_cover,
            play_tracks,
            toggle_play,
            next_track,
            prev_track,
            seek,
            seek_relative,
            set_volume,
            toggle_mute,
            set_repeat,
            set_shuffle,
            reshuffle_tracks,
            set_speed,
            set_sleep_timer,
            set_eq,
            set_replaygain,
            get_eq_presets,
            set_eq_presets,
            log_path,
            get_theme,
            set_theme,
            list_audio_devices,
            set_audio_device,
            get_audio_device,
            get_audio_exclusive,
            set_audio_exclusive,
            get_crossfade,
            set_crossfade,
            stop_playback,
            get_player_state,
            get_lyrics,
            write_tags,
            toggle_mini_player,
            list_playlists,
            create_playlist,
            rename_playlist,
            delete_playlist,
            get_playlist_tracks,
            add_to_playlist,
            remove_from_playlist,
            reorder_playlist,
            import_m3u,
            export_m3u,
            get_queue,
            reorder_queue,
            remove_from_queue,
            clear_queue,
            save_queue_as_playlist,
            get_online_cover,
            set_online_cover
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Stop libmpv cleanly when the app closes (pump thread join + destroy).
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<AppState>() {
                // Flush the latest position/settings before libmpv goes away,
                // so a clean close never loses where playback was.
                persist::write_now(&state.db_path, &state.player);
                if let Some(p) = state.player.lock().unwrap().take() {
                    p.shutdown();
                }
            }
        }
    });
}
