//! Generates test models with known geometry using OCCT.
//!
//! Usage: `gen-fixtures [out-dir]` (default `tests/fixtures/generated`). Every dimension below
//! is the ground truth that tests check extracted profiles against.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flatbello_core::{Point3, Vec3};
use flatbello_occt::{OcctError, Shape};

fn main() -> ExitCode {
    let dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("tests/fixtures/generated"), PathBuf::from);
    match generate(&dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gen-fixtures: {e}");
            ExitCode::FAILURE
        }
    }
}

fn boxed(x: f64, y: f64, z: f64, dx: f64, dy: f64, dz: f64) -> Result<Shape, OcctError> {
    Shape::make_box(Point3::new(x, y, z), Vec3::new(dx, dy, dz))
}

/// A cylinder along `axis` from `base`.
fn rod(base: (f64, f64, f64), axis: Vec3, r: f64, h: f64) -> Result<Shape, OcctError> {
    Shape::make_cylinder(Point3::new(base.0, base.1, base.2), axis, r, h)
}

/// A through-hole along z in a plate of thickness `t` lying on z = 0.
fn hole_z(x: f64, y: f64, r: f64, t: f64) -> Result<Shape, OcctError> {
    rod((x, y, -1.0), Vec3::Z, r, t + 2.0)
}

/// 100 × 60 × 5 plate; Ø10 hole at (20, 30), Ø6 hole at (80, 30).
fn plate_holes() -> Result<Shape, OcctError> {
    boxed(0.0, 0.0, 0.0, 100.0, 60.0, 5.0)?
        .cut(&hole_z(20.0, 30.0, 5.0, 5.0)?)?
        .cut(&hole_z(80.0, 30.0, 3.0, 5.0)?)
}

/// 80 × 40 × 4 plate with an 8 mm slot, arc centres (25, 20) and (55, 20), and a Ø6 hole at
/// (68, 30).
fn plate_slots() -> Result<Shape, OcctError> {
    let slot = boxed(25.0, 16.0, -1.0, 30.0, 8.0, 6.0)?
        .fuse(&hole_z(25.0, 20.0, 4.0, 4.0)?)?
        .fuse(&hole_z(55.0, 20.0, 4.0, 4.0)?)?;
    boxed(0.0, 0.0, 0.0, 80.0, 40.0, 4.0)?
        .cut(&slot)?
        .cut(&hole_z(68.0, 30.0, 3.0, 4.0)?)
}

/// An L-bracket: a 60 × 40 × 5 base and a 5 × 40 × 50 upright at x = 0, with a Ø8 hole through
/// the upright at (y, z) = (20, 30) and a Ø8 hole through the base at (40, 20).
fn l_bracket() -> Result<Shape, OcctError> {
    boxed(0.0, 0.0, 0.0, 60.0, 40.0, 5.0)?
        .fuse(&boxed(0.0, 0.0, 0.0, 5.0, 40.0, 50.0)?)?
        .cut(&rod((-1.0, 20.0, 30.0), Vec3::X, 4.0, 7.0)?)?
        .cut(&hole_z(40.0, 20.0, 4.0, 5.0)?)
}

/// The control points of the B-spline part's curved edge.
pub const BSPLINE_POLES: [(f64, f64); 5] = [
    (0.0, 0.0),
    (15.0, 25.0),
    (40.0, 30.0),
    (60.0, 10.0),
    (70.0, 0.0),
];

/// A 5 mm prism over a clamped cubic B-spline through BSPLINE_POLES, closed by the line from
/// (70, 0) back to (0, 0).
fn bspline_part() -> Result<Shape, OcctError> {
    let poles: Vec<Point3> = BSPLINE_POLES
        .iter()
        .map(|&(x, y)| Point3::new(x, y, 0.0))
        .collect();
    Shape::make_bspline_prism(&poles, 3, 5.0)
}

/// 50 × 30 × 3 plate with one Ø6 hole near a corner, at (40, 22), so that a mirrored bottom
/// face would be obvious.
fn asym_plate() -> Result<Shape, OcctError> {
    boxed(0.0, 0.0, 0.0, 50.0, 30.0, 3.0)?.cut(&hole_z(40.0, 22.0, 3.0, 3.0)?)
}

/// Tessellation levels for the STL fixtures: name and linear deflection in mm.
const STL_LEVELS: [(&str, f64); 3] = [("coarse", 0.1), ("medium", 0.01), ("fine", 0.002)];

/// Builds one fixture part.
type Builder = fn() -> Result<Shape, OcctError>;

fn generate(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let parts: [(&str, Builder); 5] = [
        ("plate_holes", plate_holes),
        ("plate_slots", plate_slots),
        ("l_bracket", l_bracket),
        ("bspline_part", bspline_part),
        ("asym_plate", asym_plate),
    ];
    for (name, build) in parts {
        let shape = build().map_err(|e| format!("{name}: {e}"))?;
        let path = dir.join(format!("{name}.step"));
        shape
            .write_step(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    // STL exports at three tessellation levels, for the mesh pipeline. One medium file is
    // ASCII, to exercise the ASCII reader.
    for (name, build) in parts {
        for (level, deflection) in STL_LEVELS {
            let path = dir.join(format!("{name}_{level}.stl"));
            let ascii = name == "plate_holes" && level == "medium";
            build()
                .and_then(|s| s.write_stl(&path, deflection, ascii))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }
    // One IGES file, to exercise the IGES importer.
    let path = dir.join("plate_holes.igs");
    plate_holes()
        .and_then(|s| s.write_iges(&path))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}
