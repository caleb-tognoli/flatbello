//! Golden-file tests. Set `FLATBELLO_BLESS=1` to rewrite the expected files after a deliberate
//! change, then review the diff.

// Test-only code: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use flatbello_core::{ExportOptions, Tolerance, layout_side_by_side, sample};
use flatbello_export_svg::write_svg;

fn check_golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/golden")
        .join(name);
    if std::env::var_os("FLATBELLO_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected =
        std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    assert!(
        actual.as_bytes() == expected.as_slice(),
        "{name} differs from {}; rerun with FLATBELLO_BLESS=1 to update it",
        path.display()
    );
}

#[test]
fn rounded_plate() {
    let svg = write_svg(
        &[sample::rounded_plate().unwrap()],
        &ExportOptions::default(),
    )
    .unwrap();
    check_golden("rounded_plate.svg", &svg);
}

#[test]
fn slotted_plate() {
    let svg = write_svg(
        &[sample::slotted_plate().unwrap()],
        &ExportOptions::default(),
    )
    .unwrap();
    check_golden("slotted_plate.svg", &svg);
}

#[test]
fn two_plates_side_by_side() {
    let tol = Tolerance::default();
    let laid = layout_side_by_side(
        &[
            sample::rounded_plate().unwrap(),
            sample::slotted_plate().unwrap(),
        ],
        10.0,
        &tol,
    );
    let svg = write_svg(&laid, &ExportOptions::default()).unwrap();
    check_golden("two_plates.svg", &svg);
}
