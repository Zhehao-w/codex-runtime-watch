use codex_runtime_watch::{
    codex::{probe, rollout::Correlator, watcher::scan_file},
    db::Database,
    settings::Settings,
    Observation,
};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager};
struct State {
    db: Mutex<Database>,
    settings_path: PathBuf,
    app_dir: PathBuf,
}
#[derive(Serialize)]
struct View {
    #[serde(flatten)]
    o: Observation,
    result: &'static str,
}
fn codex_home(s: &Settings) -> PathBuf {
    s.codex_home
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
        .unwrap_or_default()
}
#[tauri::command]
fn history(
    state: tauri::State<State>,
    filter: String,
    limit: u32,
    offset: u32,
) -> Result<Vec<View>, String> {
    let rows = state
        .db
        .lock()
        .map_err(|_| "database unavailable")?
        .history(limit, offset)
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .filter(|o| match filter.as_str() {
            "mismatches" => o.result().contains("mismatch") || o.result().contains("reroute"),
            "runtime" => o.kind == "Runtime",
            "probes" => o.kind == "Probe",
            _ => true,
        })
        .map(|o| {
            let result = o.result();
            View { o, result }
        })
        .collect())
}
#[tauri::command]
fn get_settings(state: tauri::State<State>) -> Settings {
    Settings::load(&state.settings_path)
}
#[tauri::command]
fn save_settings(state: tauri::State<State>, settings: Settings) -> Result<(), String> {
    settings
        .save(&state.settings_path)
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn delete_observation(state: tauri::State<State>, id: i64) -> Result<(), String> {
    state
        .db
        .lock()
        .map_err(|_| "database unavailable")?
        .delete(id)
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn clear_history(state: tauri::State<State>) -> Result<(), String> {
    state
        .db
        .lock()
        .map_err(|_| "database unavailable")?
        .clear()
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn verify_backend(
    state: tauri::State<State>,
    model: String,
    effort: String,
) -> Result<View, String> {
    if model.trim().is_empty() {
        return Err("Model is required".into());
    }
    let mut o = probe::run(model, effort);
    o.id = Some(
        state
            .db
            .lock()
            .map_err(|_| "database unavailable")?
            .insert(&o)
            .map_err(|e| e.to_string())?,
    );
    let result = o.result();
    Ok(View { o, result })
}
#[tauri::command]
fn open_folder(state: tauri::State<State>, kind: String) -> Result<(), String> {
    let p = if kind == "app" {
        state.app_dir.clone()
    } else {
        codex_home(&Settings::load(&state.settings_path))
    };
    opener::open(p).map_err(|e| e.to_string())
}
fn rollout(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == "jsonl") && path.to_string_lossy().contains("sessions")
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let app_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;
            let settings_path = app_dir.join("settings.json");
            let db = Database::open(&app_dir.join("runtime-watch.sqlite"))?;
            let settings = Settings::load(&settings_path);
            let home = codex_home(&settings);
            let initial_scan_days = settings.initial_scan_days;
            let state = Arc::new(Mutex::new(db));
            app.manage(State {
                db: Mutex::new(Database::open(&app_dir.join("runtime-watch.sqlite"))?),
                settings_path,
                app_dir,
            });
            if home.exists() {
                let handle = app.handle().clone();
                let db = state.clone();
                std::thread::spawn(move || {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let mut watcher = notify::recommended_watcher(tx).ok();
                    if let Some(w) = watcher.as_mut() {
                        let _ = w.watch(&home, RecursiveMode::Recursive);
                    }
                    let mut corr = Correlator::default();
                    for entry in walkdir::WalkDir::new(&home)
                        .into_iter()
                        .filter_map(Result::ok)
                        .filter(|e| rollout(e.path()))
                        .filter(|e| {
                            e.metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|m| m.elapsed().ok())
                                .map(|age| age.as_secs() <= u64::from(initial_scan_days) * 86_400)
                                .unwrap_or(false)
                        })
                    {
                        if let Ok(d) = db.lock() {
                            let _ = scan_file(&d, entry.path(), &mut corr);
                        }
                    }
                    for ev in rx.into_iter().flatten() {
                        for p in ev.paths.into_iter().filter(|p| rollout(p)) {
                            if let Ok(d) = db.lock() {
                                if scan_file(&d, &p, &mut corr).unwrap_or(0) > 0 {
                                    let _ = handle.emit("runtime-watch-update", ());
                                }
                            }
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            history,
            get_settings,
            save_settings,
            delete_observation,
            clear_history,
            verify_backend,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("application runtime failed")
}
