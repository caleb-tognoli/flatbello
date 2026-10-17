//! Exact profiles from planar B-rep faces.
//!
//! The face's frame has z along the outward normal, so the profile is what you see looking at the
//! face from outside, never mirrored. The x axis follows [`XAxisRule`]. The finished profile is
//! moved so that its bounding box starts at (0, 0).

use core::f64::consts::TAU;
use core::fmt;

use flatbello_core::{
    BrepEdge, BrepFace, BrepPlane, BrepWire, Curve3, Edge, FaceId, Frame3, GeometryError,
    LengthUnit, Loop, Orientation, Profile2D, Segment, SegmentError, Spline, Tolerance, Vec2, Vec3,
};

/// How the profile's x axis is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XAxisRule {
    /// Along the longest straight edge of the outer loop, in its counter-clockwise direction.
    /// Falls back to the plane's x direction when the outer loop has no straight edge.
    #[default]
    LongestStraightEdge,
    /// The plane's own x direction.
    PlaneX,
}

#[derive(Clone, Debug, Default)]
pub struct ExtractOptions {
    pub tolerance: Tolerance,
    pub x_axis: XAxisRule,
    /// Mesh faces only: round circular holes to the nearest standard drill size when they are
    /// within [`crate::mesh::SNAP_DISTANCE`] of one. Off by default.
    pub snap_hole_sizes: bool,
}

#[derive(Clone, Debug)]
pub enum ExtractError {
    NotPlanar(FaceId),
    NoWires(FaceId),
    DegenerateFrame(FaceId),
    Geometry { face: FaceId, error: GeometryError },
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPlanar(id) => write!(f, "face {} is not planar", id.0),
            Self::NoWires(id) => write!(f, "face {} has no boundary", id.0),
            Self::DegenerateFrame(id) => write!(f, "face {} has a degenerate plane", id.0),
            Self::Geometry { face, error } => write!(f, "face {}: {error}", face.0),
        }
    }
}

impl std::error::Error for ExtractError {}

/// Extracts the exact 2D profile of a planar face.
pub fn extract_planar(
    face: &BrepFace,
    units: LengthUnit,
    options: &ExtractOptions,
) -> Result<Profile2D, ExtractError> {
    let tol = &options.tolerance;
    let id = face.id;
    let plane = face.plane.ok_or(ExtractError::NotPlanar(id))?;
    if face.wires.is_empty() {
        return Err(ExtractError::NoWires(id));
    }
    let geom = |error| ExtractError::Geometry { face: id, error };

    let plane_frame = plane_frame(&plane, tol).ok_or(ExtractError::DegenerateFrame(id))?;
    let frame = match options.x_axis {
        XAxisRule::PlaneX => plane_frame,
        XAxisRule::LongestStraightEdge => {
            let (outer, _) = map_wires(&face.wires, &plane_frame, tol).map_err(geom)?;
            let outer = outer.oriented(Orientation::CounterClockwise, tol);
            match longest_line_direction(&outer, tol) {
                Some(d) => {
                    let hint = plane_frame.x_axis() * d.x + plane_frame.y_axis() * d.y;
                    Frame3::from_normal(plane.origin, plane.normal, hint, tol)
                        .ok_or(ExtractError::DegenerateFrame(id))?
                }
                None => plane_frame,
            }
        }
    };
    let (outer, holes) = map_wires(&face.wires, &frame, tol).map_err(geom)?;
    let profile = Profile2D::new(outer, holes, id, frame, units, tol).map_err(geom)?;
    Ok(profile.at_origin(tol))
}

/// The plane's frame, using any perpendicular direction if its x direction is unusable.
fn plane_frame(plane: &BrepPlane, tol: &Tolerance) -> Option<Frame3> {
    Frame3::from_normal(plane.origin, plane.normal, plane.x_dir, tol).or_else(|| {
        let n = plane.normal.normalized()?;
        let other = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        Frame3::from_normal(plane.origin, n, other, tol)
    })
}

/// Maps every wire into `frame`. The outer loop is the wire flagged as outer, or else the one
/// enclosing the largest area.
fn map_wires(
    wires: &[BrepWire],
    frame: &Frame3,
    tol: &Tolerance,
) -> Result<(Loop, Vec<Loop>), GeometryError> {
    let mut loops = wires
        .iter()
        .map(|w| map_wire(w, frame, tol))
        .collect::<Result<Vec<Loop>, _>>()?;
    let outer_index = wires.iter().position(|w| w.is_outer).unwrap_or_else(|| {
        let mut best = 0;
        for (i, l) in loops.iter().enumerate() {
            if l.signed_area(tol).abs() > loops[best].signed_area(tol).abs() {
                best = i;
            }
        }
        best
    });
    let outer = loops.remove(outer_index);
    Ok((outer, loops))
}

fn map_wire(wire: &BrepWire, frame: &Frame3, tol: &Tolerance) -> Result<Loop, GeometryError> {
    let mut edges: Vec<Edge> = Vec::new();
    for (i, e) in wire.edges.iter().enumerate() {
        let edge = map_edge(e, frame, tol);
        match edge.segment.validate(tol) {
            // An edge shorter than the tolerance leaves a gap the tolerance absorbs.
            Err(SegmentError::Degenerate)
                if edge
                    .segment
                    .start_point()
                    .distance(edge.segment.end_point())
                    <= tol.linear() =>
            {
                continue;
            }
            Err(error) => return Err(GeometryError::InvalidSegment { edge: i, error }),
            Ok(()) => edges.push(edge),
        }
    }
    Loop::new(chain(edges, tol), tol)
}

/// Flips edges whose direction does not continue the chain, trusting geometry over the
/// orientation flags when they disagree.
fn chain(edges: Vec<Edge>, tol: &Tolerance) -> Vec<Edge> {
    let flip = |e: Edge| Edge {
        segment: e.segment.reversed(),
        provenance: e.provenance,
    };
    let mut out: Vec<Edge> = Vec::with_capacity(edges.len());
    for e in edges {
        if let Some(prev) = out.last() {
            let end = prev.segment.end_point();
            if tol.points_coincide(e.segment.start_point(), end) {
                out.push(e);
            } else if tol.points_coincide(e.segment.end_point(), end) {
                out.push(flip(e));
            } else if out.len() == 1 {
                // The first edge may be the one that runs backwards.
                let start = prev.segment.start_point();
                if tol.points_coincide(e.segment.start_point(), start) {
                    let first = out.remove(0);
                    out.push(flip(first));
                    out.push(e);
                } else if tol.points_coincide(e.segment.end_point(), start) {
                    let first = out.remove(0);
                    out.push(flip(first));
                    out.push(flip(e));
                } else {
                    out.push(e);
                }
            } else {
                // Loop::new reports the gap.
                out.push(e);
            }
        } else {
            out.push(e);
        }
    }
    out
}

/// The 2D components of a 3D direction in `frame`.
fn dir2(frame: &Frame3, v: Vec3) -> Vec2 {
    Vec2::new(v.dot(frame.x_axis()), v.dot(frame.y_axis()))
}

fn map_edge(edge: &BrepEdge, frame: &Frame3, tol: &Tolerance) -> Edge {
    let p2 = |p| frame.to_local_2d(p);
    let (segment, deviation) = match &edge.curve {
        Curve3::Line { start, end } => (
            Segment::Line {
                a: p2(*start),
                b: p2(*end),
            },
            None,
        ),
        Curve3::Circle {
            center,
            axis,
            x_dir,
            radius,
            first,
            last,
        } => {
            // The circle turns counter-clockwise in the profile when its axis faces the viewer.
            let ccw = axis.dot(frame.z_axis()) >= 0.0;
            let phi = dir2(frame, *x_dir).angle();
            let s = if ccw { 1.0 } else { -1.0 };
            let full = (TAU - (last - first)) * radius <= tol.linear();
            let seg = if full {
                Segment::Circle {
                    center: p2(*center),
                    radius: *radius,
                    ccw,
                }
            } else {
                Segment::Arc {
                    center: p2(*center),
                    radius: *radius,
                    start_angle: phi + s * first,
                    end_angle: phi + s * last,
                    ccw,
                }
            };
            (seg, None)
        }
        Curve3::Ellipse {
            center,
            axis,
            x_dir,
            major_radius,
            minor_radius,
            first,
            last,
        } => {
            let ccw = axis.dot(frame.z_axis()) >= 0.0;
            let major = dir2(frame, *x_dir).normalized().unwrap_or(Vec2::X) * *major_radius;
            // Seen from behind, the minor axis is mirrored, which negates the parameter.
            let s = if ccw { 1.0 } else { -1.0 };
            (
                Segment::Ellipse {
                    center: p2(*center),
                    major_axis: major,
                    ratio: minor_radius / major_radius,
                    start: s * first,
                    end: s * last,
                    ccw,
                },
                None,
            )
        }
        Curve3::BSpline {
            degree,
            knots,
            poles,
            weights,
        } => (
            Segment::Spline(Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: poles.iter().map(|p| p2(*p)).collect(),
                weights: weights.clone(),
            }),
            None,
        ),
        Curve3::Sampled {
            points,
            max_deviation,
        } => (
            Segment::Polyline {
                points: points.iter().map(|p| p2(*p)).collect(),
            },
            Some(*max_deviation),
        ),
    };
    let segment = if edge.reversed {
        segment.reversed()
    } else {
        segment
    };
    match deviation {
        Some(d) => Edge::fitted(segment, d),
        None => Edge::exact(segment),
    }
}

/// The unit direction of the longest line in the loop, if it has one. Ties go to the first.
fn longest_line_direction(l: &Loop, tol: &Tolerance) -> Option<Vec2> {
    let mut best: Option<(f64, Vec2)> = None;
    for e in l.edges() {
        if let Segment::Line { a, b } = e.segment {
            let len = a.distance(b);
            if best.is_none_or(|(l, _)| len > l + tol.linear()) {
                best = Some((len, b - a));
            }
        }
    }
    best.and_then(|(_, d)| d.normalized())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::{FRAC_PI_2, PI};
    use flatbello_core::{Point2, Point3};

    /// Profile coordinates of a world point.
    fn local(profile: &Profile2D, p: Point3) -> Point2 {
        profile.frame().to_local_2d(p)
    }

    fn tol() -> Tolerance {
        Tolerance::default()
    }

    fn line(a: (f64, f64, f64), b: (f64, f64, f64)) -> BrepEdge {
        BrepEdge {
            curve: Curve3::Line {
                start: Point3::new(a.0, a.1, a.2),
                end: Point3::new(b.0, b.1, b.2),
            },
            reversed: false,
        }
    }

    fn circle(c: (f64, f64, f64), axis: Vec3, r: f64, first: f64, last: f64) -> BrepEdge {
        BrepEdge {
            curve: Curve3::Circle {
                center: Point3::new(c.0, c.1, c.2),
                axis,
                x_dir: Vec3::X,
                radius: r,
                first,
                last,
            },
            reversed: false,
        }
    }

    fn square_wire(z: f64, size: f64) -> BrepWire {
        BrepWire {
            edges: vec![
                line((0.0, 0.0, z), (size, 0.0, z)),
                line((size, 0.0, z), (size, size, z)),
                line((size, size, z), (0.0, size, z)),
                line((0.0, size, z), (0.0, 0.0, z)),
            ],
            is_outer: true,
        }
    }

    fn face(normal: Vec3, z: f64, wires: Vec<BrepWire>) -> BrepFace {
        BrepFace {
            id: FaceId(3),
            plane: Some(BrepPlane {
                origin: Point3::new(0.0, 0.0, z),
                normal,
                x_dir: Vec3::X,
            }),
            wires,
        }
    }

    fn plane_x() -> ExtractOptions {
        ExtractOptions {
            x_axis: XAxisRule::PlaneX,
            ..ExtractOptions::default()
        }
    }

    fn hole_center(p: &Profile2D) -> Point2 {
        match p.holes()[0].edges()[0].segment {
            Segment::Circle { center, .. } => center,
            ref s => panic!("expected a circle, got {s:?}"),
        }
    }

    #[test]
    fn top_face_with_hole() {
        let hole = BrepWire {
            edges: vec![circle((8.0, 2.0, 5.0), Vec3::Z, 1.0, 0.0, TAU)],
            is_outer: false,
        };
        let f = face(Vec3::Z, 5.0, vec![square_wire(5.0, 10.0), hole]);
        let p = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        assert_eq!(p.face(), FaceId(3));
        let b = p.bbox(&tol());
        assert!(b.min.distance(Point2::ORIGIN) < 1e-9);
        assert!(b.max.distance(Point2::new(10.0, 10.0)) < 1e-9);
        assert!(hole_center(&p).distance(Point2::new(8.0, 2.0)) < 1e-9);
        assert_eq!(p.outer().orientation(&tol()), Orientation::CounterClockwise);
        assert_eq!(p.holes()[0].orientation(&tol()), Orientation::Clockwise);
    }

    #[test]
    fn bottom_face_is_seen_from_below_not_mirrored() {
        // Normal −Z, x along +X: y points along −Y, so world (8, 2) is at local (8, −2), which
        // is (8, 8) once the square [0, 10]² moves to the origin.
        let hole = BrepWire {
            edges: vec![circle((8.0, 2.0, 0.0), Vec3::Z, 1.0, 0.0, TAU)],
            is_outer: false,
        };
        let f = face(-Vec3::Z, 0.0, vec![square_wire(0.0, 10.0), hole]);
        let p = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        assert!(hole_center(&p).distance(Point2::new(8.0, 8.0)) < 1e-9);
        let back = local(&p, Point3::new(8.0, 2.0, 0.0));
        assert!(back.distance(Point2::new(8.0, 8.0)) < 1e-9);
        // Orientation is still right as seen from outside.
        assert_eq!(p.outer().orientation(&tol()), Orientation::CounterClockwise);
        assert_eq!(p.holes()[0].orientation(&tol()), Orientation::Clockwise);
    }

    #[test]
    fn x_axis_follows_longest_straight_edge() {
        // A 20 × 5 rectangle rotated by 30° about z.
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        let rot = |x: f64, y: f64| (x * c - y * s, x * s + y * c, 0.0);
        let pts = [rot(0.0, 0.0), rot(20.0, 0.0), rot(20.0, 5.0), rot(0.0, 5.0)];
        let wire = BrepWire {
            edges: (0..4).map(|i| line(pts[i], pts[(i + 1) % 4])).collect(),
            is_outer: true,
        };
        let f = face(Vec3::Z, 0.0, vec![wire]);
        let p = extract_planar(&f, LengthUnit::Millimetre, &ExtractOptions::default()).unwrap();
        let b = p.bbox(&tol());
        assert!((b.width() - 20.0).abs() < 1e-9 && (b.height() - 5.0).abs() < 1e-9);
        // With the plane's x axis the box is the rotated rectangle's bounding box instead.
        let q = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        assert!((q.bbox(&tol()).width() - (20.0 * c + 5.0 * s)).abs() < 1e-9);
    }

    #[test]
    fn arcs_seen_from_behind_turn_the_other_way() {
        // A slot on the top face (normal +Z) whose arc edges have axis −Z, as OCCT may store them.
        let z = 0.0;
        let neg = -Vec3::Z;
        // With axis −Z and x_dir +X, the parameter t maps to angle −t in the profile.
        let wire = BrepWire {
            edges: vec![
                line((0.0, -1.0, z), (4.0, -1.0, z)),
                // Right cap: angles −90° → 90° ccw are t = 90° → −90°, so t from −90° to 90°
                // traversed backwards.
                BrepEdge {
                    reversed: true,
                    ..circle((4.0, 0.0, z), neg, 1.0, -FRAC_PI_2, FRAC_PI_2)
                },
                line((4.0, 1.0, z), (0.0, 1.0, z)),
                // Left cap: angles 90° → 270° ccw are t = −90° → −270°, so t from −270° to −90°
                // traversed backwards.
                BrepEdge {
                    reversed: true,
                    ..circle((0.0, 0.0, z), neg, 1.0, -1.5 * PI, -FRAC_PI_2)
                },
            ],
            is_outer: true,
        };
        let f = face(Vec3::Z, z, vec![wire]);
        let p = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        let area = p.outer().signed_area(&tol());
        assert!((area - (8.0 + PI)).abs() < 1e-9, "{area}");
        let b = p.bbox(&tol());
        assert!((b.width() - 6.0).abs() < 1e-9 && (b.height() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn wrong_orientation_flags_are_repaired_from_geometry() {
        let mut wire = square_wire(0.0, 10.0);
        wire.edges[0].reversed = true;
        wire.edges[2].reversed = true;
        let f = face(Vec3::Z, 0.0, vec![wire]);
        let p = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        assert!((p.outer().signed_area(&tol()) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn tiny_edges_are_dropped() {
        let mut wire = square_wire(0.0, 10.0);
        wire.edges[1] = line((10.0, 0.0, 0.0), (10.0, 0.0005, 0.0));
        wire.edges
            .insert(2, line((10.0, 0.0005, 0.0), (10.0, 10.0, 0.0)));
        let f = face(Vec3::Z, 0.0, vec![wire]);
        let p = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        assert_eq!(p.outer().edges().len(), 4);
    }

    #[test]
    fn ellipse_and_spline_edges() {
        // A half-ellipse (axis −Z) closed by a straight B-spline along the x axis.
        let ellipse = BrepEdge {
            curve: Curve3::Ellipse {
                center: Point3::ORIGIN,
                axis: -Vec3::Z,
                x_dir: Vec3::X,
                major_radius: 4.0,
                minor_radius: 2.0,
                first: PI,
                last: TAU,
            },
            reversed: false,
        };
        let spline = BrepEdge {
            curve: Curve3::BSpline {
                degree: 1,
                knots: vec![0.0, 0.0, 1.0, 1.0],
                poles: vec![Point3::new(4.0, 0.0, 0.0), Point3::new(-4.0, 0.0, 0.0)],
                weights: vec![],
            },
            reversed: false,
        };
        let f = face(
            Vec3::Z,
            0.0,
            vec![BrepWire {
                edges: vec![ellipse, spline],
                is_outer: true,
            }],
        );
        let p = extract_planar(&f, LengthUnit::Millimetre, &plane_x()).unwrap();
        // With axis −Z, t ∈ [π, 2π] has sin t ≤ 0, so y = −2 sin t ≥ 0: the upper half.
        let b = p.bbox(&tol());
        assert!((b.width() - 8.0).abs() < 1e-9 && (b.height() - 2.0).abs() < 1e-9);
        assert!((p.outer().signed_area(&tol()) - 4.0 * PI).abs() < 1e-6);
    }

    #[test]
    fn errors() {
        let mut f = face(Vec3::Z, 0.0, vec![square_wire(0.0, 10.0)]);
        f.plane = None;
        assert!(matches!(
            extract_planar(&f, LengthUnit::Millimetre, &plane_x()),
            Err(ExtractError::NotPlanar(_))
        ));
        let f = face(Vec3::Z, 0.0, vec![]);
        assert!(matches!(
            extract_planar(&f, LengthUnit::Millimetre, &plane_x()),
            Err(ExtractError::NoWires(_))
        ));
        let mut wire = square_wire(0.0, 10.0);
        wire.edges.pop();
        let f = face(Vec3::Z, 0.0, vec![wire]);
        assert!(matches!(
            extract_planar(&f, LengthUnit::Millimetre, &plane_x()),
            Err(ExtractError::Geometry { .. })
        ));
    }
}
