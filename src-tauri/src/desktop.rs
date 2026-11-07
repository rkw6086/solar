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
    if label == "sun" && SUN_REQUESTED.load(Ordering::SeqCst) {
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
        SUN_REQUESTED.store(false, Ordering::SeqCst);
        EXPANDED.store(false, Ordering::SeqCst);
        engine.desktop_expanded.store(false, Ordering::SeqCst);
        ANIMATION_EPOCH.fetch_add(1, Ordering::SeqCst);
        ANIMATING.store(false, Ordering::SeqCst);
        for (label, window) in app.webview_windows() {
            if label == "sun" || label.starts_with("orbit-") {
                let _ = window.hide();
            }
        }
    }
    engine.changed();
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
fn open_full_chat(app: tauri::AppHandle, conversation_id: String) -> Result<(), String> {
    show_launcher(&app)?;
    app.emit_to(
        "main",
        "solar://navigate",
        serde_json::json!({"conversationId":conversation_id}),
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
fn save_position(engine: AppState<'_>, orbit_id: String, x: f64, y: f64) -> Result<(), String> {
    let mut orbit = engine.orbit(&orbit_id)?;
    orbit.x = Some(x);
    orbit.y = Some(y);
    engine.save_orbit(orbit)?;
    Ok(())
}
#[tauri::command]
fn set_desktop_expanded(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    expanded: bool,
) -> Result<(), String> {
    ensure_sun(&app)?;
    EXPANDED.store(expanded, Ordering::SeqCst);
    engine.desktop_expanded.store(expanded, Ordering::SeqCst);
    let epoch = ANIMATION_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    ANIMATING.store(true, Ordering::SeqCst);
    let sun = app
        .get_webview_window("sun")
        .ok_or("Desktop sun is unavailable")?;
    let sun_position = sun
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(sun.scale_factor().map_err(|e| e.to_string())?);
    let origin = (sun_position.x + 40.0, sun_position.y + 40.0);
    let mut movements = Vec::new();
    if expanded {
        let orbits: Vec<_> = engine
            .bootstrap()?
            .orbits
            .into_iter()
            .filter(|o| o.enabled)
            .collect();
        let count = orbits.len().max(1);
        for (index, orbit) in orbits.iter().enumerate() {
            let angle = std::f64::consts::TAU * index as f64 / count as f64;
            let x = orbit.x.unwrap_or(origin.0 + angle.cos() * 150.0);
            let y = orbit.y.unwrap_or(origin.1 + angle.sin() * 150.0);
            let target = clamp_to_screen(&app, x, y, 112.0, 112.0);
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
                window
                    .set_position(tauri::LogicalPosition::new(origin.0, origin.1))
                    .map_err(|e| e.to_string())?;
                movements.push((window, origin, target, Some(orbit.id.clone())));
            }
        }
    } else {
        for (label, window) in app.webview_windows() {
            if label.starts_with("orbit-") {
                let start = window
                    .outer_position()
                    .map_err(|e| e.to_string())?
                    .to_logical::<f64>(window.scale_factor().map_err(|e| e.to_string())?);
                movements.push((window, (start.x, start.y), origin, None));
            }
        }
    }
    let reduced_motion = engine.settings()?.reduced_motion;
    let owned_engine = engine.inner().clone();
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
                            .all(|(window, _, _, _)| widgets.contains(window.label()))
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
            movements.retain(|(window, _, _, _)| {
                let ready = READY_WIDGETS
                    .lock()
                    .map(|widgets| widgets.contains(window.label()))
                    .unwrap_or(false);
                if !ready {
                    eprintln!("Desktop widget did not become ready: {}", window.label());
                }
                ready
            });
            for (window, _, _, _) in &movements {
                let _ = window.show();
            }
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
            for (window, from, to, _) in &movements {
                let _ = window.set_position(tauri::LogicalPosition::new(
                    from.0 + (to.0 - from.0) * eased,
                    from.1 + (to.1 - from.1) * eased,
                ));
            }
            if !reduced_motion {
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        }
        if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
            return;
        }
        for (window, _, to, orbit_id) in movements {
            if expanded {
                if let Some(id) = orbit_id {
                    if let Ok(mut orbit) = owned_engine.orbit(&id) {
                        orbit.x = Some(to.0);
                        orbit.y = Some(to.1);
                        let _ = owned_engine.store.put("orbits", &id, &orbit);
                    }
                }
            } else {
                let _ = window.hide();
            }
        }
        ANIMATING.store(false, Ordering::SeqCst);
    });
    let _ = app.emit("solar://desktop", expanded);
    engine.changed();
    Ok(())
}

#[tauri::command]
fn open_orbit_chat(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    orbit_id: String,
) -> Result<(), String> {
    let orbit = engine.orbit(&orbit_id)?;
    let label = format!("chat-{orbit_id}");
    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    let position = app
        .get_webview_window(&format!("orbit-{orbit_id}"))
        .and_then(|w| Some((w.outer_position().ok()?, w.scale_factor().ok()?)))
        .map(|(p, s)| p.to_logical::<f64>(s));
    let mut builder = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::App(format!("index.html#chat/{orbit_id}").into()),
    )
    .title(format!("{} · Solar", orbit.name))
    .background_color(tauri::webview::Color(22, 31, 36, 255))
    .inner_size(410.0, 560.0)
    .min_inner_size(320.0, 400.0)
    .always_on_top(true)
    .skip_taskbar(true);
    if let Some(p) = position {
        let (x, y) = clamp_to_screen(&app, p.x + 90.0, p.y - 180.0, 410.0, 560.0);
        builder = builder.position(x, y);
    }
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

fn show_launcher(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Launcher window is unavailable")?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
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
        if let WindowEvent::Moved(position) = event {
            if ANIMATING.load(Ordering::SeqCst)
                || (owned_label.starts_with("orbit-") && !EXPANDED.load(Ordering::SeqCst))
            {
                return;
            }
            if owned_label == "sun" {
                if let Some(window) = app_for_event.get_webview_window("sun") {
                    if let Ok(scale) = window.scale_factor() {
                        let logical = position.to_logical::<f64>(scale);
                        let _ = app_for_event.state::<Arc<Engine>>().store.put(
                            "desktop",
                            "sun",
                            &(logical.x, logical.y),
                        );
                    }
                }
            }
            if let Some(id) = owned_label.strip_prefix("orbit-") {
                if let Some(window) = app_for_event.get_webview_window(&owned_label) {
                    if let Ok(scale) = window.scale_factor() {
                        let logical = position.to_logical::<f64>(scale);
                        let engine = app_for_event.state::<Arc<Engine>>();
                        if let Ok(mut orbit) = engine.orbit(id) {
                            orbit.x = Some(logical.x);
                            orbit.y = Some(logical.y);
                            let _ = engine.store.put("orbits", id, &orbit);
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

fn ensure_sun(app: &tauri::AppHandle) -> Result<(), String> {
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
                let size = m.size().to_logical::<f64>(m.scale_factor());
                (size.width - 235.0, size.height - 245.0)
            })
            .unwrap_or((100.0, 100.0))
    });
    create_companion(app, "sun", "sun", x, y)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let engine = Engine::open(data_dir).map_err(std::io::Error::other)?;
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
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = app_handle.emit("solar://state", ());
                        }
                        Err(_) => break,
                    }
                }
            });
            tauri::async_runtime::spawn({
                let engine = engine.clone();
                async move {
                    engine.start_routines();
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
                        let engine = app.state::<Arc<Engine>>();
                        let _ = set_desktop_expanded(app.clone(), engine, false);
                        SUN_REQUESTED.store(false, Ordering::SeqCst);
                        if let Some(sun) = app.get_webview_window("sun") {
                            let _ = sun.hide();
                        }
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
                loop {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    for (label, window) in app_handle.webview_windows() {
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
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
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
            save_position,
            set_desktop_expanded,
            open_orbit_chat
        ])
        .build(tauri::generate_context!())
        .expect("Solar could not start")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !QUITTING.swap(true, Ordering::SeqCst) {
                    api.prevent_exit();
                    let engine = app.state::<Arc<Engine>>().inner().clone();
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(5), engine.shutdown()).await;
                        app.exit(0);
                    });
                }
            }
        });
}
