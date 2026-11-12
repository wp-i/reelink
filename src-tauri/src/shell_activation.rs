//! Turn the Shell's existing-window activation into the same capture request as --capture.

#[cfg(windows)]
pub use platform::{install, sync_owned};

#[cfg(not(windows))]
pub fn install(_: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn sync_owned(_: &tauri::AppHandle, _: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use tauri::{AppHandle, Manager};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Shell::{
        DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, SendMessageW, SC_HOTKEY, WM_GETHOTKEY, WM_NCDESTROY,
        WM_SETHOTKEY, WM_SYSCOMMAND,
    };

    const SUBCLASS_ID: usize = 0x524C484B;

    #[cfg(debug_assertions)]
    fn trace(message: impl AsRef<str>) {
        if std::env::var_os("REELINK_QA_SHORTCUT_TRACE").is_some() {
            eprintln!("capture-shortcut window: {}", message.as_ref());
        }
    }

    #[cfg(not(debug_assertions))]
    fn trace(_: impl AsRef<str>) {}

    fn is_capture_activation(message: u32, wparam: WPARAM) -> bool {
        message == WM_SYSCOMMAND && (wparam & 0xfff0) == SC_HOTKEY as usize
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        context: usize,
    ) -> LRESULT {
        if is_capture_activation(message, wparam) {
            // Clone before emitting: a nested message may destroy the HWND and
            // release its context. Never retain a borrowed context across dispatch.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                trace(format!("SC_HOTKEY received on HWND {hwnd:p}"));
                let app = unsafe { (&*(context as *const AppHandle)).clone() };
                if let Some(state) = app.try_state::<crate::capture_launch::CaptureLaunchState>() {
                    state.request_capture(&app);
                };
            }));
            // Do not activate the main window over the screen being captured.
            return 0;
        }
        if message == WM_NCDESTROY {
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                if RemoveWindowSubclass(hwnd, Some(window_proc), subclass_id) != 0 {
                    drop(Box::from_raw(context as *mut AppHandle));
                }
            }));
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    fn main_hwnd(app: &AppHandle) -> Result<HWND, String> {
        app.get_webview_window("main")
            .ok_or_else(|| "主窗口已关闭。".to_string())?
            .hwnd()
            .map(|handle| handle.0)
            .map_err(|error| format!("无法读取主窗口：{error}"))
    }

    fn on_window_thread(
        app: &AppHandle,
        operation: impl FnOnce(&AppHandle) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        let hwnd = main_hwnd(app)?;
        let thread = unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) };
        if thread == 0 {
            return Err("主窗口已关闭。".into());
        }
        let current = unsafe { GetCurrentThreadId() };
        trace(format!(
            "HWND {hwnd:p} owner thread {thread}, current thread {current}"
        ));
        if thread == current {
            return operation(app);
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let handle = app.clone();
        app.run_on_main_thread(move || {
            trace(format!(
                "dispatched callback thread {} (HWND owner {thread})",
                unsafe { GetCurrentThreadId() }
            ));
            let result = catch_unwind(AssertUnwindSafe(|| operation(&handle)))
                .unwrap_or_else(|_| Err("窗口快捷键处理发生异常。".into()));
            let _ = sender.send(result);
        })
        .map_err(|error| format!("无法更新窗口快捷键：{error}"))?;
        receiver.recv().map_err(|_| "窗口快捷键更新未完成。")?
    }

    fn install_on_window_thread(app: &AppHandle) -> Result<(), String> {
        let hwnd = main_hwnd(app)?;
        let mut existing = 0;
        if unsafe { GetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, &mut existing) } != 0 {
            trace(format!("subclass already installed on HWND {hwnd:p}"));
            return Ok(());
        }
        let context = Box::into_raw(Box::new(app.clone()));
        if unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, context as usize) } == 0
        {
            unsafe { drop(Box::from_raw(context)) };
            return Err("无法接收当前窗口的系统截图快捷键。".into());
        }
        trace(format!("subclass installed on HWND {hwnd:p}"));
        Ok(())
    }

    pub fn install(app: &AppHandle) -> Result<(), String> {
        on_window_thread(app, install_on_window_thread)
    }

    fn sync_binding(
        requested: u16,
        mut read: impl FnMut() -> usize,
        mut write: impl FnMut(u16) -> isize,
    ) -> Result<(), String> {
        // A previous WM_SETHOTKEY result of 2 already changes the window's
        // value but reports a conflict. Recheck even an equal value on retry.
        let result = write(requested);
        let actual = read();
        trace(format!(
            "WM_SETHOTKEY requested 0x{requested:04X}, result {result}, WM_GETHOTKEY 0x{actual:04X}"
        ));
        if result != 1 || actual != requested as usize {
            return Err(format!(
                "当前窗口快捷键未完整同步（系统返回 {result}，窗口键值 0x{actual:04X}）；请关闭应用后重新打开并检查快捷键冲突。"
            ));
        }
        Ok(())
    }

    // Call only after verifying ownership and successfully loading preferences.
    pub fn sync_owned(app: &AppHandle, shortcut: &str) -> Result<(), String> {
        let requested = crate::shell_hotkey::hotkey_word(shortcut)?;
        on_window_thread(app, move |app| {
            install_on_window_thread(app)?;
            let hwnd = main_hwnd(app)?;
            // Ordinary desktop launches may have no Shell-provided association.
            // Establish one so an open installed app handles SC_HOTKEY reliably.
            sync_binding(
                requested,
                || unsafe { SendMessageW(hwnd, WM_GETHOTKEY, 0, 0) } as usize,
                |key| unsafe { SendMessageW(hwnd, WM_SETHOTKEY, key as usize, 0) },
            )
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn capture_only_handles_hotkey_system_command() {
            assert!(is_capture_activation(WM_SYSCOMMAND, SC_HOTKEY as usize));
            assert!(is_capture_activation(WM_SYSCOMMAND, SC_HOTKEY as usize | 3));
            assert!(!is_capture_activation(WM_SETHOTKEY, SC_HOTKEY as usize));
            assert!(!is_capture_activation(WM_SYSCOMMAND, 0xf060));
        }

        #[test]
        fn owned_ordinary_launches_gain_a_window_binding() {
            let current = std::cell::Cell::new(0usize);
            assert!(sync_binding(
                0x0451,
                || current.get(),
                |key| {
                    current.set(key as usize);
                    1
                },
            )
            .is_ok());
            assert_eq!(current.get(), 0x0451);
        }

        #[test]
        fn unsuccessful_or_unconfirmed_bindings_are_reported() {
            for result in [-1, 0, 2] {
                assert!(sync_binding(0x0451, || 0x0451, |_| result).is_err());
            }
            assert!(sync_binding(0x0451, || 0, |_| 1).is_err());
        }

        #[test]
        fn a_conflicting_success_is_rechecked_on_retry() {
            use std::cell::Cell;
            let current = Cell::new(0x0451usize);
            let calls = Cell::new(0);
            for _ in 0..2 {
                let result = sync_binding(
                    0x0457,
                    || current.get(),
                    |key| {
                        current.set(key as usize);
                        calls.set(calls.get() + 1);
                        2
                    },
                );
                assert!(result.is_err());
            }
            assert_eq!(calls.get(), 2);
            assert!(sync_binding(0x0457, || current.get(), |_| 1).is_ok());
        }
    }
}
