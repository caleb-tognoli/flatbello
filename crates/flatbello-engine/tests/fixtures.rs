//! Extraction from the generated STEP fixtures, checked against the dimensions they were built
//! with in tools/gen-fixtures.

// Test-only code: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::path::PathBuf;

use core::f64::consts::PI;

use flatbello_core::angle::sweep;
use flatbello_core::{
    Face, FaceId, FaceKind, ImportOptions, LengthUnit, Orientation, Point2, Point3, Profile2D,
    Segment, Tolerance, Vec3,
};
use flatbello_engine::{Engine, EngineError, ExtractOptions, ModelId};

const EPS: f64 = 1e-6;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/generated")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn open(name: &str) -> (Engine, ModelId) {
    let mut e = Engine::new();
    let id = e
        .open(&fixture(name), name, &ImportOptions::default())
        .unwrap();
    (e, id)
}

fn tol() -> Tolerance {
    Tolerance::default()
}

/// The largest planar face whose outward normal is `normal`.
fn face_facing(e: &Engine, id: ModelId, normal: Vec3) -> FaceId {
    let model = e.model(id).unwrap();
    let flatbello_core::ModelGeometry::Brep(breps) = &model.geometry;
    let mut best: Option<&Face> = None;
    for f in e.faces(id).unwrap() {
        let facing = breps[f.id.0 as usize]
            .plane
            .is_some_and(|p| p.normal.dot(normal) > 1.0 - 1e-9);
        if f.kind == FaceKind::Planar && facing && best.is_none_or(|b| f.area > b.area) {
            best = Some(f);
        }
    }
    best.expect("no face with that normal").id
}

fn extract(e: &Engine, id: ModelId, face: FaceId) -> Profile2D {
    e.extract(id, &[face], &ExtractOptions::default())
        .unwrap()
        .remove(0)
}

/// Checks the frame is right-handed with z along `normal`, which rules out mirroring.
fn assert_frame(p: &Profile2D, normal: Vec3) {
    let f = p.frame();
    assert!(
        (f.z_axis().dot(normal) - 1.0).abs() < EPS,
        "z = {:?}",
        f.z_axis()
    );
    assert!((f.x_axis().cross(f.y_axis()).dot(f.z_axis()) - 1.0).abs() < EPS);
}

fn assert_size(p: &Profile2D, w: f64, h: f64) {
    let b = p.bbox(&tol());
    assert!(b.min.distance(Point2::ORIGIN) < EPS, "min {:?}", b.min);
    assert!(
        (b.width() - w).abs() < EPS && (b.height() - h).abs() < EPS,
        "size {} × {}",
        b.width(),
        b.height()
    );
}

/// The circle holes as (centre, radius), in hole order.
fn circles(p: &Profile2D) -> Vec<(Point2, f64)> {
    p.holes()
        .iter()
        .filter_map(|h| match (h.edges().len(), &h.edges()[0].segment) {
            (1, Segment::Circle { center, radius, .. }) => Some((*center, *radius)),
            _ => None,
        })
        .collect()
}

/// Asserts there is a circular hole of radius `r` centred on the world point `c`.
fn assert_hole(p: &Profile2D, c: Point3, r: f64) {
    let want = p.frame().to_local_2d(c);
    assert!(
        circles(p)
            .iter()
            .any(|(centre, radius)| centre.distance(want) < EPS && (radius - r).abs() < EPS),
        "no hole r={r} at {want:?}; holes {:?}",
        circles(p)
    );
}

fn line_lengths(l: &flatbello_core::Loop) -> Vec<f64> {
    let mut v: Vec<f64> = l
        .edges()
        .iter()
        .filter_map(|e| match e.segment {
            Segment::Line { a, b } => Some(a.distance(b)),
            _ => None,
        })
        .collect();
    v.sort_by(f64::total_cmp);
    v
}

fn assert_lengths(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < EPS, "{got:?} vs {want:?}");
    }
}

#[test]
fn plate_holes_top_and_bottom() {
    let (e, id) = open("plate_holes.step");
    assert_eq!(e.model(id).unwrap().units, LengthUnit::Millimetre);
    for (normal, z) in [(Vec3::Z, 5.0), (-Vec3::Z, 0.0)] {
        let p = extract(&e, id, face_facing(&e, id, normal));
        assert_frame(&p, normal);
        assert_size(&p, 100.0, 60.0);
        assert_eq!(p.outer().orientation(&tol()), Orientation::CounterClockwise);
        assert_lengths(&line_lengths(p.outer()), &[60.0, 60.0, 100.0, 100.0]);
        assert_eq!(p.holes().len(), 2);
        assert_hole(&p, Point3::new(20.0, 30.0, z), 5.0);
        assert_hole(&p, Point3::new(80.0, 30.0, z), 3.0);
        for h in p.holes() {
            assert_eq!(h.orientation(&tol()), Orientation::Clockwise);
        }
    }
}

#[test]
fn plate_slots_top() {
    let (e, id) = open("plate_slots.step");
    let p = extract(&e, id, face_facing(&e, id, Vec3::Z));
    assert_frame(&p, Vec3::Z);
    assert_size(&p, 80.0, 40.0);
    assert_eq!(p.holes().len(), 2);
    assert_hole(&p, Point3::new(68.0, 30.0, 4.0), 3.0);
    let slot = p
        .holes()
        .iter()
        .find(|h| h.edges().len() == 4)
        .expect("slot hole");
    assert_lengths(&line_lengths(slot), &[30.0, 30.0]);
    let centres = [Point3::new(25.0, 20.0, 4.0), Point3::new(55.0, 20.0, 4.0)];
    for e in slot.edges() {
        if let Segment::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ccw,
        } = e.segment
        {
            assert!((radius - 4.0).abs() < EPS);
            assert!(!ccw, "hole arcs run clockwise");
            assert!((sweep(start_angle, end_angle, ccw) - PI).abs() < 1e-9);
            assert!(
                centres
                    .iter()
                    .any(|c| p.frame().to_local_2d(*c).distance(center) < EPS)
            );
        }
    }
    let slot_area = 30.0 * 8.0 + PI * 16.0;
    assert!((slot.signed_area(&tol()) + slot_area).abs() < 1e-6);
}

#[test]
fn l_bracket_upright_outside_face() {
    let (e, id) = open("l_bracket.step");
    let p = extract(&e, id, face_facing(&e, id, -Vec3::X));
    assert_frame(&p, -Vec3::X);
    assert_size(&p, 50.0, 40.0);
    assert_hole(&p, Point3::new(0.0, 20.0, 30.0), 4.0);
    // The base's top face is an L-shaped region minus nothing: 55 × 40 with the Ø8 hole.
    let base = extract(&e, id, face_facing(&e, id, Vec3::Z));
    assert_frame(&base, Vec3::Z);
    assert_hole(&base, Point3::new(40.0, 20.0, 5.0), 4.0);
}

#[test]
fn bspline_edge_is_exact() {
    let (e, id) = open("bspline_part.step");
    let p = extract(&e, id, face_facing(&e, id, Vec3::Z));
    assert_frame(&p, Vec3::Z);
    assert_lengths(&line_lengths(p.outer()), &[70.0]);
    let spline = p
        .outer()
        .edges()
        .iter()
        .find_map(|e| match &e.segment {
            Segment::Spline(s) => Some(s.clone()),
            _ => None,
        })
        .expect("a spline edge");
    assert_eq!(spline.degree, 3);
    let poles = [
        (0.0, 0.0),
        (15.0, 25.0),
        (40.0, 30.0),
        (60.0, 10.0),
        (70.0, 0.0),
    ];
    assert_eq!(spline.control_points.len(), poles.len());
    // The control polygon may run either way round, depending on the loop direction.
    let world: Vec<Point2> = poles
        .iter()
        .map(|&(x, y)| p.frame().to_local_2d(Point3::new(x, y, 5.0)))
        .collect();
    let forward = spline
        .control_points
        .iter()
        .zip(&world)
        .all(|(a, b)| a.distance(*b) < EPS);
    let backward = spline
        .control_points
        .iter()
        .zip(world.iter().rev())
        .all(|(a, b)| a.distance(*b) < EPS);
    assert!(forward || backward, "{:?}", spline.control_points);
}

#[test]
fn asym_plate_bottom_is_not_mirrored() {
    let (e, id) = open("asym_plate.step");
    let p = extract(&e, id, face_facing(&e, id, -Vec3::Z));
    assert_frame(&p, -Vec3::Z);
    assert_size(&p, 50.0, 30.0);
    assert_hole(&p, Point3::new(40.0, 22.0, 0.0), 3.0);
}

#[test]
fn every_planar_face_extracts() {
    for name in [
        "plate_holes.step",
        "plate_slots.step",
        "l_bracket.step",
        "bspline_part.step",
        "asym_plate.step",
    ] {
        let (e, id) = open(name);
        let planar: Vec<FaceId> = e
            .faces(id)
            .unwrap()
            .iter()
            .filter(|f| f.kind == FaceKind::Planar)
            .map(|f| f.id)
            .collect();
        assert!(!planar.is_empty());
        let profiles = e.extract(id, &planar, &ExtractOptions::default()).unwrap();
        assert_eq!(profiles.len(), planar.len(), "{name}");
        assert!(e.render_mesh(id).unwrap().triangle_count() > 0);
    }
}

#[test]
fn cylindrical_faces_are_not_planar() {
    let (e, id) = open("plate_holes.step");
    let cyl = e
        .faces(id)
        .unwrap()
        .iter()
        .find(|f| f.kind == FaceKind::Cylindrical)
        .expect("a hole wall")
        .id;
    assert!(matches!(
        e.extract(id, &[cyl], &ExtractOptions::default()),
        Err(EngineError::Extract(_))
    ));
}

#[test]
fn truncated_and_corrupt_files_are_errors() {
    let full = fixture("plate_holes.step");
    let mut e = Engine::new();
    let opts = ImportOptions::default();
    for cut in [full.len() / 2, full.len() / 10, 40] {
        let r = e.open(&full[..cut], "broken.step", &opts);
        // A truncated file may still contain a partial shape; it must never crash.
        if let Ok(id) = r {
            let _ = e.faces(id);
        }
    }
    let mut garbled = full.clone();
    for b in garbled.iter_mut().skip(400).step_by(7) {
        *b = b'#';
    }
    let _ = e.open(&garbled, "garbled.step", &opts);
    assert!(e.open(b"ISO-10303-21;\nnonsense", "x.step", &opts).is_err());
}
