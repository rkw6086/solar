use crate::{engine::Engine, model::Settings};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tauri::{Manager, State};

static MUSIC_EPOCH: AtomicU64 = AtomicU64::new(0);
static REQUEST_EPOCH: AtomicU64 = AtomicU64::new(0);
static MUSIC_ACTIVE: AtomicBool = AtomicBool::new(false);
static MUSIC_LEVEL: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
pub(crate) struct MusicState {
    playing: bool,
}

fn require_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("The welcome tour belongs to the launcher".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn save_onboarding_progress(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    step: String,
) -> Result<Settings, String> {
    require_main(&window)?;
    engine.save_onboarding_progress(&step)
}

#[tauri::command]
pub(crate) fn complete_onboarding(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
) -> Result<Settings, String> {
    require_main(&window)?;
    let settings = engine.complete_onboarding()?;
    stop_music(&app);
    if settings.desktop_enabled {
        crate::desktop::ensure_sun(&app)?;
    }
    crate::desktop_attention::refresh(&app);
    Ok(settings)
}

#[tauri::command]
pub(crate) async fn reset_solar(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    confirmed: bool,
) -> Result<(), String> {
    require_main(&window)?;
    stop_music(&app);
    crate::reset::prepare_reset(engine.inner(), confirmed).await?;
    crate::desktop::restart_after_reset(&app);
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_onboarding_music(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    playing: bool,
) -> Result<MusicState, String> {
    require_main(&window)?;
    let request = REQUEST_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = if REQUEST_EPOCH.load(Ordering::SeqCst) == request {
            set_music(&handle, playing)
        } else {
            Ok(MusicState { playing: false })
        };
        let _ = send.send(result);
    })
    .map_err(|error| error.to_string())?;
    receive
        .await
        .map_err(|_| "Music service stopped".to_owned())?
}

/// Fade rather than cut the music when a user mutes, finishes or hides it.
fn set_music(app: &tauri::AppHandle, playing: bool) -> Result<MusicState, String> {
    let settings = app.state::<Arc<Engine>>().settings()?;
    let playing = playing && settings.onboarding_music;
    if playing {
        if !MUSIC_ACTIVE.load(Ordering::SeqCst) {
            if let Err(error) = crate::space_score::play(0.0) {
                if cfg!(not(target_os = "macos")) {
                    return Ok(MusicState { playing: false });
                }
                return Err(error);
            }
            MUSIC_LEVEL.store(0.0_f64.to_bits(), Ordering::SeqCst);
        }
        MUSIC_ACTIVE.store(true, Ordering::SeqCst);
        fade(app, f64::from(settings.onboarding_volume), false);
        return Ok(MusicState { playing: true });
    }
    MUSIC_ACTIVE.store(false, Ordering::SeqCst);
    fade(app, 0.0, true);
    Ok(MusicState { playing: false })
}

fn fade(app: &tauri::AppHandle, target: f64, stop: bool) {
    let epoch = MUSIC_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let from = f64::from_bits(MUSIC_LEVEL.load(Ordering::SeqCst));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for step in 1..=20 {
            tokio::time::sleep(std::time::Duration::from_millis(18)).await;
            if MUSIC_EPOCH.load(Ordering::SeqCst) != epoch {
                return;
            }
            let volume = from + (target - from) * step as f64 / 20.0;
            let _ = app.run_on_main_thread(move || {
                if MUSIC_EPOCH.load(Ordering::SeqCst) != epoch {
                    return;
                }
                MUSIC_LEVEL.store(volume.to_bits(), Ordering::SeqCst);
                let _ = crate::space_score::set_volume(volume as f32);
                if step == 20 && stop {
                    let _ = crate::space_score::stop();
                }
            });
        }
    });
}

pub(crate) fn stop_music(app: &tauri::AppHandle) {
    REQUEST_EPOCH.fetch_add(1, Ordering::SeqCst);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = set_music(&handle, false);
    });
}

pub(crate) fn settings_changed(app: &tauri::AppHandle, settings: &Settings) {
    if !settings.onboarding_music {
        stop_music(app);
    } else if MUSIC_ACTIVE.load(Ordering::SeqCst) {
        fade(app, f64::from(settings.onboarding_volume), false);
    }
}
