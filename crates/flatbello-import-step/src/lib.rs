//! STEP/IGES importer built on `flatbello-occt`.
//!
//! Loading copies everything later steps need out of OCCT: the faces with their kind and area,
//! a render mesh, and the exact boundary of every planar face (see [`flatbello_core::brep`]).

use flatbello_core::{
    BrepEdge, BrepFace, BrepPlane, BrepWire, Curve3, Face, FaceId, FaceKind, ImportError,
    ImportOptions, Importer, LengthUnit, ModelGeometry, Point3, RenderMesh, SelectableModel, Vec3,
};
use flatbello_occt::{CurveKind, OcctError, Shape, SurfaceKind};

pub struct StepImporter;
pub struct IgesImporter;

impl Importer for StepImporter {
    fn name(&self) -> &'static str {
        "STEP"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["step", "stp"]
    }

    fn sniff(&self, bytes: &[u8]) -> bool {
        let start = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let start = start.trim_ascii_start();
        start.starts_with(b"ISO-10303-21")
    }

    fn load(&self, bytes: &[u8], options: &ImportOptions) -> Result<SelectableModel, ImportError> {
        build_model(Shape::load_step(bytes).map_err(import_error)?, options)
    }
}

impl Importer for IgesImporter {
    fn name(&self) -> &'static str {
        "IGES"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["iges", "igs"]
    }

    /// IGES files are 80-column records; the first is in the Start section, flagged by an `S`
    /// in column 73.
    fn sniff(&self, bytes: &[u8]) -> bool {
        let first = bytes.split(|&b| b == b'\n').next().unwrap_or(&[]);
        first.len() >= 73 && first[72] == b'S'
    }

    fn load(&self, bytes: &[u8], options: &ImportOptions) -> Result<SelectableModel, ImportError> {
        build_model(Shape::load_iges(bytes).map_err(import_error)?, options)
    }
}

fn import_error(e: OcctError) -> ImportError {
    match e {
        OcctError::Read(m) | OcctError::Empty(m) => ImportError::Malformed(m),
        other => ImportError::Backend(other.to_string()),
    }
}

/// The closest named unit for a millimetre scale.
fn length_unit(mm_per_unit: f64) -> LengthUnit {
    [
        LengthUnit::Millimetre,
        LengthUnit::Centimetre,
        LengthUnit::Metre,
        LengthUnit::Inch,
        LengthUnit::Foot,
    ]
    .into_iter()
    .find(|u| {
        u.millimetres_per_unit()
            .is_some_and(|s| (s - mm_per_unit).abs() <= 1e-9 * s)
    })
    .unwrap_or(LengthUnit::Millimetre)
}

fn build_model(shape: Shape, options: &ImportOptions) -> Result<SelectableModel, ImportError> {
    let err = import_error;
    let units = length_unit(shape.file_mm_per_unit().map_err(err)?);
    let count = shape.face_count().map_err(err)?;
    shape.tessellate(options.mesh_deflection).map_err(err)?;

    let mut faces = Vec::with_capacity(count);
    let mut breps = Vec::with_capacity(count);
    let mut mesh = MeshBuilder::default();
    for i in 0..count {
        let id = FaceId(
            u32::try_from(i).map_err(|_| ImportError::Unsupported("too many faces".into()))?,
        );
        let info = shape.face_info(i).map_err(err)?;
        let kind = match info.surface {
            SurfaceKind::Plane => FaceKind::Planar,
            SurfaceKind::Cylinder => FaceKind::Cylindrical,
            SurfaceKind::Other => FaceKind::Other,
        };
        faces.push(Face {
            id,
            kind,
            area: info.area,
        });
        let (plane, wires) = if info.surface == SurfaceKind::Plane {
            let plane = BrepPlane {
                origin: info.origin,
                normal: info.normal,
                x_dir: info.x_dir,
            };
            let wires = (0..info.wire_count)
                .map(|w| read_wire(&shape, i, w, options))
                .collect::<Result<Vec<_>, _>>()?;
            (Some(plane), wires)
        } else {
            (None, Vec::new())
        };
        breps.push(BrepFace { id, plane, wires });
        mesh.add_face(id, shape.face_mesh(i).map_err(err)?);
    }
    Ok(SelectableModel {
        faces,
        mesh: mesh.finish()?,
        geometry: ModelGeometry::Brep(breps),
        units,
    })
}

fn read_wire(
    shape: &Shape,
    face: usize,
    wire: usize,
    options: &ImportOptions,
) -> Result<BrepWire, ImportError> {
    let err = import_error;
    let info = shape.wire_info(face, wire).map_err(err)?;
    let mut edges = Vec::with_capacity(info.edge_count);
    for e in 0..info.edge_count {
        let ei = shape.edge_info(face, wire, e).map_err(err)?;
        if ei.degenerated {
            continue;
        }
        let curve = match ei.curve {
            CurveKind::Line => Curve3::Line {
                start: ei.start,
                end: ei.end,
            },
            CurveKind::Circle => Curve3::Circle {
                center: ei.center,
                axis: ei.axis,
                x_dir: ei.x_dir,
                radius: ei.radius1,
                first: ei.first,
                last: ei.last,
            },
            CurveKind::Ellipse => Curve3::Ellipse {
                center: ei.center,
                axis: ei.axis,
                x_dir: ei.x_dir,
                major_radius: ei.radius1,
                minor_radius: ei.radius2,
                first: ei.first,
                last: ei.last,
            },
            CurveKind::BSpline => {
                let b = shape.edge_bspline(face, wire, e, &ei).map_err(err)?;
                Curve3::BSpline {
                    degree: ei.degree,
                    knots: b.knots,
                    poles: b.poles,
                    weights: b.weights,
                }
            }
            CurveKind::Other => {
                let deviation = options.tolerance.fit();
                Curve3::Sampled {
                    points: shape.edge_sample(face, wire, e, deviation).map_err(err)?,
                    max_deviation: deviation,
                }
            }
        };
        edges.push(BrepEdge {
            curve,
            reversed: ei.reversed,
        });
    }
    Ok(BrepWire {
        edges,
        is_outer: info.is_outer,
    })
}

/// Collects per-face triangles into one mesh. Faces never share vertices; normals are the
/// area-weighted average of a vertex's triangles within its face.
#[derive(Default)]
struct MeshBuilder {
    positions: Vec<Point3>,
    normals: Vec<Vec3>,
    face_ids: Vec<FaceId>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn add_face(&mut self, id: FaceId, mesh: flatbello_occt::FaceMesh) {
        let base = self.positions.len();
        let Ok(base32) = u32::try_from(base) else {
            return;
        };
        let mut normals = vec![Vec3::ZERO; mesh.nodes.len()];
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| i as usize);
            let (Some(&pa), Some(&pb), Some(&pc)) =
                (mesh.nodes.get(a), mesh.nodes.get(b), mesh.nodes.get(c))
            else {
                continue;
            };
            let n = (pb - pa).cross(pc - pa);
            for i in [a, b, c] {
                normals[i] = normals[i] + n;
            }
            self.indices.extend(t.map(|i| base32 + i));
        }
        self.positions.extend(&mesh.nodes);
        self.normals.extend(
            normals
                .into_iter()
                .map(|n| n.normalized().unwrap_or(Vec3::Z)),
        );
        self.face_ids
            .extend(std::iter::repeat_n(id, mesh.nodes.len()));
    }

    fn finish(self) -> Result<RenderMesh, ImportError> {
        Ok(RenderMesh::new(
            self.positions,
            self.normals,
            self.face_ids,
            self.indices,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing() {
        assert!(StepImporter.sniff(b"ISO-10303-21;\nHEADER;"));
        assert!(StepImporter.sniff(b"\xEF\xBB\xBF  \r\nISO-10303-21;"));
        assert!(!StepImporter.sniff(b"solid cube"));
        let mut iges = vec![b' '; 72];
        iges.extend_from_slice(b"S      1\n");
        assert!(IgesImporter.sniff(&iges));
        assert!(!IgesImporter.sniff(b"ISO-10303-21;"));
    }

    #[test]
    fn units_map_to_names() {
        assert_eq!(length_unit(25.4), LengthUnit::Inch);
        assert_eq!(length_unit(1.0), LengthUnit::Millimetre);
        assert_eq!(length_unit(1000.0), LengthUnit::Metre);
        assert_eq!(length_unit(0.001), LengthUnit::Millimetre);
    }

    #[test]
    fn malformed_files_are_errors() {
        let opts = ImportOptions::default();
        for bytes in [
            &b""[..],
            b"garbage",
            b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n",
        ] {
            assert!(StepImporter.load(bytes, &opts).is_err());
        }
        assert!(IgesImporter.load(b"junk", &opts).is_err());
    }
}
