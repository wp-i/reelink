mod capture_launch;
mod preferences;
mod qr;
mod rename;
mod resources;
#[cfg(windows)]
mod screen_capture;
mod shell_activation;
mod shell_hotkey;
mod tmdb;

use preferences::{PreferencesView, SavedPreferences};
use rename::{HistorySummary, RenameEdit, RenameEngine, RenameItem, RenameOutcome};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

struct PreferencesRuntime {
    saved: SavedPreferences,
    active_shortcut: Option<Shortcut>,
    shell_mode: ShellMode,
    cold_start_configured: bool,
    cold_start_error: Option<String>,
    saved_loaded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellMode {
    Portable,
    Owned,
    Unknown,
}

fn initial_shell_state(
    loaded: &Result<SavedPreferences, String>,
    inspect: impl FnOnce() -> Result<bool, String>,
    configure: impl FnOnce(&str) -> Result<bool, String>,
) -> (ShellMode, bool, Option<String>) {
    match inspect() {
        Ok(false) => (ShellMode::Portable, false, loaded.as_ref().err().cloned()),
        Err(error) => (
            ShellMode::Unknown,
            false,
            Some(format!("无法确认系统截图快捷方式：{error}。请检查后重试。")),
        ),
        Ok(true) => match loaded {
            Err(error) => (ShellMode::Owned, false, Some(error.clone())),
            Ok(saved) => match configure(&saved.shortcut) {
                Ok(true) => (ShellMode::Owned, true, None),
                Ok(false) => (
                    ShellMode::Owned,
                    false,
                    Some("系统截图快捷方式已变化，请检查后重试。".into()),
                ),
                Err(error) => (
                    ShellMode::Owned,
                    false,
                    Some(format!("无法同步系统截图快捷方式：{error}。请检查后重试。")),
                ),
            },
        },
    }
}

impl PreferencesRuntime {
    fn view(&self) -> PreferencesView {
        let mut view = self.saved.view(self.active_shortcut.is_some());
        view.cold_start_configured = self.cold_start_configured;
        view.cold_start_error = self.cold_start_error.clone();
        view
    }

    fn shell_sync_failure(&mut self, error: String) -> String {
        let message = format!(
            "{error} 保存设置仍为 {}，请检查后重试。",
            self.saved.shortcut
        );
        self.cold_start_configured = false;
        self.cold_start_error = Some(message.clone());
        message
    }
}

fn shell_save_failure(
    current: &mut PreferencesRuntime,
    save_error: String,
    rollback: Result<bool, String>,
) -> String {
    match rollback {
        Ok(true) => {
            current.cold_start_configured = true;
            current.cold_start_error = None;
            save_error
        }
        Ok(false) => {
            let message = format!(
                "{save_error}；系统截图快捷方式已变化，无法恢复原快捷键。保存设置仍为 {}，请检查后重试。",
                current.saved.shortcut
            );
            current.cold_start_configured = false;
            current.cold_start_error = Some(message.clone());
            message
        }
        Err(rollback_error) => {
            let message = format!(
                "{save_error}；恢复系统截图快捷键失败：{rollback_error}。保存设置仍为 {}，请检查后重试。",
                current.saved.shortcut
            );
            current.cold_start_configured = false;
            current.cold_start_error = Some(message.clone());
            message
        }
    }
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
    Ok(current.view())
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
    if !current.saved_loaded {
        return Err(current
            .cold_start_error
            .clone()
            .unwrap_or_else(|| "无法读取已保存的快捷键设置，请检查设置文件。".into()));
    }
    if current.shell_mode != ShellMode::Portable {
        match shell_hotkey::inspect_existing() {
            Ok(true) => current.shell_mode = ShellMode::Owned,
            Ok(false) => {
                return Err(current
                    .shell_sync_failure("开始菜单中的 Reelink 截图入口已被移动或修改。".into()));
            }
            Err(error) => {
                return Err(
                    current.shell_sync_failure(format!("无法确认系统截图快捷方式：{error}。"))
                );
            }
        }
        match shell_hotkey::configure_existing(&canonical) {
            Ok(true) => {}
            Ok(false) => {
                return Err(current
                    .shell_sync_failure("开始菜单中的 Reelink 截图入口已被移动或修改。".into()));
            }
            Err(error) => {
                return Err(
                    current.shell_sync_failure(format!("无法同步系统截图快捷方式：{error}。"))
                );
            }
        }
        let updated = SavedPreferences {
            shortcut: canonical,
        };
        if let Err(error) = preferences::save(&state.preferences_path, &updated) {
            let rollback = shell_hotkey::configure_existing(&current.saved.shortcut);
            return Err(shell_save_failure(&mut current, error, rollback));
        }
        current.saved = updated;
        current.cold_start_configured = true;
        current.cold_start_error = None;
        if let Err(error) = shell_activation::sync_owned(&app, &current.saved.shortcut) {
            let message = format!("快捷键 {} 已保存，但{error}", current.saved.shortcut);
            current.cold_start_configured = false;
            current.cold_start_error = Some(message.clone());
            return Err(message);
        }
        return Ok(current.view());
    }
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
    Ok(current.view())
}

pub fn run() {
    let context = tauri::generate_context!();
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    #[cfg(debug_assertions)]
    if std::env::var_os("REELINK_QA_SHORTCUT_TRACE").is_some() {
        eprintln!("capture-shortcut QA arguments: {args:?}");
    }
    if args.len() == 1 {
        let result = match args[0].to_str() {
            Some("--install-capture-shortcut") => Some((|| {
                let path = dirs::data_local_dir()
                    .ok_or("无法查找本机应用数据目录。")?
                    .join(&context.config().identifier)
                    .join("preferences.json");
                let saved = preferences::load(&path)?;
                shell_hotkey::install_current(&saved.shortcut)
            })()),
            Some("--remove-capture-shortcut") => Some(shell_hotkey::remove_current().map(|_| ())),
            _ => None,
        };
        if let Some(result) = result {
            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
    }
    let initial_capture = args.iter().any(|arg| arg == "--capture");
    tauri::Builder::default()
        .manage(capture_launch::CaptureLaunchState::new(initial_capture))
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .on_page_load(|webview, payload| {
            if webview.label() == "main" {
                match payload.event() {
                    tauri::webview::PageLoadEvent::Started => webview
                        .state::<capture_launch::CaptureLaunchState>()
                        .frontend_loading(),
                    tauri::webview::PageLoadEvent::Finished => {
                        let _ = webview.set_focus();
                    }
                }
            }
        })
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if args.iter().skip(1).any(|arg| arg == "--capture") {
                app.state::<capture_launch::CaptureLaunchState>()
                    .request_capture(app);
                return;
            }
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
                        app.state::<capture_launch::CaptureLaunchState>()
                            .request_capture(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let preferences_path = data_dir.join("preferences.json");
            app.manage(resources::ResourceStore::new(
                data_dir.join("resource-sites.json"),
            ));
            let loaded = preferences::load(&preferences_path);
            // Inspect before syncing. A readable owned link remains in Shell mode
            // when a write fails; an unreadable link is ambiguous and must not
            // enable the portable in-process hotkey.
            let (shell_mode, mut cold_start_configured, mut cold_start_error) = initial_shell_state(
                &loaded,
                shell_hotkey::inspect_existing,
                shell_hotkey::configure_existing,
            );
            let saved_loaded = loaded.is_ok();
            let saved = loaded.unwrap_or_default();
            let shortcut = preferences::normalize_shortcut(&saved.shortcut)
                .map_err(std::io::Error::other)?
                .1;
            // Only a verified owned installation with readable preferences may
            // establish a window binding. Unknown/corrupt state must not bind
            // fallback defaults. Portable launches keep RegisterHotKey below.
            let window_binding = if shell_mode == ShellMode::Owned && saved_loaded {
                shell_activation::sync_owned(app.handle(), &saved.shortcut)
            } else {
                shell_activation::install(app.handle())
            };
            if let Err(error) = window_binding {
                cold_start_configured = false;
                cold_start_error = Some(match cold_start_error {
                    Some(previous) => format!("{previous}；{error}"),
                    None => error,
                });
            }
            let active_shortcut = if shell_mode != ShellMode::Portable {
                None
            } else {
                app.global_shortcut()
                    .register(shortcut)
                    .ok()
                    .map(|_| shortcut)
            };
            app.manage(AppState {
                renamer: Mutex::new(RenameEngine::new(data_dir)),
                preferences: Mutex::new(PreferencesRuntime {
                    saved,
                    active_shortcut,
                    shell_mode,
                    cold_start_configured,
                    cold_start_error,
                    saved_loaded,
                }),
                preferences_path,
            });
            Ok(())
        })
        .invoke_handler(|invoke: tauri::ipc::Invoke<tauri::Wry>| {
            let browser_command = matches!(
                invoke.message.command(),
                "resource_browser_state" | "resource_browser_action"
            );
            // The local browser toolbar may only control its own remote view.
            // All filesystem, clipboard and future app commands remain main-only.
            let authorized = if browser_command {
                resources::is_trusted_resource_toolbar(&invoke.message.webview())
            } else {
                resources::is_main_window(invoke.message.webview().label())
            };
            if !authorized {
                invoke.resolver.reject("此窗口不能调用本地应用命令。");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                qr::decode_clipboard,
                qr::decode_image,
                qr::decode_image_path,
                qr::capture_qr,
                qr::copy_text,
                capture_launch::capture_frontend_ready,
                preview_renames,
                apply_renames,
                undo_rename,
                rename_history,
                tmdb::search_tmdb,
                shortcut_available,
                get_preferences,
                update_preferences,
                resources::list_resource_sites,
                resources::save_resource_site,
                resources::delete_resource_site,
                resources::open_resource_site,
                resources::browser::resource_browser_state,
                resources::browser::resource_browser_action,
            ];
            handler(invoke)
        })
        .run(context)
        .expect("Reelink could not start");
}

#[cfg(test)]
mod shell_startup_tests {
    use super::*;

    fn owned_runtime() -> PreferencesRuntime {
        PreferencesRuntime {
            saved: SavedPreferences {
                shortcut: "Alt+Q".into(),
            },
            active_shortcut: None,
            shell_mode: ShellMode::Owned,
            cold_start_configured: true,
            cold_start_error: None,
            saved_loaded: true,
        }
    }

    #[test]
    fn failed_rollback_marks_shell_unsynced_without_changing_saved_preference() {
        let mut current = owned_runtime();
        let error = shell_save_failure(&mut current, "无法保存设置".into(), Err("拒绝访问".into()));
        assert!(error.contains("拒绝访问"));
        assert!(error.contains("Alt+Q"));
        assert_eq!(current.saved.shortcut, "Alt+Q");
        assert_eq!(current.shell_mode, ShellMode::Owned);
        assert!(!current.cold_start_configured);
        assert_eq!(current.cold_start_error.as_deref(), Some(error.as_str()));

        let restored = shell_save_failure(&mut current, "无法保存设置".into(), Ok(true));
        assert_eq!(restored, "无法保存设置");
        assert!(current.cold_start_configured);
        assert!(current.cold_start_error.is_none());
    }

    #[test]
    fn missing_link_during_rollback_is_reported_as_unsynced() {
        let mut current = owned_runtime();
        let error = shell_save_failure(&mut current, "无法保存设置".into(), Ok(false));
        assert!(error.contains("快捷方式已变化"));
        assert!(!current.cold_start_configured);
    }

    #[test]
    fn failed_shell_sync_clears_success_status_and_keeps_saved_preference_and_mode() {
        for mode in [ShellMode::Owned, ShellMode::Unknown] {
            let mut current = owned_runtime();
            current.shell_mode = mode;
            let error = current.shell_sync_failure("无法同步系统截图快捷方式：拒绝访问。".into());
            let view = current.view();

            assert_eq!(current.shell_mode, mode);
            assert_eq!(current.saved.shortcut, "Alt+Q");
            assert!(!view.cold_start_configured);
            assert_eq!(view.cold_start_error.as_deref(), Some(error.as_str()));
            assert!(error.contains("拒绝访问"));
            assert!(error.contains("Alt+Q"));
        }
    }

    #[test]
    fn owned_link_write_failure_stays_in_shell_mode_and_can_retry() {
        let loaded = Ok(SavedPreferences {
            shortcut: "Alt+Q".into(),
        });
        let (mode, configured, error) =
            initial_shell_state(&loaded, || Ok(true), |_| Err("拒绝访问".into()));
        assert_eq!(mode, ShellMode::Owned);
        assert!(!configured);
        assert!(error.unwrap().contains("拒绝访问"));

        let (mode, configured, error) = initial_shell_state(&loaded, || Ok(true), |_| Ok(true));
        assert_eq!(mode, ShellMode::Owned);
        assert!(configured);
        assert!(error.is_none());
    }

    #[test]
    fn unreadable_link_and_unreadable_preferences_never_rewrite_from_defaults() {
        let loaded = Err("设置文件格式错误".into());
        let (mode, configured, error) = initial_shell_state(
            &loaded,
            || Err("拒绝访问".into()),
            |_| panic!("must not configure an unreadable link"),
        );
        assert_eq!(mode, ShellMode::Unknown);
        assert!(!configured);
        assert!(error.unwrap().contains("拒绝访问"));

        let (mode, configured, error) = initial_shell_state(
            &loaded,
            || Ok(true),
            |_| panic!("must not write a link from fallback preferences"),
        );
        assert_eq!(mode, ShellMode::Owned);
        assert!(!configured);
        assert!(error.unwrap().contains("设置文件格式错误"));
    }
}
