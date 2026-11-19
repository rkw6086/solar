//! Deterministic, sun-relative desktop positions in logical screen coordinates.

use std::f64::consts::{FRAC_PI_2, TAU};

const SUN_SIZE: f64 = 192.0;
const PLANET_SIZE: f64 = 112.0;
const EDGE_PADDING: f64 = 8.0;
const FIRST_RING_RADIUS: f64 = 200.0;
pub(crate) const ORBIT_PERIOD_SECONDS: f64 = 120.0;

/// Keep crowded teams readable and leave unambiguous pointer targets.
pub(crate) fn planet_art_size(count: usize) -> f64 {
    if count > 36 {
        56.0
    } else if count > 18 {
        72.0
    } else {
        88.0
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub sun: (f64, f64),
    pub planets: Vec<(f64, f64)>,
}

#[derive(Clone, Copy)]
struct Extents {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

fn within(value: f64, low: f64, high: f64) -> f64 {
    if low <= high {
        value.clamp(low, high)
    } else {
        // A screen smaller than the sun cannot contain its fixed-size window.
        // Center that unavoidable overflow instead of producing invalid coordinates.
        (low + high) / 2.0
    }
}

/// Return top-left positions for the sun and each planet in caller-supplied order.
///
/// Each lane is evenly populated, so repeating an expansion for the same
/// ordered roster produces the same positions. Move the formation as a unit at
/// screen edges; never clamp planets separately. Complete paths stay in bounds
/// as they rotate. Tiny displays and very large rosters necessarily get denser.
pub(crate) fn orbit_layout(bounds: Rect, sun: (f64, f64), count: usize) -> Layout {
    orbit_layout_at(bounds, sun, count, 0.0)
}

/// All members of a lane keep the same angular speed. Adjacent lanes are in a
/// 2:1 resonance, matching the launcher scene. Reserve each complete circular
/// path at the screen edge so moving planets never shove the sun around.
pub(crate) fn orbit_layout_at(
    bounds: Rect,
    sun: (f64, f64),
    count: usize,
    elapsed_seconds: f64,
) -> Layout {
    let bounds = Rect {
        x: if bounds.x.is_finite() { bounds.x } else { 0.0 },
        y: if bounds.y.is_finite() { bounds.y } else { 0.0 },
        width: if bounds.width.is_finite() {
            bounds.width.max(1.0)
        } else {
            SUN_SIZE
        },
        height: if bounds.height.is_finite() {
            bounds.height.max(1.0)
        } else {
            SUN_SIZE
        },
    };
    let padding_x = EDGE_PADDING.min(((bounds.width - SUN_SIZE) / 2.0).max(0.0));
    let padding_y = EDGE_PADDING.min(((bounds.height - SUN_SIZE) / 2.0).max(0.0));
    let available_width = bounds.width - padding_x * 2.0;
    let available_height = bounds.height - padding_y * 2.0;

    let mut offsets = Vec::with_capacity(count);
    let mut remaining = count;
    let mut ring = 0usize;
    let elapsed_seconds = if elapsed_seconds.is_finite() {
        elapsed_seconds.max(0.0)
    } else {
        0.0
    };
    while remaining > 0 {
        let on_ring = remaining.min(6 * (ring + 1));
        let radius = FIRST_RING_RADIUS * (ring + 1) as f64;
        let phase = if ring % 2 == 1 {
            std::f64::consts::PI / on_ring as f64
        } else {
            0.0
        };
        let travel = elapsed_seconds / (ORBIT_PERIOD_SECONDS * 2.0_f64.powi(ring as i32)) * TAU;
        for index in 0..on_ring {
            let angle = -FRAC_PI_2 + TAU * index as f64 / on_ring as f64 + phase + travel;
            offsets.push((angle.cos() * radius, angle.sin() * radius));
        }
        remaining -= on_ring;
        ring += 1;
    }
    let radius = FIRST_RING_RADIUS * ring as f64;
    let available_radius = ((available_width.min(available_height) - PLANET_SIZE) / 2.0).max(0.0);
    let scale = if radius > 0.0 {
        (available_radius / radius).min(1.0)
    } else {
        1.0
    };
    let extent_radius =
        (radius * scale + if count > 0 { PLANET_SIZE / 2.0 } else { 0.0 }).max(SUN_SIZE / 2.0);
    let extent = Extents {
        left: -extent_radius,
        top: -extent_radius,
        right: extent_radius,
        bottom: extent_radius,
    };
    let desired_x = if sun.0.is_finite() {
        sun.0 + SUN_SIZE / 2.0
    } else {
        bounds.x + bounds.width / 2.0
    };
    let desired_y = if sun.1.is_finite() {
        sun.1 + SUN_SIZE / 2.0
    } else {
        bounds.y + bounds.height / 2.0
    };
    let center_x = within(
        desired_x,
        bounds.x + padding_x - extent.left,
        bounds.x + bounds.width - padding_x - extent.right,
    );
    let center_y = within(
        desired_y,
        bounds.y + padding_y - extent.top,
        bounds.y + bounds.height - padding_y - extent.bottom,
    );

    Layout {
        sun: (center_x - SUN_SIZE / 2.0, center_y - SUN_SIZE / 2.0),
        planets: offsets
            .into_iter()
            .map(|(x, y)| {
                (
                    center_x + x * scale - PLANET_SIZE / 2.0,
                    center_y + y * scale - PLANET_SIZE / 2.0,
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect {
            x: 0.0,
            y: 28.0,
            width: 2560.0,
            height: 1412.0,
        }
    }

    fn assert_inside(position: (f64, f64), size: f64, bounds: Rect) {
        assert!(position.0 >= bounds.x - 0.000001);
        assert!(position.1 >= bounds.y - 0.000001);
        assert!(position.0 + size <= bounds.x + bounds.width + 0.000001);
        assert!(position.1 + size <= bounds.y + bounds.height + 0.000001);
    }

    fn separated(a: (f64, f64), a_size: f64, b: (f64, f64), b_size: f64) -> bool {
        a.0 + a_size <= b.0 + 0.000001
            || b.0 + b_size <= a.0 + 0.000001
            || a.1 + a_size <= b.1 + 0.000001
            || b.1 + b_size <= a.1 + 0.000001
    }

    #[test]
    fn repeated_expansions_have_identical_slots_and_encircle_the_sun() {
        let layout = orbit_layout(screen(), (1100.0, 550.0), 6);
        assert_eq!(layout, orbit_layout(screen(), (1100.0, 550.0), 6));
        let center = (layout.sun.0 + 96.0, layout.sun.1 + 96.0);
        let centers: Vec<_> = layout
            .planets
            .iter()
            .map(|&(x, y)| (x + 56.0, y + 56.0))
            .collect();
        assert!(centers.iter().any(|&(x, _)| x < center.0));
        assert!(centers.iter().any(|&(x, _)| x > center.0));
        assert!(centers.iter().any(|&(_, y)| y < center.1));
        assert!(centers.iter().any(|&(_, y)| y > center.1));
        for (x, y) in centers {
            assert!(((x - center.0).hypot(y - center.1) - FIRST_RING_RADIUS).abs() < 0.000001);
        }
    }

    #[test]
    fn ordinary_rosters_have_clear_planet_windows_and_clear_the_sun() {
        for count in 1..=18 {
            let layout = orbit_layout(screen(), (1100.0, 550.0), count);
            for (index, &planet) in layout.planets.iter().enumerate() {
                assert_inside(planet, PLANET_SIZE, screen());
                assert!(
                    separated(planet, PLANET_SIZE, layout.sun, SUN_SIZE),
                    "sun collision for {count}"
                );
                for &other in &layout.planets[..index] {
                    assert!(
                        separated(planet, PLANET_SIZE, other, PLANET_SIZE),
                        "planet collision for {count}"
                    );
                }
            }
        }
    }

    #[test]
    fn screen_edges_translate_the_whole_formation_without_distortion() {
        let centered = orbit_layout(screen(), (1100.0, 550.0), 8);
        let edge = orbit_layout(screen(), (-500.0, -500.0), 8);
        let shift = (edge.sun.0 - centered.sun.0, edge.sun.1 - centered.sun.1);
        assert_inside(edge.sun, SUN_SIZE, screen());
        for (&original, &moved) in centered.planets.iter().zip(&edge.planets) {
            assert_inside(moved, PLANET_SIZE, screen());
            assert!((moved.0 - original.0 - shift.0).abs() < 0.000001);
            assert!((moved.1 - original.1 - shift.1).abs() < 0.000001);
        }
    }

    #[test]
    fn negative_origin_and_small_work_area_keep_every_window_visible() {
        let bounds = Rect {
            x: -1280.0,
            y: -850.0,
            width: 1280.0,
            height: 822.0,
        };
        for count in [0, 3, 8, 22, 64] {
            let layout = orbit_layout(bounds, (-9000.0, -9000.0), count);
            assert_inside(layout.sun, SUN_SIZE, bounds);
            assert_eq!(layout.planets.len(), count);
            for planet in layout.planets {
                assert_inside(planet, PLANET_SIZE, bounds);
            }
        }
        let bounds = Rect {
            x: 20.0,
            y: 40.0,
            width: 220.0,
            height: 240.0,
        };
        let layout = orbit_layout(bounds, (0.0, 0.0), 20);
        assert_inside(layout.sun, SUN_SIZE, bounds);
        for planet in layout.planets {
            assert_inside(planet, PLANET_SIZE, bounds);
        }
    }

    #[test]
    fn eighteen_orbits_remain_separated_during_resonance_and_stay_in_bounds() {
        for seconds in [0.0, 15.0, 30.0, 90.0, 120.0, 240.0] {
            let bounds = Rect {
                x: 0.0,
                y: 28.0,
                width: 1440.0,
                height: 872.0,
            };
            let layout = orbit_layout_at(bounds, (620.0, 350.0), 18, seconds);
            for (index, &planet) in layout.planets.iter().enumerate() {
                assert_inside(planet, PLANET_SIZE, bounds);
                let center_distance = (planet.0 + 56.0 - layout.sun.0 - 96.0)
                    .hypot(planet.1 + 56.0 - layout.sun.1 - 96.0);
                assert!(
                    center_distance >= 110.0,
                    "visible sun collision at {seconds}s"
                );
                for &other in &layout.planets[..index] {
                    assert!(
                        separated(planet, PLANET_SIZE, other, PLANET_SIZE),
                        "collision at {seconds}s"
                    );
                }
            }
        }
        assert_eq!(
            orbit_layout_at(screen(), (1100.0, 550.0), 18, 0.0).sun,
            orbit_layout_at(screen(), (1100.0, 550.0), 18, 120.0).sun
        );
    }

    #[test]
    fn larger_rosters_have_distinct_lanes_and_visible_character_spacing() {
        let bounds = Rect {
            x: 0.0,
            y: 28.0,
            width: 1280.0,
            height: 772.0,
        };
        for count in [20, 40] {
            let minimum = planet_art_size(count) * 0.75 + 4.0;
            for seconds in [0.0, 30.0, 60.0, 120.0, 240.0, 480.0] {
                let layout = orbit_layout_at(bounds, (550.0, 320.0), count, seconds);
                for (index, &planet) in layout.planets.iter().enumerate() {
                    assert_inside(planet, PLANET_SIZE, bounds);
                    let sun_distance = (planet.0 + 56.0 - layout.sun.0 - 96.0)
                        .hypot(planet.1 + 56.0 - layout.sun.1 - 96.0);
                    assert!(sun_distance > 36.0 + minimum / 2.0);
                    for &other in &layout.planets[..index] {
                        assert!(
                            (planet.0 - other.0).hypot(planet.1 - other.1) >= minimum,
                            "crowded roster collision for {count} at {seconds}s"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_geometry_never_panics_or_produces_nonfinite_positions() {
        let layout = orbit_layout(
            Rect {
                x: f64::NAN,
                y: f64::INFINITY,
                width: -1.0,
                height: f64::NAN,
            },
            (f64::NAN, f64::INFINITY),
            3,
        );
        assert!(layout.sun.0.is_finite() && layout.sun.1.is_finite());
        assert!(
            layout
                .planets
                .iter()
                .all(|&(x, y)| x.is_finite() && y.is_finite())
        );
    }
}
