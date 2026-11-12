use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

#[cfg(debug_assertions)]
fn trace(stage: &str) {
    if std::env::var_os("REELINK_QA_SHORTCUT_TRACE").is_some() {
        eprintln!("capture-shortcut stage: {stage}");
    }
}

#[cfg(not(debug_assertions))]
fn trace(_: &str) {}

struct CaptureStatus {
    frontend_ready: bool,
    pending: bool,
}

/// Keeps startup and single-instance capture requests until the main webview subscribes.
pub struct CaptureLaunchState(Mutex<CaptureStatus>);

impl CaptureLaunchState {
    pub fn new(initial_capture: bool) -> Self {
        Self(Mutex::new(CaptureStatus {
            frontend_ready: false,
            pending: initial_capture,
        }))
    }

    pub fn request_capture(&self, app: &AppHandle) {
        trace("request received");
        if crate::qr::capture_active() {
            trace("request dropped: capture active");
            return;
        }
        self.queue_capture(|| app.emit_to("main", "capture-requested", ()).is_ok());
    }

    fn queue_capture(&self, emit: impl FnOnce() -> bool) {
        let mut status = self.0.lock().unwrap_or_else(|error| error.into_inner());
        status.pending = true;
        if status.frontend_ready {
            if emit() {
                status.pending = false;
                trace("event emitted to ready frontend");
            } else {
                trace("event delivery failed; request pending");
            }
        } else {
            trace("request queued: frontend not ready");
        }
    }

    pub fn frontend_loading(&self) {
        trace("frontend loading");
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .frontend_ready = false;
    }

    fn frontend_ready(&self) -> bool {
        let mut status = self.0.lock().unwrap_or_else(|error| error.into_inner());
        status.frontend_ready = true;
        let pending = std::mem::take(&mut status.pending);
        if pending {
            trace("frontend ready: pending request consumed");
        } else {
            trace("frontend ready: no pending request");
        }
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_capture_is_consumed_once_after_frontend_subscribes() {
        let state = CaptureLaunchState::new(true);
        assert!(state.frontend_ready());
        assert!(!state.frontend_ready());
        assert!(!CaptureLaunchState::new(false).frontend_ready());
    }

    #[test]
    fn repeated_requests_before_ready_coalesce_without_emission() {
        let state = CaptureLaunchState::new(false);
        for _ in 0..3 {
            state.queue_capture(|| panic!("no listener exists yet"));
        }
        assert!(state.frontend_ready());
        assert!(!state.frontend_ready());
    }

    #[test]
    fn reload_and_failed_delivery_retain_a_pending_request() {
        let state = CaptureLaunchState::new(false);
        assert!(!state.frontend_ready());
        state.queue_capture(|| false);
        state.frontend_loading();
        state.queue_capture(|| panic!("reloading listener cannot receive events"));
        assert!(state.frontend_ready());
        state.queue_capture(|| true);
        assert!(!state.frontend_ready());
    }
}

#[tauri::command]
pub fn capture_frontend_ready(state: State<'_, CaptureLaunchState>) -> bool {
    state.frontend_ready()
}
