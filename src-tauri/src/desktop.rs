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
static ORBIT_MOTION: LazyLock<Mutex<crate::desktop_motion::MotionState>> =
    LazyLock::new(|| Mutex::new(crate::desktop_motion::MotionState::default()));
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

fn widget_hit(label: &str, point: crate::desktop_pointer::PointerGeometry) -> bool {
    let radius = if label == "sun" {
        60.0
    } else {
        let (count, hovered) = ORBIT_MOTION
            .lock()
            .map(|motion| (motion.roster.len(), motion.is_hovered(label)))
            .unwrap_or((0, false));
        crate::desktop_layout::planet_art_size(count) * 0.46 * if hovered { 1.12 } else { 1.0 }
    };
    point.contains_widget(radius, label == "sun")
}

#[tauri::command]
fn widget_pointer_hover(window: tauri::WebviewWindow) -> Result<bool, String> {
    if window.label() != "sun" && !window.label().starts_with("orbit-") {
        return Err("This window is not a desktop widget".into());
    }
    Ok(crate::desktop_pointer::geometry(&window)
        .is_some_and(|point| widget_hit(window.label(), point)))
}

#[tauri::command]
fn desktop_interaction(
    window: tauri::WebviewWindow,
    hovered: Option<bool>,
    focused: Option<bool>,
) -> Result<(), String> {
    let label = window.label();
    if label != "sun" && !label.starts_with("orbit-") && !label.starts_with("chat-") {
        return Err("This window is not a desktop companion".into());
    }
    let mut motion = ORBIT_MOTION
        .lock()
        .map_err(|_| "Desktop motion is unavailable")?;
    let now = std::time::Instant::now();
    if let Some(hovered) = hovered {
        motion.hover(label, hovered, now);
    }
    if let Some(focused) = focused {
        motion.hover(&format!("focus:{label}"), focused, now);
    }
    if hovered.is_none() && focused.is_none() {
        motion.activity(now);
    }
    Ok(())
}

/// Await each submitted frame so slow native window servers never accumulate
/// stale movement callbacks. Both transitions and orbiting use this boundary.
async fn desktop_frame(app: &tauri::AppHandle, render: impl FnOnce() + Send + 'static) -> bool {
    let (complete, rendered) = tokio::sync::oneshot::channel();
    if app
        .run_on_main_thread(move || {
            render();
            let _ = complete.send(());
        })
        .is_err()
    {
        return false;
    }
    rendered.await.is_ok()
}

fn start_orbital_clock(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut clock = tokio::time::interval(crate::desktop_motion::FRAME_INTERVAL);
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            clock.tick().await;
            if QUITTING.load(Ordering::SeqCst) {
                return;
            }
            if !EXPANDED.load(Ordering::SeqCst) || ANIMATING.load(Ordering::SeqCst) {
                continue;
            }
            let tick_app = app.clone();
            if !desktop_frame(&app, move || {
                let expanded = EXPANDED.load(Ordering::SeqCst)
                    && !ANIMATING.load(Ordering::SeqCst)
                    && !launcher_open(&tick_app);
                let chat_open = tick_app
                    .webview_windows()
                    .keys()
                    .any(|label| label.starts_with("chat-"));
                let engine = tick_app.state::<Arc<Engine>>();
                let reduced = engine.settings().map(|s| s.reduced_motion).unwrap_or(true);
                let (collapse, moved) = ORBIT_MOTION
                    .lock()
                    .map(|mut motion| {
                        let previous = motion.elapsed;
                        let collapse =
                            motion.tick(std::time::Instant::now(), expanded, chat_open, reduced);
                        (collapse, motion.elapsed != previous)
                    })
                    .unwrap_or((false, false));
                if collapse {
                    let _ = set_desktop_expanded(tick_app.clone(), engine, false);
                } else if moved {
                    let _ = align_companions(&tick_app);
                }
            })
            .await
            {
                return;
            }
        }
    });
}

#[derive(Clone, serde::Serialize)]
struct PopupOrigin {
    x: f64,
    y: f64,
}

fn chat_origin(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> Result<PopupOrigin, String> {
    let id = window
        .label()
        .strip_prefix("chat-")
        .ok_or("This is not a Sprite chat")?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let chat = window.outer_position().map_err(|e| e.to_string())?;
    let planet = app.get_webview_window(&format!("orbit-{id}"));
    let source = planet.or_else(|| app.get_webview_window("sun"));
    if let Some(source) = source {
        let point = source.outer_position().map_err(|e| e.to_string())?;
        let source_scale = source.scale_factor().map_err(|e| e.to_string())?;
        let center = if source.label() == "sun" { 96.0 } else { 56.0 };
        Ok(PopupOrigin {
            x: (f64::from(point.x) + center * source_scale - f64::from(chat.x)) / scale,
            y: (f64::from(point.y) + center * source_scale - f64::from(chat.y)) / scale,
        })
    } else {
        Ok(PopupOrigin { x: 70.0, y: 70.0 })
    }
}

#[tauri::command]
fn popup_ready(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<PopupOrigin, String> {
    let origin = chat_origin(&app, &window)?;
    if let Some(id) = window.label().strip_prefix("chat-") {
        if let Some(planet) = app.get_webview_window(&format!("orbit-{id}")) {
            let _ = planet.hide();
        }
        if let Ok(mut motion) = ORBIT_MOTION.lock() {
            motion.remove(&format!("orbit-{id}"), std::time::Instant::now());
        }
    }
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(origin)
}

pub(crate) fn dismiss_orbit_chat(window: &tauri::WebviewWindow) {
    let origin =
        chat_origin(window.app_handle(), window).unwrap_or(PopupOrigin { x: 70.0, y: 70.0 });
    let _ = window.emit("solar://popup-close", origin);
}

pub(crate) fn cancel_orbit_chat_dismissal(window: &tauri::WebviewWindow) {
    let _ = window.emit("solar://popup-open", ());
}

#[tauri::command]
fn get_bootstrap(engine: AppState<'_>) -> Result<Bootstrap, String> {
    engine.bootstrap()
}
#[tauri::command]
fn get_orbit_cli_setup(
    engine: AppState<'_>,
    orbit_id: String,
) -> Result<crate::orbit_cli::OrbitCliSetup, String> {
    crate::orbit_cli::describe(&engine, &orbit_id)
}
#[tauri::command]
async fn open_orbit_cli_setup(engine: AppState<'_>, orbit_id: String) -> Result<(), String> {
    crate::orbit_cli::open(&engine, &orbit_id).await
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
async fn save_project(engine: AppState<'_>, project: Project) -> Result<Project, String> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.save_project(project))
        .await
        .map_err(|_| "Could not save the project".to_owned())?
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
    engine.update_routine_with(&routine.id, |stored| {
        stored.enabled = routine.enabled;
        stored.trigger = routine.trigger;
        stored.cron = routine.cron;
        Ok(())
    })
}
#[tauri::command]
fn delete_routine(engine: AppState<'_>, id: String) -> Result<(), String> {
    engine.delete_routine(&id)
}
#[tauri::command]
fn run_routine(engine: AppState<'_>, id: String) -> Result<Conversation, String> {
    engine.inner().clone().run_routine(&id)
}
#[tauri::command]
async fn start_screen_share(
    window: tauri::WebviewWindow,
    engine: AppState<'_>,
    orbit_id: String,
) -> Result<(), String> {
    let _popup = crate::desktop_popup::hold(&window);
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
fn open_orbit_settings(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    orbit_id: String,
) -> Result<(), String> {
    engine.orbit(&orbit_id)?;
    show_launcher(&app)?;
    app.emit_to(
        "main",
        "solar://edit-orbit",
        serde_json::json!({"orbitId":orbit_id}),
    )
    .map_err(|error| error.to_string())
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
fn toggle_desktop_expanded(app: tauri::AppHandle, engine: AppState<'_>) -> Result<bool, String> {
    // Decide against native state, not the webview's last asynchronous
    // bootstrap snapshot. A missed refresh must not consume the next click.
    let expanded = !EXPANDED.load(Ordering::SeqCst);
    set_desktop_expanded(app, engine, expanded)?;
    Ok(EXPANDED.load(Ordering::SeqCst))
}

#[tauri::command]
pub(crate) fn set_desktop_expanded(
    app: tauri::AppHandle,
    engine: AppState<'_>,
    expanded: bool,
) -> Result<(), String> {
    let settings = engine.settings()?;
    if !settings.onboarding_complete {
        return Err("Finish or skip the welcome tour before opening desktop Sprites".into());
    }
    if expanded && !settings.desktop_enabled {
        return Err("Enable desktop companions in Settings before opening desktop Sprites".into());
    }
    if expanded {
        crate::desktop_popup::cancel(&app);
    } else {
        crate::desktop_popup::dismiss_chats(&app);
    }
    if expanded {
        COMPANIONS_HIDDEN.store(false, Ordering::SeqCst);
        if launcher_open(&app) {
            hide_launcher(&app)?;
        }
    }
    ensure_sun(&app)?;
    // Duplicate state updates must not restart an in-flight transition.
    if EXPANDED.load(Ordering::SeqCst) == expanded {
        return Ok(());
    }
    EXPANDED.store(expanded, Ordering::SeqCst);
    engine.desktop_expanded.store(expanded, Ordering::SeqCst);
    let epoch = ANIMATION_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    ANIMATING.store(true, Ordering::SeqCst);
    let result = (|| -> Result<(), String> {
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
        if let Ok(mut motion) = ORBIT_MOTION.lock() {
            motion.reset(
                orbits.iter().map(|o| o.id.clone()).collect(),
                std::time::Instant::now(),
            );
        }
        let layout = crate::desktop_layout::orbit_layout(
            bounds,
            (sun_position.x, sun_position.y),
            if expanded { orbits.len() } else { 0 },
        );
        let sun_start = (sun_position.x, sun_position.y);
        let sun_target = layout.sun;
        let origin = if expanded {
            (sun_start.0 + 40.0, sun_start.1 + 40.0)
        } else {
            (sun_target.0 + 40.0, sun_target.1 + 40.0)
        };
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
                    // A reversed collapse starts from the current on-screen point,
                    // not from the sun, so a rapid second click never teleports it.
                    let start = if window.is_visible().unwrap_or(false) {
                        let point = window
                            .outer_position()
                            .map_err(|e| e.to_string())?
                            .to_logical::<f64>(formation_scale);
                        (point.x, point.y)
                    } else {
                        position_window(&window, origin, formation_scale)?;
                        origin
                    };
                    movements.push((window, start, target));
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
                if !desktop_frame(&animation_app, move || {
                    if ANIMATION_EPOCH.load(Ordering::SeqCst) == epoch && !launcher_open(&show_app)
                    {
                        for window in windows {
                            if !show_app
                                .webview_windows()
                                .contains_key(&window.label().replacen("orbit-", "chat-", 1))
                            {
                                let _ = window.show();
                            }
                        }
                    }
                })
                .await
                {
                    return;
                }
            }
            let started = std::time::Instant::now();
            let mut previous_sun_frame = sun_start;
            let mut clock = tokio::time::interval(crate::desktop_motion::FRAME_INTERVAL);
            clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                clock.tick().await;
                if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
                    return;
                }
                let eased =
                    crate::desktop_motion::transition_progress(started.elapsed(), reduced_motion);
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
                let sun_frame = (
                    sun_start.0 + (sun_target.0 - sun_start.0) * eased,
                    sun_start.1 + (sun_target.1 - sun_start.1) * eased,
                );
                let sun_step = (
                    ((sun_frame.0 * formation_scale).round()
                        - (previous_sun_frame.0 * formation_scale).round())
                        / formation_scale,
                    ((sun_frame.1 * formation_scale).round()
                        - (previous_sun_frame.1 * formation_scale).round())
                        / formation_scale,
                );
                if !desktop_frame(&animation_app, move || {
                    if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch || launcher_open(&move_app) {
                        return;
                    }
                    let shift = move_app
                        .get_webview_window("sun")
                        .and_then(|sun| {
                            let p = sun.outer_position().ok()?;
                            let actual = (
                                f64::from(p.x) / formation_scale + sun_step.0,
                                f64::from(p.y) / formation_scale + sun_step.1,
                            );
                            // Move the sun inward gradually when its full system would
                            // reach a screen edge. Incremental, pixel-aligned movement
                            // preserves any user drag made during this transition.
                            let _ = position_window(&sun, actual, formation_scale);
                            Some((actual.0 - sun_frame.0, actual.1 - sun_frame.1))
                        })
                        .unwrap_or((0.0, 0.0));
                    for (window, position) in positions {
                        let _ = position_window(
                            &window,
                            (position.0 + shift.0, position.1 + shift.1),
                            formation_scale,
                        );
                    }
                })
                .await
                {
                    return;
                }
                previous_sun_frame = sun_frame;
                if eased >= 1.0 {
                    break;
                }
            }
            if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch {
                return;
            }
            let finish_app = animation_app.clone();
            let _ = desktop_frame(&animation_app, move || {
                if ANIMATION_EPOCH.load(Ordering::SeqCst) != epoch || launcher_open(&finish_app) {
                    return;
                }
                if !expanded {
                    for (window, _, _) in movements {
                        let _ = window.hide();
                    }
                }
                if let Ok(mut motion) = ORBIT_MOTION.lock() {
                    motion.resume(std::time::Instant::now());
                }
                ANIMATING.store(false, Ordering::SeqCst);
                let _ = align_companions(&finish_app);
            })
            .await;
        });
        let _ = app.emit("solar://desktop", expanded);
        engine.changed();
        Ok(())
    })();
    if result.is_err()
        && ANIMATION_EPOCH
            .compare_exchange(epoch, epoch + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        // A failed webview or position lookup must leave the sun ready for a
        // fresh click, never stranded in an expanded/animating state.
        EXPANDED.store(false, Ordering::SeqCst);
        ANIMATING.store(false, Ordering::SeqCst);
        engine.desktop_expanded.store(false, Ordering::SeqCst);
        if let Ok(mut motion) = ORBIT_MOTION.lock() {
            motion.reset(Vec::new(), std::time::Instant::now());
        }
        for (label, window) in app.webview_windows() {
            if label.starts_with("orbit-") {
                let _ = window.hide();
            }
        }
        let _ = app.emit("solar://desktop", false);
        engine.changed();
    }
    result
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
        .or_else(|| app.get_webview_window("sun"))
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
    .decorations(false)
    .visible(false)
    .transparent(true)
    .background_color(tauri::webview::Color(0, 0, 0, 0))
    .inner_size(410.0, 560.0)
    .min_inner_size(320.0, 400.0)
    .always_on_top(true)
    .skip_taskbar(true);
    if let Some(p) = position {
        let (x, y) = clamp_to_screen(&app, p.x - 150.0, p.y - 160.0, 410.0, 560.0);
        builder = builder.position(x, y);
    }
    let window = builder.build().map_err(|error| {
        crate::desktop_chat::clear_window(&app, &label);
        error.to_string()
    })?;
    crate::desktop_popup::attach(&window);
    // popup_ready shows the window once its first frame and transition origin are ready.
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
    if let Ok(mut motion) = ORBIT_MOTION.lock() {
        motion.reset(Vec::new(), std::time::Instant::now());
    }
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
    let (bounds, formation_scale) = desktop_screen(app, &sun);
    let (orbits, elapsed) = ORBIT_MOTION
        .lock()
        .map(|motion| (motion.roster.clone(), motion.elapsed.as_secs_f64()))
        .map_err(|_| "Desktop motion is unavailable")?;
    let p = sun
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(formation_scale);
    let expanded = EXPANDED.load(Ordering::SeqCst);
    let layout = crate::desktop_layout::orbit_layout_at(
        bounds,
        (p.x, p.y),
        if expanded { orbits.len() } else { 0 },
        elapsed,
    );
    position_window(&sun, layout.sun, formation_scale)?;
    if expanded {
        for (orbit, target) in orbits.iter().zip(layout.planets) {
            if let Some(window) = app.get_webview_window(&format!("orbit-{orbit}")) {
                position_window(&window, target, formation_scale)?;
            }
        }
    }
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
    // `focused(false)` only applies when the window is built. Showing a hidden
    // Tao window again asks macOS to make it key, so keep companion surfaces
    // nonfocusable throughout their lifetime. WKWebView still receives the
    // first mouse click below; editable popup chats use a separate builder.
    .focusable(false)
    .accept_first_mouse(true)
    .build()
    .map_err(|e| e.to_string())?;
    crate::desktop_popup::attach(&window);
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
        if matches!(event, WindowEvent::Destroyed)
            && let Ok(mut motion) = ORBIT_MOTION.lock()
        {
            let now = std::time::Instant::now();
            motion.hover(&owned_label, false, now);
            motion.hover(&format!("focus:{owned_label}"), false, now);
        }
        if (owned_label == "sun" && matches!(event, WindowEvent::Moved(_)))
            || matches!(event, WindowEvent::ScaleFactorChanged { .. })
        {
            let _ = align_companions(&app_for_event);
            if owned_label == "sun" {
                if let (Some(engine), Some(sun)) = (
                    app_for_event.try_state::<Arc<Engine>>(),
                    app_for_event.get_webview_window("sun"),
                ) {
                    if let (Ok(point), Ok(scale)) = (sun.outer_position(), sun.scale_factor()) {
                        let point = point.to_logical::<f64>(scale);
                        let _ = engine.store.put("desktop", "sun", &(point.x, point.y));
                    }
                }
            }
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
            app.manage(crate::desktop_popup::PopupDismissals::default());
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
            start_orbital_clock(app.handle().clone());
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
                .tooltip("Solar — your Sprites")
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
                let pointer_hover = Arc::new(Mutex::new(std::collections::HashMap::new()));
                loop {
                    tokio::time::sleep(crate::desktop_motion::FRAME_INTERVAL).await;
                    let frame_app = app_handle.clone();
                    let hover_state = pointer_hover.clone();
                    // AppKit coordinates and cursor state are read together on
                    // the UI thread, regardless of which application has focus.
                    if !desktop_frame(&app_handle, move || {
                        let Ok(mut pointer_hover) = hover_state.lock() else {
                            return;
                        };
                        #[cfg(target_os = "macos")]
                        let had_widget_hover = pointer_hover.values().any(|hovered| *hovered);
                        let mut inside_companions = false;
                        let mut chat_open = false;
                        let windows = frame_app.webview_windows();
                        pointer_hover.retain(|label, _| windows.contains_key(label));
                        for (label, window) in windows {
                            let point = crate::desktop_pointer::geometry(&window);
                            if (label.starts_with("chat-") || label == "attention")
                                && window.is_visible().unwrap_or(false)
                            {
                                chat_open |= label.starts_with("chat-");
                                inside_companions |=
                                    point.is_some_and(|point| point.contains_window());
                            }
                            if label == "attention" {
                                crate::desktop_attention::update_hit_region(&window);
                                continue;
                            }
                            if label != "sun" && !label.starts_with("orbit-") {
                                continue;
                            }
                            let inside = point.is_some_and(|point| widget_hit(&label, point));
                            inside_companions |= inside;
                            if let Ok(mut motion) = ORBIT_MOTION.lock() {
                                let now = std::time::Instant::now();
                                motion.hover(&label, inside, now);
                                if point.is_none() {
                                    motion.hover(&format!("focus:{label}"), false, now);
                                }
                            }
                            if pointer_hover.insert(label.clone(), inside) != Some(inside) {
                                let _ = window.emit("solar://widget-hover", inside);
                                let _ = window.set_cursor_icon(if inside {
                                    tauri::CursorIcon::Hand
                                } else {
                                    tauri::CursorIcon::Default
                                });
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.set_ignore_cursor_events(!inside);
                                }
                            }
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let widget_hover = pointer_hover.values().any(|hovered| *hovered);
                            if widget_hover || had_widget_hover {
                                use objc2::{class, msg_send, runtime::AnyObject};
                                // Inactive WKWebViews do not always refresh their
                                // CSS cursor. Set it on the UI thread without
                                // activating Solar, and release it once on exit.
                                unsafe {
                                    let cursor: *mut AnyObject = if widget_hover {
                                        msg_send![class!(NSCursor), pointingHandCursor]
                                    } else {
                                        msg_send![class!(NSCursor), arrowCursor]
                                    };
                                    let _: () = msg_send![cursor, set];
                                }
                            }
                        }
                        crate::desktop_popup::pointer(
                            &frame_app,
                            crate::desktop_popup::pointer_state(),
                            inside_companions,
                            EXPANDED.load(Ordering::SeqCst) || chat_open,
                        );
                    })
                    .await
                    {
                        return;
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
                crate::orbit_prompt::cancel_window_analyses(window.label());
                crate::desktop_chat::clear_window(window.app_handle(), window.label());
                crate::desktop_attention::refresh(window.app_handle());
                if let Ok(mut motion) = ORBIT_MOTION.lock() {
                    motion.remove(window.label(), std::time::Instant::now());
                }
                if let Some(id) = window.label().strip_prefix("chat-") {
                    let enabled = window
                        .app_handle()
                        .state::<Arc<Engine>>()
                        .orbit(id)
                        .map(|orbit| orbit.enabled)
                        .unwrap_or(false);
                    if enabled
                        && EXPANDED.load(Ordering::SeqCst)
                        && !launcher_open(window.app_handle())
                    {
                        if let Some(planet) = window
                            .app_handle()
                            .get_webview_window(&format!("orbit-{id}"))
                        {
                            let _ = planet.show();
                        }
                    }
                }
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
            crate::orbit_prompt::create_orbit,
            crate::orbit_prompt::update_orbit_profile,
            crate::orbit_prompt::set_orbit_avatar,
            crate::orbit_prompt::cancel_orbit_prompt_analysis,
            crate::orbit_reset::reset_orbit,
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
            widget_pointer_hover,
            get_bootstrap,
            get_orbit_cli_setup,
            open_orbit_cli_setup,
            save_orbit,
            delete_orbit,
            save_provider,
            discover_models,
            start_login,
            complete_login,
            login_status,
            logout_provider,
            save_project,
            crate::sharing::discover_home_shares,
            crate::sharing::save_shared_folder,
            crate::sharing::delete_shared_folder,
            crate::sharing::choose_shared_directory,
            crate::project_location::choose_project_directory,
            crate::project_location::resolve_project_directory,
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
            delete_routine,
            run_routine,
            start_screen_share,
            stop_screen_share,
            save_mcp_server,
            connect_mcp,
            delete_mcp_server,
            open_launcher,
            open_orbit_settings,
            open_external_url,
            open_full_chat,
            desktop_interaction,
            popup_ready,
            set_desktop_expanded,
            toggle_desktop_expanded,
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
