use super::{validate_url, ResourceStore};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager, Webview};

#[cfg(windows)]
mod native;

pub const TOOLBAR_HEIGHT: f64 = 56.0;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBrowserState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

#[derive(Clone)]
pub struct BrowserSession {
    pub state: Arc<Mutex<ResourceBrowserState>>,
    pub native_ready: Arc<AtomicBool>,
    pub home: String,
    pub toolbar_url: tauri::Url,
}

impl BrowserSession {
    pub fn new(home: String, title: String, toolbar_url: tauri::Url) -> Self {
        Self {
            state: Arc::new(Mutex::new(ResourceBrowserState {
                url: home.clone(),
                title,
                loading: true,
                can_go_back: false,
                can_go_forward: false,
            })),
            native_ready: Arc::new(AtomicBool::new(false)),
            home,
            toolbar_url,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceBrowserAction {
    Back,
    Forward,
    Reload,
    Stop,
    Home,
    Navigate,
}

fn toolbar_id(label: &str) -> Option<&str> {
    let id = label.strip_prefix("resource-toolbar-")?;
    let parsed = uuid::Uuid::parse_str(id).ok()?;
    (parsed.to_string() == id).then_some(id)
}

#[cfg(test)]
pub fn is_resource_toolbar(label: &str) -> bool {
    toolbar_id(label).is_some()
}

fn authorized_session(webview: &Webview) -> Result<(String, BrowserSession), String> {
    let id = toolbar_id(webview.label()).ok_or("此页面不能控制网站导航。")?;
    let label = format!("resource-{id}");
    if webview.window().label() != label {
        return Err("网站工具栏窗口不匹配。".into());
    }
    let store = webview.state::<ResourceStore>();
    let session = {
        let sessions = store.browsers.lock().map_err(|_| "网站状态异常")?;
        sessions.get(&label).cloned().ok_or("网站窗口已关闭。")?
    };
    if webview.url().map_err(|e| e.to_string())? != session.toolbar_url {
        return Err("网站工具栏来源无效。".into());
    }
    Ok((label, session))
}

// A stable bookmark label can be reused after closing a window. Native callbacks
// and queued keyboard actions must belong to this particular opening as well.
pub fn is_current_session(app: &tauri::AppHandle, label: &str, session: &BrowserSession) -> bool {
    app.state::<ResourceStore>()
        .browsers
        .lock()
        .ok()
        .and_then(|sessions| {
            sessions
                .get(label)
                .map(|current| Arc::ptr_eq(&current.state, &session.state))
        })
        .unwrap_or(false)
}

pub fn is_trusted_resource_toolbar(webview: &Webview) -> bool {
    authorized_session(webview).is_ok()
}

#[tauri::command]
pub fn resource_browser_state(webview: Webview) -> Result<ResourceBrowserState, String> {
    let (_, session) = authorized_session(&webview)?;
    let state = session.state.lock().map_err(|_| "网站状态异常")?.clone();
    Ok(state)
}

#[tauri::command]
pub async fn resource_browser_action(
    webview: Webview,
    action: ResourceBrowserAction,
    url: Option<String>,
) -> Result<(), String> {
    let (label, session) = authorized_session(&webview)?;
    let destination = match action {
        ResourceBrowserAction::Navigate => {
            Some(validate_url(url.as_deref().ok_or("请输入网址。")?)?)
        }
        ResourceBrowserAction::Home => Some(validate_url(&session.home)?),
        _ => None,
    };
    let remote = webview
        .app_handle()
        .get_webview(&label)
        .ok_or("网页仍在启动，请稍后重试。")?;
    #[cfg(windows)]
    {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let app = webview.app_handle().clone();
        remote
            .with_webview(move |platform| {
                if !is_current_session(&app, &label, &session) {
                    let _ = sender.send(Err("网站窗口已关闭。".to_string()));
                    return;
                }
                let result = native::perform(&platform.controller(), action, destination.as_ref());
                if matches!(action, ResourceBrowserAction::Stop) && result.is_ok() {
                    set_loading(&app, &label, &session, false);
                }
                let _ = sender.send(result.map_err(|e| format!("无法完成网站导航：{e}")));
            })
            .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            receiver.recv().map_err(|_| "网页已关闭。".to_string())?
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (remote, destination);
        Err("资源导航仅支持 Windows。".into())
    }
}

pub fn emit_state(app: &tauri::AppHandle, label: &str, session: &BrowserSession) {
    if !is_current_session(app, label, session) {
        return;
    }
    let state = session.state.lock().ok().map(|state| state.clone());
    if let Some(state) = state {
        let toolbar = label.replacen("resource-", "resource-toolbar-", 1);
        let _ = app.emit_to(
            tauri::EventTarget::webview(toolbar),
            "resource-browser-state",
            state,
        );
    }
}

pub fn set_loading(app: &tauri::AppHandle, label: &str, session: &BrowserSession, loading: bool) {
    if let Ok(mut state) = session.state.lock() {
        state.loading = loading;
    }
    emit_state(app, label, session);
}

pub fn install(remote: &Webview, session: BrowserSession) -> Result<(), String> {
    #[cfg(windows)]
    {
        let app = remote.app_handle().clone();
        let label = remote.label().to_string();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        remote
            .with_webview(move |platform| {
                let result = native::install(&platform.controller(), app, label, session.clone());
                if result.is_ok() {
                    session.native_ready.store(true, Ordering::Release);
                }
                let _ = sender.send(result.map_err(|e| format!("无法初始化网站导航：{e}")));
            })
            .map_err(|e| e.to_string())?;
        receiver.recv().map_err(|_| "网页已关闭。".to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (remote, session);
        Ok(())
    }
}

pub fn layout(window: &tauri::Window) {
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
        return;
    };
    let size = size.to_logical::<f64>(scale);
    let toolbar_label = window.label().replacen("resource-", "resource-toolbar-", 1);
    if let Some(toolbar) = window.app_handle().get_webview(&toolbar_label) {
        let _ = toolbar.set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(0.0, 0.0).into(),
            size: tauri::LogicalSize::new(size.width, TOOLBAR_HEIGHT).into(),
        });
    }
    if let Some(remote) = window.app_handle().get_webview(window.label()) {
        let _ = remote.set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(0.0, TOOLBAR_HEIGHT).into(),
            size: tauri::LogicalSize::new(size.width, (size.height - TOOLBAR_HEIGHT).max(1.0))
                .into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toolbar_labels_require_canonical_uuid() {
        assert!(is_resource_toolbar(
            "resource-toolbar-12345678-1234-1234-1234-123456789abc"
        ));
        for label in [
            "main",
            "resource-12345678-1234-1234-1234-123456789abc",
            "resource-toolbar-123",
            "resource-toolbar-12345678123412341234123456789abc",
        ] {
            assert!(!is_resource_toolbar(label));
        }
    }
}
