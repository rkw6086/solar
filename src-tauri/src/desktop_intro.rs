//! The welcome tour temporarily uses a borderless, screen-sized launcher.
//! This never enters or leaves macOS native fullscreen or creates a Space.

use std::sync::Mutex;
use tauri::Manager;

#[derive(Default)]
pub(crate) struct IntroWindowState(Mutex<BorderlessLease>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowSnapshot {
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    decorated: bool,
    resizable: bool,
    #[cfg(not(target_os = "macos"))]
    maximized: bool,
}

#[derive(Default)]
struct BorderlessLease {
    previous: Option<WindowSnapshot>,
    active: bool,
}

impl BorderlessLease {
    fn enter(
        &mut self,
        capture: impl FnOnce() -> Result<WindowSnapshot, String>,
        apply: impl FnOnce() -> Result<(), String>,
        mut restore: impl FnMut(&WindowSnapshot) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.active {
            return Ok(());
        }
        // A previous failed restore must finish before taking another snapshot
        // of a potentially half-restored window.
        if let Some(previous) = &self.previous {
            restore(previous)?;
            self.previous = None;
        }
        self.previous = Some(capture()?);
        if let Err(error) = apply() {
            let previous = self.previous.as_ref().expect("snapshot captured above");
            return match restore(previous) {
                Ok(()) => {
                    self.previous = None;
                    Err(error)
                }
                Err(restore_error) => Err(format!(
                    "{error}; restoring the launcher also failed: {restore_error}"
                )),
            };
        }
        self.active = true;
        Ok(())
    }

    fn leave(
        &mut self,
        mut restore: impl FnMut(&WindowSnapshot) -> Result<(), String>,
    ) -> Result<(), String> {
        self.active = false;
        if let Some(previous) = &self.previous {
            restore(previous)?;
            self.previous = None;
        }
        Ok(())
    }
}

fn capture_window(window: &tauri::WebviewWindow) -> Result<WindowSnapshot, String> {
    // Keep the exact visible frame. In particular, do not unmaximize to inspect
    // hidden restore bounds: Tao queues macOS zoom changes asynchronously.
    Ok(WindowSnapshot {
        position: window.outer_position().map_err(|error| error.to_string())?,
        size: window.inner_size().map_err(|error| error.to_string())?,
        decorated: window.is_decorated().map_err(|error| error.to_string())?,
        resizable: window.is_resizable().map_err(|error| error.to_string())?,
        #[cfg(not(target_os = "macos"))]
        maximized: window.is_maximized().map_err(|error| error.to_string())?,
    })
}

fn restore_window(window: &tauri::WebviewWindow, previous: &WindowSnapshot) -> Result<(), String> {
    // Try every property even after one fails. The lease retains its snapshot
    // on failure, allowing the close handler or a later request to retry.
    let mut errors = Vec::new();
    #[cfg(not(target_os = "macos"))]
    if let Err(error) = window.unmaximize() {
        errors.push(error.to_string());
    }
    for result in [
        window.set_decorations(previous.decorated),
        window.set_resizable(previous.resizable),
        window.set_size(previous.size),
        window.set_position(previous.position),
    ] {
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    #[cfg(not(target_os = "macos"))]
    if previous.maximized
        && let Err(error) = window.maximize()
    {
        errors.push(error.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn set_on_main_thread(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    active: bool,
) -> Result<(), String> {
    let state = app
        .try_state::<IntroWindowState>()
        .ok_or("The welcome window state is not ready")?;
    let mut lease = state
        .0
        .lock()
        .map_err(|_| "The welcome window state is unavailable")?;
    if !active {
        return lease.leave(|previous| restore_window(window, previous));
    }
    if lease.active {
        return Ok(());
    }
    // A user-owned native fullscreen window stays exactly as it is. In
    // particular, replay must not trigger a macOS fullscreen transition.
    if lease.previous.is_none() && window.is_fullscreen().map_err(|error| error.to_string())? {
        return Ok(());
    }
    let monitor = match window
        .current_monitor()
        .map_err(|error| error.to_string())?
    {
        Some(monitor) => Some(monitor),
        None => app.primary_monitor().map_err(|error| error.to_string())?,
    }
    .ok_or("No display is available for the welcome tour")?;
    let position = *monitor.position();
    let size = *monitor.size();
    if size.width == 0 || size.height == 0 {
        return Err("The welcome display has invalid dimensions".into());
    }
    lease.enter(
        || capture_window(window),
        || {
            #[cfg(not(target_os = "macos"))]
            window.unmaximize().map_err(|error| error.to_string())?;
            window
                .set_decorations(false)
                .map_err(|error| error.to_string())?;
            window
                .set_resizable(false)
                .map_err(|error| error.to_string())?;
            // Physical bounds preserve the selected display's origin and DPI,
            // including displays to the left/above the main monitor.
            window.set_size(size).map_err(|error| error.to_string())?;
            window
                .set_position(position)
                .map_err(|error| error.to_string())?;
            Ok(())
        },
        |previous| restore_window(window, previous),
    )
}

#[tauri::command]
pub(crate) fn set_intro_fullscreen(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    active: bool,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("The welcome tour belongs to the launcher".into());
    }
    // Commands and the native close handler share one serialized main-thread
    // operation. Never hold the lease mutex while waiting for that thread.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = send.send(set_on_main_thread(&handle, &window, active));
    })
    .map_err(|error| error.to_string())?;
    receive
        .recv()
        .map_err(|_| "The welcome window service stopped".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn snapshot() -> WindowSnapshot {
        WindowSnapshot {
            position: tauri::PhysicalPosition::new(-1500, 90),
            size: tauri::PhysicalSize::new(1220, 800),
            decorated: true,
            resizable: true,
            #[cfg(not(target_os = "macos"))]
            maximized: false,
        }
    }

    #[test]
    fn borderless_lease_is_idempotent_and_restores_exact_window_state() {
        let mut lease = BorderlessLease::default();
        let previous = snapshot();
        let calls = RefCell::new(Vec::new());
        lease
            .enter(
                || Ok(previous.clone()),
                || {
                    calls.borrow_mut().push("enter");
                    Ok(())
                },
                |_| panic!("entry succeeds"),
            )
            .unwrap();
        lease
            .enter(
                || panic!("already leased"),
                || panic!("already leased"),
                |_| panic!("already leased"),
            )
            .unwrap();
        lease
            .leave(|state| {
                assert_eq!(state, &previous);
                calls.borrow_mut().push("restore");
                Ok(())
            })
            .unwrap();
        lease.leave(|_| panic!("already restored")).unwrap();
        assert_eq!(*calls.borrow(), ["enter", "restore"]);
        assert!(lease.previous.is_none());
    }

    #[test]
    fn a_partially_applied_entry_is_rolled_back() {
        let mut lease = BorderlessLease::default();
        let restored = RefCell::new(None);
        assert!(
            lease
                .enter(
                    || Ok(snapshot()),
                    || Err("resize failed after decorations changed".into()),
                    |state| {
                        *restored.borrow_mut() = Some(state.clone());
                        Ok(())
                    },
                )
                .is_err()
        );
        assert_eq!(*restored.borrow(), Some(snapshot()));
        assert!(!lease.active);
        assert!(lease.previous.is_none());
    }

    #[test]
    fn failed_restore_is_retained_and_retried_before_a_new_capture() {
        let mut lease = BorderlessLease::default();
        lease
            .enter(|| Ok(snapshot()), || Ok(()), |_| Ok(()))
            .unwrap();
        assert!(lease.leave(|_| Err("restore failed".into())).is_err());
        assert_eq!(lease.previous, Some(snapshot()));
        assert!(!lease.active);
        let steps = RefCell::new(Vec::new());
        lease
            .enter(
                || {
                    steps.borrow_mut().push("capture");
                    Ok(snapshot())
                },
                || {
                    steps.borrow_mut().push("apply");
                    Ok(())
                },
                |_| {
                    steps.borrow_mut().push("retry restore");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*steps.borrow(), ["retry restore", "capture", "apply"]);
    }

    #[test]
    fn failed_entry_and_rollback_keep_the_original_snapshot_for_cleanup() {
        let mut lease = BorderlessLease::default();
        let error = lease
            .enter(
                || Ok(snapshot()),
                || Err("entry failed".into()),
                |_| Err("rollback failed".into()),
            )
            .unwrap_err();
        assert!(error.contains("entry failed") && error.contains("rollback failed"));
        assert_eq!(lease.previous, Some(snapshot()));
        assert!(!lease.active);
        lease
            .leave(|state| {
                assert_eq!(state, &snapshot());
                Ok(())
            })
            .unwrap();
        assert!(lease.previous.is_none());
    }

    #[test]
    fn capture_failure_does_not_create_a_lease() {
        let mut lease = BorderlessLease::default();
        assert!(
            lease
                .enter(
                    || Err("window unavailable".into()),
                    || panic!("capture failed"),
                    |_| panic!("capture failed"),
                )
                .is_err()
        );
        assert!(lease.previous.is_none());
    }
}
