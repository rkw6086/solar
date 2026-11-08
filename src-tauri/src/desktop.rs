use crate::{
    engine::{Engine, McpServerDraft, ProviderDraft},
    model::*,
};
use std::{
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

type AppState<'a> = State<'a, Arc<Engine>>;
static EXPANDED: AtomicBool = AtomicBool::new(false);
static SUN_REQUESTED: AtomicBool = AtomicBool::new(false);
static LAUNCHER_OPEN: AtomicBool = AtomicBool::new(true);
static COMPANIONS_HIDDEN: AtomicBool = AtomicBool::new(false);
static QUITTING: AtomicBool = AtomicBool::new(false);
static ANIMATING: AtomicBool = AtomicBool::new(false);
static ANIMATION_EPOCH: AtomicU64 = AtomicU64::new(0);
static READY_WIDGETS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

#[tauri::command]
fn widget_ready(window: tauri::WebviewWindow) -> Result<(), String> {
    let label = window.label();
    if label != "sun" && !label.starts_with("orbit-") {
        return Err("This window is not a desktop widget".into());
    }
    READY_WIDGETS
        .lock()
        .map_err(|_| "Widget state unavailable")?
        .insert(label.to_owned());
    if label == "sun" && SUN_REQUESTED.load(Ordering::SeqCst) && !launcher_open(window.app_handle())
    {
        window.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn get_bootstrap(engine: AppState<'_>) -> Result<Bootstrap, String> {
    engine.bootstrap()
}
#[tauri::command]
fn save_orbit(app: tauri::AppHandle, engine: AppState<'_>, orbit: Orbit) -> Result<Orbit, String> {
    let orbit = engine.save_orbit(orbit)?;
    if !orbit.enabled
        && let Some(window) = app.get_webview_window(&format!("orbit-{}", orbit.id))
    {
        let _ = window.hide();
    }
    Ok(orbit)
}
#[tauri::command]
fn delete_orbit(app: tauri::AppHandle, engine: AppState<'_>, id: String) -> Result<(), String> {
    engine.delete_orbit(&id)?;
    for label in [format!("orbit-{id}"), format!("chat-{id}")] {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
    }
    Ok(())
}
#[tauri::command]
fn save_provider(engine: AppState<'_>, provider: ProviderDraft) -> Result<ProviderView, String> {
    engine.save_provider(provider)
}
#[tauri::command]
async fn discover_models(engine: AppState<'_>, id: String) -> Result<Vec<String>, String> {
    engine.discover_models(&id).await
}
#[tauri::command]
async fn start_login(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    provider_id: String,
) -> Result<serde_json::Value, String> {
    let result = engine.start_login(&provider_id).await?;
    if let Some(url) = result["url"].as_str() {
        app.opener()
            .open_url(url, None::<String>)
            .map_err(|e| e.to_string())?;
    }
    Ok(result)
}
#[tauri::command]
async fn complete_login(
    engine: AppState<'_>,
    provider_id: String,
    callback: String,
) -> Result<(), String> {
    engine
        .providers
        .complete_login(&engine.provider(&provider_id)?, &callback)
        .await?;
    engine.changed();
    Ok(())
}
#[tauri::command]
fn login_status(
    engine: AppState<'_>,
    provider_id: String,
) -> Result<solar_providers::LoginStatus, String> {
    engine
        .providers
        .login_status(&engine.provider(&provider_id)?)
}
#[tauri::command]
fn logout_provider(engine: AppState<'_>, id: String) -> Result<(), String> {
    engine.logout_provider(&id)
}
#[tauri::command]
fn save_project(engine: AppState<'_>, project: Project) -> Result<Project, String> {
    engine.save_project(project)
}
#[tauri::command]
fn create_conversation(
    engine: AppState<'_>,
    orbit_ids: Vec<String>,
    lead_orbit_id: String,
    title: String,
    project_id: Option<String>,
) -> Result<Conversation, String> {
    engine.create_conversation(orbit_ids, lead_orbit_id, title, project_id)
}
#[tauri::command]
fn get_messages(engine: AppState<'_>, conversation_id: String) -> Result<Vec<Message>, String> {
    engine.messages(&conversation_id)
}
#[tauri::command]
fn send_message(
    engine: AppState<'_>,
    conversation_id: String,
    content: String,
) -> Result<Task, String> {
    engine.inner().send_message(&conversation_id, content)
}
#[tauri::command]
fn stop_task(engine: AppState<'_>, task_id: String) -> Result<(), String> {
    engine.stop_task(&task_id)
}
#[tauri::command]
fn resolve_approval(engine: AppState<'_>, id: String, allow: bool) -> Result<(), String> {
    engine.resolve_approval(&id, allow)
}
#[tauri::command]
fn get_memories(
    engine: AppState<'_>,
    orbit_id: Option<String>,
    project_id: Option<String>,
) -> Result<Vec<Memory>, String> {
    engine.memories(orbit_id.as_deref(), project_id.as_deref())
}
#[tauri::command]
fn save_memory(engine: AppState<'_>, memory: Memory) -> Result<Memory, String> {
    engine.save_memory(memory)
}
#[tauri::command]
fn delete_memory(engine: AppState<'_>, id: String) -> Result<(), String> {
    engine.delete_memory(&id)
}
#[tauri::command]
fn export_orbit(engine: AppState<'_>, id: String) -> Result<String, String> {
    engine.export_orbit(&id)
}
#[tauri::command]
fn import_orbit(engine: AppState<'_>, profile_json: String) -> Result<Orbit, String> {
    engine.import_orbit(&profile_json)
}
#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    settings: Settings,
) -> Result<Settings, String> {
    let settings = engine.save_settings(settings)?;
    if settings.desktop_enabled {
        ensure_sun(&app)?;
    } else {
        hide_companions(&app);
    }
    engine.changed();
    crate::desktop_attention::refresh(&app);
    crate::desktop_onboarding::settings_changed(&app, &settings);
    Ok(settings)
}
#[tauri::command]
fn save_routine(engine: AppState<'_>, routine: Routine) -> Result<Routine, String> {
    engine.save_routine(routine)
}
#[tauri::command]
async fn start_screen_share(engine: AppState<'_>, orbit_id: String) -> Result<(), String> {
    engine.start_screen_share(&orbit_id).await
}
#[tauri::command]
async fn stop_screen_share(engine: AppState<'_>, orbit_id: String) -> Result<(), String> {
    engine.stop_screen_share(&orbit_id).await
}
#[tauri::command]
async fn save_mcp_server(
    engine: AppState<'_>,
    server: McpServerDraft,
) -> Result<McpServer, String> {
    engine.save_mcp_server(server).await
}
#[tauri::command]
async fn connect_mcp(engine: AppState<'_>, id: String) -> Result<usize, String> {
    engine.connect_mcp(&id).await
}
#[tauri::command]
async fn delete_mcp_server(engine: AppState<'_>, id: String) -> Result<(), String> {
    engine.delete_mcp_server(&id).await
}
#[tauri::command]
fn open_launcher(app: tauri::AppHandle) -> Result<(), String> {
    show_launcher(&app)
}
#[tauri::command]
fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid link")?;
    if !["http", "https"].contains(&parsed.scheme()) || parsed.host_str().is_none() {
        return Err("Only web links can be opened externally".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<String>)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) fn open_full_chat(app: tauri::AppHandle, conversation_id: String) -> Result<(), String> {
    show_launcher(&app)?;
    app.emit_to(
        "main",
        "solar://navigate",
        serde_json::json!({"conversationId":conversation_id}),
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
fn set_desktop_expanded(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    expanded: bool,
) -> Result<(), String> {
    let settings = engine.settings()?;
    if !settings.onboarding_complete {
        return Err("Finish or skip the welcome tour before opening desktop Orbits".into());
    }
    if expanded && !settings.desktop_enabled {
        return Err("Enable desktop companions in Settings before opening desktop Orbits".into());
    }
    if expanded {
        COMPANIONS_HIDDEN.store(false, Ordering::SeqCst);
        if launcher_open(&app) {
            hide_launcher(&app)?;
        }
    }
    ensure_sun(&app)?;
    EXPANDED.store(expanded, Ordering::SeqCst);
    engine.desktop_expanded.store(expanded, Ordering::SeqCst);
    let epoch = ANIMATION_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    ANIMATING.store(true, Ordering::SeqCst);
    let sun = app
        .get_webview_window("sun")
        .ok_or("Desktop sun is unavailable")?;
    let (bounds, formation_scale) = desktop_screen(&app, &sun);
    let sun_position = sun
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(formation_scale);
    let mut orbits: Vec<_> = engine
        .bootstrap()?
        .orbits
        .into_iter()
        .filter(|o| o.enabled)
        .collect();
    orbits.sort_by(|a, b| a.id.cmp(&b.id));
    let layout = crate::desktop_layout::orbit_layout(
        bounds,
        (sun_position.x, sun_position.y),
        if expanded { orbits.len() } else { 0 },
    );
    position_window(&sun, layout.sun, formation_scale)?;
    let origin = (layout.sun.0 + 40.0, layout.sun.1 + 40.0);
    let mut movements = Vec::new();
    if expanded {
        for (orbit, target) in orbits.iter().zip(layout.planets) {
            let label = format!("orbit-{}", orbit.id);
            if app.get_webview_window(&label).is_none() {
                create_companion(
                    &app,
                    &label,
                    &format!("orbit/{}", orbit.id),
                    origin.0,
                    origin.1,
                )?;
            }
            if let Some(window) = app.get_webview_window(&label) {
                position_window(&window, origin, formation_scale)?;
                movements.push((window, origin, target));
            }
        }
    } else {
        for (label, window) in app.webview_windows() {
            if label.starts_with("orbit-") {
                let start = window
                    .outer_position()
                    .map_err(|e| e.to_string())?
                    .to_logical::<f64>(formation_scale);
                movements.push((window, (start.x, start.y), origin));
            }
        }
    }
    let reduced_motion = engine.settings()?.reduced_motion;
    let animation_app = app.clone();
    tauri::async_runtime::spawn(async move {
        if expanded {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
                    return;
                }
                let ready = READY_WIDGETS
                    .lock()
                    .map(|widgets| {
                        movements
                            .iter()
                            .all(|(window, _, _)| widgets.contains(window.label()))
                    })
                    .unwrap_or(false);
                if ready || std::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
                return;
            }
            // A failed webview must stay hidden rather than exposing a blank window.
            movements.retain(|(window, _, _)| {
                let ready = READY_WIDGETS
                    .lock()
                    .map(|widgets| widgets.contains(window.label()))
                    .unwrap_or(false);
                if !ready {
                    eprintln!("Desktop widget did not become ready: {}", window.label());
                    crate::remote_logging::event(
                        "warn",
                        "widget_ready_timeout",
                        "A desktop widget did not become ready",
                        serde_json::json!({"window": window.label()}),
                    );
                }
                ready
            });
            let windows = movements
                .iter()
                .map(|(window, _, _)| window.clone())
                .collect::<Vec<_>>();
            let show_app = animation_app.clone();
            let _ = animation_app.run_on_main_thread(move || {
                if ANIMATION_EPOCH.load(Ordering::SeqCst) == epoch && !launcher_open(&show_app) {
                    for window in windows {
                        let _ = window.show();
                    }
                }
            });
            // Give newly visible surfaces a presentation cycle before moving them.
            tokio::time::sleep(Duration::from_millis(32)).await;
        }
        let steps = if reduced_motion { 1 } else { 18 };
        for step in 1..=steps {
            if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
                return;
            }
            let progress = step as f64 / steps as f64;
            let eased = 1.0 - (1.0 - progress).powi(3);
            let positions = movements
                .iter()
                .map(|(window, from, to)| {
                    (
                        window.clone(),
                        (
                            from.0 + (to.0 - from.0) * eased,
                            from.1 + (to.1 - from.1) * eased,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let move_app = animation_app.clone();
            let initial_sun = layout.sun;
            let _ = animation_app.run_on_main_thread(move || {
                if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch || launcher_open(&move_app) {
                    return;
                }
                let shift = move_app
                    .get_webview_window("sun")
                    .and_then(|sun| sun.outer_position().ok())
                    .map(|p| {
                        (
                            f64::from(p.x) / formation_scale - initial_sun.0,
                            f64::from(p.y) / formation_scale - initial_sun.1,
                        )
                    })
                    .unwrap_or((0.0, 0.0));
                for (window, position) in positions {
                    let _ = position_window(
                        &window,
                        (position.0 + shift.0, position.1 + shift.1),
                        formation_scale,
                    );
                }
            });
            if !reduced_motion {
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        }
        if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
            return;
        }
        let finish_app = animation_app.clone();
        let _ = animation_app.run_on_main_thread(move || {
            if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch || launcher_open(&finish_app) {
                return;
            }
            if !expanded {
                for (window, _, _) in movements {
                    let _ = window.hide();
                }
            }
            ANIMATING.store(false, Ordering::SeqCst);
            let _ = align_companions(&finish_app);
        });
    });
    let _ = app.emit("solar://desktop", expanded);
    engine.changed();
    Ok(())
}

#[tauri::command]
pub(crate) fn open_orbit_chat(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    orbit_id: String,
) -> Result<(), String> {
    let orbit = engine.orbit(&orbit_id)?;
    if launcher_open(&app) {
        let conversation = engine
            .bootstrap()?
            .conversations
            .into_iter()
            .find(|c| c.orbit_ids.len() == 1 && c.orbit_ids[0] == orbit_id)
            .map(Ok)
            .unwrap_or_else(|| {
                engine.create_conversation(
                    vec![orbit_id.clone()],
                    orbit_id.clone(),
                    orbit.name.clone(),
                    None,
                )
            })?;
        return open_full_chat(app, conversation.id);
    }
    let label = format!("chat-{orbit_id}");
    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        crate::desktop_attention::refresh(&app);
        return Ok(());
    }
    let position = app
        .get_webview_window(&format!("orbit-{orbit_id}"))
        .and_then(|w| Some((w.outer_position().ok()?, w.scale_factor().ok()?)))
        .map(|(p, s)| p.to_logical::<f64>(s));
    let conversation = engine
        .bootstrap()?
        .conversations
        .into_iter()
        .find(|c| c.orbit_ids.len() == 1 && c.orbit_ids[0] == orbit_id)
        .map(Ok)
        .unwrap_or_else(|| {
            engine.create_conversation(
                vec![orbit_id.clone()],
                orbit_id.clone(),
                orbit.name.clone(),
                None,
            )
        })?;
    crate::desktop_chat::register_popup(&app, &label, &conversation);
    let mut builder = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::App(format!("index.html#chat/{orbit_id}").into()),
    )
    .title(format!("{} · Solar", orbit.name))
    .incognito(true)
    .background_color(tauri::webview::Color(22, 31, 36, 255))
    .inner_size(410.0, 560.0)
    .min_inner_size(320.0, 400.0)
    .always_on_top(true)
    .skip_taskbar(true);
    if let Some(p) = position {
        let (x, y) = clamp_to_screen(&app, p.x + 90.0, p.y - 180.0, 410.0, 560.0);
        builder = builder.position(x, y);
    }
    if let Err(error) = builder.build() {
        crate::desktop_chat::clear_window(&app, &label);
        return Err(error.to_string());
    }
    crate::desktop_attention::refresh(&app);
    Ok(())
}

pub(crate) fn launcher_open(_: &tauri::AppHandle) -> bool {
    LAUNCHER_OPEN.load(Ordering::SeqCst)
}

pub(crate) fn attention_is_visible(app: &tauri::AppHandle, item: &AttentionItem) -> bool {
    crate::desktop_chat::attention_is_visible(app, item)
}

fn hide_companions(app: &tauri::AppHandle) {
    SUN_REQUESTED.store(false, Ordering::SeqCst);
    EXPANDED.store(false, Ordering::SeqCst);
    ANIMATION_EPOCH.fetch_add(1, Ordering::SeqCst);
    ANIMATING.store(false, Ordering::SeqCst);
    if let Some(engine) = app.try_state::<Arc<Engine>>() {
        engine.desktop_expanded.store(false, Ordering::SeqCst);
        engine.changed();
    }
    for (label, window) in app.webview_windows() {
        if label == "sun" || label.starts_with("orbit-") {
            let _ = window.hide();
        }
    }
    let _ = app.emit("solar://desktop", false);
}

fn set_launcher_presence(app: &tauri::AppHandle, open: bool) {
    let was_open = LAUNCHER_OPEN.swap(open, Ordering::SeqCst);
    if open {
        hide_companions(app);
        for (label, window) in app.webview_windows() {
            if label.starts_with("chat-") {
                let _ = window.hide();
                let _ = window.close();
            }
        }
    } else if was_open && !COMPANIONS_HIDDEN.load(Ordering::SeqCst) {
        let _ = ensure_sun(app);
    }
    crate::desktop_attention::refresh(app);
}

fn hide_launcher(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Launcher window is unavailable")?;
    window.hide().map_err(|e| e.to_string())?;
    set_launcher_presence(app, false);
    Ok(())
}

fn show_launcher(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Launcher window is unavailable")?;
    set_launcher_presence(app, true);
    window.unminimize().map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

fn desktop_screen(
    app: &tauri::AppHandle,
    sun: &tauri::WebviewWindow,
) -> (crate::desktop_layout::Rect, f64) {
    let monitor = sun
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        (
            crate::desktop_layout::Rect {
                x: f64::from(area.position.x) / scale,
                y: f64::from(area.position.y) / scale,
                width: f64::from(area.size.width) / scale,
                height: f64::from(area.size.height) / scale,
            },
            scale,
        )
    } else {
        (
            crate::desktop_layout::Rect {
                x: 0.0,
                y: 28.0,
                width: 1440.0,
                height: 872.0,
            },
            1.0,
        )
    }
}

fn position_window(
    window: &tauri::WebviewWindow,
    position: (f64, f64),
    scale: f64,
) -> Result<(), String> {
    let target = tauri::PhysicalPosition::new(
        (position.0 * scale).round() as i32,
        (position.1 * scale).round() as i32,
    );
    let current = window.outer_position().map_err(|e| e.to_string())?;
    if current != target {
        window.set_position(target).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn align_companions(app: &tauri::AppHandle) -> Result<(), String> {
    if ANIMATING.load(Ordering::SeqCst) || launcher_open(app) {
        return Ok(());
    }
    let Some(sun) = app.get_webview_window("sun") else {
        return Ok(());
    };
    let engine = app.state::<Arc<Engine>>();
    let (bounds, formation_scale) = desktop_screen(app, &sun);
    let mut orbits: Vec<_> = engine
        .bootstrap()?
        .orbits
        .into_iter()
        .filter(|o| o.enabled)
        .collect();
    orbits.sort_by(|a, b| a.id.cmp(&b.id));
    let p = sun
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(formation_scale);
    let expanded = EXPANDED.load(Ordering::SeqCst);
    let layout = crate::desktop_layout::orbit_layout(
        bounds,
        (p.x, p.y),
        if expanded { orbits.len() } else { 0 },
    );
    position_window(&sun, layout.sun, formation_scale)?;
    if expanded {
        for (orbit, target) in orbits.iter().zip(layout.planets) {
            if let Some(window) = app.get_webview_window(&format!("orbit-{}", orbit.id)) {
                position_window(&window, target, formation_scale)?;
            }
        }
    }
    engine.store.put("desktop", "sun", &layout.sun)?;
    Ok(())
}

fn clamp_to_screen(app: &tauri::AppHandle, x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
    let Ok(monitors) = app.available_monitors() else {
        return (x.max(0.0), y.max(0.0));
    };
    let selected = monitors
        .iter()
        .find(|m| {
            let p = m.position().to_logical::<f64>(m.scale_factor());
            let size = m.size().to_logical::<f64>(m.scale_factor());
            x >= p.x && x < p.x + size.width && y >= p.y && y < p.y + size.height
        })
        .or_else(|| monitors.first());
    if let Some(m) = selected {
        let p = m.position().to_logical::<f64>(m.scale_factor());
        let s = m.size().to_logical::<f64>(m.scale_factor());
        (
            x.clamp(p.x, (p.x + s.width - width).max(p.x)),
            y.clamp(p.y + 28.0, (p.y + s.height - height).max(p.y + 28.0)),
        )
    } else {
        (x, y)
    }
}

fn create_companion(
    app: &tauri::AppHandle,
    label: &str,
    route: &str,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let size = if label == "sun" { 192.0 } else { 112.0 };
    let (x, y) = clamp_to_screen(app, x, y, size, size);
    READY_WIDGETS
        .lock()
        .map_err(|_| "Widget state unavailable")?
        .remove(label);
    let window = WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::App(format!("index.html#{route}").into()),
    )
    .title("Solar")
    .incognito(true)
    .inner_size(size, size)
    .position(x, y)
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
            // Join ordinary Spaces without fullScreenAuxiliary. Companions therefore stay
            // outside other applications' fullscreen Spaces and never activate on hover.
            unsafe {
                let ns = handle as *mut AnyObject;
                let _: () = msg_send![ns, setCollectionBehavior: 1usize | 16usize | 64usize];
            }
        }
    }
    let owned_label = label.to_owned();
    let app_for_event = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed)
            && let Ok(mut widgets) = READY_WIDGETS.lock()
        {
            widgets.remove(&owned_label);
        }
        if matches!(
            event,
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            let _ = align_companions(&app_for_event);
        }
    });
    Ok(())
}

pub(crate) fn ensure_sun(app: &tauri::AppHandle) -> Result<(), String> {
    let settings = app.state::<Arc<Engine>>().settings()?;
    if launcher_open(app)
        || COMPANIONS_HIDDEN.load(Ordering::SeqCst)
        || !settings.desktop_enabled
        || !settings.onboarding_complete
    {
        if let Some(window) = app.get_webview_window("sun") {
            let _ = window.hide();
        }
        return Ok(());
    }
    SUN_REQUESTED.store(true, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("sun") {
        if READY_WIDGETS
            .lock()
            .map_err(|_| "Widget state unavailable")?
            .contains("sun")
        {
            window.show().map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let saved = app
        .state::<Arc<Engine>>()
        .store
        .get::<(f64, f64)>("desktop", "sun")
        .ok()
        .flatten();
    let (x, y) = saved.unwrap_or_else(|| {
        app.primary_monitor()
            .ok()
            .flatten()
            .map(|m| {
                let area = m.work_area();
                let scale = m.scale_factor();
                (
                    f64::from(area.position.x) / scale + f64::from(area.size.width) / scale - 235.0,
                    f64::from(area.position.y) / scale + f64::from(area.size.height) / scale
                        - 245.0,
                )
            })
            .unwrap_or((100.0, 100.0))
    });
    create_companion(app, "sun", "sun", x, y)
}

pub(crate) fn restart_after_reset(app: &tauri::AppHandle) {
    // Reset already quiesced work. Preserve Tauri's restart exit code rather
    // than routing it through the ordinary quit-and-drain handler.
    QUITTING.store(true, Ordering::SeqCst);
    app.request_restart();
}

pub fn run() {
    crate::remote_logging::init();
    crate::remote_logging::event(
        "info",
        "app_starting",
        "Solar is starting",
        serde_json::json!({}),
    );
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(crate::desktop_intro::IntroWindowState::default());
            app.manage(crate::desktop_chat::ChatPresence::default());
            let home_dir = app.path().home_dir()?;
            let data_dir =
                crate::storage_root::prepare(&home_dir).map_err(std::io::Error::other)?;
            if crate::reset::apply_pending_reset(&data_dir).map_err(std::io::Error::other)? {
                crate::storage_root::prepare(&home_dir).map_err(std::io::Error::other)?;
            }
            let engine = Engine::open(data_dir).map_err(std::io::Error::other)?;
            crate::remote_logging::event(
                "info",
                "app_ready",
                "Solar storage and engine are ready",
                serde_json::json!({}),
            );
            let mut events = engine.subscribe();
            app.manage(engine.clone());
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(AppEvent::State) => {
                            let _ = app_handle.emit("solar://state", ());
                        }
                        Ok(AppEvent::Message(message)) => {
                            let _ = app_handle.emit("solar://message", message);
                        }
                        Ok(AppEvent::Task(task)) => {
                            let _ = app_handle.emit("solar://task", task);
                        }
                        Ok(AppEvent::Approval(approval)) => {
                            let _ = app_handle.emit("solar://approval", approval);
                        }
                        Ok(AppEvent::Attention) => {
                            crate::desktop_attention::refresh(&app_handle);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = app_handle.emit("solar://state", ());
                            crate::desktop_attention::refresh(&app_handle);
                        }
                        Err(_) => break,
                    }
                }
            });
            tauri::async_runtime::spawn({
                let engine = engine.clone();
                async move {
                    if let Err(error) = engine.start_routines() {
                        crate::remote_logging::event(
                            "error",
                            "routine_scheduler_failed",
                            "The routine scheduler could not start",
                            serde_json::json!({"error": error}),
                        );
                    }
                }
            });
            let show = MenuItem::with_id(app, "show", "Open Solar", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide companions", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Solar", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
            let pixels: Vec<u8> = (0..32 * 32)
                .flat_map(|n| {
                    let x = n % 32;
                    let y = n / 32;
                    let lit = (x - 16) * (x - 16) + (y - 16) * (y - 16) < 100;
                    if lit {
                        [250, 185, 72, 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
                .collect();
            TrayIconBuilder::new()
                .icon(tauri::image::Image::new_owned(pixels, 32, 32))
                .tooltip("Solar — your Orbits")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        let _ = show_launcher(app);
                    }
                    "hide" => {
                        COMPANIONS_HIDDEN.store(true, Ordering::SeqCst);
                        hide_companions(app);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            if engine.settings().map(|s| s.desktop_enabled).unwrap_or(true) {
                ensure_sun(app.handle()).map_err(std::io::Error::other)?;
            }
            let app_handle = app.handle().clone();
            // Mouse transparency follows the visible circular character region. This keeps
            // the transparent corners from intercepting clicks in other applications.
            tauri::async_runtime::spawn(async move {
                let mut attention_ticks = 0u32;
                loop {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    for (label, window) in app_handle.webview_windows() {
                        if label == "attention" {
                            crate::desktop_attention::update_hit_region(&window);
                            continue;
                        }
                        if label != "sun" && !label.starts_with("orbit-") {
                            continue;
                        }
                        if !window.is_visible().unwrap_or(false) {
                            continue;
                        }
                        if let (Ok(cursor), Ok(position), Ok(size)) = (
                            window.cursor_position(),
                            window.outer_position(),
                            window.outer_size(),
                        ) {
                            let dx = cursor.x - position.x as f64 - size.width as f64 / 2.0;
                            let dy = cursor.y - position.y as f64 - size.height as f64 / 2.0;
                            let scale = window.scale_factor().unwrap_or(1.0);
                            let radius = if label == "sun" {
                                60.0 * scale
                            } else {
                                size.width.min(size.height) as f64 * 0.44
                            };
                            let launcher = label == "sun"
                                && cursor.x - position.x as f64 > size.width as f64 - 40.0 * scale
                                && cursor.y - position.y as f64 > size.height as f64 - 40.0 * scale;
                            let _ = window.set_ignore_cursor_events(
                                dx * dx + dy * dy > radius * radius && !launcher,
                            );
                        }
                    }
                    attention_ticks += 1;
                    if attention_ticks.is_multiple_of(10) {
                        let visibility_app = app_handle.clone();
                        let _ = app_handle.run_on_main_thread(move || {
                            if let Some(main) = visibility_app.get_webview_window("main") {
                                let open = main.is_visible().unwrap_or(false)
                                    && !main.is_minimized().unwrap_or(false);
                                if open != launcher_open(&visibility_app) {
                                    set_launcher_presence(&visibility_app, open);
                                }
                            }
                        });
                    }
                    if attention_ticks == 50 {
                        attention_ticks = 0;
                        if app_handle
                            .get_webview_window("attention")
                            .is_some_and(|window| window.is_visible().unwrap_or(false))
                        {
                            crate::desktop_attention::refresh(&app_handle);
                        }
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) {
                crate::desktop_chat::clear_window(window.app_handle(), window.label());
                crate::desktop_attention::refresh(window.app_handle());
            }
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                crate::desktop_onboarding::stop_effects(window.app_handle());
                if let Some(main) = window.app_handle().get_webview_window("main") {
                    let _ = crate::desktop_intro::set_intro_fullscreen(
                        window.app_handle().clone(),
                        main,
                        false,
                    );
                }
                let _ = hide_launcher(window.app_handle());
            } else if window.label() == "main"
                && matches!(event, WindowEvent::Focused(_) | WindowEvent::Resized(_))
            {
                let open =
                    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
                set_launcher_presence(window.app_handle(), open);
            } else if window.label().starts_with("chat-")
                && matches!(event, WindowEvent::Focused(_) | WindowEvent::Resized(_))
            {
                crate::desktop_attention::refresh(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::desktop_chat::set_active_chat,
            crate::desktop_logging::report_frontend_event,
            crate::desktop_onboarding::reset_solar,
            crate::desktop_onboarding::save_onboarding_progress,
            crate::desktop_onboarding::complete_onboarding,
            crate::desktop_onboarding::play_sound_effect,
            crate::desktop_onboarding::stop_sound_effects,
            crate::desktop_intro::set_intro_fullscreen,
            crate::desktop_attention::get_attention_state,
            crate::desktop_attention::attention_ready,
            crate::desktop_attention::attention_bounds,
            crate::desktop_attention::dismiss_attention,
            crate::desktop_attention::open_attention,
            crate::desktop_attention::preview_attention,
            widget_ready,
            get_bootstrap,
            save_orbit,
            delete_orbit,
            save_provider,
            discover_models,
            start_login,
            complete_login,
            login_status,
            logout_provider,
            save_project,
            create_conversation,
            get_messages,
            send_message,
            stop_task,
            resolve_approval,
            get_memories,
            save_memory,
            delete_memory,
            export_orbit,
            import_orbit,
            save_settings,
            save_routine,
            start_screen_share,
            stop_screen_share,
            save_mcp_server,
            connect_mcp,
            delete_mcp_server,
            open_launcher,
            open_external_url,
            open_full_chat,
            set_desktop_expanded,
            open_orbit_chat
        ])
        .build(tauri::generate_context!())
        .expect("Solar could not start")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                let _ = show_launcher(app);
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !QUITTING.swap(true, Ordering::SeqCst) {
                    api.prevent_exit();
                    let engine = app.state::<Arc<Engine>>().inner().clone();
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(5), engine.shutdown()).await;
                        crate::remote_logging::event(
                            "info",
                            "app_stopped",
                            "Solar shut down",
                            serde_json::json!({}),
                        );
                        crate::remote_logging::flush(Duration::from_secs(2)).await;
                        app.exit(0);
                    });
                }
            }
        });
}
