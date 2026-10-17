//! Hand-written DXF writer.
//!
//! R12 (`AC1009`) text DXF with `LINE`, `ARC`, `CIRCLE`, and `POLYLINE` for curves R12 cannot
//! represent. R12 has no units header: coordinates are millimetres. Outer loops go on layer
//! `CUT_OUTER`, holes on `CUT_INNER`. All entities lie at z = 0.

use flatbello_core::angle::normalize_angle;
use flatbello_core::format::{DECIMALS, fixed};
use flatbello_core::{
    Bbox2, ExportError, ExportOptions, Exporter, Loop, Point2, Profile2D, Segment, Tolerance,
};

pub const LAYER_OUTER: &str = "CUT_OUTER";
pub const LAYER_INNER: &str = "CUT_INNER";

/// Layer name and ACI colour. Distinct colours let cutter software map outer and inner cuts to
/// different settings or cut order.
const LAYERS: [(&str, u8); 3] = [("0", 7), (LAYER_OUTER, 7), (LAYER_INNER, 1)];

pub struct DxfR12Exporter;

impl Exporter for DxfR12Exporter {
    fn id(&self) -> &'static str {
        "dxf-r12"
    }

    fn name(&self) -> &'static str {
        "DXF R12"
    }

    fn extension(&self) -> &'static str {
        "dxf"
    }

    fn write(
        &self,
        profiles: &[Profile2D],
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ExportError> {
        write_r12(profiles, options).map(String::into_bytes)
    }
}

/// Accumulates group-code/value pairs: the code right-aligned in three columns, CRLF line ends.
struct Dxf {
    out: String,
}

impl Dxf {
    fn pair(&mut self, code: u16, value: &str) {
        self.out.push_str(&format!("{code:>3}\r\n{value}\r\n"));
    }

    fn real(&mut self, code: u16, value: f64) {
        self.pair(code, &fixed(value, DECIMALS));
    }

    fn int(&mut self, code: u16, value: i32) {
        self.pair(code, &value.to_string());
    }

    /// A point as codes `base`, `base + 10` and `base + 20` (x, y, z = 0).
    fn point(&mut self, base: u16, p: Point2) {
        self.real(base, p.x);
        self.real(base + 10, p.y);
        self.real(base + 20, 0.0);
    }

    fn header_point(&mut self, name: &str, p: Point2) {
        self.pair(9, name);
        self.point(10, p);
    }

    fn begin_section(&mut self, name: &str) {
        self.pair(0, "SECTION");
        self.pair(2, name);
    }

    fn end_section(&mut self) {
        self.pair(0, "ENDSEC");
    }
}

pub fn write_r12(profiles: &[Profile2D], options: &ExportOptions) -> Result<String, ExportError> {
    if profiles.is_empty() {
        return Err(ExportError::NoProfiles);
    }
    let tol = &options.tolerance;
    let bbox = profiles
        .iter()
        .fold(Bbox2::EMPTY, |b, p| b.union(&p.bbox(tol)));
    let mut d = Dxf { out: String::new() };

    d.begin_section("HEADER");
    d.pair(9, "$ACADVER");
    d.pair(1, "AC1009");
    d.header_point("$INSBASE", Point2::ORIGIN);
    d.header_point("$EXTMIN", bbox.min);
    d.header_point("$EXTMAX", bbox.max);
    d.end_section();

    d.begin_section("TABLES");
    d.pair(0, "TABLE");
    d.pair(2, "LTYPE");
    d.int(70, 1);
    d.pair(0, "LTYPE");
    d.pair(2, "CONTINUOUS");
    d.int(70, 0);
    d.pair(3, "Solid line");
    d.int(72, 65);
    d.int(73, 0);
    d.real(40, 0.0);
    d.pair(0, "ENDTAB");
    d.pair(0, "TABLE");
    d.pair(2, "LAYER");
    d.int(70, LAYERS.len() as i32);
    for (name, color) in LAYERS {
        d.pair(0, "LAYER");
        d.pair(2, name);
        d.int(70, 0);
        d.int(62, i32::from(color));
        d.pair(6, "CONTINUOUS");
    }
    d.pair(0, "ENDTAB");
    d.end_section();

    d.begin_section("ENTITIES");
    for p in profiles {
        write_loop(&mut d, p.outer(), LAYER_OUTER, tol);
        for h in p.holes() {
            write_loop(&mut d, h, LAYER_INNER, tol);
        }
    }
    d.end_section();
    d.pair(0, "EOF");
    Ok(d.out)
}

fn write_loop(d: &mut Dxf, l: &Loop, layer: &str, tol: &Tolerance) {
    for e in l.edges() {
        match &e.segment {
            Segment::Line { a, b } => {
                d.pair(0, "LINE");
                d.pair(8, layer);
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
                // DXF arcs always run counter-clockwise from code 50 to code 51, so a clockwise
                // arc is written with its ends swapped.
                let (from, to) = if *ccw {
                    (*start_angle, *end_angle)
                } else {
                    (*end_angle, *start_angle)
                };
                d.pair(0, "ARC");
                d.pair(8, layer);
                d.point(10, *center);
                d.real(40, *radius);
                d.real(50, degrees(from));
                d.real(51, degrees(to));
            }
            Segment::Circle { center, radius, .. } => {
                d.pair(0, "CIRCLE");
                d.pair(8, layer);
                d.point(10, *center);
                d.real(40, *radius);
            }
            Segment::Ellipse { .. } | Segment::Spline(_) | Segment::Polyline { .. } => {
                d.pair(0, "POLYLINE");
                d.pair(8, layer);
                d.int(66, 1);
                d.point(10, Point2::ORIGIN);
                d.int(70, 0);
                for p in e.segment.flatten(tol.fit()) {
                    d.pair(0, "VERTEX");
                    d.pair(8, layer);
                    d.point(10, p);
                }
                d.pair(0, "SEQEND");
                d.pair(8, layer);
            }
        }
    }
}

/// An angle in degrees in [0, 360), with values that would print as 360 written as 0.
fn degrees(radians: f64) -> f64 {
    let d = normalize_angle(radians).to_degrees();
    if d < 360.0 - 0.5e-6 { d } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::PI;
    use flatbello_core::{Edge, FaceId, Frame3, LengthUnit, Vec2, sample};

    fn opts() -> ExportOptions {
        ExportOptions::default()
    }

    /// The (code, value) pairs of a DXF file.
    fn pairs(dxf: &str) -> Vec<(u16, String)> {
        let lines: Vec<&str> = dxf.split("\r\n").collect();
        lines
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[code, value]| (code.trim().parse().unwrap(), value.to_string()))
            .collect()
    }

    /// Each entity as its list of pairs, starting with its (0, type) pair.
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

    fn value(entity: &[(u16, String)], code: u16) -> &str {
        &entity.iter().find(|(c, _)| *c == code).unwrap().1
    }

    fn arcs_at(dxf: &str, cx: &str, cy: &str) -> Vec<(String, String)> {
        entities(dxf)
            .iter()
            .filter(|e| e[0].1 == "ARC" && value(e, 10) == cx && value(e, 20) == cy)
            .map(|e| (value(e, 50).to_string(), value(e, 51).to_string()))
            .collect()
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(matches!(
            write_r12(&[], &opts()),
            Err(ExportError::NoProfiles)
        ));
    }

    #[test]
    fn file_structure() {
        let dxf = write_r12(&[sample::rounded_plate().unwrap()], &opts()).unwrap();
        assert!(dxf.starts_with(
            "  0\r\nSECTION\r\n  2\r\nHEADER\r\n  9\r\n$ACADVER\r\n  1\r\nAC1009\r\n"
        ));
        assert!(dxf.ends_with("  0\r\nEOF\r\n"));
        assert!(!dxf.replace("\r\n", "").contains('\n'));
        let all = pairs(&dxf);
        for layer in [LAYER_OUTER, LAYER_INNER] {
            assert!(all.iter().any(|(c, v)| *c == 2 && v == layer));
        }
        // Every entity sits at z = 0.
        assert!(
            all.iter()
                .filter(|(c, _)| (30..=31).contains(c))
                .all(|(_, v)| v == "0.0")
        );
    }

    #[test]
    fn layers_split_outer_and_holes() {
        let dxf = write_r12(&[sample::rounded_plate().unwrap()], &opts()).unwrap();
        let ents = entities(&dxf);
        let circles: Vec<_> = ents.iter().filter(|e| e[0].1 == "CIRCLE").collect();
        assert_eq!(circles.len(), 2);
        assert!(circles.iter().all(|e| value(e, 8) == LAYER_INNER));
        let outer: Vec<_> = ents.iter().filter(|e| value(e, 8) == LAYER_OUTER).collect();
        assert_eq!(outer.len(), 8);
    }

    #[test]
    fn ccw_arcs_keep_their_angles() {
        let dxf = write_r12(&[sample::rounded_plate().unwrap()], &opts()).unwrap();
        // Bottom-right corner, centre (95, 5), counter-clockwise from -90° to 0°.
        assert_eq!(
            arcs_at(&dxf, "95.0", "5.0"),
            vec![("270.0".into(), "0.0".into())]
        );
        // Top-left corner, from 90° to 180°.
        assert_eq!(
            arcs_at(&dxf, "5.0", "55.0"),
            vec![("90.0".into(), "180.0".into())]
        );
    }

    #[test]
    fn clockwise_arcs_are_written_with_swapped_ends() {
        let p = sample::slotted_plate().unwrap();
        // The slot hole is clockwise: its left arc runs 270° → 90° clockwise through 180°.
        let left = p.holes()[0]
            .edges()
            .iter()
            .find_map(|e| match e.segment {
                Segment::Arc {
                    center,
                    start_angle,
                    end_angle,
                    ccw,
                    ..
                } if center.x < 40.0 => Some((start_angle, end_angle, ccw)),
                _ => None,
            })
            .unwrap();
        assert!(!left.2);
        assert!((degrees(left.0) - 270.0).abs() < 1e-9 && (degrees(left.1) - 90.0).abs() < 1e-9);

        let dxf = write_r12(&[p], &opts()).unwrap();
        // DXF goes counter-clockwise from 50 to 51, so the left half is 90° → 270°…
        assert_eq!(
            arcs_at(&dxf, "25.0", "20.0"),
            vec![("90.0".into(), "270.0".into())]
        );
        // …and the right half is 270° → 90° through 0°.
        assert_eq!(
            arcs_at(&dxf, "55.0", "20.0"),
            vec![("270.0".into(), "90.0".into())]
        );
    }

    #[test]
    fn ellipse_becomes_a_polyline() {
        let tol = Tolerance::default();
        let ellipse = Loop::new(
            vec![Edge::exact(Segment::Ellipse {
                center: Point2::new(10.0, 5.0),
                major_axis: Vec2::new(10.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                end: 2.0 * PI,
                ccw: true,
            })],
            &tol,
        )
        .unwrap();
        let p = Profile2D::new(
            ellipse,
            vec![],
            FaceId(0),
            Frame3::WORLD,
            LengthUnit::Millimetre,
            &tol,
        )
        .unwrap();
        let dxf = write_r12(&[p], &opts()).unwrap();
        let ents = entities(&dxf);
        assert_eq!(ents[0][0].1, "POLYLINE");
        let vertices: Vec<_> = ents.iter().filter(|e| e[0].1 == "VERTEX").collect();
        assert!(vertices.len() > 8);
        assert_eq!(value(vertices[0], 10), "20.0");
        assert_eq!(ents.last().unwrap()[0].1, "SEQEND");
    }

    #[test]
    fn degrees_never_print_as_360() {
        assert!(degrees(2.0 * PI - 1e-12).abs() < 1e-15);
        assert!((degrees(-PI / 2.0) - 270.0).abs() < 1e-12);
    }

    #[test]
    fn output_is_deterministic() {
        let p = [
            sample::rounded_plate().unwrap(),
            sample::slotted_plate().unwrap(),
        ];
        assert_eq!(
            write_r12(&p, &opts()).unwrap(),
            write_r12(&p, &opts()).unwrap()
        );
    }
}
