use crate::{
    attention_geometry::{self, AttentionPlacement},
    engine::Engine,
    model::AttentionItem,
};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

static READY: AtomicBool = AtomicBool::new(false);
static REVISION: AtomicU64 = AtomicU64::new(0);
static PRESENTED: Mutex<Option<String>> = Mutex::new(None);
static LAST_SOUND: Mutex<Option<Instant>> = Mutex::new(None);
static PANEL_HEIGHT: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttentionState {
    items: Vec<AttentionItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<AttentionItem>,
    geometry: AttentionPlacement,
    reduced_motion: bool,
    sound_enabled: bool,
}

fn state(app: &tauri::AppHandle) -> Result<AttentionState, String> {
    let engine = app.state::<Arc<Engine>>();
    let items = engine.attention_items()?;
    let settings = engine.settings()?;
    Ok(AttentionState {
        current: items.first().cloned(),
        items,
        geometry: attention_geometry::placement(app)?,
        reduced_motion: settings.reduced_motion,
        sound_enabled: settings.attention_sound,
    })
}

#[tauri::command]
pub(crate) fn get_attention_state(app: tauri::AppHandle) -> Result<AttentionState, String> {
    state(&app)
}

#[tauri::command]
pub(crate) fn attention_ready(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    if window.label() != "attention" {
        return Err("This window is not an attention panel".into());
    }
    READY.store(true, Ordering::SeqCst);
    refresh(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn attention_bounds(window: tauri::WebviewWindow, height: f64) -> Result<(), String> {
    if window.label() != "attention" || !height.is_finite() || !(0.0..=1000.0).contains(&height) {
        return Err("Invalid attention panel bounds".into());
    }
    PANEL_HEIGHT.store(height.to_bits(), Ordering::SeqCst);
    Ok(())
}

pub(crate) fn update_hit_region(window: &tauri::WebviewWindow) {
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    if let (Ok(cursor), Ok(position), Ok(size), Ok(scale)) = (
        window.cursor_position(),
        window.outer_position(),
        window.outer_size(),
        window.scale_factor(),
    ) {
        let x = (cursor.x - position.x as f64) / scale;
        let y = (cursor.y - position.y as f64) / scale;
        let width = size.width as f64 / scale;
        let height = f64::from_bits(PANEL_HEIGHT.load(Ordering::SeqCst));
        if height <= 0.0 {
            return;
        }
        let left = ((width - 430.0) / 2.0).max(12.0);
        let right = width - left;
        let mut inside = x >= left && x <= right && y >= 0.0 && y <= height;
        if inside && y > height - 32.0 {
            let center = if x < left + 32.0 {
                Some(left + 32.0)
            } else if x > right - 32.0 {
                Some(right - 32.0)
            } else {
                None
            };
            if let Some(cx) = center {
                inside = (x - cx).powi(2) + (y - (height - 32.0)).powi(2) <= 32.0_f64.powi(2);
            }
        }
        let _ = window.set_ignore_cursor_events(!inside);
    }
}

#[tauri::command]
pub(crate) fn dismiss_attention(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
    id: String,
) -> Result<(), String> {
    engine.dismiss_attention(&id)?;
    refresh(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn open_attention(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
    id: String,
) -> Result<(), String> {
    let item = engine
        .attention_items()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or("This update has already been dismissed")?;
    if item.conversation_id.is_empty() {
        crate::desktop::open_orbit_chat(app.clone(), engine.clone(), item.orbit_id)?;
    } else {
        crate::desktop::open_full_chat(app.clone(), item.conversation_id)?;
    }
    // Opening a permission request only opens the existing approval UI.
    // It never grants permission or changes the task's authorization.
    engine.dismiss_attention(&id)?;
    refresh(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn preview_attention(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
    orbit_id: String,
) -> Result<(), String> {
    let orbit = engine.orbit(&orbit_id)?;
    if !orbit.enabled {
        return Err("Choose an enabled Orbit to preview".into());
    }
    engine.request_attention(AttentionItem {
        id: format!("preview:{}", crate::model::id()),
        orbit_id,
        task_id: String::new(),
        conversation_id: String::new(),
        kind: "message".into(),
        title: "Here's how I'll get your attention".into(),
        created_at: crate::model::now(),
    })?;
    refresh(&app);
    Ok(())
}

/// Window and AppKit operations stay on the application's main thread.
pub(crate) fn refresh(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(error) = present(&handle) {
            eprintln!("Could not present Orbit attention: {error}");
        }
    });
}

fn present(app: &tauri::AppHandle) -> Result<(), String> {
    let snapshot = state(app)?;
    let revision = REVISION.fetch_add(1, Ordering::SeqCst) + 1;
    let engine = app.state::<Arc<Engine>>();
    let settings = engine.settings()?;
    let preview = snapshot
        .current
        .as_ref()
        .is_some_and(|item| item.id.starts_with("preview:"));
    if snapshot.current.is_none()
        || ((!settings.desktop_enabled || !settings.onboarding_complete) && !preview)
    {
        *PRESENTED
            .lock()
            .map_err(|_| "Attention state unavailable")? = None;
        if let Some(window) = app.get_webview_window("attention") {
            window
                .emit("solar://attention", &snapshot)
                .map_err(|e| e.to_string())?;
            let app = app.clone();
            let delay = if snapshot.reduced_motion { 0 } else { 190 };
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                if REVISION.load(Ordering::SeqCst) == revision {
                    let _ = app.run_on_main_thread({
                        let app = app.clone();
                        move || {
                            if REVISION.load(Ordering::SeqCst) != revision {
                                return;
                            }
                            if let Some(window) = app.get_webview_window("attention") {
                                let _ = window.hide();
                            }
                        }
                    });
                }
            });
        }
        return Ok(());
    }

    let geometry = &snapshot.geometry;
    let window = if let Some(window) = app.get_webview_window("attention") {
        window
    } else {
        READY.store(false, Ordering::SeqCst);
        let window = WebviewWindowBuilder::new(
            app,
            "attention",
            WebviewUrl::App("index.html#attention".into()),
        )
        .title("Solar · Orbit attention")
        .inner_size(geometry.width, geometry.height)
        .position(geometry.x, geometry.y)
        .transparent(true)
        .background_color(tauri::webview::Color(0, 0, 0, 0))
        .visible(false)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        {
            use objc2::{msg_send, runtime::AnyObject};
            if let Ok(handle) = window.ns_window() {
                // A transient alert can join fullscreen Spaces without stealing focus.
                unsafe {
                    let ns = handle as *mut AnyObject;
                    let _: () =
                        msg_send![ns, setCollectionBehavior: 1usize | 16usize | 64usize | 256usize];
                    let _: () = msg_send![ns, setLevel: 25isize];
                }
            }
        }
        window.on_window_event(|event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                READY.store(false, Ordering::SeqCst);
            }
        });
        window
    };
    window
        .set_size(tauri::LogicalSize::new(geometry.width, geometry.height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(geometry.x, geometry.y))
        .map_err(|e| e.to_string())?;
    window
        .emit("solar://attention", &snapshot)
        .map_err(|e| e.to_string())?;
    if !READY.load(Ordering::SeqCst) {
        return Ok(());
    }
    show_without_focus(&window)?;
    let current = snapshot.current.as_ref().expect("checked above");
    let mut presented = PRESENTED
        .lock()
        .map_err(|_| "Attention state unavailable")?;
    if presented.as_deref() != Some(&current.id) {
        *presented = Some(current.id.clone());
        let collapsed = !engine.desktop_expanded.load(Ordering::SeqCst);
        if settings.attention_sound && (collapsed || preview) {
            let mut last = LAST_SOUND.lock().map_err(|_| "Sound state unavailable")?;
            if last.is_none_or(|time| time.elapsed() >= Duration::from_millis(1000)) {
                *last = Some(Instant::now());
                if let Err(error) =
                    crate::attention_sound::play(&current.orbit_id, settings.attention_volume)
                {
                    eprintln!("Could not play Orbit attention sound: {error}");
                }
            }
        }
    }
    Ok(())
}

fn show_without_focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{msg_send, runtime::AnyObject};
        let handle = window.ns_window().map_err(|error| error.to_string())?;
        // Standard show() may make the NSWindow key. Attention should reveal
        // itself without interrupting typing in the user's current app.
        unsafe {
            let ns = handle as *mut AnyObject;
            let _: () = msg_send![ns, orderFrontRegardless];
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    window.show().map_err(|error| error.to_string())
}
