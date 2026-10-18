//! Kerf compensation: offsetting a profile with OCCT's planar offset.

use flatbello_core::{
    BrepFace, BrepPlane, ImportError, ImportOptions, Loop, Point2, Point3, Profile2D, Segment,
    Tolerance, Vec2, Vec3,
};
use flatbello_extract::{ExtractOptions, XAxisRule, extract_planar};
use flatbello_occt::{Shape, seg};

use crate::{import_error, read_wire};

/// Encodes a loop for [`Shape::offset_loops`]; curves other than lines, arcs and circles are
/// flattened into lines within the fit tolerance.
fn encode(l: &Loop, tol: &Tolerance, out: &mut Vec<f64>) {
    let mut edges: Vec<f64> = Vec::new();
    let mut count = 0;
    let line = |a: Point2, b: Point2, edges: &mut Vec<f64>| {
        edges.extend([seg::LINE, a.x, a.y, b.x, b.y]);
    };
    for e in l.edges() {
        match &e.segment {
            Segment::Line { a, b } => {
                line(*a, *b, &mut edges);
                count += 1;
            }
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                edges.extend([
                    seg::ARC,
                    center.x,
                    center.y,
                    *radius,
                    *start_angle,
                    *end_angle,
                    f64::from(u8::from(*ccw)),
                ]);
                count += 1;
            }
            Segment::Circle {
                center,
                radius,
                ccw,
            } => {
                edges.extend([
                    seg::CIRCLE,
                    center.x,
                    center.y,
                    *radius,
                    f64::from(u8::from(*ccw)),
                ]);
                count += 1;
            }
            other => {
                for w in other.flatten(tol.fit()).windows(2) {
                    if !tol.points_coincide(w[0], w[1]) {
                        line(w[0], w[1], &mut edges);
                        count += 1;
                    }
                }
            }
        }
    }
    out.push(count as f64);
    out.extend(edges);
}

/// The profile offset by `distance` mm, with round corners: positive grows the part (the outline
/// moves out and holes shrink), which compensates for a cutter's kerf of `2 × distance`. Holes
/// that close up disappear. The result keeps the profile's face, units and frame.
pub fn offset_profile(
    p: &Profile2D,
    distance: f64,
    tol: &Tolerance,
) -> Result<Profile2D, ImportError> {
    let mut data = Vec::new();
    encode(p.outer(), tol, &mut data);
    // OCCT offsets a region relative to its loop's direction, so every loop goes in
    // counter-clockwise: holes are reversed.
    for h in p.holes() {
        encode(&h.reversed(), tol, &mut data);
    }
    let shape = Shape::offset_loops(&data, distance).map_err(import_error)?;
    let options = ImportOptions {
        tolerance: *tol,
        ..ImportOptions::default()
    };
    let mut wires = Vec::new();
    for f in 0..shape.face_count().map_err(import_error)? {
        let info = shape.face_info(f).map_err(import_error)?;
        for w in 0..info.wire_count {
            let mut wire = read_wire(&shape, f, w, &options)?;
            // The largest loop becomes the outer boundary.
            wire.is_outer = false;
            wires.push(wire);
        }
    }
    let face = BrepFace {
        id: p.face(),
        plane: Some(BrepPlane {
            origin: Point3::ORIGIN,
            normal: Vec3::Z,
            x_dir: Vec3::X,
        }),
        wires,
    };
    let options = ExtractOptions {
        tolerance: *tol,
        x_axis: XAxisRule::PlaneX,
        ..ExtractOptions::default()
    };
    let q = extract_planar(&face, p.units(), &options)
        .map_err(|e| ImportError::Backend(e.to_string()))?;
    // q is in the profile's own coordinates moved by `shift` to the origin; carry the
    // profile's frame along by the same shift.
    let shift = Vec2::new(-q.frame().origin().x, -q.frame().origin().y);
    Profile2D::new(
        q.outer().clone(),
        q.holes().to_vec(),
        p.face(),
        p.frame().shifted_local(shift),
        p.units(),
        tol,
    )
    .map_err(ImportError::Geometry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::PI;
    use flatbello_core::sample;

    #[test]
    fn grows_the_outline_and_shrinks_holes() {
        let tol = Tolerance::default();
        let p = sample::rounded_plate().unwrap();
        let q = offset_profile(&p, 0.5, &tol).unwrap();
        let b = q.bbox(&tol);
        assert!((b.width() - 101.0).abs() < 1e-6 && (b.height() - 61.0).abs() < 1e-6);
        assert_eq!(q.holes().len(), 2);
        let radii: Vec<f64> = q
            .holes()
            .iter()
            .filter_map(|h| match h.edges()[0].segment {
                Segment::Circle { radius, .. } => Some(radius),
                _ => None,
            })
            .collect();
        let mut radii = radii;
        radii.sort_by(f64::total_cmp);
        assert!(
            (radii[0] - 2.5).abs() < 1e-6 && (radii[1] - 4.5).abs() < 1e-6,
            "{radii:?}"
        );
        // A rounded rectangle offset by d has area A + perimeter·d + π·d².
        let (w, h, r, d) = (100.0, 60.0, 5.0, 0.5);
        let area = w * h - (4.0 - PI) * r * r;
        let perimeter = 2.0 * (w + h) - 8.0 * r + 2.0 * PI * r;
        let want = area + perimeter * d + PI * d * d;
        assert!((q.outer().signed_area(&tol) - want).abs() < 1e-3);
    }

    #[test]
    fn negative_offsets_shrink_and_close_small_holes() {
        let tol = Tolerance::default();
        let p = sample::rounded_plate().unwrap();
        let q = offset_profile(&p, -0.5, &tol).unwrap();
        assert!((q.bbox(&tol).width() - 99.0).abs() < 1e-6);
        // Growing the material by 3.5 closes the Ø6 hole and leaves the Ø10 one.
        let r = offset_profile(&p, 3.5, &tol).unwrap();
        assert_eq!(r.holes().len(), 1);
    }

    #[test]
    fn keeps_the_frame_consistent() {
        let tol = Tolerance::default();
        let p = sample::slotted_plate().unwrap();
        let q = offset_profile(&p, 1.0, &tol).unwrap();
        // The original corner (0, 0) is now 1 mm inside the grown outline's corner.
        let corner = q.frame().to_local_2d(p.frame().to_world(Point3::ORIGIN));
        assert!(corner.distance(Point2::new(1.0, 1.0)) < 1e-6, "{corner:?}");
    }
}
