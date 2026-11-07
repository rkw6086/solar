//! Fullscreen is leased by the welcome tour and restored when that tour leaves.

use std::sync::Mutex;
use tauri::Manager;

#[derive(Default)]
pub(crate) struct IntroWindowState(Mutex<FullscreenLease>);

#[derive(Default)]
struct FullscreenLease {
    previous: Option<bool>,
}

impl FullscreenLease {
    fn enter(
        &mut self,
        current: impl FnOnce() -> Result<bool, String>,
        mut change: impl FnMut(bool) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.previous.is_some() {
            return Ok(());
        }
        let previous = current()?;
        if !previous {
            change(true)?;
        }
        self.previous = Some(previous);
        Ok(())
    }

    fn leave(&mut self, mut change: impl FnMut(bool) -> Result<(), String>) -> Result<(), String> {
        if let Some(previous) = self.previous {
            // Always queue the restore, even if a fullscreen entry is still
            // animating and is_fullscreen() has not reflected it yet.
            change(previous)?;
            self.previous = None;
        }
        Ok(())
    }
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
    let state = app
        .try_state::<IntroWindowState>()
        .ok_or("The welcome window state is not ready")?;
    let mut lease = state
        .0
        .lock()
        .map_err(|_| "The welcome window state is unavailable")?;
    let change = |fullscreen| {
        window
            .set_fullscreen(fullscreen)
            .map_err(|error| error.to_string())
    };
    if active {
        lease.enter(
            || window.is_fullscreen().map_err(|error| error.to_string()),
            change,
        )
    } else {
        lease.leave(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tour_fullscreen_is_idempotent_and_restores_windowed_state() {
        let mut lease = FullscreenLease::default();
        let mut calls = Vec::new();
        lease
            .enter(
                || Ok(false),
                |value| {
                    calls.push(value);
                    Ok(())
                },
            )
            .unwrap();
        lease
            .enter(|| panic!("already leased"), |_| panic!("already leased"))
            .unwrap();
        lease
            .leave(|value| {
                calls.push(value);
                Ok(())
            })
            .unwrap();
        lease.leave(|_| panic!("already restored")).unwrap();
        assert_eq!(calls, [true, false]);
    }

    #[test]
    fn a_preexisting_fullscreen_window_stays_fullscreen() {
        let mut lease = FullscreenLease::default();
        lease
            .enter(|| Ok(true), |_| panic!("already fullscreen"))
            .unwrap();
        let mut restored = None;
        lease
            .leave(|value| {
                restored = Some(value);
                Ok(())
            })
            .unwrap();
        assert_eq!(restored, Some(true));
    }

    #[test]
    fn failed_window_changes_can_be_retried() {
        let mut lease = FullscreenLease::default();
        assert!(
            lease
                .enter(|| Ok(false), |_| Err("entry failed".into()))
                .is_err()
        );
        assert!(lease.previous.is_none());
        lease.enter(|| Ok(false), |_| Ok(())).unwrap();
        assert!(lease.leave(|_| Err("restore failed".into())).is_err());
        assert_eq!(lease.previous, Some(false));
        lease.leave(|_| Ok(())).unwrap();
        assert!(lease.previous.is_none());
    }
}
