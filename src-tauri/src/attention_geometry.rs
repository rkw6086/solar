//! Top-edge placement uses screen geometry, never a Mac model-name heuristic.
//!
//! macOS reports the camera housing through NSScreen's safe area and auxiliary
//! rectangles. Core Graphics supplies the same top-left, logical coordinates
//! used by Tauri/tao, so mixed-density and negative-origin displays need no
//! conversion through the primary screen's scale factor.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionPlacement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub notch_width: f64,
    pub notch_height: f64,
    pub mode: &'static str,
}

#[derive(Clone, Copy)]
struct ScreenGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    menu_height: f64,
    notch: Option<(f64, f64, f64)>, // center relative to screen, width, height
}

fn calculate(screen: ScreenGeometry) -> Result<AttentionPlacement, String> {
    if ![screen.x, screen.y, screen.width, screen.height]
        .into_iter()
        .all(f64::is_finite)
        || screen.width < 120.0
        || screen.height < 120.0
    {
        return Err("The screen has no usable attention area".into());
    }
    let notch = screen.notch.filter(|(center, width, height)| {
        center.is_finite()
            && width.is_finite()
            && height.is_finite()
            && *width > 0.0
            && *height > 0.0
            && *width < screen.width
            && *height < screen.height / 2.0
            && *center - *width / 2.0 >= 0.0
            && *center + *width / 2.0 <= screen.width
    });
    let (center, notch_width, notch_height, mode) = match notch {
        Some((center, width, height)) => (center, width, height, "notch"),
        None => (screen.width / 2.0, 0.0, 0.0, "edge"),
    };
    let width = 454.0_f64.max(notch_width + 64.0).min(screen.width - 16.0);
    let top_inset = if notch.is_some() {
        0.0
    } else if screen.menu_height.is_finite() {
        screen.menu_height.clamp(0.0, screen.height / 4.0)
    } else {
        0.0
    };
    let height = (notch_height + 230.0).min(screen.height - top_inset - 8.0);
    Ok(AttentionPlacement {
        x: screen.x + (center - width / 2.0).clamp(8.0, screen.width - width - 8.0),
        y: screen.y + top_inset,
        width,
        height,
        notch_width,
        notch_height,
        mode,
    })
}

/// Re-read placement each time an alert is shown, including after hot-plugging
/// a display. NSScreen access is confined to the AppKit main thread.
pub fn placement(app: &tauri::AppHandle) -> Result<AttentionPlacement, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        if let Some(mtm) = MainThreadMarker::new() {
            return macos_placement(mtm);
        }
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let result = MainThreadMarker::new()
                .ok_or_else(|| "AppKit main thread is unavailable".to_owned())
                .and_then(macos_placement);
            let _ = send.send(result);
        })
        .map_err(|error| error.to_string())?;
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "Timed out locating the attention display".to_owned())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let monitor = app
            .primary_monitor()
            .map_err(|error| error.to_string())?
            .ok_or("No display is available")?;
        let position = monitor.position().to_logical::<f64>(monitor.scale_factor());
        let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
        calculate(ScreenGeometry {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            menu_height: 0.0,
            notch: None,
        })
    }
}

#[cfg(target_os = "macos")]
fn macos_placement(mtm: objc2::MainThreadMarker) -> Result<AttentionPlacement, String> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSNumber, NSRect, ns_string};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayIsBuiltin(display: u32) -> u32;
        fn CGDisplayBounds(display: u32) -> NSRect;
    }

    fn display_id(screen: &NSScreen) -> Option<u32> {
        screen
            .deviceDescription()
            .objectForKey(ns_string!("NSScreenNumber"))?
            .downcast::<NSNumber>()
            .ok()
            .map(|number| number.unsignedIntValue())
    }

    let screens = NSScreen::screens(mtm);
    let selected = screens
        .iter()
        .find(|screen| {
            display_id(screen)
                // SAFETY: IDs come from the current NSScreen inventory.
                .map(|id| unsafe { CGDisplayIsBuiltin(id) != 0 })
                .unwrap_or(false)
        })
        .or_else(|| screens.firstObject())
        .ok_or("No display is available")?;
    let id = display_id(&selected).ok_or("The display has no native identifier")?;
    // SAFETY: Core Graphics returns a by-value CGRect for an active display ID.
    // NSRect aliases CGRect on supported 64-bit macOS targets.
    let bounds = unsafe { CGDisplayBounds(id) };
    let frame = selected.frame();
    let visible = selected.visibleFrame();
    let safe_top = selected.safeAreaInsets().top;
    let notch = if safe_top > 0.0 {
        let left = selected.auxiliaryTopLeftArea();
        let right = selected.auxiliaryTopRightArea();
        let left_edge = left.origin.x + left.size.width - frame.origin.x;
        let right_edge = right.origin.x - frame.origin.x;
        Some((
            (left_edge + right_edge) / 2.0,
            right_edge - left_edge,
            safe_top,
        ))
    } else {
        None
    };
    calculate(ScreenGeometry {
        x: bounds.origin.x,
        y: bounds.origin.y,
        width: bounds.size.width,
        height: bounds.size.height,
        menu_height: frame.origin.y + frame.size.height - (visible.origin.y + visible.size.height),
        notch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> ScreenGeometry {
        ScreenGeometry {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: 982.0,
            menu_height: 37.0,
            notch: Some((756.0, 180.0, 37.0)),
        }
    }

    #[test]
    fn notched_screen_reserves_the_camera_housing() {
        let placement = calculate(screen()).unwrap();
        assert_eq!(placement.x, 529.0);
        assert_eq!(placement.y, 0.0);
        assert_eq!(placement.notch_width, 180.0);
        assert_eq!(placement.notch_height, 37.0);
        assert_eq!(placement.height, 267.0);
        assert_eq!(placement.mode, "notch");
    }

    #[test]
    fn negative_origins_are_not_scaled_by_the_main_display() {
        let placement = calculate(ScreenGeometry {
            x: -1920.0,
            y: -300.0,
            ..screen()
        })
        .unwrap();
        assert_eq!(placement.x, -1391.0);
        assert_eq!(placement.y, -300.0);
    }

    #[test]
    fn plain_screen_sits_below_its_menu_bar() {
        let placement = calculate(ScreenGeometry {
            notch: None,
            menu_height: 24.0,
            ..screen()
        })
        .unwrap();
        assert_eq!(placement.mode, "edge");
        assert_eq!(placement.notch_width, 0.0);
        assert_eq!(placement.notch_height, 0.0);
        assert_eq!(placement.y, 24.0);
    }

    #[test]
    fn invalid_camera_geometry_falls_back_to_the_top_edge() {
        let placement = calculate(ScreenGeometry {
            notch: Some((20.0, 200.0, 37.0)),
            ..screen()
        })
        .unwrap();
        assert_eq!(placement.mode, "edge");
        assert_eq!(placement.y, 37.0);
    }

    #[test]
    fn small_display_keeps_the_panel_inside_its_bounds() {
        let placement = calculate(ScreenGeometry {
            width: 320.0,
            height: 240.0,
            menu_height: 24.0,
            notch: None,
            ..screen()
        })
        .unwrap();
        assert_eq!(placement.x, 8.0);
        assert_eq!(placement.width, 304.0);
        assert!(placement.y + placement.height <= 240.0);
    }
}
