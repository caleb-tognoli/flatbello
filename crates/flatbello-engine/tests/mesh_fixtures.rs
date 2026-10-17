//! Mesh extraction from the STL fixtures, compared with exact extraction of the same faces from
//! the STEP files the meshes were tessellated from.
//!
//! Medium and fine meshes must reproduce every circle and arc radius within 0.01 mm and the
//! exact number of lines, arcs and circles. Coarse meshes are reported (run with
//! `--nocapture`); only their holes are checked.

// Test-only code: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::path::PathBuf;

use flatbello_core::{FaceId, ImportOptions, Loop, Profile2D, Segment, Tolerance, Vec3};
use flatbello_engine::{Engine, ExtractOptions, ModelId};

const RADIUS_TOL: f64 = 0.01;

fn open(name: &str) -> (Engine, ModelId) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/generated")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut e = Engine::new();
    let id = e.open(&bytes, name, &ImportOptions::default()).unwrap();
    (e, id)
}

/// The largest face whose outward normal is `normal`.
fn face_facing(e: &Engine, id: ModelId, normal: Vec3) -> FaceId {
    let model = e.model(id).unwrap();
    model
        .faces
        .iter()
        .filter(|f| {
            model
                .face_normal(f.id)
                .is_some_and(|n| n.dot(normal) > 1.0 - 1e-6)
        })
        .max_by(|a, b| a.area.total_cmp(&b.area))
        .expect("no face with that normal")
        .id
}

fn profile(name: &str, normal: Vec3) -> Profile2D {
    let (e, id) = open(name);
    let f = face_facing(&e, id, normal);
    e.extract(id, &[f], &ExtractOptions::default())
        .unwrap()
        .remove(0)
}

/// (lines, arcs, circles) of a loop, and the radii of its arcs and circles.
fn census(l: &Loop) -> ((usize, usize, usize), Vec<f64>) {
    let (mut lines, mut arcs, mut circles, mut radii) = (0, 0, 0, Vec::new());
    for e in l.edges() {
        match e.segment {
            Segment::Line { .. } => lines += 1,
            Segment::Arc { radius, .. } => {
                arcs += 1;
                radii.push(radius);
            }
            Segment::Circle { radius, .. } => {
                circles += 1;
                radii.push(radius);
            }
            _ => {}
        }
    }
    ((lines, arcs, circles), radii)
}

/// Every loop's census, outer first, holes sorted so that loop order does not matter.
fn loops(p: &Profile2D) -> Vec<((usize, usize, usize), Vec<f64>)> {
    let mut holes: Vec<_> = p.holes().iter().map(census).collect();
    holes.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.iter().sum::<f64>().total_cmp(&b.1.iter().sum::<f64>()))
    });
    std::iter::once(census(p.outer())).chain(holes).collect()
}

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v
}

fn size(p: &Profile2D) -> (f64, f64) {
    let b = p.bbox(&Tolerance::default());
    (b.width(), b.height())
}

/// Compares a mesh profile with the exact one. With `strict`, segment counts and radii must
/// match; otherwise only the number of holes.
fn compare(part: &str, level: &str, normal: Vec3, strict: bool) -> String {
    let exact = profile(&format!("{part}.step"), normal);
    let mesh = profile(&format!("{part}_{level}.stl"), normal);
    assert_eq!(
        mesh.holes().len(),
        exact.holes().len(),
        "{part} {level}: holes"
    );
    let (want, got) = (loops(&exact), loops(&mesh));
    let (ew, eh) = size(&exact);
    let (mw, mh) = size(&mesh);
    if strict {
        for (i, (w, g)) in want.iter().zip(&got).enumerate() {
            assert_eq!(g.0, w.0, "{part} {level}: loop {i} segment counts");
            for (rw, rg) in sorted(w.1.clone()).iter().zip(sorted(g.1.clone())) {
                assert!(
                    (rw - rg).abs() <= RADIUS_TOL,
                    "{part} {level}: loop {i} radius {rg} vs {rw}"
                );
            }
        }
        assert!(
            (ew - mw).abs() <= RADIUS_TOL && (eh - mh).abs() <= RADIUS_TOL,
            "{part} {level}: size {mw} × {mh} vs {ew} × {eh}"
        );
    }
    let fmt = |l: &[((usize, usize, usize), Vec<f64>)]| {
        l.iter()
            .map(|((a, b, c), _)| format!("{a}L{b}A{c}C"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let worst = want
        .iter()
        .zip(&got)
        .filter(|(w, g)| w.1.len() == g.1.len())
        .flat_map(|(w, g)| {
            sorted(w.1.clone())
                .into_iter()
                .zip(sorted(g.1.clone()))
                .map(|(a, b)| (a - b).abs())
        })
        .fold(0.0, f64::max);
    format!(
        "{part:<13} {level:<6} exact [{}] mesh [{}] size Δ {:.4} max radius Δ {worst:.5}",
        fmt(&want),
        fmt(&got),
        (ew - mw).abs().max((eh - mh).abs())
    )
}

const PRISMATIC: [(&str, fn() -> Vec3); 4] = [
    ("plate_holes", || Vec3::Z),
    ("plate_slots", || Vec3::Z),
    ("asym_plate", || -Vec3::Z),
    ("l_bracket", || -Vec3::X),
];

#[test]
fn medium_and_fine_meshes_reproduce_the_exact_profile() {
    for (part, normal) in PRISMATIC {
        for level in ["medium", "fine"] {
            println!("{}", compare(part, level, normal(), true));
        }
    }
}

#[test]
fn coarse_meshes_keep_every_hole() {
    for (part, normal) in PRISMATIC {
        println!("{}", compare(part, "coarse", normal(), false));
    }
}

#[test]
fn bspline_part_is_approximated_within_tolerance() {
    let exact = profile("bspline_part.step", Vec3::Z);
    let (ew, eh) = size(&exact);
    for level in ["coarse", "medium", "fine"] {
        let mesh = profile(&format!("bspline_part_{level}.stl"), Vec3::Z);
        let (mw, mh) = size(&mesh);
        let ((l, a, c), _) = census(mesh.outer());
        println!(
            "bspline_part  {level:<6} mesh [{l}L{a}A{c}C] size Δ {:.4}",
            (ew - mw).abs().max((eh - mh).abs())
        );
        // The spline becomes arcs; the outline must still match within the tessellation.
        let allowed = if level == "coarse" { 0.1 } else { 0.02 };
        assert!(
            (ew - mw).abs() <= allowed && (eh - mh).abs() <= allowed,
            "{level}"
        );
        assert!(l >= 1, "the straight base line survives");
    }
}

#[test]
fn ascii_and_binary_stl_agree() {
    // plate_holes_medium.stl is ASCII; plate_holes_fine.stl is binary.
    let a = profile("plate_holes_medium.stl", Vec3::Z);
    let b = profile("plate_holes_fine.stl", Vec3::Z);
    assert_eq!(loops(&a).len(), loops(&b).len());
}
