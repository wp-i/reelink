mod preferences;
mod qr;
mod rename;
mod tmdb;

use preferences::{PreferencesView, SavedPreferences};
use rename::{HistorySummary, RenameEdit, RenameEngine, RenameItem, RenameOutcome};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

struct PreferencesRuntime {
    saved: SavedPreferences,
    active_shortcut: Option<Shortcut>,
}

struct AppState {
    renamer: Mutex<RenameEngine>,
    preferences: Mutex<PreferencesRuntime>,
    preferences_path: PathBuf,
}

#[tauri::command]
fn preview_renames(state: State<AppState>, paths: Vec<String>) -> Result<Vec<RenameItem>, String> {
    state
        .renamer
        .lock()
        .map_err(|_| "文件操作状态异常，请重新启动应用。".to_string())?
        .preview(paths)
}

#[tauri::command]
fn apply_renames(state: State<AppState>, items: Vec<RenameEdit>) -> Result<RenameOutcome, String> {
    state
        .renamer
        .lock()
        .map_err(|_| "文件操作状态异常，请重新启动应用。".to_string())?
        .apply(items)
}

#[tauri::command]
fn undo_rename(state: State<AppState>) -> Result<RenameOutcome, String> {
    state
        .renamer
        .lock()
        .map_err(|_| "文件操作状态异常，请重新启动应用。".to_string())?
        .undo()
}

#[tauri::command]
fn rename_history(state: State<AppState>) -> Result<Option<HistorySummary>, String> {
    state
        .renamer
        .lock()
        .map_err(|_| "文件操作状态异常，请重新启动应用。".to_string())?
        .history()
}

#[tauri::command]
fn shortcut_available(state: State<AppState>) -> bool {
    state
        .preferences
        .lock()
        .is_ok_and(|value| value.active_shortcut.is_some())
}

#[tauri::command]
fn get_preferences(state: State<AppState>) -> Result<PreferencesView, String> {
    let current = state
        .preferences
        .lock()
        .map_err(|_| "设置状态异常，请重启应用。")?;
    Ok(current.saved.view(current.active_shortcut.is_some()))
}

#[tauri::command]
fn update_preferences(
    app: tauri::AppHandle,
    state: State<AppState>,
    shortcut: String,
) -> Result<PreferencesView, String> {
    let (canonical, next_shortcut) = preferences::normalize_shortcut(&shortcut)?;
    let mut current = state
        .preferences
        .lock()
        .map_err(|_| "设置状态异常，请重启应用。")?;
    let previous = current.active_shortcut;
    let needs_registration =
        previous != Some(next_shortcut) && !app.global_shortcut().is_registered(next_shortcut);
    if needs_registration {
        app.global_shortcut()
            .register(next_shortcut)
            .map_err(|_| "快捷键已被占用或不可用，请换一个组合。原设置未更改。".to_string())?;
    }
    let updated = SavedPreferences {
        shortcut: canonical,
    };
    if let Err(err) = preferences::save(&state.preferences_path, &updated) {
        if needs_registration {
            let _ = app.global_shortcut().unregister(next_shortcut);
        }
        return Err(err);
    }
    current.saved = updated;
    current.active_shortcut = Some(next_shortcut);
    if let Some(old) = previous.filter(|old| *old != next_shortcut) {
        // If removal fails, the handler still ignores this inactive binding.
        let _ = app.global_shortcut().unregister(old);
    }
    Ok(current.saved.view(true))
}

pub fn run() {
    tauri::Builder::default()
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.set_focus();
            }
        })
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
                let webview: &tauri::Webview = window.as_ref();
                let _ = webview.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, key, event| {
                    let active = app
                        .try_state::<AppState>()
                        .and_then(|state| {
                            state
                                .preferences
                                .lock()
                                .ok()
                                .map(|prefs| prefs.active_shortcut == Some(*key))
                        })
                        .unwrap_or(false);
                    if active && event.state() == ShortcutState::Pressed {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.emit("capture-requested", ());
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let preferences_path = data_dir.join("preferences.json");
            let saved = preferences::load(&preferences_path).unwrap_or_default();
            let shortcut = preferences::normalize_shortcut(&saved.shortcut)
                .map_err(std::io::Error::other)?
                .1;
            let active_shortcut = app
                .global_shortcut()
                .register(shortcut)
                .ok()
                .map(|_| shortcut);
            app.manage(AppState {
                renamer: Mutex::new(RenameEngine::new(data_dir)),
                preferences: Mutex::new(PreferencesRuntime {
                    saved,
                    active_shortcut,
                }),
                preferences_path,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            qr::decode_clipboard,
            qr::decode_image,
            qr::decode_image_path,
            qr::capture_qr,
            qr::copy_text,
            preview_renames,
            apply_renames,
            undo_rename,
            rename_history,
            tmdb::search_tmdb,
            shortcut_available,
            get_preferences,
            update_preferences,
        ])
        .run(tauri::generate_context!())
        .expect("Reelink could not start");
}
