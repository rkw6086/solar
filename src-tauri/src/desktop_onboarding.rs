use crate::{engine::Engine, model::Settings};
use std::sync::Arc;
use tauri::{Manager, State};

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
    stop_effects(&app);
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
    stop_effects(&app);
    crate::reset::prepare_reset(engine.inner(), confirmed).await?;
    crate::desktop::restart_after_reset(&app);
    Ok(())
}

#[tauri::command]
pub(crate) async fn play_sound_effect(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    cue: String,
    variant: Option<u32>,
) -> Result<bool, String> {
    if !["click", "ripple", "formation", "wink"].contains(&cue.as_str()) {
        return Err("Unknown sound effect".into());
    }
    if cue != "click" {
        require_main(&window)?;
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            let settings = handle.state::<Arc<Engine>>().settings()?;
            if !settings.sound_effects || !window.is_visible().unwrap_or(false) {
                return Ok(false);
            }
            match crate::sound_effects::play(
                &cue,
                variant.unwrap_or(0),
                settings.sound_effects_volume,
            ) {
                Ok(()) => Ok(true),
                Err(_) if cfg!(not(target_os = "macos")) => Ok(false),
                Err(error) => Err(error),
            }
        })();
        let _ = send.send(result);
    })
    .map_err(|error| error.to_string())?;
    receive
        .await
        .map_err(|_| "Sound service stopped".to_owned())?
}

#[tauri::command]
pub(crate) fn stop_sound_effects(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    require_main(&window)?;
    stop_effects(&app);
    Ok(())
}

pub(crate) fn stop_effects(app: &tauri::AppHandle) {
    let _ = app.run_on_main_thread(|| {
        let _ = crate::sound_effects::stop();
    });
}

pub(crate) fn settings_changed(app: &tauri::AppHandle, settings: &Settings) {
    if !settings.sound_effects {
        stop_effects(app);
        let _ = app.run_on_main_thread(|| {
            let _ = crate::attention_sound::stop();
        });
    }
}
