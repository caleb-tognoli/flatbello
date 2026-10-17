//! Closed loops and 2D face profiles.

use crate::bbox::Bbox2;
use crate::math::{Frame3, Point2, Vec2};
use crate::model::{FaceId, LengthUnit};
use crate::segment::Edge;
use crate::{GeometryError, Tolerance};

/// Winding direction, as seen from the face normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    CounterClockwise,
    Clockwise,
}

/// An ordered, closed chain of edges. Each edge ends where the next begins, within the linear
/// tolerance, and the last edge ends where the first begins.
#[derive(Clone, Debug)]
pub struct Loop {
    edges: Vec<Edge>,
}

impl Loop {
    /// Validates every segment, the chain's continuity and closure, and that it encloses area.
    pub fn new(edges: Vec<Edge>, tol: &Tolerance) -> Result<Self, GeometryError> {
        if edges.is_empty() {
            return Err(GeometryError::EmptyLoop);
        }
        for (i, e) in edges.iter().enumerate() {
            e.segment
                .validate(tol)
                .map_err(|error| GeometryError::InvalidSegment { edge: i, error })?;
            if e.segment.is_closed_curve() && edges.len() > 1 {
                return Err(GeometryError::ClosedCurveNotAlone { edge: i });
            }
        }
        for (i, e) in edges.iter().enumerate() {
            let next = &edges[(i + 1) % edges.len()];
            let distance = e.segment.end_point().distance(next.segment.start_point());
            if distance > tol.linear() {
                return Err(GeometryError::Gap { edge: i, distance });
            }
        }
        let l = Loop { edges };
        if l.signed_area(tol).abs() <= tol.linear() * tol.linear() {
            return Err(GeometryError::ZeroArea);
        }
        Ok(l)
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The enclosed area: positive when counter-clockwise.
    pub fn signed_area(&self, tol: &Tolerance) -> f64 {
        0.5 * self
            .edges
            .iter()
            .map(|e| e.segment.area_term(tol.fit()))
            .sum::<f64>()
    }

    pub fn orientation(&self, tol: &Tolerance) -> Orientation {
        if self.signed_area(tol) >= 0.0 {
            Orientation::CounterClockwise
        } else {
            Orientation::Clockwise
        }
    }

    /// The loop traversed backwards.
    pub fn reversed(&self) -> Loop {
        Loop {
            edges: self
                .edges
                .iter()
                .rev()
                .map(|e| Edge {
                    segment: e.segment.reversed(),
                    provenance: e.provenance,
                })
                .collect(),
        }
    }

    /// This loop, reversed if needed so that it winds as `orientation`.
    pub fn oriented(self, orientation: Orientation, tol: &Tolerance) -> Loop {
        if self.orientation(tol) == orientation {
            self
        } else {
            self.reversed()
        }
    }

    pub fn translated(&self, v: Vec2) -> Loop {
        Loop {
            edges: self
                .edges
                .iter()
                .map(|e| Edge {
                    segment: e.segment.translated(v),
                    provenance: e.provenance,
                })
                .collect(),
        }
    }

    pub fn bbox(&self, tol: &Tolerance) -> Bbox2 {
        self.edges
            .iter()
            .fold(Bbox2::EMPTY, |b, e| b.union(&e.segment.bbox(tol.fit())))
    }

    /// The loop flattened to a polygon within the fit tolerance, without repeating the first point.
    pub fn to_polygon(&self, tol: &Tolerance) -> Vec<Point2> {
        let mut pts: Vec<Point2> = Vec::new();
        for e in &self.edges {
            let flat = e.segment.flatten(tol.fit());
            let skip = usize::from(!pts.is_empty());
            pts.extend(flat.into_iter().skip(skip));
        }
        if pts.len() > 1 {
            pts.pop();
        }
        pts
    }

    /// True if `p` lies inside the loop's polygon (see [`Loop::to_polygon`]).
    /// Points within the fit tolerance of the boundary may go either way.
    pub fn contains_point(&self, p: Point2, tol: &Tolerance) -> bool {
        point_in_polygon(p, &self.to_polygon(tol))
    }

    /// True if every vertex of `other`'s polygon lies inside this loop.
    pub fn contains_loop(&self, other: &Loop, tol: &Tolerance) -> bool {
        let poly = self.to_polygon(tol);
        other
            .to_polygon(tol)
            .iter()
            .all(|&p| point_in_polygon(p, &poly))
    }
}

/// Even-odd crossing test against a closed polygon.
fn point_in_polygon(p: Point2, poly: &[Point2]) -> bool {
    let mut inside = false;
    for (i, &a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// One face as a 2D profile: an outer loop wound counter-clockwise and holes wound clockwise.
///
/// The 2D coordinates are `frame.to_local_2d(p)` of the world points; translating the profile
/// moves the frame's origin so that this stays true.
#[derive(Clone, Debug)]
pub struct Profile2D {
    outer: Loop,
    holes: Vec<Loop>,
    face: FaceId,
    frame: Frame3,
    units: LengthUnit,
}

impl Profile2D {
    /// Orients the outer loop counter-clockwise and the holes clockwise, and checks that each
    /// hole lies inside the outer loop and outside every other hole.
    pub fn new(
        outer: Loop,
        holes: Vec<Loop>,
        face: FaceId,
        frame: Frame3,
        units: LengthUnit,
        tol: &Tolerance,
    ) -> Result<Self, GeometryError> {
        let outer = outer.oriented(Orientation::CounterClockwise, tol);
        let holes: Vec<Loop> = holes
            .into_iter()
            .map(|h| h.oriented(Orientation::Clockwise, tol))
            .collect();
        for (i, h) in holes.iter().enumerate() {
            if !outer.contains_loop(h, tol) {
                return Err(GeometryError::HoleOutsideOuter { hole: i });
            }
            for (j, other) in holes.iter().enumerate() {
                if i != j && h.contains_loop(other, tol) {
                    return Err(GeometryError::NestedHoles {
                        outer_hole: i,
                        inner_hole: j,
                    });
                }
            }
        }
        Ok(Self {
            outer,
            holes,
            face,
            frame,
            units,
        })
    }

    pub fn outer(&self) -> &Loop {
        &self.outer
    }

    pub fn holes(&self) -> &[Loop] {
        &self.holes
    }

    pub fn face(&self) -> FaceId {
        self.face
    }

    pub fn frame(&self) -> &Frame3 {
        &self.frame
    }

    /// The units of the source model. Profile coordinates are always millimetres.
    pub fn units(&self) -> LengthUnit {
        self.units
    }

    /// The bounding box. Holes lie inside the outer loop, so it is the outer loop's box.
    pub fn bbox(&self, tol: &Tolerance) -> Bbox2 {
        self.outer.bbox(tol)
    }

    pub fn translated(&self, v: Vec2) -> Profile2D {
        Profile2D {
            outer: self.outer.translated(v),
            holes: self.holes.iter().map(|h| h.translated(v)).collect(),
            face: self.face,
            frame: self.frame.shifted_local(v),
            units: self.units,
        }
    }

    /// The profile moved so that its bounding box's minimum corner is at (0, 0).
    pub fn at_origin(&self, tol: &Tolerance) -> Profile2D {
        let min = self.bbox(tol).min;
        self.translated(Point2::ORIGIN - min)
    }
}

/// Places the profiles left to right with `gap` mm between their bounding boxes, every box
/// bottom-aligned at y = 0 and the first starting at x = 0.
pub fn layout_side_by_side(profiles: &[Profile2D], gap: f64, tol: &Tolerance) -> Vec<Profile2D> {
    let mut x = 0.0;
    profiles
        .iter()
        .map(|p| {
            let b = p.bbox(tol);
            let moved = p.translated(Point2::new(x, 0.0) - b.min);
            x += b.width() + gap;
            moved
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::Segment;
    use core::f64::consts::{FRAC_PI_2, PI};

    fn tol() -> Tolerance {
        Tolerance::default()
    }

    fn line(ax: f64, ay: f64, bx: f64, by: f64) -> Edge {
        Edge::exact(Segment::Line {
            a: Point2::new(ax, ay),
            b: Point2::new(bx, by),
        })
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
        Loop::new(
            vec![
                line(x0, y0, x1, y0),
                line(x1, y0, x1, y1),
                line(x1, y1, x0, y1),
                line(x0, y1, x0, y0),
            ],
            &tol(),
        )
        .unwrap()
    }

    fn circle(cx: f64, cy: f64, r: f64) -> Loop {
        Loop::new(
            vec![Edge::exact(Segment::Circle {
                center: Point2::new(cx, cy),
                radius: r,
                ccw: true,
            })],
            &tol(),
        )
        .unwrap()
    }

    fn profile(outer: Loop, holes: Vec<Loop>) -> Result<Profile2D, GeometryError> {
        Profile2D::new(
            outer,
            holes,
            FaceId(0),
            Frame3::WORLD,
            LengthUnit::Millimetre,
            &tol(),
        )
    }

    #[test]
    fn rectangle_area_and_orientation() {
        let r = rect(0.0, 0.0, 4.0, 3.0);
        assert!((r.signed_area(&tol()) - 12.0).abs() < 1e-12);
        assert_eq!(r.orientation(&tol()), Orientation::CounterClockwise);
        let rev = r.reversed();
        assert!((rev.signed_area(&tol()) + 12.0).abs() < 1e-12);
        assert_eq!(rev.orientation(&tol()), Orientation::Clockwise);
    }

    #[test]
    fn rounded_rectangle_area_is_exact() {
        let (w, h, r) = (10.0, 6.0, 1.0);
        let arc = |cx: f64, cy: f64, a0: f64| {
            Edge::exact(Segment::Arc {
                center: Point2::new(cx, cy),
                radius: r,
                start_angle: a0,
                end_angle: a0 + FRAC_PI_2,
                ccw: true,
            })
        };
        let l = Loop::new(
            vec![
                line(r, 0.0, w - r, 0.0),
                arc(w - r, r, -FRAC_PI_2),
                line(w, r, w, h - r),
                arc(w - r, h - r, 0.0),
                line(w - r, h, r, h),
                arc(r, h - r, FRAC_PI_2),
                line(0.0, h - r, 0.0, r),
                arc(r, r, PI),
            ],
            &tol(),
        )
        .unwrap();
        let expected = w * h - (4.0 - PI) * r * r;
        assert!((l.signed_area(&tol()) - expected).abs() < 1e-12);
        let b = l.bbox(&tol());
        assert!(b.min.distance(Point2::ORIGIN) < 1e-12);
        assert!(b.max.distance(Point2::new(w, h)) < 1e-12);
    }

    #[test]
    fn rejects_gaps_and_lonely_violations() {
        let gap = Loop::new(
            vec![
                line(0.0, 0.0, 1.0, 0.0),
                line(1.0, 0.0, 1.0, 1.0),
                line(1.0, 1.0, 0.0, 0.01),
            ],
            &tol(),
        );
        assert!(matches!(gap, Err(GeometryError::Gap { edge: 2, .. })));
        assert!(matches!(
            Loop::new(vec![], &tol()),
            Err(GeometryError::EmptyLoop)
        ));
        let c = Edge::exact(Segment::Circle {
            center: Point2::ORIGIN,
            radius: 1.0,
            ccw: true,
        });
        let not_alone = Loop::new(vec![c, line(1.0, 0.0, 2.0, 0.0)], &tol());
        assert!(matches!(
            not_alone,
            Err(GeometryError::ClosedCurveNotAlone { edge: 0 })
        ));
        let flat = Loop::new(
            vec![line(0.0, 0.0, 1.0, 0.0), line(1.0, 0.0, 0.0, 0.0)],
            &tol(),
        );
        assert!(matches!(flat, Err(GeometryError::ZeroArea)));
    }

    #[test]
    fn containment() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains_point(Point2::new(5.0, 5.0), &tol()));
        assert!(!r.contains_point(Point2::new(11.0, 5.0), &tol()));
        assert!(!r.contains_point(Point2::new(5.0, -0.1), &tol()));
        assert!(r.contains_loop(&circle(5.0, 5.0, 2.0), &tol()));
        assert!(!r.contains_loop(&circle(9.0, 5.0, 2.0), &tol()));
        assert!(circle(0.0, 0.0, 1.0).contains_point(Point2::new(0.6, 0.6), &tol()));
        assert!(!circle(0.0, 0.0, 1.0).contains_point(Point2::new(0.8, 0.8), &tol()));
    }

    #[test]
    fn profile_orients_loops() {
        let p = profile(
            rect(0.0, 0.0, 10.0, 10.0).reversed(),
            vec![circle(5.0, 5.0, 2.0)],
        )
        .unwrap();
        assert_eq!(p.outer().orientation(&tol()), Orientation::CounterClockwise);
        assert_eq!(p.holes()[0].orientation(&tol()), Orientation::Clockwise);
    }

    #[test]
    fn profile_rejects_bad_holes() {
        let outside = profile(rect(0.0, 0.0, 10.0, 10.0), vec![circle(20.0, 5.0, 2.0)]);
        assert!(matches!(
            outside,
            Err(GeometryError::HoleOutsideOuter { hole: 0 })
        ));
        let nested = profile(
            rect(0.0, 0.0, 10.0, 10.0),
            vec![circle(5.0, 5.0, 1.0), circle(5.0, 5.0, 3.0)],
        );
        assert!(matches!(
            nested,
            Err(GeometryError::NestedHoles {
                outer_hole: 1,
                inner_hole: 0
            })
        ));
    }

    #[test]
    fn moves_to_origin_and_keeps_frame_consistent() {
        let p = profile(rect(5.0, -3.0, 10.0, 1.0), vec![]).unwrap();
        let q = p.at_origin(&tol());
        let b = q.bbox(&tol());
        assert!(b.min.distance(Point2::ORIGIN) < 1e-12);
        assert!(b.max.distance(Point2::new(5.0, 4.0)) < 1e-12);
        // World point (5, -3, 0) was the corner; it now maps to (0, 0).
        let local = q
            .frame()
            .to_local_2d(crate::math::Point3::new(5.0, -3.0, 0.0));
        assert!(local.distance(Point2::ORIGIN) < 1e-12);
    }

    #[test]
    fn side_by_side_layout() {
        let a = profile(rect(5.0, 5.0, 9.0, 7.0), vec![]).unwrap();
        let b = profile(rect(-3.0, 2.0, 0.0, 10.0), vec![]).unwrap();
        let laid = layout_side_by_side(&[a, b], 2.0, &tol());
        let b0 = laid[0].bbox(&tol());
        let b1 = laid[1].bbox(&tol());
        assert!(b0.min.distance(Point2::ORIGIN) < 1e-12);
        assert!(b1.min.distance(Point2::new(6.0, 0.0)) < 1e-12);
        assert!(b1.max.distance(Point2::new(9.0, 8.0)) < 1e-12);
    }
}
