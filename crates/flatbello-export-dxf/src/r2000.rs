//! DXF R2000 (`AC1015`): `$INSUNITS = 4` (millimetres), handles and owners, the tables AutoCAD
//! requires, and `LWPOLYLINE` for polylines. Optionally native `ELLIPSE`/`SPLINE` entities and
//! whole loops as closed `LWPOLYLINE`s with bulges.

use core::f64::consts::TAU;

use flatbello_core::angle::{normalize_angle, sweep};
use flatbello_core::{
    Bbox2, ExportError, ExportOptions, Exporter, Loop, Point2, Profile2D, Segment, Spline,
    Tolerance,
};

use crate::{Dxf, LAYER_INNER, LAYER_OUTER, LAYERS, degrees};

pub struct DxfR2000Exporter;

impl Exporter for DxfR2000Exporter {
    fn id(&self) -> &'static str {
        "dxf-r2000"
    }

    fn name(&self) -> &'static str {
        "DXF R2000"
    }

    fn extension(&self) -> &'static str {
        "dxf"
    }

    fn write(
        &self,
        profiles: &[Profile2D],
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ExportError> {
        write_r2000(profiles, options).map(String::into_bytes)
    }
}

/// Hands out entity handles: upper-case hex, never 0.
struct Handles(u32);

impl Handles {
    fn next(&mut self) -> String {
        self.0 += 1;
        format!("{:X}", self.0)
    }
}

/// Context for writing model-space entities.
struct Space<'a> {
    owner: &'a str,
    tol: &'a Tolerance,
    options: &'a ExportOptions,
}

impl Dxf {
    fn handle(&mut self, h: &str) {
        self.pair(5, h);
    }

    /// The common start of every entity in model space.
    fn entity(&mut self, kind: &str, handles: &mut Handles, space: &Space<'_>, layer: &str) {
        self.pair(0, kind);
        self.handle(&handles.next());
        self.pair(330, space.owner);
        self.pair(100, "AcDbEntity");
        self.pair(8, layer);
    }

    fn table(&mut self, name: &str, handle: &str, count: usize) {
        self.pair(0, "TABLE");
        self.pair(2, name);
        self.handle(handle);
        self.pair(330, "0");
        self.pair(100, "AcDbSymbolTable");
        self.int(70, count as i32);
    }

    fn record(&mut self, kind: &str, handle: &str, table: &str, subclass: &str, name: &str) {
        self.pair(0, kind);
        self.handle(handle);
        self.pair(330, table);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, subclass);
        self.pair(2, name);
        self.int(70, 0);
    }
}

pub fn write_r2000(profiles: &[Profile2D], options: &ExportOptions) -> Result<String, ExportError> {
    if profiles.is_empty() {
        return Err(ExportError::NoProfiles);
    }
    let tol = &options.tolerance;
    let bbox = profiles
        .iter()
        .fold(Bbox2::EMPTY, |b, p| b.union(&p.bbox(tol)));
    let mut h = Handles(0);
    let mut d = Dxf { out: String::new() };

    // TABLES ------------------------------------------------------------------------------
    d.begin_section("TABLES");
    let vport = h.next();
    d.table("VPORT", &vport, 0);
    d.pair(0, "ENDTAB");

    let ltype = h.next();
    d.table("LTYPE", &ltype, 3);
    for (name, desc) in [
        ("ByBlock", ""),
        ("ByLayer", ""),
        ("Continuous", "Solid line"),
    ] {
        d.record("LTYPE", &h.next(), &ltype, "AcDbLinetypeTableRecord", name);
        d.pair(3, desc);
        d.int(72, 65);
        d.int(73, 0);
        d.real(40, 0.0);
    }
    d.pair(0, "ENDTAB");

    let layer = h.next();
    d.table("LAYER", &layer, LAYERS.len());
    for (name, color) in LAYERS {
        d.record("LAYER", &h.next(), &layer, "AcDbLayerTableRecord", name);
        d.int(62, i32::from(color));
        d.pair(6, "Continuous");
    }
    d.pair(0, "ENDTAB");

    let style = h.next();
    d.table("STYLE", &style, 1);
    d.record(
        "STYLE",
        &h.next(),
        &style,
        "AcDbTextStyleTableRecord",
        "Standard",
    );
    d.real(40, 0.0);
    d.real(41, 1.0);
    d.real(50, 0.0);
    d.int(71, 0);
    d.real(42, 2.5);
    d.pair(3, "txt");
    d.pair(4, "");
    d.pair(0, "ENDTAB");

    for name in ["VIEW", "UCS"] {
        let t = h.next();
        d.table(name, &t, 0);
        d.pair(0, "ENDTAB");
    }

    let appid = h.next();
    d.table("APPID", &appid, 1);
    d.record("APPID", &h.next(), &appid, "AcDbRegAppTableRecord", "ACAD");
    d.pair(0, "ENDTAB");

    // DIMSTYLE has an extra subclass, and its records use group 105 for the handle.
    let dimstyle = h.next();
    d.table("DIMSTYLE", &dimstyle, 1);
    d.pair(100, "AcDbDimStyleTable");
    d.pair(0, "DIMSTYLE");
    d.pair(105, &h.next());
    d.pair(330, &dimstyle);
    d.pair(100, "AcDbSymbolTableRecord");
    d.pair(100, "AcDbDimStyleTableRecord");
    d.pair(2, "Standard");
    d.int(70, 0);
    d.pair(0, "ENDTAB");

    let block_records = h.next();
    let (model, paper) = (h.next(), h.next());
    d.table("BLOCK_RECORD", &block_records, 2);
    for (handle, name) in [(&model, "*Model_Space"), (&paper, "*Paper_Space")] {
        d.pair(0, "BLOCK_RECORD");
        d.handle(handle);
        d.pair(330, &block_records);
        d.pair(100, "AcDbSymbolTableRecord");
        d.pair(100, "AcDbBlockTableRecord");
        d.pair(2, name);
    }
    d.pair(0, "ENDTAB");
    d.end_section();

    // BLOCKS ------------------------------------------------------------------------------
    d.begin_section("BLOCKS");
    for (owner, name) in [(&model, "*Model_Space"), (&paper, "*Paper_Space")] {
        d.pair(0, "BLOCK");
        d.handle(&h.next());
        d.pair(330, owner);
        d.pair(100, "AcDbEntity");
        d.pair(8, "0");
        d.pair(100, "AcDbBlockBegin");
        d.pair(2, name);
        d.int(70, 0);
        d.point(10, Point2::ORIGIN);
        d.pair(3, name);
        d.pair(1, "");
        d.pair(0, "ENDBLK");
        d.handle(&h.next());
        d.pair(330, owner);
        d.pair(100, "AcDbEntity");
        d.pair(8, "0");
        d.pair(100, "AcDbBlockEnd");
    }
    d.end_section();

    // ENTITIES ----------------------------------------------------------------------------
    d.begin_section("ENTITIES");
    let space = Space {
        owner: &model,
        tol,
        options,
    };
    for p in profiles {
        write_loop(&mut d, &mut h, &space, p.outer(), LAYER_OUTER);
        for hole in p.holes() {
            write_loop(&mut d, &mut h, &space, hole, LAYER_INNER);
        }
    }
    d.end_section();

    // OBJECTS -----------------------------------------------------------------------------
    d.begin_section("OBJECTS");
    let (root, groups) = (h.next(), h.next());
    d.pair(0, "DICTIONARY");
    d.handle(&root);
    d.pair(330, "0");
    d.pair(100, "AcDbDictionary");
    d.int(281, 1);
    d.pair(3, "ACAD_GROUP");
    d.pair(350, &groups);
    d.pair(0, "DICTIONARY");
    d.handle(&groups);
    d.pair(330, &root);
    d.pair(100, "AcDbDictionary");
    d.int(281, 1);
    d.end_section();
    d.pair(0, "EOF");

    // The header comes first but needs the final handle seed.
    let mut head = Dxf { out: String::new() };
    head.begin_section("HEADER");
    head.pair(9, "$ACADVER");
    head.pair(1, "AC1015");
    head.pair(9, "$HANDSEED");
    head.pair(5, &h.next());
    head.pair(9, "$INSUNITS");
    head.int(70, 4);
    head.pair(9, "$MEASUREMENT");
    head.int(70, 1);
    head.header_point("$INSBASE", Point2::ORIGIN);
    head.header_point("$EXTMIN", bbox.min);
    head.header_point("$EXTMAX", bbox.max);
    head.end_section();
    head.begin_section("CLASSES");
    head.end_section();
    head.out.push_str(&d.out);
    Ok(head.out)
}

fn write_loop(d: &mut Dxf, h: &mut Handles, space: &Space<'_>, l: &Loop, layer: &str) {
    if space.options.loops_as_polylines {
        write_closed_polyline(d, h, space, l, layer);
        return;
    }
    for e in l.edges() {
        match &e.segment {
            Segment::Line { a, b } => {
                d.entity("LINE", h, space, layer);
                d.pair(100, "AcDbLine");
                d.point(10, *a);
                d.point(11, *b);
            }
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                // DXF arcs run counter-clockwise from 50 to 51; swap the ends of clockwise arcs.
                let (from, to) = if *ccw {
                    (*start_angle, *end_angle)
                } else {
                    (*end_angle, *start_angle)
                };
                d.entity("ARC", h, space, layer);
                d.pair(100, "AcDbCircle");
                d.point(10, *center);
                d.real(40, *radius);
                d.pair(100, "AcDbArc");
                d.real(50, degrees(from));
                d.real(51, degrees(to));
            }
            Segment::Circle { center, radius, .. } => {
                d.entity("CIRCLE", h, space, layer);
                d.pair(100, "AcDbCircle");
                d.point(10, *center);
                d.real(40, *radius);
            }
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } if space.options.native_curves => {
                // Parameters run counter-clockwise from 41 to 42, as for arcs.
                let (from, to) = if *ccw { (*start, *end) } else { (*end, *start) };
                let from = normalize_angle(from);
                let to = from + sweep(from, to, true);
                d.entity("ELLIPSE", h, space, layer);
                d.pair(100, "AcDbEllipse");
                d.point(10, *center);
                d.point(11, Point2::new(major_axis.x, major_axis.y));
                d.real(210, 0.0);
                d.real(220, 0.0);
                d.real(230, 1.0);
                d.real(40, *ratio);
                d.real(41, from);
                d.real(42, to.min(from + TAU));
            }
            Segment::Spline(s) if space.options.native_curves => {
                write_spline(d, h, space, s, layer)
            }
            Segment::Ellipse { .. } | Segment::Spline(_) | Segment::Polyline { .. } => {
                let pts = e.segment.flatten(space.tol.fit());
                write_lwpolyline(
                    d,
                    h,
                    space,
                    layer,
                    &pts.iter().map(|p| (*p, 0.0)).collect::<Vec<_>>(),
                    false,
                );
            }
        }
    }
}

fn write_spline(d: &mut Dxf, h: &mut Handles, space: &Space<'_>, s: &Spline, layer: &str) {
    d.entity("SPLINE", h, space, layer);
    d.pair(100, "AcDbSpline");
    d.real(210, 0.0);
    d.real(220, 0.0);
    d.real(230, 1.0);
    // 8: planar; 4: rational.
    d.int(70, if s.is_rational() { 12 } else { 8 });
    d.int(71, s.degree as i32);
    d.int(72, s.knots.len() as i32);
    d.int(73, s.control_points.len() as i32);
    d.int(74, 0);
    d.real(42, 1e-10);
    d.real(43, 1e-10);
    for k in &s.knots {
        d.real(40, *k);
    }
    for w in &s.weights {
        d.real(41, *w);
    }
    for p in &s.control_points {
        d.point(10, *p);
    }
}

/// An `LWPOLYLINE` through `(point, bulge)` vertices; the bulge applies to the segment that
/// starts at the vertex.
fn write_lwpolyline(
    d: &mut Dxf,
    h: &mut Handles,
    space: &Space<'_>,
    layer: &str,
    vertices: &[(Point2, f64)],
    closed: bool,
) {
    d.entity("LWPOLYLINE", h, space, layer);
    d.pair(100, "AcDbPolyline");
    d.int(90, vertices.len() as i32);
    d.int(70, i32::from(closed));
    for (p, bulge) in vertices {
        d.real(10, p.x);
        d.real(20, p.y);
        if bulge.abs() > 0.0 {
            d.real(42, *bulge);
        }
    }
}

/// A whole loop as one closed `LWPOLYLINE`: arcs become bulges (tan of a quarter of the sweep,
/// negative for clockwise), circles two half-turn bulges, other curves flattened vertices.
fn write_closed_polyline(d: &mut Dxf, h: &mut Handles, space: &Space<'_>, l: &Loop, layer: &str) {
    let mut vertices: Vec<(Point2, f64)> = Vec::new();
    for e in l.edges() {
        match &e.segment {
            Segment::Line { a, .. } => vertices.push((*a, 0.0)),
            Segment::Arc {
                start_angle,
                end_angle,
                ccw,
                ..
            } => {
                let s = sweep(*start_angle, *end_angle, *ccw);
                let bulge = (s / 4.0).tan();
                vertices.push((e.segment.start_point(), if *ccw { bulge } else { -bulge }));
            }
            Segment::Circle {
                center,
                radius,
                ccw,
            } => {
                let b = if *ccw { 1.0 } else { -1.0 };
                vertices.push((Point2::new(center.x + radius, center.y), b));
                vertices.push((Point2::new(center.x - radius, center.y), b));
            }
            other => {
                let pts = other.flatten(space.tol.fit());
                vertices.extend(pts[..pts.len().saturating_sub(1)].iter().map(|p| (*p, 0.0)));
            }
        }
    }
    write_lwpolyline(d, h, space, layer, &vertices, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::PI;
    use flatbello_core::{Edge, FaceId, Frame3, LengthUnit, Vec2, sample};
    use std::collections::{HashMap, HashSet};

    fn opts() -> ExportOptions {
        ExportOptions::default()
    }

    fn pairs(dxf: &str) -> Vec<(u16, String)> {
        let lines: Vec<&str> = dxf.split("\r\n").collect();
        lines
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[c, v]| (c.trim().parse().unwrap(), v.to_string()))
            .collect()
    }

    fn entities(dxf: &str) -> Vec<Vec<(u16, String)>> {
        let all = pairs(dxf);
        let start = all
            .iter()
            .position(|(c, v)| *c == 2 && v == "ENTITIES")
            .unwrap()
            + 1;
        let mut out: Vec<Vec<(u16, String)>> = Vec::new();
        for (c, v) in &all[start..] {
            if *c == 0 {
                if v == "ENDSEC" {
                    break;
                }
                out.push(Vec::new());
            }
            out.last_mut().unwrap().push((*c, v.clone()));
        }
        out
    }

    fn value(e: &[(u16, String)], code: u16) -> &str {
        &e.iter().find(|(c, _)| *c == code).unwrap().1
    }

    #[test]
    fn header_units_and_sections() {
        let dxf = write_r2000(&[sample::rounded_plate().unwrap()], &opts()).unwrap();
        let all = pairs(&dxf);
        let after = |name: &str| {
            let i = all.iter().position(|(c, v)| *c == 9 && v == name).unwrap();
            all[i + 1].1.clone()
        };
        assert_eq!(after("$ACADVER"), "AC1015");
        assert_eq!(after("$INSUNITS"), "4");
        let sections: Vec<&str> = all
            .windows(2)
            .filter(|w| w[0].1 == "SECTION")
            .map(|w| w[1].1.as_str())
            .collect();
        assert_eq!(
            sections,
            [
                "HEADER", "CLASSES", "TABLES", "BLOCKS", "ENTITIES", "OBJECTS"
            ]
        );
        assert!(dxf.ends_with("  0\r\nEOF\r\n"));
    }

    #[test]
    fn handles_are_unique_owners_exist_and_the_seed_is_above_them() {
        let dxf = write_r2000(
            &[
                sample::rounded_plate().unwrap(),
                sample::slotted_plate().unwrap(),
            ],
            &opts(),
        )
        .unwrap();
        let all = pairs(&dxf);
        let seed_at = all
            .iter()
            .position(|(c, v)| *c == 9 && v == "$HANDSEED")
            .unwrap();
        let seed = u32::from_str_radix(&all[seed_at + 1].1, 16).unwrap();
        let mut handles = HashSet::new();
        for (i, (c, v)) in all.iter().enumerate() {
            // Every 5 or 105 is an object handle, except the seed's value in the header.
            if (*c == 5 || *c == 105) && i != seed_at + 1 {
                assert!(handles.insert(v.clone()), "duplicate handle {v}");
                assert!(u32::from_str_radix(v, 16).unwrap() < seed);
            }
        }
        for (c, v) in &all {
            if *c == 330 && v != "0" {
                assert!(handles.contains(v), "owner {v} does not exist");
            }
        }
        // Every entity is owned by the model space block record.
        let ents = entities(&dxf);
        let owners: HashMap<&str, usize> = ents.iter().fold(HashMap::new(), |mut m, e| {
            *m.entry(value(e, 330)).or_default() += 1;
            m
        });
        assert_eq!(owners.len(), 1);
    }

    #[test]
    fn clockwise_arcs_are_swapped() {
        let dxf = write_r2000(&[sample::slotted_plate().unwrap()], &opts()).unwrap();
        let arcs: Vec<(String, String, String)> = entities(&dxf)
            .iter()
            .filter(|e| e[0].1 == "ARC")
            .map(|e| {
                (
                    value(e, 10).into(),
                    value(e, 50).into(),
                    value(e, 51).into(),
                )
            })
            .collect();
        assert!(arcs.contains(&("25.0".into(), "90.0".into(), "270.0".into())));
        assert!(arcs.contains(&("55.0".into(), "270.0".into(), "90.0".into())));
    }

    #[test]
    fn loops_as_closed_polylines_with_bulges() {
        let o = ExportOptions {
            loops_as_polylines: true,
            ..ExportOptions::default()
        };
        let dxf = write_r2000(&[sample::slotted_plate().unwrap()], &o).unwrap();
        let ents = entities(&dxf);
        assert_eq!(ents.len(), 2);
        assert!(
            ents.iter()
                .all(|e| e[0].1 == "LWPOLYLINE" && value(e, 70) == "1")
        );
        let slot = ents.iter().find(|e| value(e, 8) == LAYER_INNER).unwrap();
        assert_eq!(value(slot, 90), "4");
        // Two clockwise half circles: bulge −1.
        let bulges: Vec<&str> = slot
            .iter()
            .filter(|(c, _)| *c == 42)
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(bulges, ["-1.0", "-1.0"]);
        // Rounded corners are quarter arcs: bulge tan(22.5°).
        let dxf = write_r2000(&[sample::rounded_plate().unwrap()], &o).unwrap();
        let outer = entities(&dxf)
            .into_iter()
            .find(|e| value(e, 8) == LAYER_OUTER)
            .unwrap();
        let b = (PI / 8.0).tan();
        let expected = flatbello_core::format::fixed(b, 6);
        assert_eq!(
            outer
                .iter()
                .filter(|(c, v)| *c == 42 && *v == expected)
                .count(),
            4
        );
        // Circles become two half-turn bulges.
        let hole = entities(&dxf)
            .into_iter()
            .find(|e| value(e, 8) == LAYER_INNER)
            .unwrap();
        assert_eq!(value(&hole, 90), "2");
    }

    fn ellipse_profile() -> Profile2D {
        let tol = Tolerance::default();
        let l = Loop::new(
            vec![Edge::exact(Segment::Ellipse {
                center: Point2::new(10.0, 5.0),
                major_axis: Vec2::new(10.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                end: TAU,
                ccw: true,
            })],
            &tol,
        )
        .unwrap();
        Profile2D::new(
            l,
            vec![],
            FaceId(0),
            Frame3::WORLD,
            LengthUnit::Millimetre,
            &tol,
        )
        .unwrap()
    }

    #[test]
    fn curves_flatten_by_default_and_stay_native_on_request() {
        let flat = write_r2000(&[ellipse_profile()], &opts()).unwrap();
        let e = entities(&flat);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0][0].1, "LWPOLYLINE");
        let native = write_r2000(
            &[ellipse_profile()],
            &ExportOptions {
                native_curves: true,
                ..ExportOptions::default()
            },
        )
        .unwrap();
        let e = entities(&native);
        assert_eq!(e[0][0].1, "ELLIPSE");
        assert_eq!(value(&e[0], 11), "10.0");
        assert_eq!(value(&e[0], 40), "0.5");
        assert_eq!(value(&e[0], 41), "0.0");
        assert_eq!(value(&e[0], 42), flatbello_core::format::fixed(TAU, 6));
    }

    #[test]
    fn deterministic() {
        let p = [
            sample::rounded_plate().unwrap(),
            sample::slotted_plate().unwrap(),
        ];
        assert_eq!(
            write_r2000(&p, &opts()).unwrap(),
            write_r2000(&p, &opts()).unwrap()
        );
    }
}
