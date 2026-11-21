//! Clicking away folds chats back into their Sprites without gathering the
//! formation. Native choosers and saves hold dismissal open.
use crate::desktop_motion::CompanionDismissal;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, WebviewWindow, WindowEvent};

#[derive(Default)]
pub(crate) struct PopupDismissals {
    windows: Mutex<HashMap<String, Arc<Mutex<WindowState>>>>,
    group: Mutex<CompanionDismissal>,
}

#[derive(Default)]
struct WindowState {
    holds: usize,
    destroyed: bool,
}

fn companion(label: &str) -> bool {
    label == "sun" || label.starts_with("orbit-") || label.starts_with("chat-")
}

fn protected(app: &tauri::AppHandle) -> bool {
    let Some(registry) = app.try_state::<PopupDismissals>() else {
        return true;
    };
    registry
        .windows
        .lock()
        .map(|windows| {
            windows.values().any(|state| {
                state
                    .lock()
                    .map(|state| !state.destroyed && state.holds > 0)
                    .unwrap_or(true)
            })
        })
        .unwrap_or(true)
}

fn native_group_focused(app: &tauri::AppHandle) -> bool {
    app.webview_windows().into_iter().any(|(label, window)| {
        companion(&label)
            && window.is_visible().unwrap_or(false)
            && window.is_focused().unwrap_or(true)
    })
}

fn reopen_chats(app: &tauri::AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("chat-") {
            crate::desktop::cancel_orbit_chat_dismissal(&window);
        }
    }
}

pub(crate) fn cancel(app: &tauri::AppHandle) {
    if let Some(registry) = app.try_state::<PopupDismissals>() {
        let cancelled = registry
            .group
            .lock()
            .map(|mut group| group.cancel())
            .unwrap_or(false);
        if cancelled {
            reopen_chats(app);
        }
    }
}

fn schedule_blur(app: tauri::AppHandle, revision: u64) {
    tauri::async_runtime::spawn(async move {
        // NSWindow transfers focus in separate events. Let the new member of
        // the companion group receive focus before interpreting the blur.
        tokio::time::sleep(Duration::from_millis(160)).await;
        let on_main = app.clone();
        // Losing key-window status is not a deliberate request to gather the
        // planets. Closing a popup can itself move focus through its restored
        // Sprite before another app takes it, producing a second blur after
        // there are no chats left. Focus changes never gather the group.
        let _ = app.run_on_main_thread(move || dismiss(&on_main, Some(revision)));
    });
}

fn dismiss(app: &tauri::AppHandle, expected: Option<u64>) {
    if protected(app) || (expected.is_some() && native_group_focused(app)) {
        return;
    }
    let Some(registry) = app.try_state::<PopupDismissals>() else {
        return;
    };
    let chats: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with("chat-"))
        .map(|(_, window)| window)
        .collect();
    let Some(revision) = registry
        .group
        .lock()
        .ok()
        .and_then(|mut group| group.begin(expected, !chats.is_empty()))
    else {
        return;
    };
    for window in &chats {
        crate::desktop::dismiss_orbit_chat(window);
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(280)).await;
        let on_main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if protected(&on_main) {
                cancel(&on_main);
                return;
            }
            let Some(registry) = on_main.try_state::<PopupDismissals>() else {
                return;
            };
            let close = registry
                .group
                .lock()
                .map(|mut group| group.finish(revision))
                .unwrap_or(false);
            if close {
                // Keep the original handles: a new popup using the same label
                // must never be closed by an older transition.
                for window in chats {
                    let _ = window.close();
                }
            }
        });
    });
}

/// A deliberate Sun contraction closes its chats as part of the same motion.
pub(crate) fn dismiss_chats(app: &tauri::AppHandle) {
    dismiss(app, None);
}

/// Called by the existing native hit-test loop. A click outside all visible
/// companion regions works even for nonactivating macOS widget windows.
pub(crate) fn pointer(
    app: &tauri::AppHandle,
    sample: Option<(u64, Option<u64>)>,
    inside: bool,
    active: bool,
) {
    let Some((buttons, clicks)) = sample else {
        return;
    };
    let Some(registry) = app.try_state::<PopupDismissals>() else {
        return;
    };
    let pressed = registry
        .group
        .lock()
        .map(|mut group| group.pressed(buttons, clicks))
        .unwrap_or(false);
    if pressed && inside {
        cancel(app);
    } else if pressed && active {
        let on_main = app.clone();
        let _ = app.run_on_main_thread(move || dismiss(&on_main, None));
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn pointer_state() -> Option<(u64, Option<u64>)> {
    // Querying button state does not install an event tap or require screen
    // recording/accessibility privileges. NSEvent is an AppKit class method.
    use objc2::{class, msg_send};
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceCounterForEventType(state: i32, event_type: u32) -> u32;
    }
    let buttons: usize = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
    // Session mouse-down counters also detect a tap which is pressed and
    // released entirely between two samples. No global event tap is installed.
    let clicks = unsafe {
        u64::from(CGEventSourceCounterForEventType(0, 1))
            + u64::from(CGEventSourceCounterForEventType(0, 3))
            + u64::from(CGEventSourceCounterForEventType(0, 25))
    };
    Some((buttons as u64, Some(clicks)))
}

#[cfg(target_os = "windows")]
pub(crate) fn pointer_state() -> Option<(u64, Option<u64>)> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    Some((
        [1, 2, 4, 5, 6]
            .into_iter()
            .enumerate()
            .fold(0, |mask, (index, key)| {
                mask | if unsafe { GetAsyncKeyState(key) } < 0 {
                    1 << index
                } else {
                    0
                }
            }),
        None,
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn pointer_state() -> Option<(u64, Option<u64>)> {
    None
}

pub(crate) fn attach(window: &WebviewWindow) {
    if !companion(window.label()) {
        return;
    }
    let state = Arc::new(Mutex::new(WindowState::default()));
    let registry = window.state::<PopupDismissals>();
    let Ok(mut windows) = registry.windows.lock() else {
        return;
    };
    windows.insert(window.label().into(), state.clone());
    drop(windows);
    cancel(window.app_handle());
    let original = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(focused) => {
            let registry = original.state::<PopupDismissals>();
            // A native callback from a destroyed popup may arrive after its
            // label has been reused. It must not cancel a newer transition.
            let registered = registry
                .windows
                .lock()
                .map(|windows| {
                    windows
                        .get(original.label())
                        .is_some_and(|current| Arc::ptr_eq(current, &state))
                })
                .unwrap_or(false);
            if !registered || state.lock().map(|state| state.destroyed).unwrap_or(true) {
                return;
            }
            let (cancelled, pending) = registry
                .group
                .lock()
                .map(|mut group| {
                    let cancelled = group.focus(original.label(), *focused);
                    (cancelled, group.pending())
                })
                .unwrap_or((false, None));
            if cancelled {
                reopen_chats(original.app_handle());
            }
            if let Some(revision) = pending {
                schedule_blur(original.app_handle().clone(), revision);
            }
        }
        WindowEvent::Destroyed => {
            if let Ok(mut state) = state.lock() {
                state.destroyed = true;
            }
            let registry = original.state::<PopupDismissals>();
            let mut cancelled = false;
            let mut pending = None;
            if let Ok(mut windows) = registry.windows.lock()
                && windows
                    .get(original.label())
                    .is_some_and(|current| Arc::ptr_eq(current, &state))
            {
                windows.remove(original.label());
                if let Ok(mut group) = registry.group.lock() {
                    cancelled = group.remove(original.label());
                    pending = group.pending();
                }
            }
            if cancelled {
                reopen_chats(original.app_handle());
            }
            if let Some(revision) = pending {
                schedule_blur(original.app_handle().clone(), revision);
            }
        }
        _ => {}
    });
}

pub(crate) struct PopupHold {
    window: WebviewWindow,
    state: Arc<Mutex<WindowState>>,
}

/// No-op for the main app, other widgets, or an already destroyed popup.
pub(crate) fn hold(window: &WebviewWindow) -> Option<PopupHold> {
    if !window.label().starts_with("chat-") {
        return None;
    }
    let registry = window.try_state::<PopupDismissals>()?;
    let state = registry.windows.lock().ok()?.get(window.label())?.clone();
    {
        let mut state = state.lock().ok()?;
        if state.destroyed {
            return None;
        }
        state.holds += 1;
    }
    cancel(window.app_handle());
    Some(PopupHold {
        window: window.clone(),
        state,
    })
}

impl Drop for PopupHold {
    fn drop(&mut self) {
        let alive = self
            .state
            .lock()
            .map(|mut state| {
                state.holds = state.holds.saturating_sub(1);
                !state.destroyed
            })
            .unwrap_or(false);
        if alive && !protected(self.window.app_handle()) {
            let registry = self.window.state::<PopupDismissals>();
            let pending = registry.group.lock().ok().and_then(|group| group.pending());
            if let Some(revision) = pending {
                schedule_blur(self.window.app_handle().clone(), revision);
            }
        }
    }
}
