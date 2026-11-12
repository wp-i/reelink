use super::{emit_state, is_current_session, set_loading, BrowserSession, ResourceBrowserAction};
use std::{cell::Cell, rc::Rc};
use tauri::{Emitter, Manager};
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler, DocumentTitleChangedEventHandler,
    HistoryChangedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2Controller, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    },
    NavigationCompletedEventHandler, NavigationStartingEventHandler, SourceChangedEventHandler,
};
use windows::core::{BOOL, HSTRING, PWSTR};

fn refresh(
    core: &ICoreWebView2,
    app: &tauri::AppHandle,
    label: &str,
    session: &BrowserSession,
) -> windows::core::Result<()> {
    if !is_current_session(app, label, session) {
        return Ok(());
    }
    unsafe {
        let mut source = PWSTR::null();
        core.Source(&mut source)?;
        let source = take_pwstr(source);
        let mut title = PWSTR::null();
        core.DocumentTitle(&mut title)?;
        let title = take_pwstr(title);
        let mut back = BOOL::default();
        let mut forward = BOOL::default();
        core.CanGoBack(&mut back)?;
        core.CanGoForward(&mut forward)?;
        if let Ok(mut state) = session.state.lock() {
            // The displayed address always comes from the browser's committed Source.
            if super::validate_url(&source).is_ok() {
                state.url = source;
            }
            state.title = title;
            state.can_go_back = back.as_bool();
            state.can_go_forward = forward.as_bool();
        }
    }
    emit_state(app, label, session);
    Ok(())
}

pub fn perform(
    controller: &ICoreWebView2Controller,
    action: ResourceBrowserAction,
    destination: Option<&tauri::Url>,
) -> windows::core::Result<()> {
    unsafe {
        let core = controller.CoreWebView2()?;
        match action {
            ResourceBrowserAction::Back => {
                let mut enabled = BOOL::default();
                core.CanGoBack(&mut enabled)?;
                if enabled.as_bool() {
                    core.GoBack()?;
                }
            }
            ResourceBrowserAction::Forward => {
                let mut enabled = BOOL::default();
                core.CanGoForward(&mut enabled)?;
                if enabled.as_bool() {
                    core.GoForward()?;
                }
            }
            ResourceBrowserAction::Reload => core.Reload()?,
            ResourceBrowserAction::Stop => core.Stop()?,
            ResourceBrowserAction::Home | ResourceBrowserAction::Navigate => {
                if let Some(url) = destination {
                    core.Navigate(&HSTRING::from(url.as_str()))?;
                }
            }
        }
    }
    Ok(())
}

pub fn install(
    controller: &ICoreWebView2Controller,
    app: tauri::AppHandle,
    label: String,
    session: BrowserSession,
) -> windows::core::Result<()> {
    if !is_current_session(&app, &label, &session) {
        return Ok(());
    }
    unsafe {
        let core = controller.CoreWebView2()?;
        let mut token = 0;
        // Registered handlers live with this native WebView. None retains the COM
        // source/controller, so closing a resource window releases its handlers.
        macro_rules! observe {
            ($method:ident, $handler:ident) => {{
                let app = app.clone();
                let label = label.clone();
                let session = session.clone();
                core.$method(
                    &$handler::create(Box::new(move |sender, _| {
                        if let Some(core) = sender {
                            refresh(&core, &app, &label, &session)?;
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
            }};
        }
        observe!(add_SourceChanged, SourceChangedEventHandler);
        observe!(add_HistoryChanged, HistoryChangedEventHandler);
        observe!(add_DocumentTitleChanged, DocumentTitleChangedEventHandler);
        let navigation = Rc::new(Cell::new(0u64));
        {
            let app = app.clone();
            let label = label.clone();
            let session = session.clone();
            let navigation = navigation.clone();
            core.add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |sender, args| {
                    if let Some(args) = args {
                        let mut cancelled = BOOL::default();
                        args.Cancel(&mut cancelled)?;
                        let mut id = 0;
                        args.NavigationId(&mut id)?;
                        navigation.set(id);
                        set_loading(&app, &label, &session, !cancelled.as_bool());
                    }
                    if let Some(core) = sender {
                        refresh(&core, &app, &label, &session)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
        {
            let app = app.clone();
            let label = label.clone();
            let session = session.clone();
            core.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
                    if let Some(args) = args {
                        let mut id = 0;
                        args.NavigationId(&mut id)?;
                        if navigation.get() == 0 || navigation.get() == id {
                            set_loading(&app, &label, &session, false);
                        }
                    }
                    if let Some(core) = sender {
                        refresh(&core, &app, &label, &session)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
        {
            let app = app.clone();
            let label = label.clone();
            let session = session.clone();
            controller.add_AcceleratorKeyPressed(
                &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                    if !is_current_session(&app, &label, &session) {
                        return Ok(());
                    }
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                    args.KeyEventKind(&mut kind)?;
                    if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                        && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                    {
                        return Ok(());
                    }
                    let mut key = 0;
                    args.VirtualKey(&mut key)?;
                    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                        GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
                    };
                    let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
                    let alt = GetKeyState(VK_MENU as i32) < 0;
                    let shift = GetKeyState(VK_SHIFT as i32) < 0;
                    let focus_address = ctrl && !alt && !shift && key == 0x4c;
                    let action = match (ctrl, alt, shift, key) {
                        (false, true, false, 0x25) => Some(ResourceBrowserAction::Back),
                        (false, true, false, 0x27) => Some(ResourceBrowserAction::Forward),
                        (true, false, false, 0x52) | (false, false, false, 0x74) => {
                            Some(ResourceBrowserAction::Reload)
                        }
                        (false, false, false, 0x1b)
                            if session
                                .state
                                .lock()
                                .map(|state| state.loading)
                                .unwrap_or(false) =>
                        {
                            Some(ResourceBrowserAction::Stop)
                        }
                        _ => None,
                    };
                    if !focus_address && action.is_none() {
                        return Ok(());
                    }
                    args.SetHandled(true)?;
                    let task_app = app.clone();
                    let label = label.clone();
                    let session = session.clone();
                    // Accelerator events are synchronous; defer focus/navigation until
                    // after returning to WebView2 rather than re-entering its handler.
                    // run_on_main_thread executes immediately when called on the
                    // UI thread. Dispatch from an async worker to actually leave
                    // this synchronous WebView2 callback before touching COM.
                    tauri::async_runtime::spawn(async move {
                        let dispatch_app = task_app.clone();
                        let _ = dispatch_app.run_on_main_thread(move || {
                            if !is_current_session(&task_app, &label, &session) {
                                return;
                            }
                            if focus_address {
                                let toolbar = label.replacen("resource-", "resource-toolbar-", 1);
                                if let Some(webview) = task_app.get_webview(&toolbar) {
                                    let _ = webview.set_focus();
                                    let _ = task_app.emit_to(
                                        tauri::EventTarget::webview(toolbar),
                                        "resource-browser-focus-address",
                                        (),
                                    );
                                }
                            } else if let Some(action) = action {
                                if let Some(remote) = task_app.get_webview(&label) {
                                    let _ = remote.with_webview(move |platform| {
                                        if !is_current_session(&task_app, &label, &session) {
                                            return;
                                        }
                                        let result = perform(&platform.controller(), action, None);
                                        if matches!(action, ResourceBrowserAction::Stop)
                                            && result.is_ok()
                                        {
                                            set_loading(&task_app, &label, &session, false);
                                        }
                                        if let Ok(core) = platform.controller().CoreWebView2() {
                                            let _ = refresh(&core, &task_app, &label, &session);
                                        }
                                    });
                                }
                            }
                        });
                    });
                    Ok(())
                })),
                &mut token,
            )?;
        }
        refresh(&core, &app, &label, &session)?;
    }
    Ok(())
}
