//! Angle helpers. Angles are radians, measured counter-clockwise from +x.

use core::f64::consts::{PI, TAU};

/// `angle` reduced to [0, 2π).
pub fn normalize_angle(angle: f64) -> f64 {
    let r = angle.rem_euclid(TAU);
    // rem_euclid can round up to exactly TAU for tiny negative inputs.
    if r < TAU { r } else { 0.0 }
}

/// `a - b` reduced to [-π, π).
pub fn signed_angle_difference(a: f64, b: f64) -> f64 {
    normalize_angle(a - b + PI) - PI
}

/// The angle swept going from `start` to `end`, counter-clockwise if `ccw`, else clockwise.
///
/// The result is in (0, 2π]: equal angles mean a full turn.
pub fn sweep(start: f64, end: f64, ccw: bool) -> f64 {
    let d = if ccw { end - start } else { start - end };
    let s = normalize_angle(d);
    if s > 0.0 { s } else { TAU }
}

/// True if `angle` lies on the sweep that starts at `start` and turns by `sweep` (ccw or cw).
pub fn angle_in_sweep(angle: f64, start: f64, sweep: f64, ccw: bool) -> bool {
    let offset = if ccw {
        normalize_angle(angle - start)
    } else {
        normalize_angle(start - angle)
    };
    offset <= sweep
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::FRAC_PI_2;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn normalizes_into_range() {
        assert!(close(normalize_angle(-FRAC_PI_2), 1.5 * PI));
        assert!(close(normalize_angle(5.0 * PI), PI));
        assert!(close(normalize_angle(TAU), 0.0));
        let tiny = normalize_angle(-1e-300);
        assert!((0.0..TAU).contains(&tiny));
    }

    #[test]
    fn signed_difference() {
        assert!(close(signed_angle_difference(0.1, TAU - 0.1), 0.2));
        assert!(close(signed_angle_difference(TAU - 0.1, 0.1), -0.2));
    }

    #[test]
    fn sweep_directions() {
        assert!(close(sweep(0.0, FRAC_PI_2, true), FRAC_PI_2));
        assert!(close(sweep(0.0, FRAC_PI_2, false), 1.5 * PI));
        assert!(close(sweep(-FRAC_PI_2, 0.0, true), FRAC_PI_2));
        assert!(close(sweep(1.0, 1.0, true), TAU));
    }

    #[test]
    fn membership() {
        assert!(angle_in_sweep(0.0, -FRAC_PI_2, FRAC_PI_2, true));
        assert!(!angle_in_sweep(PI, -FRAC_PI_2, FRAC_PI_2, true));
        assert!(angle_in_sweep(PI, FRAC_PI_2 * 3.0, PI, false));
        assert!(!angle_in_sweep(0.0, FRAC_PI_2 * 3.0, PI, false));
    }
}
