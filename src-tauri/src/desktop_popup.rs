//! Widget chats dismiss after focus leaves them, except while their own native
//! chooser or profile save is active. Each timer belongs to one window lifetime.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, WebviewWindow, WindowEvent};

#[derive(Default)]
pub(crate) struct PopupDismissals(Mutex<HashMap<String, Arc<Mutex<FocusState>>>>);

struct FocusState {
    armed: bool,
    focused: bool,
    holds: usize,
    alive: bool,
    revision: u64,
    closing_revision: Option<u64>,
}

impl Default for FocusState {
    fn default() -> Self {
        Self {
            armed: false,
            focused: false,
            holds: 0,
            alive: true,
            revision: 0,
            closing_revision: None,
        }
    }
}

impl FocusState {
    fn pending(&self) -> Option<u64> {
        (self.alive && self.armed && !self.focused && self.holds == 0).then_some(self.revision)
    }

    fn focus(&mut self, focused: bool) -> Option<u64> {
        self.revision += 1;
        self.focused = focused;
        self.armed |= focused;
        self.pending()
    }

    fn acquire(&mut self) {
        self.holds += 1;
        self.revision += 1;
    }

    fn release(&mut self) -> Option<u64> {
        self.holds = self.holds.saturating_sub(1);
        self.revision += 1;
        self.pending()
    }

    fn destroy(&mut self) {
        self.alive = false;
        self.revision += 1;
        self.closing_revision = None;
    }

    fn can_close(&self, revision: u64, native_focused: bool) -> bool {
        !native_focused && self.pending() == Some(revision)
    }
    fn begin_close(&mut self, revision: u64, native_focused: bool) -> bool {
        if !self.can_close(revision, native_focused) || self.closing_revision == Some(revision) {
            return false;
        }
        self.closing_revision = Some(revision);
        true
    }

    fn cancel_animation(&mut self) -> bool {
        self.closing_revision.take().is_some()
    }

    fn finish_close(&mut self, revision: u64, native_focused: bool) -> Option<bool> {
        if self.closing_revision != Some(revision) {
            return None;
        }
        self.closing_revision = None;
        Some(self.can_close(revision, native_focused))
    }
}

fn schedule(window: WebviewWindow, state: Arc<Mutex<FocusState>>, revision: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let app = window.app_handle().clone();
        let _ = app.run_on_main_thread(move || {
            // Use this original handle rather than looking up a label that may
            // already have been reused by a newly opened chat.
            let focused = window.is_focused().unwrap_or(true);
            let close = state
                .lock()
                .map(|mut state| state.begin_close(revision, focused))
                .unwrap_or(false);
            if close {
                crate::desktop::dismiss_orbit_chat(&window);
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(280)).await;
                    let app = window.app_handle().clone();
                    let _ = app.run_on_main_thread(move || {
                        let focused = window.is_focused().unwrap_or(true);
                        let close = state
                            .lock()
                            .ok()
                            .and_then(|mut state| state.finish_close(revision, focused));
                        match close {
                            Some(true) => {
                                let _ = window.close();
                            }
                            Some(false) => crate::desktop::cancel_orbit_chat_dismissal(&window),
                            None => {}
                        }
                    });
                });
            }
        });
    });
}

pub(crate) fn attach(window: &WebviewWindow) {
    if !window.label().starts_with("chat-") {
        return;
    }
    let state = Arc::new(Mutex::new(FocusState::default()));
    let registry = window.state::<PopupDismissals>();
    let Ok(mut windows) = registry.0.lock() else {
        return;
    };
    windows.insert(window.label().into(), state.clone());
    drop(windows);
    let original = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(focused) => {
            let (revision, cancel_animation) = state
                .lock()
                .map(|mut state| {
                    let revision = state.focus(*focused);
                    let cancel = *focused && state.cancel_animation();
                    (revision, cancel)
                })
                .unwrap_or((None, false));
            if cancel_animation {
                crate::desktop::cancel_orbit_chat_dismissal(&original);
            }
            if let Some(revision) = revision {
                schedule(original.clone(), state.clone(), revision);
            }
        }
        WindowEvent::Destroyed => {
            if let Ok(mut state) = state.lock() {
                state.destroy();
            }
            let registry = original.state::<PopupDismissals>();
            if let Ok(mut windows) = registry.0.lock()
                && windows
                    .get(original.label())
                    .is_some_and(|current| Arc::ptr_eq(current, &state))
            {
                windows.remove(original.label());
            };
        }
        _ => {}
    });
}

pub(crate) struct PopupHold {
    window: WebviewWindow,
    state: Arc<Mutex<FocusState>>,
}

/// No-op for the main app, other widgets, or an already destroyed popup.
pub(crate) fn hold(window: &WebviewWindow) -> Option<PopupHold> {
    if !window.label().starts_with("chat-") {
        return None;
    }
    let registry = window.try_state::<PopupDismissals>()?;
    let state = registry.0.lock().ok()?.get(window.label())?.clone();
    let cancel_animation = {
        let mut state = state.lock().ok()?;
        if !state.alive {
            return None;
        }
        state.acquire();
        state.cancel_animation()
    };
    if cancel_animation {
        crate::desktop::cancel_orbit_chat_dismissal(window);
    }
    Some(PopupHold {
        window: window.clone(),
        state,
    })
}

impl Drop for PopupHold {
    fn drop(&mut self) {
        let revision = self.state.lock().ok().and_then(|mut state| state.release());
        if let Some(revision) = revision {
            schedule(self.window.clone(), self.state.clone(), revision);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_blur_is_ignored_until_the_popup_has_focus() {
        let mut state = FocusState::default();
        assert!(state.focus(false).is_none());
        assert!(state.focus(true).is_none());
        let revision = state.focus(false).unwrap();
        assert!(state.can_close(revision, false));
        assert!(!state.can_close(revision, true));
    }

    #[test]
    fn refocus_and_destruction_invalidate_pending_close() {
        let mut state = FocusState::default();
        state.focus(true);
        let revision = state.focus(false).unwrap();
        state.focus(true);
        assert!(!state.can_close(revision, false));
        let revision = state.focus(false).unwrap();
        state.destroy();
        assert!(!state.can_close(revision, false));
        // A replacement window has separate state even if it reuses a label.
        assert!(FocusState::default().pending().is_none());
    }

    #[test]
    fn final_animation_frame_rechecks_focus_and_protected_operations() {
        let mut state = FocusState::default();
        state.focus(true);
        let revision = state.focus(false).unwrap();
        assert!(state.begin_close(revision, false));
        assert!(!state.begin_close(revision, false));
        state.focus(true);
        assert_eq!(state.finish_close(revision, true), Some(false));

        let revision = state.focus(false).unwrap();
        assert!(state.begin_close(revision, false));
        state.acquire();
        assert_eq!(state.finish_close(revision, false), Some(false));
        let revision = state.release().unwrap();
        assert!(state.begin_close(revision, false));
        assert_eq!(state.finish_close(revision, false), Some(true));
    }

    #[test]
    fn an_obsolete_animation_cannot_close_or_reopen_a_newer_one() {
        let mut state = FocusState::default();
        state.focus(true);
        let old = state.focus(false).unwrap();
        assert!(state.begin_close(old, false));
        state.focus(true);
        assert!(state.cancel_animation());
        let next = state.focus(false).unwrap();
        assert!(state.begin_close(next, false));
        assert_eq!(state.finish_close(old, false), None);
        assert_eq!(state.finish_close(next, false), Some(true));
    }

    #[test]
    fn nested_holds_delay_dismissal_until_the_last_operation_finishes() {
        let mut state = FocusState::default();
        state.focus(true);
        let revision = state.focus(false).unwrap();
        state.acquire();
        state.acquire();
        assert!(!state.can_close(revision, false));
        assert!(state.release().is_none());
        let revision = state.release().unwrap();
        assert!(state.can_close(revision, false));
        state.focus(true);
        state.acquire();
        assert!(state.release().is_none());
    }
}
