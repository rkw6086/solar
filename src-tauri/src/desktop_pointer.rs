//! Hit-test in a window's logical coordinates. In particular, do not mix the
//! primary display's cursor scale with a different display's window scale.

#[derive(Clone, Copy, Debug)]
pub(crate) struct PointerGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PointerGeometry {
    pub fn valid(self) -> Option<Self> {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite)
            .then_some(self)
            .filter(|point| point.width > 0.0 && point.height > 0.0)
    }

    #[cfg(any(target_os = "macos", test))]
    pub fn from_appkit(mouse: (f64, f64), origin: (f64, f64), size: (f64, f64)) -> Option<Self> {
        Self {
            x: mouse.0 - origin.0,
            y: origin.1 + size.1 - mouse.1,
            width: size.0,
            height: size.1,
        }
        .valid()
    }

    pub fn contains_window(self) -> bool {
        self.x >= 0.0 && self.y >= 0.0 && self.x < self.width && self.y < self.height
    }

    pub fn contains_widget(self, radius: f64, sun: bool) -> bool {
        if !self.contains_window() {
            return false;
        }
        let dx = self.x - self.width / 2.0;
        let dy = self.y - self.height / 2.0;
        dx * dx + dy * dy <= radius * radius
            || (sun
                && crate::desktop_motion::launcher_hit(
                    self.x,
                    self.y,
                    self.width,
                    self.height,
                    1.0,
                ))
    }
}

/// Call on the UI thread, including for passive, non-key companion windows.
#[cfg(feature = "desktop")]
pub(crate) fn geometry(window: &tauri::WebviewWindow) -> Option<PointerGeometry> {
    if !window.is_visible().ok()? {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::{MainThreadMarker, class, msg_send, runtime::AnyObject};
        use objc2_foundation::{NSPoint, NSRect};
        let _main = MainThreadMarker::new()?;
        let ns = window.ns_window().ok()? as *mut AnyObject;
        // Both values are in AppKit screen points, even across Retina/non-Retina
        // displays and screens to the left of or above the primary display.
        let (mouse, frame): (NSPoint, NSRect) = unsafe {
            (
                msg_send![class!(NSEvent), mouseLocation],
                msg_send![ns, frame],
            )
        };
        PointerGeometry::from_appkit(
            (mouse.x, mouse.y),
            (frame.origin.x, frame.origin.y),
            (frame.size.width, frame.size.height),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let cursor = window.cursor_position().ok()?;
        let origin = window.outer_position().ok()?;
        let size = window.outer_size().ok()?;
        let scale = window.scale_factor().ok()?;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        PointerGeometry {
            x: (cursor.x - f64::from(origin.x)) / scale,
            y: (cursor.y - f64::from(origin.y)) / scale,
            width: f64::from(size.width) / scale,
            height: f64::from(size.height) / scale,
        }
        .valid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appkit_hit_is_independent_of_display_scaling_and_screen_origin() {
        for origin in [(200.0, 80.0), (-1920.0, 200.0), (40.0, 1200.0)] {
            let mouse = (origin.0 + 96.0, origin.1 + 96.0);
            let point = PointerGeometry::from_appkit(mouse, origin, (192.0, 192.0)).unwrap();
            assert_eq!((point.x, point.y), (96.0, 96.0));
            assert!(point.contains_widget(60.0, true));
            let away =
                PointerGeometry::from_appkit((mouse.0 + 200.0, mouse.1), origin, (192.0, 192.0))
                    .unwrap();
            assert!(!away.contains_widget(60.0, true));
        }
    }

    #[test]
    fn appkit_y_axis_and_transparent_corners_do_not_create_hover() {
        let point =
            PointerGeometry::from_appkit((10.0, 190.0), (0.0, 0.0), (192.0, 192.0)).unwrap();
        assert_eq!(point.y, 2.0);
        assert!(point.contains_window());
        assert!(!point.contains_widget(60.0, true));
        let launcher =
            PointerGeometry::from_appkit((168.0, 24.0), (0.0, 0.0), (192.0, 192.0)).unwrap();
        assert!(launcher.contains_widget(60.0, true));
        assert!(!launcher.contains_widget(60.0, false));
    }

    #[test]
    fn invalid_geometry_cannot_latch_a_hover() {
        assert!(
            PointerGeometry::from_appkit((f64::NAN, 0.0), (0.0, 0.0), (192.0, 192.0)).is_none()
        );
        assert!(PointerGeometry::from_appkit((0.0, 0.0), (0.0, 0.0), (0.0, 192.0)).is_none());
    }
}
