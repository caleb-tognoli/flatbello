//! Profile segments: lines, arcs, circles, ellipses, splines and polylines.

use core::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::angle::{angle_in_sweep, sweep};
use crate::bbox::Bbox2;
use crate::math::{Point2, Vec2};
use crate::spline::Spline;
use crate::{SegmentError, Tolerance};

/// Upper bound on the points produced when flattening one segment.
const MAX_PIECES: usize = 100_000;

/// One piece of a loop. Angles are radians, counter-clockwise from +x.
///
/// Every segment has a direction, so that a loop can be walked from edge to edge.
#[derive(Clone, Debug)]
pub enum Segment {
    Line {
        a: Point2,
        b: Point2,
    },
    /// The arc from `start_angle` to `end_angle`, turning counter-clockwise if `ccw`.
    Arc {
        center: Point2,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        ccw: bool,
    },
    /// A full circle starting and ending at angle 0, turning counter-clockwise if `ccw`.
    Circle {
        center: Point2,
        radius: f64,
        ccw: bool,
    },
    /// An elliptical arc `center + major·cos t + minor·sin t`, where `minor` is `major` turned by
    /// +90° and scaled by `ratio` (0 < ratio ≤ 1). `start` and `end` are values of the parameter `t`.
    Ellipse {
        center: Point2,
        major_axis: Vec2,
        ratio: f64,
        start: f64,
        end: f64,
        ccw: bool,
    },
    Spline(Spline),
    /// A fallback chain of straight pieces.
    Polyline {
        points: Vec<Point2>,
    },
}

/// Where a segment came from.
#[derive(Clone, Copy, Debug)]
pub enum Provenance {
    /// Exact geometry from the source model.
    Exact,
    /// Fitted to sampled data, deviating from it by at most `max_deviation` mm.
    Fitted { max_deviation: f64 },
}

/// A segment of a loop, with its provenance.
#[derive(Clone, Debug)]
pub struct Edge {
    pub segment: Segment,
    pub provenance: Provenance,
}

impl Edge {
    pub fn exact(segment: Segment) -> Self {
        Self {
            segment,
            provenance: Provenance::Exact,
        }
    }

    pub fn fitted(segment: Segment, max_deviation: f64) -> Self {
        Self {
            segment,
            provenance: Provenance::Fitted { max_deviation },
        }
    }
}

/// The point at `angle` on a circle.
pub fn circle_point(center: Point2, radius: f64, angle: f64) -> Point2 {
    center + Vec2::from_angle(angle) * radius
}

/// The point at parameter `t` on the ellipse described in [`Segment::Ellipse`].
pub fn ellipse_point(center: Point2, major: Vec2, ratio: f64, t: f64) -> Point2 {
    let (s, c) = t.sin_cos();
    center + major * c + major.perp() * (ratio * s)
}

/// The number of equal steps that keep the chords of a circular arc within `fit` of it.
fn pieces_for(radius: f64, sweep: f64, fit: f64) -> usize {
    let max_step = 2.0 * (1.0 - fit / radius).clamp(-1.0, 1.0).acos();
    let by_error = if max_step > 0.0 {
        (sweep / max_step).ceil()
    } else {
        MAX_PIECES as f64
    };
    // At least one piece per quarter turn keeps the polygon's shape.
    let n = by_error.max((sweep / FRAC_PI_2).ceil()).max(1.0);
    (n as usize).min(MAX_PIECES)
}

fn signed(sweep: f64, ccw: bool) -> f64 {
    if ccw { sweep } else { -sweep }
}

impl Segment {
    /// Checks that the values are finite and the segment is not degenerate.
    pub fn validate(&self, tol: &Tolerance) -> Result<(), SegmentError> {
        let lin = tol.linear();
        match self {
            Segment::Line { a, b } => {
                if !a.is_finite() || !b.is_finite() {
                    Err(SegmentError::NonFinite)
                } else if a.distance(*b) <= lin {
                    Err(SegmentError::Degenerate)
                } else {
                    Ok(())
                }
            }
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                if !center.is_finite()
                    || !radius.is_finite()
                    || !start_angle.is_finite()
                    || !end_angle.is_finite()
                {
                    return Err(SegmentError::NonFinite);
                }
                let s = sweep(*start_angle, *end_angle, *ccw);
                // A full turn must be a Circle, so both the arc and its complement must have length.
                if *radius <= lin || s * radius <= lin || (TAU - s) * radius <= lin {
                    Err(SegmentError::Degenerate)
                } else {
                    Ok(())
                }
            }
            Segment::Circle { center, radius, .. } => {
                if !center.is_finite() || !radius.is_finite() {
                    Err(SegmentError::NonFinite)
                } else if *radius <= lin {
                    Err(SegmentError::Degenerate)
                } else {
                    Ok(())
                }
            }
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } => {
                if !center.is_finite()
                    || !major_axis.is_finite()
                    || !ratio.is_finite()
                    || !start.is_finite()
                    || !end.is_finite()
                {
                    return Err(SegmentError::NonFinite);
                }
                let minor = major_axis.length() * ratio;
                if *ratio > 1.0 || minor <= lin || sweep(*start, *end, *ccw) * minor <= lin {
                    Err(SegmentError::Degenerate)
                } else {
                    Ok(())
                }
            }
            Segment::Spline(s) => {
                s.validate()?;
                let pts = s.flatten(tol.fit());
                if polyline_length(&pts) <= lin {
                    Err(SegmentError::Degenerate)
                } else {
                    Ok(())
                }
            }
            Segment::Polyline { points } => {
                if !points.iter().all(|p| p.is_finite()) {
                    Err(SegmentError::NonFinite)
                } else if points.len() < 2 || polyline_length(points) <= lin {
                    Err(SegmentError::Degenerate)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// True for segments that close on themselves and so must be alone in a loop.
    pub fn is_closed_curve(&self) -> bool {
        matches!(self, Segment::Circle { .. })
    }

    pub fn start_point(&self) -> Point2 {
        match self {
            Segment::Line { a, .. } => *a,
            Segment::Arc {
                center,
                radius,
                start_angle,
                ..
            } => circle_point(*center, *radius, *start_angle),
            Segment::Circle { center, radius, .. } => circle_point(*center, *radius, 0.0),
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                ..
            } => ellipse_point(*center, *major_axis, *ratio, *start),
            Segment::Spline(s) => s.start_point(),
            Segment::Polyline { points } => points.first().copied().unwrap_or(Point2::ORIGIN),
        }
    }

    pub fn end_point(&self) -> Point2 {
        match self {
            Segment::Line { b, .. } => *b,
            Segment::Arc {
                center,
                radius,
                end_angle,
                ..
            } => circle_point(*center, *radius, *end_angle),
            Segment::Circle { center, radius, .. } => circle_point(*center, *radius, 0.0),
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                end,
                ..
            } => ellipse_point(*center, *major_axis, *ratio, *end),
            Segment::Spline(s) => s.end_point(),
            Segment::Polyline { points } => points.last().copied().unwrap_or(Point2::ORIGIN),
        }
    }

    /// The same geometry traversed in the opposite direction.
    pub fn reversed(&self) -> Segment {
        match self {
            Segment::Line { a, b } => Segment::Line { a: *b, b: *a },
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => Segment::Arc {
                center: *center,
                radius: *radius,
                start_angle: *end_angle,
                end_angle: *start_angle,
                ccw: !ccw,
            },
            Segment::Circle {
                center,
                radius,
                ccw,
            } => Segment::Circle {
                center: *center,
                radius: *radius,
                ccw: !ccw,
            },
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } => Segment::Ellipse {
                center: *center,
                major_axis: *major_axis,
                ratio: *ratio,
                start: *end,
                end: *start,
                ccw: !ccw,
            },
            Segment::Spline(s) => Segment::Spline(s.reversed()),
            Segment::Polyline { points } => Segment::Polyline {
                points: points.iter().rev().copied().collect(),
            },
        }
    }

    pub fn translated(&self, v: Vec2) -> Segment {
        let mut s = self.clone();
        match &mut s {
            Segment::Line { a, b } => {
                *a = *a + v;
                *b = *b + v;
            }
            Segment::Arc { center, .. }
            | Segment::Circle { center, .. }
            | Segment::Ellipse { center, .. } => *center = *center + v,
            Segment::Spline(sp) => *sp = sp.translated(v.x, v.y),
            Segment::Polyline { points } => {
                for p in points {
                    *p = *p + v;
                }
            }
        }
        s
    }

    /// Points along the segment, start and end included, whose polyline stays within `fit` of it.
    pub fn flatten(&self, fit: f64) -> Vec<Point2> {
        match self {
            Segment::Line { a, b } => vec![*a, *b],
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                let s = signed(sweep(*start_angle, *end_angle, *ccw), *ccw);
                let n = pieces_for(*radius, s.abs(), fit);
                (0..=n)
                    .map(|i| circle_point(*center, *radius, start_angle + s * i as f64 / n as f64))
                    .collect()
            }
            Segment::Circle {
                center,
                radius,
                ccw,
            } => {
                let s = signed(TAU, *ccw);
                let n = pieces_for(*radius, TAU, fit);
                (0..=n)
                    .map(|i| circle_point(*center, *radius, s * i as f64 / n as f64))
                    .collect()
            }
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } => {
                let s = signed(sweep(*start, *end, *ccw), *ccw);
                // An ellipse is a circle of the major radius squashed along one axis, which
                // cannot increase chord error, so the circle's step count is enough.
                let n = pieces_for(major_axis.length(), s.abs(), fit);
                (0..=n)
                    .map(|i| {
                        ellipse_point(
                            *center,
                            *major_axis,
                            *ratio,
                            start + s * i as f64 / n as f64,
                        )
                    })
                    .collect()
            }
            Segment::Spline(sp) => sp.flatten(fit),
            Segment::Polyline { points } => points.clone(),
        }
    }

    /// This segment's term in ∮(x dy − y dx), which is twice the signed area of a closed loop.
    ///
    /// Exact for lines, arcs, circles, ellipses and polylines; splines use their flattening.
    pub fn area_term(&self, fit: f64) -> f64 {
        match self {
            Segment::Line { a, b } => a.to_vec().cross(b.to_vec()),
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                let d = self.end_point() - self.start_point();
                let s = signed(sweep(*start_angle, *end_angle, *ccw), *ccw);
                center.to_vec().cross(d) + radius * radius * s
            }
            Segment::Circle { radius, ccw, .. } => radius * radius * signed(TAU, *ccw),
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } => {
                let d = self.end_point() - self.start_point();
                let s = signed(sweep(*start, *end, *ccw), *ccw);
                let minor = major_axis.perp() * *ratio;
                center.to_vec().cross(d) + major_axis.cross(minor) * s
            }
            Segment::Spline(sp) => shoelace(&sp.flatten(fit)),
            Segment::Polyline { points } => shoelace(points),
        }
    }

    /// The bounding box: exact except for splines, which use their flattening and may fall
    /// short by up to `fit`.
    pub fn bbox(&self, fit: f64) -> Bbox2 {
        let mut b = Bbox2::EMPTY;
        match self {
            Segment::Line { a, b: end } => {
                b.include(*a);
                b.include(*end);
            }
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                b.include(self.start_point());
                b.include(self.end_point());
                let s = sweep(*start_angle, *end_angle, *ccw);
                for k in 0..4 {
                    let axis = k as f64 * FRAC_PI_2;
                    if angle_in_sweep(axis, *start_angle, s, *ccw) {
                        b.include(circle_point(*center, *radius, axis));
                    }
                }
            }
            Segment::Circle { center, radius, .. } => {
                b.include(*center - Vec2::new(*radius, *radius));
                b.include(*center + Vec2::new(*radius, *radius));
            }
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } => {
                b.include(self.start_point());
                b.include(self.end_point());
                let s = sweep(*start, *end, *ccw);
                let minor = major_axis.perp() * *ratio;
                // x(t) and y(t) are extreme where their derivatives vanish: tan t = minor / major.
                let tx = minor.x.atan2(major_axis.x);
                let ty = minor.y.atan2(major_axis.y);
                for t in [tx, tx + PI, ty, ty + PI] {
                    if angle_in_sweep(t, *start, s, *ccw) {
                        b.include(ellipse_point(*center, *major_axis, *ratio, t));
                    }
                }
            }
            Segment::Polyline { points } => points.iter().for_each(|p| b.include(*p)),
            Segment::Spline(_) => self.flatten(fit).iter().for_each(|p| b.include(*p)),
        }
        b
    }
}

/// ∮(x dy − y dx) along an open chain of points, without closing it.
fn shoelace(points: &[Point2]) -> f64 {
    points
        .windows(2)
        .map(|w| w[0].to_vec().cross(w[1].to_vec()))
        .sum()
}

fn polyline_length(points: &[Point2]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn closep(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-9
    }

    fn quarter_arc(ccw: bool) -> Segment {
        // From (1,0) to (0,1) around the origin, ccw; or the long way round, cw.
        Segment::Arc {
            center: Point2::ORIGIN,
            radius: 1.0,
            start_angle: 0.0,
            end_angle: FRAC_PI_2,
            ccw,
        }
    }

    #[test]
    fn arc_end_points() {
        let a = quarter_arc(true);
        assert!(closep(a.start_point(), Point2::new(1.0, 0.0)));
        assert!(closep(a.end_point(), Point2::new(0.0, 1.0)));
    }

    #[test]
    fn reversal_keeps_geometry() {
        let tol = Tolerance::default();
        for seg in [
            Segment::Line {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(2.0, 1.0),
            },
            quarter_arc(true),
            quarter_arc(false),
            Segment::Ellipse {
                center: Point2::new(1.0, 1.0),
                major_axis: Vec2::new(2.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                end: 2.0,
                ccw: true,
            },
            Segment::Polyline {
                points: vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                    Point2::new(1.0, 1.0),
                ],
            },
        ] {
            assert!(seg.validate(&tol).is_ok());
            let r = seg.reversed();
            assert!(closep(r.start_point(), seg.end_point()));
            assert!(closep(r.end_point(), seg.start_point()));
            assert!(close(r.area_term(1e-6), -seg.area_term(1e-6)));
            let (b1, b2) = (seg.bbox(1e-6), r.bbox(1e-6));
            assert!(closep(b1.min, b2.min) && closep(b1.max, b2.max));
        }
    }

    #[test]
    fn arc_area_term_matches_flattening() {
        for seg in [quarter_arc(true), quarter_arc(false)] {
            let exact = seg.area_term(1e-9);
            let approx = shoelace(&seg.flatten(1e-9));
            assert!((exact - approx).abs() < 1e-6);
        }
    }

    #[test]
    fn ellipse_area_term_matches_flattening() {
        let seg = Segment::Ellipse {
            center: Point2::new(3.0, -1.0),
            major_axis: Vec2::new(3.0, 4.0),
            ratio: 0.4,
            start: 0.5,
            end: 4.0,
            ccw: false,
        };
        let exact = seg.area_term(1e-9);
        let approx = shoelace(&seg.flatten(1e-9));
        assert!((exact - approx).abs() < 1e-5);
    }

    #[test]
    fn circle_area_term() {
        let c = Segment::Circle {
            center: Point2::new(5.0, 5.0),
            radius: 2.0,
            ccw: true,
        };
        assert!(close(c.area_term(0.01), 2.0 * PI * 4.0));
        assert!(close(c.reversed().area_term(0.01), -2.0 * PI * 4.0));
    }

    #[test]
    fn arc_bbox_includes_axis_extremes() {
        // Clockwise from 0 to π/2 is the long way round, through -y and -x.
        let b = quarter_arc(false).bbox(0.01);
        assert!(closep(b.min, Point2::new(-1.0, -1.0)));
        assert!(closep(b.max, Point2::new(1.0, 1.0)));
        let b = quarter_arc(true).bbox(0.01);
        assert!(closep(b.min, Point2::new(0.0, 0.0)));
        assert!(closep(b.max, Point2::new(1.0, 1.0)));
    }

    #[test]
    fn flattened_arc_stays_within_fit() {
        let fit = 0.01;
        let seg = Segment::Arc {
            center: Point2::new(2.0, 3.0),
            radius: 50.0,
            start_angle: 0.3,
            end_angle: 2.9,
            ccw: true,
        };
        let pts = seg.flatten(fit);
        assert!(closep(pts[0], seg.start_point()));
        assert!(closep(*pts.last().unwrap(), seg.end_point()));
        for w in pts.windows(2) {
            let mid = w[0].lerp(w[1], 0.5);
            assert!(50.0 - mid.distance(Point2::new(2.0, 3.0)) <= fit);
        }
    }

    #[test]
    fn validation() {
        let tol = Tolerance::default();
        let bad = [
            Segment::Line {
                a: Point2::ORIGIN,
                b: Point2::new(0.0005, 0.0),
            },
            Segment::Line {
                a: Point2::ORIGIN,
                b: Point2::new(f64::NAN, 0.0),
            },
            Segment::Arc {
                center: Point2::ORIGIN,
                radius: 1.0,
                start_angle: 0.0,
                end_angle: TAU,
                ccw: true,
            },
            Segment::Circle {
                center: Point2::ORIGIN,
                radius: 0.0,
                ccw: true,
            },
            Segment::Ellipse {
                center: Point2::ORIGIN,
                major_axis: Vec2::new(1.0, 0.0),
                ratio: 1.5,
                start: 0.0,
                end: 1.0,
                ccw: true,
            },
            Segment::Polyline {
                points: vec![Point2::ORIGIN],
            },
        ];
        for seg in bad {
            assert!(seg.validate(&tol).is_err(), "{seg:?}");
        }
        assert!(matches!(
            Segment::Arc {
                center: Point2::ORIGIN,
                radius: 1.0,
                start_angle: f64::INFINITY,
                end_angle: 0.0,
                ccw: true,
            }
            .validate(&tol),
            Err(SegmentError::NonFinite)
        ));
    }

    #[test]
    fn ellipse_bbox_is_exact() {
        // Major (3, 4), minor (-1.6, 1.2): half-extents are √(3² + 1.6²) and √(4² + 1.2²).
        let full = Segment::Ellipse {
            center: Point2::new(1.0, 2.0),
            major_axis: Vec2::new(3.0, 4.0),
            ratio: 0.4,
            start: 0.0,
            end: TAU,
            ccw: true,
        };
        let b = full.bbox(1.0);
        let (hx, hy) = (11.56_f64.sqrt(), 17.44_f64.sqrt());
        assert!(closep(b.min, Point2::new(1.0 - hx, 2.0 - hy)));
        assert!(closep(b.max, Point2::new(1.0 + hx, 2.0 + hy)));
        // Half an axis-aligned ellipse, from +x round through +y to −x.
        let half = Segment::Ellipse {
            center: Point2::ORIGIN,
            major_axis: Vec2::new(2.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            end: PI,
            ccw: true,
        };
        let b = half.bbox(1.0);
        assert!(closep(b.min, Point2::new(-2.0, 0.0)));
        assert!(closep(b.max, Point2::new(2.0, 1.0)));
    }

    #[test]
    fn translation() {
        let v = Vec2::new(1.0, -2.0);
        let seg = quarter_arc(true).translated(v);
        assert!(closep(seg.start_point(), Point2::new(2.0, -2.0)));
        let sp = Segment::Spline(Spline {
            degree: 1,
            knots: vec![0.0, 0.0, 1.0, 1.0],
            control_points: vec![Point2::ORIGIN, Point2::new(1.0, 0.0)],
            weights: vec![],
        })
        .translated(v);
        assert!(closep(sp.end_point(), Point2::new(2.0, -2.0)));
    }
}
