//! Deterministic, sun-relative desktop positions in logical screen coordinates.

use std::f64::consts::{FRAC_PI_2, TAU};

const SUN_SIZE: f64 = 192.0;
const PLANET_SIZE: f64 = 112.0;
const EDGE_PADDING: f64 = 8.0;
const FIRST_ORBIT_RADIUS: f64 = 200.0;
const INNER_ORBIT_RATIO: f64 = 0.22 / 0.42;
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
/// Every Sprite has its own radius. Repeating an expansion for the same ordered
/// roster produces the same positions. Move the formation as a unit at screen
/// edges; never clamp planets separately. Complete paths stay in bounds as they
/// rotate. Tiny displays and very large rosters necessarily get denser.
pub(crate) fn orbit_layout(bounds: Rect, sun: (f64, f64), count: usize) -> Layout {
    orbit_layout_at(bounds, sun, count, 0.0)
}

/// Unique concentric paths share a 1:1 resonance, matching the launcher scene.
/// Golden-angle phases and equal-area radii preserve separation as the entire
/// formation turns. Reserve the complete outer path at the screen edge so
/// moving Sprites never shove the sun around.
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

    // Increase the available envelope with team size without sharing a path.
    let mut remaining = count;
    let mut envelope_steps = 0;
    while remaining > 0 {
        envelope_steps += 1;
        remaining = remaining.saturating_sub(6 * envelope_steps);
    }
    let radius = FIRST_ORBIT_RADIUS * envelope_steps as f64;
    let elapsed_seconds = if elapsed_seconds.is_finite() {
        elapsed_seconds.max(0.0)
    } else {
        0.0
    };
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    let travel = elapsed_seconds / ORBIT_PERIOD_SECONDS * TAU;
    let offsets: Vec<_> = (0..count)
        .map(|index| {
            let fraction = if count > 1 {
                index as f64 / (count - 1) as f64
            } else {
                1.0
            };
            let orbit_radius = radius
                * (INNER_ORBIT_RATIO.powi(2) + (1.0 - INNER_ORBIT_RATIO.powi(2)) * fraction).sqrt();
            let angle = -FRAC_PI_2 + index as f64 * golden_angle + travel;
            (angle.cos() * orbit_radius, angle.sin() * orbit_radius)
        })
        .collect();
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

    fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
        (a.0 - b.0).hypot(a.1 - b.1)
    }

    #[test]
    fn repeated_expansions_have_identical_distinct_paths_and_encircle_the_sun() {
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
        let radii: Vec<_> = centers
            .iter()
            .map(|&point| distance(point, center))
            .collect();
        assert!(radii.windows(2).all(|pair| pair[1] > pair[0]));
    }

    #[test]
    fn each_sprite_owns_a_separate_radius_throughout_motion() {
        for count in [1, 3, 6, 18, 40, 64] {
            let first = orbit_layout_at(screen(), (1100.0, 550.0), count, 0.0);
            for seconds in [0.0, 15.0, 30.0, 90.0, 120.0, 240.0] {
                let layout = orbit_layout_at(screen(), (1100.0, 550.0), count, seconds);
                assert_eq!(layout.sun, first.sun);
                let center = (layout.sun.0 + 96.0, layout.sun.1 + 96.0);
                let mut previous_radius = 0.0;
                for (&planet, &initial) in layout.planets.iter().zip(&first.planets) {
                    assert_inside(planet, PLANET_SIZE, screen());
                    let radius = distance((planet.0 + 56.0, planet.1 + 56.0), center);
                    let initial_radius = distance((initial.0 + 56.0, initial.1 + 56.0), center);
                    assert!(radius > previous_radius + 1e-6);
                    assert!((radius - initial_radius).abs() < 1e-6);
                    previous_radius = radius;
                }
            }
        }
    }

    #[test]
    fn ordinary_rosters_have_clear_visible_characters_and_clear_the_sun() {
        for count in 1..=18 {
            let layout = orbit_layout(screen(), (1100.0, 550.0), count);
            for (index, &planet) in layout.planets.iter().enumerate() {
                assert_inside(planet, PLANET_SIZE, screen());
                // Transparent native-window padding may overlap. The actual
                // pixel bodies (including hover growth) must remain separate.
                let center = (planet.0 + 56.0, planet.1 + 56.0);
                let sun_center = (layout.sun.0 + 96.0, layout.sun.1 + 96.0);
                assert!(
                    distance(center, sun_center) > 40.0 + planet_art_size(count) * 0.75 * 1.1 / 2.0
                );
                for &other in &layout.planets[..index] {
                    assert!(distance(planet, other) > planet_art_size(count) * 0.75 * 1.1 + 4.0);
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
    fn moving_sprites_stay_visible_and_separated_on_a_laptop() {
        let bounds = Rect {
            x: 0.0,
            y: 28.0,
            width: 1280.0,
            height: 772.0,
        };
        for count in [1, 3, 6, 18, 20, 40] {
            let minimum = planet_art_size(count) * 0.75 * 1.1 + 4.0;
            let initial = orbit_layout_at(bounds, (550.0, 320.0), count, 0.0);
            for second in 0..=120 {
                let layout = orbit_layout_at(bounds, (550.0, 320.0), count, second as f64);
                assert_eq!(layout.sun, initial.sun);
                for (index, &planet) in layout.planets.iter().enumerate() {
                    assert_inside(planet, PLANET_SIZE, bounds);
                    let center = (planet.0 + 56.0, planet.1 + 56.0);
                    let sun_center = (layout.sun.0 + 96.0, layout.sun.1 + 96.0);
                    assert!(distance(center, sun_center) > 40.0 + minimum / 2.0);
                    for &other in &layout.planets[..index] {
                        assert!(
                            distance(planet, other) >= minimum,
                            "visible collision for {count} at {second}s"
                        );
                    }
                }
            }
            let repeated = orbit_layout_at(bounds, (550.0, 320.0), count, ORBIT_PERIOD_SECONDS);
            for (&first, &last) in initial.planets.iter().zip(&repeated.planets) {
                assert!(distance(first, last) < 1e-6);
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
