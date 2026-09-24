use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iwaks_core::track::Track;
use iwaks_library::db::Library;
use iwaks_library::scan::{scan, ScanOptions, ScanProgress};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared application state. `Library` connections are opened per call,
/// so this stays `Send + Sync`.
pub struct AppState {
    db_path: PathBuf,
    scanning: Arc<AtomicBool>,
}

/// Payload pushed to the frontend during / after a scan.
#[derive(Clone, Serialize)]
struct ScanEvent {
    progress: ScanProgress,
    finished: bool,
}

fn open_lib(state: &AppState) -> Result<Library, String> {
    Library::open(&state.db_path.to_string_lossy()).map_err(|e| e.to_string())
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

struct ScanGuard(Arc<AtomicBool>);
impl Drop for ScanGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("iwaks.db");
            app.manage(AppState {
                db_path,
                scanning: Arc::new(AtomicBool::new(false)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_tracks,
            search_tracks,
            scan_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
