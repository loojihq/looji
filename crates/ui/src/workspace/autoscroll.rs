//! Edge auto-scroll, as pure functions: how fast a column scrolls for a pointer near its top or
//! bottom edge, how that speed ramps in and out over time, and how far a scroll position moves.
//! The board's drag loop applies these, so the feel is tested here without a window.
//!
//! Everything is in logical pixels. Physical pixels would feel different on high-DPI displays.

/// How deep, in logical pixels, the edge zone reaches into a column from its top and bottom.
pub const EDGE_ZONE: f32 = 48.0;

/// The fastest scroll, in logical pixels per second, at the very edge of the column.
pub const MAX_SPEED: f32 = 600.0;

/// How long the speed takes to ramp up on entering the zone, or back down on leaving it.
pub const RAMP_SECONDS: f32 = 0.2;

/// The signed scroll speed for a pointer at `pointer_y` over a column spanning `top..bottom`.
/// Negative scrolls towards the top, positive towards the bottom.
///
/// The speed is zero at the inner edge of the zone and largest at the column's own edge. It follows
/// a smoothstep of the depth, so it starts and ends gently. The result is zero outside the column.
pub fn edge_velocity(pointer_y: f32, top: f32, bottom: f32, zone: f32) -> f32 {
    if pointer_y < top || pointer_y > bottom {
        return 0.0;
    }
    let top_depth = (top + zone - pointer_y) / zone;
    if top_depth > 0.0 {
        return -MAX_SPEED * smoothstep(top_depth.min(1.0));
    }
    let bottom_depth = (pointer_y - (bottom - zone)) / zone;
    if bottom_depth > 0.0 {
        return MAX_SPEED * smoothstep(bottom_depth.min(1.0));
    }
    0.0
}

/// How far a scroll position moves on one tick. The position stays within `0..=max`, so the
/// column stops at each end and the speed doesn't push it past.
pub fn advance(scroll: f32, max: f32, delta: f32) -> f32 {
    (scroll + delta).clamp(0.0, max.max(0.0))
}

/// Eases the scroll speed in and out over time. It climbs while the pointer is in a zone and falls
/// back while it isn't, and the value it returns is eased with a smoothstep.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Ramp {
    progress: f32,
}

impl Ramp {
    /// Advances the ramp by `dt` seconds and returns the eased factor from 0 to 1.
    pub fn step(&mut self, inside: bool, dt: f32) -> f32 {
        let change = dt / RAMP_SECONDS;
        self.progress = if inside {
            (self.progress + change).min(1.0)
        } else {
            (self.progress - change).max(0.0)
        };
        smoothstep(self.progress)
    }

    /// Drops the ramp to zero at once, for example when the scrolling column changes.
    pub fn reset(&mut self) {
        self.progress = 0.0;
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{EDGE_ZONE, MAX_SPEED, RAMP_SECONDS, Ramp, advance, edge_velocity};

    // A column from y 100 to y 500.
    const TOP: f32 = 100.0;
    const BOTTOM: f32 = 500.0;

    fn velocity(y: f32) -> f32 {
        edge_velocity(y, TOP, BOTTOM, EDGE_ZONE)
    }

    #[test]
    fn nothing_scrolls_in_the_middle_of_the_column() {
        assert_eq!(velocity(300.0), 0.0);
        assert_eq!(
            velocity(TOP + EDGE_ZONE),
            0.0,
            "the inner edge of the zone is still"
        );
        assert_eq!(velocity(BOTTOM - EDGE_ZONE), 0.0);
    }

    #[test]
    fn nothing_scrolls_outside_the_column() {
        assert_eq!(velocity(TOP - 1.0), 0.0);
        assert_eq!(velocity(BOTTOM + 1.0), 0.0);
    }

    #[test]
    fn the_speed_is_largest_at_the_column_edge_and_zero_at_the_zone_edge() {
        assert_eq!(velocity(TOP), -MAX_SPEED);
        assert_eq!(velocity(BOTTOM), MAX_SPEED);
    }

    #[test]
    fn the_speed_grows_monotonically_deeper_into_the_zone() {
        let top_speeds: Vec<f32> = (0..=10)
            .map(|step| TOP + EDGE_ZONE - step as f32 * EDGE_ZONE / 10.0)
            .map(velocity)
            .collect();
        for pair in top_speeds.windows(2) {
            assert!(pair[0] >= pair[1], "{} then {}", pair[0], pair[1]);
        }
        assert!(top_speeds.first().unwrap().abs() < f32::EPSILON);
    }

    #[test]
    fn the_top_and_bottom_edges_are_symmetric() {
        for depth in [2.0, 10.0, 24.0, 40.0, 47.0] {
            let top = velocity(TOP + depth);
            let bottom = velocity(BOTTOM - depth);
            assert!(top < 0.0 && bottom > 0.0);
            assert!(
                (top + bottom).abs() < 1e-3,
                "depth {depth}: {top} vs {bottom}"
            );
        }
    }

    #[test]
    fn the_curve_eases_in_so_it_is_gentle_at_first() {
        // A quarter of the way into the zone the speed is well under a quarter of the maximum.
        let quarter = velocity(TOP + EDGE_ZONE - EDGE_ZONE / 4.0).abs();
        assert!(quarter < MAX_SPEED * 0.25);
        assert!(quarter > 0.0);
    }

    #[test]
    fn the_ramp_rises_to_one_over_the_ramp_time_and_no_further() {
        let mut ramp = Ramp::default();
        let mut last = 0.0;
        for _ in 0..10 {
            let factor = ramp.step(true, RAMP_SECONDS / 10.0);
            assert!(factor >= last);
            last = factor;
        }
        assert!((last - 1.0).abs() < 1e-6);
        assert_eq!(ramp.step(true, 1.0), 1.0);
    }

    #[test]
    fn the_ramp_falls_back_to_zero_on_exit() {
        let mut ramp = Ramp::default();
        ramp.step(true, 1.0);
        let mut last = 1.0;
        for _ in 0..10 {
            let factor = ramp.step(false, RAMP_SECONDS / 10.0);
            assert!(factor <= last);
            last = factor;
        }
        assert_eq!(last, 0.0);
    }

    #[test]
    fn a_reset_ramp_starts_again_from_zero() {
        let mut ramp = Ramp::default();
        ramp.step(true, 1.0);
        ramp.reset();
        assert_eq!(ramp.step(false, 0.0), 0.0);
    }

    #[test]
    fn a_scroll_stops_at_each_end() {
        assert_eq!(advance(0.0, 300.0, -50.0), 0.0);
        assert_eq!(advance(290.0, 300.0, 50.0), 300.0);
        assert_eq!(advance(100.0, 300.0, 25.0), 125.0);
        assert_eq!(
            advance(0.0, 0.0, 10.0),
            0.0,
            "a column that doesn't overflow can't scroll"
        );
    }
}
