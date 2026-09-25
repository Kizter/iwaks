use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use iwaks_core::track::Track;
use iwaks_library::db::Library;
use iwaks_library::scan::{scan, ScanOptions, ScanProgress};
use iwaks_player::{Options as PlayerOptions, Player, PlayerState, RepeatMode, ReplayGainMode};
use iwaks_visualizer::{Spectrum, SpectrumCache};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared application state. `Library` connections are opened per call,
/// so this stays `Send + Sync`. The player is `None` when libmpv failed to
/// initialize (e.g. the DLL is missing) — the app still runs, without audio.
pub struct AppState {
    db_path: PathBuf,
    scanning: Arc<AtomicBool>,
    player: Arc<Mutex<Option<Arc<Player>>>>,
    spectrum: SpectrumCache,
}

/// Payload pushed to the frontend during / after a scan.
#[derive(Clone, Serialize)]
struct ScanEvent {
    progress: ScanProgress,
    finished: bool,
}

const PLAYER_UNAVAILABLE: &str =
    "Playback is unavailable — libmpv DLL missing or failed to initialize";

fn open_lib(state: &AppState) -> Result<Library, String> {
    Library::open(&state.db_path.to_string_lossy()).map_err(|e| e.to_string())
}

/// Clone of the live player handle, or `None` when playback is unavailable.
fn player(state: &AppState) -> Option<Arc<Player>> {
    state.player.lock().unwrap().clone()
}

/// All library tracks, sorted artist → album → title.
#[tauri::command]
fn get_tracks(state: State<'_, AppState>) -> Result<Vec<Track>, String> {
    open_lib(&state)?.all_tracks().map_err(|e| e.to_string())
}

/// Full-text search; a blank query returns the full library.
#[tauri::command]
fn search_tracks(query: String, state: State<'_, AppState>) -> Result<Vec<Track>, String> {
    let lib = open_lib(&state)?;
    if query.trim().is_empty() {
        return lib.all_tracks().map_err(|e| e.to_string());
    }
    lib.search(&query).map_err(|e| e.to_string())
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

/// Spectrum timeline for the visualizer. `Err` when the file can't be decoded
/// (unsupported format — Opus/WavPack/WMA/DSD have no symphonia decoder).
/// Decoding runs on a blocking worker thread; `SpectrumCache` makes re-opening
/// the same track instant. (Async commands borrowing state must return
/// `Result`.)
#[tauri::command]
async fn get_spectrum(state: State<'_, AppState>, path: String) -> Result<Spectrum, String> {
    let p = PathBuf::from(&path);
    if let Some(cached) = state.spectrum.get(&p) {
        return Ok(cached);
    }
    let task = p.clone();
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        iwaks_visualizer::analyze(
            &task,
            iwaks_visualizer::DEFAULT_FPS,
            iwaks_visualizer::DEFAULT_BINS,
        )
        .ok()
    })
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "No spectrum for this track".to_string())?;
    state.spectrum.put(&p, decoded.clone());
    Ok(decoded)
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

// ---------- playback (libmpv via iwaks-player) ----------

/// Play `tracks` starting at `index`; the queue is rebuilt from this list.
#[tauri::command]
fn play_tracks(tracks: Vec<Track>, index: usize, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .play_tracks(tracks, index);
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
    Ok(())
}

#[tauri::command]
fn prev_track(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .prev();
    Ok(())
}

#[tauri::command]
fn seek(position: f64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .seek(position);
    Ok(())
}

#[tauri::command]
fn seek_relative(delta: f64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .seek_relative(delta);
    Ok(())
}

#[tauri::command]
fn set_volume(volume: i64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_volume(volume);
    Ok(())
}

#[tauri::command]
fn toggle_mute(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .toggle_mute();
    Ok(())
}

#[tauri::command]
fn set_speed(speed: f64, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_speed(speed);
    Ok(())
}

/// Arm the sleep timer (`seconds` seconds) or cancel it with `null`.
#[tauri::command]
fn set_sleep_timer(seconds: Option<f64>, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_sleep_timer(seconds);
    Ok(())
}

/// Graphic EQ: `preamp` dB master gain + 10 band gains (`eq`, dB).
#[tauri::command]
fn set_eq(preamp: f64, eq: Vec<f64>, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_eq(preamp, eq);
    Ok(())
}

#[tauri::command]
fn set_replaygain(mode: ReplayGainMode, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_replaygain(mode);
    Ok(())
}

#[tauri::command]
fn set_repeat(repeat: RepeatMode, state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .set_repeat(repeat);
    Ok(())
}

#[tauri::command]
fn stop_playback(state: State<'_, AppState>) -> Result<(), String> {
    player(&state)
        .ok_or_else(|| PLAYER_UNAVAILABLE.to_string())?
        .stop();
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("iwaks.db");

            let handle = app.handle().clone();
            let player = match Player::start(
                &PlayerOptions::default(),
                Box::new(move |s: &PlayerState| {
                    let _ = handle.emit("player-state", s.clone());
                }),
            ) {
                Ok(p) => Some(p),
                Err(e) => {
                    eprintln!("player unavailable: {e}");
                    let _ = app.emit("player-error", e);
                    None
                }
            };

            app.manage(AppState {
                db_path,
                scanning: Arc::new(AtomicBool::new(false)),
                player: Arc::new(Mutex::new(player)),
                spectrum: SpectrumCache::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_tracks,
            search_tracks,
            scan_folder,
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
            set_speed,
            set_sleep_timer,
            set_eq,
            set_replaygain,
            stop_playback,
            get_player_state,
            get_lyrics,
            get_spectrum
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Stop libmpv cleanly when the app closes (pump thread join + destroy).
    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<AppState>() {
                if let Some(p) = state.player.lock().unwrap().take() {
                    p.shutdown();
                }
            }
        }
    });
}
