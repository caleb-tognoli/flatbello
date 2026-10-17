//! Hand-built profiles with known geometry, for demos and golden-file tests.

use core::f64::consts::{FRAC_PI_2, PI};

use crate::math::{Frame3, Point2};
use crate::model::{FaceId, LengthUnit};
use crate::profile::{Loop, Profile2D};
use crate::segment::{Edge, Segment};
use crate::{GeometryError, Tolerance};

fn line(a: (f64, f64), b: (f64, f64)) -> Edge {
    Edge::exact(Segment::Line {
        a: Point2::new(a.0, a.1),
        b: Point2::new(b.0, b.1),
    })
}

fn arc(center: (f64, f64), radius: f64, start_angle: f64, end_angle: f64) -> Edge {
    Edge::exact(Segment::Arc {
        center: Point2::new(center.0, center.1),
        radius,
        start_angle,
        end_angle,
        ccw: true,
    })
}

fn circle(center: (f64, f64), radius: f64) -> Edge {
    Edge::exact(Segment::Circle {
        center: Point2::new(center.0, center.1),
        radius,
        ccw: true,
    })
}

/// A 100 × 60 mm plate with 5 mm corner radii, a Ø10 hole at (20, 30) and a Ø6 hole at (80, 30).
pub fn rounded_plate() -> Result<Profile2D, GeometryError> {
    let tol = Tolerance::default();
    let (w, h, r) = (100.0, 60.0, 5.0);
    let outer = Loop::new(
        vec![
            line((r, 0.0), (w - r, 0.0)),
            arc((w - r, r), r, -FRAC_PI_2, 0.0),
            line((w, r), (w, h - r)),
            arc((w - r, h - r), r, 0.0, FRAC_PI_2),
            line((w - r, h), (r, h)),
            arc((r, h - r), r, FRAC_PI_2, PI),
            line((0.0, h - r), (0.0, r)),
            arc((r, r), r, PI, 1.5 * PI),
        ],
        &tol,
    )?;
    let holes = vec![
        Loop::new(vec![circle((20.0, 30.0), 5.0)], &tol)?,
        Loop::new(vec![circle((80.0, 30.0), 3.0)], &tol)?,
    ];
    Profile2D::new(
        outer,
        holes,
        FaceId(0),
        Frame3::WORLD,
        LengthUnit::Millimetre,
        &tol,
    )
}

/// An 80 × 40 mm plate with an 8 mm wide slot whose arc centres are (25, 20) and (55, 20).
///
/// The slot is built counter-clockwise and reversed by [`Profile2D::new`], so its arcs run
/// clockwise.
pub fn slotted_plate() -> Result<Profile2D, GeometryError> {
    let tol = Tolerance::default();
    let outer = Loop::new(
        vec![
            line((0.0, 0.0), (80.0, 0.0)),
            line((80.0, 0.0), (80.0, 40.0)),
            line((80.0, 40.0), (0.0, 40.0)),
            line((0.0, 40.0), (0.0, 0.0)),
        ],
        &tol,
    )?;
    let slot = Loop::new(
        vec![
            line((25.0, 16.0), (55.0, 16.0)),
            arc((55.0, 20.0), 4.0, -FRAC_PI_2, FRAC_PI_2),
            line((55.0, 24.0), (25.0, 24.0)),
            arc((25.0, 20.0), 4.0, FRAC_PI_2, 1.5 * PI),
        ],
        &tol,
    )?;
    Profile2D::new(
        outer,
        vec![slot],
        FaceId(1),
        Frame3::WORLD,
        LengthUnit::Millimetre,
        &tol,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Orientation;

    #[test]
    fn samples_build_with_expected_areas() {
        let tol = Tolerance::default();
        let p = rounded_plate().unwrap();
        let expected = 100.0 * 60.0 - (4.0 - PI) * 25.0;
        assert!((p.outer().signed_area(&tol) - expected).abs() < 1e-9);
        assert_eq!(p.holes().len(), 2);
        for h in p.holes() {
            assert_eq!(h.orientation(&tol), Orientation::Clockwise);
        }

        let s = slotted_plate().unwrap();
        let slot_area = 30.0 * 8.0 + PI * 16.0;
        assert!((s.holes()[0].signed_area(&tol) + slot_area).abs() < 1e-9);
        let arcs_cw = s.holes()[0]
            .edges()
            .iter()
            .filter(|e| matches!(e.segment, Segment::Arc { ccw: false, .. }))
            .count();
        assert_eq!(arcs_cw, 2);
    }
}
