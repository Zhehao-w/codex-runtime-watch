use codex_runtime_watch::{
    codex::{probe, rollout::Correlator, watcher::scan_file_observations},
    db::Database,
    settings::Settings,
    Observation,
};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_notification::{NotificationExt, PermissionState};
struct State {
    db: Mutex<Database>,
    settings_path: PathBuf,
    app_dir: PathBuf,
    watcher: Mutex<std::sync::mpsc::Sender<WatchCommand>>,
    watcher_status: std::sync::Arc<Mutex<String>>,
    login_menu: Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
}
enum WatchCommand {
    Configure(PathBuf, u32, bool),
    Path(PathBuf),
}
#[derive(Serialize)]
struct View {
    #[serde(flatten)]
    o: Observation,
    result: String,
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
        .history(&filter, limit, offset)
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|o| {
            let result = o.result();
            View { o, result }
        })
        .collect())
}
#[tauri::command]
fn current_runtime(state: tauri::State<State>) -> Result<Option<View>, String> {
    let row = state
        .db
        .lock()
        .map_err(|_| "database unavailable")?
        .current_runtime()
        .map_err(|e| e.to_string())?;
    Ok(row.map(|o| {
        let result = o.result();
        View { o, result }
    }))
}
#[tauri::command]
fn watcher_status(state: tauri::State<State>) -> String {
    state
        .watcher_status
        .lock()
        .map(|status| status.clone())
        .unwrap_or_else(|_| "Watcher error".into())
}
#[tauri::command]
fn get_settings(app: tauri::AppHandle, state: tauri::State<State>) -> Settings {
    let mut settings = Settings::load(&state.settings_path);
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        if settings.start_at_login != enabled {
            settings.start_at_login = enabled;
            let _ = settings.save(&state.settings_path);
        }
    }
    settings
}
#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<State>,
    settings: Settings,
) -> Result<(), String> {
    settings
        .save(&state.settings_path)
        .map_err(|e| e.to_string())?;
    let autostart = app.autolaunch();
    if settings.start_at_login {
        autostart.enable()
    } else {
        autostart.disable()
    }
    .map_err(|e| e.to_string())?;
    if let Ok(menu) = state.login_menu.lock() {
        if let Some(menu) = menu.as_ref() {
            let actual = autostart.is_enabled().unwrap_or(settings.start_at_login);
            let _ = menu.set_checked(actual);
        }
    }
    if settings.notify_mismatch
        && matches!(
            app.notification().permission_state(),
            Ok(PermissionState::Prompt | PermissionState::PromptWithRationale)
        )
    {
        let _ = app.notification().request_permission();
    }
    state
        .watcher
        .lock()
        .map_err(|_| "watcher unavailable")?
        .send(WatchCommand::Configure(
            codex_home(&settings),
            settings.initial_scan_days,
            settings.notify_mismatch,
        ))
        .map_err(|_| "watcher unavailable".to_string())
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
async fn verify_backend(
    app: tauri::AppHandle,
    model: String,
    effort: String,
) -> Result<View, String> {
    if model.trim().is_empty() {
        return Err("Model is required".into());
    }
    let home = {
        let state = app.state::<State>();
        codex_home(&Settings::load(&state.settings_path))
    };
    let mut o = tauri::async_runtime::spawn_blocking(move || probe::run(&home, model, effort))
        .await
        .map_err(|e| format!("Probe task failed: {e}"))?;
    let state = app.state::<State>();
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
    path.extension().is_some_and(|x| x == "jsonl")
}
fn scan_path(
    db: &Database,
    path: &Path,
    corr: &mut Correlator,
    handle: &tauri::AppHandle,
    notify: bool,
) {
    let observations = scan_file_observations(db, path, corr).unwrap_or_default();
    if observations.is_empty() {
        return;
    }
    let _ = handle.emit("runtime-watch-update", ());
    if notify {
        for change in observations.into_iter().filter(|change| {
            change.notify
                && change.observation.kind == "Runtime"
                && change.observation.has_any_mismatch_or_reroute()
        }) {
            if !matches!(
                handle.notification().permission_state(),
                Ok(PermissionState::Granted)
            ) {
                continue;
            }
            let _ = handle
                .notification()
                .builder()
                .title("Codex Runtime Watch")
                .body(change.observation.result())
                .show();
        }
    }
}
fn show_main(app: &tauri::AppHandle, verify: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        if verify {
            let _ = app.emit("runtime-watch-open-verify", ());
        }
    }
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|arg| arg == "--hidden") {
                show_main(app, false);
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--hidden"])
                .build(),
        )
        .setup(|app| {
            let app_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;
            let settings_path = app_dir.join("settings.json");
            let mut settings = Settings::load(&settings_path);
            if let Ok(enabled) = app.autolaunch().is_enabled() {
                if settings.start_at_login != enabled {
                    settings.start_at_login = enabled;
                    settings.save(&settings_path)?;
                }
            }
            let home = codex_home(&settings);
            let initial_scan_days = settings.initial_scan_days;
            let notify_mismatch = settings.notify_mismatch;
            if notify_mismatch
                && matches!(
                    app.notification().permission_state(),
                    Ok(PermissionState::Prompt | PermissionState::PromptWithRationale)
                )
            {
                let _ = app.notification().request_permission();
            }
            let db_path = app_dir.join("runtime-watch.sqlite");
            let tray_settings_path = settings_path.clone();
            let (command_tx, command_rx) = std::sync::mpsc::channel();
            let watcher_status = std::sync::Arc::new(Mutex::new(String::from("Starting")));
            app.manage(State {
                db: Mutex::new(Database::open(&db_path)?),
                settings_path,
                app_dir,
                watcher: Mutex::new(command_tx.clone()),
                watcher_status: watcher_status.clone(),
                login_menu: Mutex::new(None),
            });
            let handle = app.handle().clone();
            let watcher_tx = command_tx.clone();
            let thread_status = watcher_status.clone();
            std::thread::spawn(move || {
                let db = match Database::open(&db_path) {
                    Ok(x) => x,
                    Err(_) => return,
                };
                let mut corr = Correlator::default();
                let mut watcher: Option<notify::RecommendedWatcher> = None;
                let mut notify_enabled = notify_mismatch;
                while let Ok(command) = command_rx.recv() {
                    match command {
                        WatchCommand::Configure(home, days, notifications) => {
                            notify_enabled = notifications;
                            drop(watcher.take());
                            let sessions = home.join("sessions");
                            if !sessions.is_dir() {
                                if let Ok(mut status) = thread_status.lock() {
                                    *status = "Codex sessions folder not found".into();
                                }
                                let _ = handle.emit("runtime-watch-update", ());
                                continue;
                            }
                            for entry in walkdir::WalkDir::new(&sessions)
                                .into_iter()
                                .filter_map(Result::ok)
                                .filter(|e| rollout(e.path()))
                                .filter(|e| {
                                    e.metadata()
                                        .ok()
                                        .and_then(|m| m.modified().ok())
                                        .and_then(|m| m.elapsed().ok())
                                        .map(|age| age.as_secs() <= u64::from(days) * 86_400)
                                        .unwrap_or(false)
                                })
                            {
                                scan_path(&db, entry.path(), &mut corr, &handle, false);
                            }
                            let tx = watcher_tx.clone();
                            watcher = notify::recommended_watcher(
                                move |event: notify::Result<notify::Event>| {
                                    if let Ok(event) = event {
                                        for path in event.paths {
                                            let _ = tx.send(WatchCommand::Path(path));
                                        }
                                    }
                                },
                            )
                            .ok();
                            if let Some(w) = watcher.as_mut() {
                                let status = if w.watch(&sessions, RecursiveMode::Recursive).is_ok()
                                {
                                    "Watching"
                                } else {
                                    "Watcher error"
                                };
                                if let Ok(mut current) = thread_status.lock() {
                                    *current = status.into();
                                }
                                let _ = handle.emit("runtime-watch-update", ());
                            } else if let Ok(mut status) = thread_status.lock() {
                                *status = "Watcher error".into();
                                let _ = handle.emit("runtime-watch-update", ());
                            }
                        }
                        WatchCommand::Path(path) => {
                            if rollout(&path) {
                                scan_path(&db, &path, &mut corr, &handle, notify_enabled);
                            }
                        }
                    }
                }
            });
            let _ = command_tx.send(WatchCommand::Configure(
                home,
                initial_scan_days,
                notify_mismatch,
            ));

            use tauri::menu::{CheckMenuItem, Menu, MenuItem};
            use tauri::tray::TrayIconBuilder;
            let open =
                MenuItem::with_id(app, "open", "Open Codex Runtime Watch", true, None::<&str>)?;
            let verify = MenuItem::with_id(app, "verify", "Verify Backend", true, None::<&str>)?;
            let login = CheckMenuItem::with_id(
                app,
                "login",
                "Start at Login",
                true,
                settings.start_at_login,
                None::<&str>,
            )?;
            if let Ok(mut item) = app.state::<State>().login_menu.lock() {
                *item = Some(login.clone());
            }
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &verify, &login, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open" => show_main(app, false),
                    "verify" => show_main(app, true),
                    "login" => {
                        let manager = app.autolaunch();
                        let enabled = manager.is_enabled().unwrap_or(false);
                        let changed = if enabled {
                            manager.disable().map(|_| false)
                        } else {
                            manager.enable().map(|_| true)
                        };
                        if let Ok(enabled) = changed {
                            let _ = login.set_checked(enabled);
                            let mut settings = Settings::load(&tray_settings_path);
                            settings.start_at_login = enabled;
                            let _ = settings.save(&tray_settings_path);
                            let _ = app.emit("runtime-watch-update", ());
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            if let Some(window) = app.get_webview_window("main") {
                let close_window = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = close_window.hide();
                    }
                });
                if std::env::args().any(|arg| arg == "--hidden") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            history,
            current_runtime,
            watcher_status,
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
