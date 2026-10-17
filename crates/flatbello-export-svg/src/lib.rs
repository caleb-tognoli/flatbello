//! Hand-written SVG writer.
//!
//! The drawing is in millimetres: `width`/`height` carry `mm` and the `viewBox` matches, so one
//! user unit is 1 mm. The y axis is flipped (SVG's points down) and the drawing is moved so that
//! its bounding box starts at (0, 0). Each loop is one stroked, unfilled `<path>`.

use core::f64::consts::PI;
use core::fmt::Write as _;

use flatbello_core::angle::sweep;
use flatbello_core::format::{DECIMALS, fixed};
use flatbello_core::segment::ellipse_point;
use flatbello_core::{
    Bbox2, ExportError, ExportOptions, Exporter, Loop, Point2, Profile2D, Segment, Tolerance,
};

/// Stroke width in mm: 0.001 in, the usual "hairline" for laser software.
const STROKE_WIDTH: &str = "0.0254";

pub struct SvgExporter;

impl Exporter for SvgExporter {
    fn id(&self) -> &'static str {
        "svg"
    }

    fn name(&self) -> &'static str {
        "SVG"
    }

    fn extension(&self) -> &'static str {
        "svg"
    }

    fn write(
        &self,
        profiles: &[Profile2D],
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ExportError> {
        write_svg(profiles, options).map(String::into_bytes)
    }
}

/// Maps profile coordinates to SVG user units: shifted to the origin and flipped vertically.
struct Mapping {
    min_x: f64,
    max_y: f64,
}

impl Mapping {
    fn point(&self, p: Point2) -> String {
        format!(
            "{} {}",
            fixed(p.x - self.min_x, DECIMALS),
            fixed(self.max_y - p.y, DECIMALS)
        )
    }
}

pub fn write_svg(profiles: &[Profile2D], options: &ExportOptions) -> Result<String, ExportError> {
    if profiles.is_empty() {
        return Err(ExportError::NoProfiles);
    }
    let tol = &options.tolerance;
    let bbox = profiles
        .iter()
        .fold(Bbox2::EMPTY, |b, p| b.union(&p.bbox(tol)));
    let map = Mapping {
        min_x: bbox.min.x,
        max_y: bbox.max.y,
    };
    let (w, h) = (
        fixed(bbox.width(), DECIMALS),
        fixed(bbox.height(), DECIMALS),
    );

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" \
         width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">"
    );
    let _ = writeln!(
        out,
        "  <g fill=\"none\" stroke=\"#000000\" stroke-width=\"{STROKE_WIDTH}\">"
    );
    for (i, profile) in profiles.iter().enumerate() {
        let loops = core::iter::once(("outer".to_string(), profile.outer())).chain(
            profile
                .holes()
                .iter()
                .enumerate()
                .map(|(j, h)| (format!("hole{j}"), h)),
        );
        for (name, l) in loops {
            let _ = writeln!(
                out,
                "    <path id=\"p{i}-{name}\" fill-rule=\"evenodd\" d=\"{}\"/>",
                path_data(l, &map, tol)
            );
        }
    }
    out.push_str("  </g>\n</svg>\n");
    Ok(out)
}

fn path_data(l: &Loop, map: &Mapping, tol: &Tolerance) -> String {
    let mut cmds: Vec<String> = Vec::new();
    if let Some(first) = l.edges().first() {
        cmds.push(format!("M {}", map.point(first.segment.start_point())));
    }
    for e in l.edges() {
        segment_commands(&e.segment, map, tol, &mut cmds);
    }
    cmds.push("Z".to_string());
    cmds.join(" ")
}

/// Appends the commands that draw `seg`, starting from its start point.
fn segment_commands(seg: &Segment, map: &Mapping, tol: &Tolerance, cmds: &mut Vec<String>) {
    let r_fmt = |r: f64| fixed(r, DECIMALS);
    match seg {
        Segment::Line { b, .. } => cmds.push(format!("L {}", map.point(*b))),
        Segment::Arc {
            radius,
            start_angle,
            end_angle,
            ccw,
            ..
        } => {
            let large = u8::from(sweep(*start_angle, *end_angle, *ccw) > PI);
            let r = r_fmt(*radius);
            cmds.push(format!(
                "A {r} {r} 0 {large} {} {}",
                sweep_flag(*ccw),
                map.point(seg.end_point())
            ));
        }
        Segment::Circle {
            center,
            radius,
            ccw,
        } => {
            // A full turn needs two arcs: through the opposite point and back.
            let r = r_fmt(*radius);
            let opposite = Point2::new(center.x - radius, center.y);
            for p in [opposite, seg.end_point()] {
                cmds.push(format!(
                    "A {r} {r} 0 0 {} {}",
                    sweep_flag(*ccw),
                    map.point(p)
                ));
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
            let total = sweep(*start, *end, *ccw);
            let dir = if *ccw { 1.0 } else { -1.0 };
            // Pieces of at most half a turn, so the large-arc flag is always 0.
            let pieces = (total / PI).ceil().max(1.0) as usize;
            let rx = r_fmt(major_axis.length());
            let ry = r_fmt(major_axis.length() * ratio);
            // The y flip turns a rotation of φ into −φ; an ellipse repeats every 180°.
            let rot = fixed(
                (-major_axis.angle().to_degrees()).rem_euclid(180.0),
                DECIMALS,
            );
            for k in 1..=pieces {
                let p = if k == pieces {
                    seg.end_point()
                } else {
                    let t = start + dir * total * k as f64 / pieces as f64;
                    ellipse_point(*center, *major_axis, *ratio, t)
                };
                cmds.push(format!(
                    "A {rx} {ry} {rot} 0 {} {}",
                    sweep_flag(*ccw),
                    map.point(p)
                ));
            }
        }
        Segment::Spline(_) | Segment::Polyline { .. } => {
            for p in seg.flatten(tol.fit()).iter().skip(1) {
                cmds.push(format!("L {}", map.point(*p)));
            }
        }
    }
}

/// SVG's y axis points down and the y flip cancels it, so the drawing keeps its look:
/// counter-clockwise stays counter-clockwise on screen, which is SVG's negative-angle
/// direction (sweep-flag 0).
fn sweep_flag(ccw: bool) -> u8 {
    u8::from(!ccw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbello_core::sample;
    use flatbello_core::{Edge, FaceId, Frame3, LengthUnit, Vec2};

    fn opts() -> ExportOptions {
        ExportOptions::default()
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(matches!(
            write_svg(&[], &opts()),
            Err(ExportError::NoProfiles)
        ));
    }

    #[test]
    fn header_has_mm_size_and_matching_viewbox() {
        let svg = write_svg(&[sample::rounded_plate().unwrap()], &opts()).unwrap();
        assert!(svg.contains("width=\"100.0mm\" height=\"60.0mm\" viewBox=\"0 0 100.0 60.0\""));
    }

    #[test]
    fn flips_y_and_sets_sweep_flags() {
        let svg = write_svg(&[sample::slotted_plate().unwrap()], &opts()).unwrap();
        // Outer starts at profile (0, 0), which is the bottom-left corner: SVG (0, 40).
        assert!(svg.contains("id=\"p0-outer\" fill-rule=\"evenodd\" d=\"M 0.0 40.0 L 80.0 40.0"));
        // The slot hole is clockwise, so its arcs have sweep-flag 1.
        let hole = svg.lines().find(|l| l.contains("p0-hole0")).unwrap();
        assert_eq!(hole.matches(" 0 0 1 ").count(), 2, "{hole}");
    }

    #[test]
    fn circle_is_two_half_arcs() {
        let svg = write_svg(&[sample::rounded_plate().unwrap()], &opts()).unwrap();
        let hole = svg.lines().find(|l| l.contains("p0-hole0")).unwrap();
        // Circle at (20, 30) r 5, clockwise: from (25, 30) via (15, 30) and back.
        assert!(
            hole.contains(
                "d=\"M 25.0 30.0 A 5.0 5.0 0 0 1 15.0 30.0 A 5.0 5.0 0 0 1 25.0 30.0 Z\""
            ),
            "{hole}"
        );
    }

    #[test]
    fn ellipse_uses_elliptical_arcs() {
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
        let svg = write_svg(&[p], &opts()).unwrap();
        assert!(
            svg.contains(
                "d=\"M 20.0 5.0 A 10.0 5.0 0.0 0 0 0.0 5.0 A 10.0 5.0 0.0 0 0 20.0 5.0 Z\""
            ),
            "{svg}"
        );
    }

    #[test]
    fn output_is_deterministic() {
        let p = [
            sample::rounded_plate().unwrap(),
            sample::slotted_plate().unwrap(),
        ];
        assert_eq!(
            write_svg(&p, &opts()).unwrap(),
            write_svg(&p, &opts()).unwrap()
        );
    }
}
