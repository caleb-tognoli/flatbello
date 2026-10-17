//! Hand-written STL and OBJ readers.
//!
//! Loading welds the triangles, splits the mesh into planar regions (one selectable face each)
//! and builds a render mesh. Mesh files carry no units: without [`ImportOptions::units`] the
//! coordinates are taken as millimetres and the model's units are reported as unknown.

pub mod read;

use flatbello_core::{
    Face, FaceId, FaceKind, ImportError, ImportOptions, Importer, LengthUnit, ModelGeometry,
    Point3, RenderMesh, SelectableModel, Vec3,
};
use flatbello_extract::mesh::{face_areas, segment, weld};

use read::{ReadError, Soup, is_ascii_stl, is_binary_stl, read_obj, read_stl};

pub struct StlImporter;
pub struct ObjImporter;

impl Importer for StlImporter {
    fn name(&self) -> &'static str {
        "STL"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["stl"]
    }

    fn sniff(&self, bytes: &[u8]) -> bool {
        is_binary_stl(bytes) || is_ascii_stl(bytes)
    }

    fn load(&self, bytes: &[u8], options: &ImportOptions) -> Result<SelectableModel, ImportError> {
        build_model(read_stl(bytes).map_err(read_error)?, options)
    }
}

impl Importer for ObjImporter {
    fn name(&self) -> &'static str {
        "OBJ"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["obj"]
    }

    /// OBJ has no signature; accept text whose first record is a known OBJ keyword.
    fn sniff(&self, bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        text.lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .and_then(|l| l.split_whitespace().next())
            .is_some_and(|k| {
                matches!(
                    k,
                    "v" | "vn" | "vt" | "f" | "o" | "g" | "s" | "mtllib" | "usemtl"
                )
            })
    }

    fn load(&self, bytes: &[u8], options: &ImportOptions) -> Result<SelectableModel, ImportError> {
        build_model(read_obj(bytes).map_err(read_error)?, options)
    }
}

fn read_error(e: ReadError) -> ImportError {
    ImportError::Malformed(e.to_string())
}

fn build_model(soup: Soup, options: &ImportOptions) -> Result<SelectableModel, ImportError> {
    let (units, scale) = match options.units {
        Some(u) => (u, u.millimetres_per_unit().unwrap_or(1.0)),
        None => (LengthUnit::Unknown, 1.0),
    };
    let scaled: Vec<[Point3; 3]> = soup
        .triangles
        .iter()
        .map(|t| t.map(|p| Point3::new(p.x * scale, p.y * scale, p.z * scale)))
        .collect();
    let (positions, triangles) = weld(&scaled);
    if triangles.is_empty() {
        return Err(ImportError::Malformed(
            "every triangle is degenerate".into(),
        ));
    }
    let mesh = segment(positions, triangles, &options.tolerance);
    let areas = face_areas(&mesh);
    let faces = (0..mesh.planes.len())
        .map(|i| Face {
            id: FaceId(i as u32),
            kind: FaceKind::Planar,
            area: areas[i],
        })
        .collect();

    // Flat-shaded render mesh, grouped by face so that each face's vertices are contiguous.
    // Vertices are shared within a face (one normal per face) but never between faces.
    let mut order: Vec<usize> = (0..mesh.triangles.len()).collect();
    order.sort_by_key(|&t| mesh.triangle_face[t]);
    let (mut pos, mut nor, mut ids, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut local: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut current = None;
    for t in order {
        let f = mesh.triangle_face[t];
        if current != Some(f) {
            current = Some(f);
            local.clear();
        }
        let normal = mesh.planes.get(f.0 as usize).map_or(Vec3::Z, |p| p.normal);
        for &v in &mesh.triangles[t] {
            let next = u32::try_from(pos.len())
                .map_err(|_| ImportError::Unsupported("mesh too large".into()))?;
            let i = *local.entry(v).or_insert_with(|| {
                pos.push(mesh.positions[v as usize]);
                nor.push(normal);
                ids.push(f);
                next
            });
            indices.push(i);
        }
    }
    Ok(SelectableModel {
        faces,
        mesh: RenderMesh::new(pos, nor, ids, indices)?,
        geometry: ModelGeometry::Mesh(mesh),
        units,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE_OBJ: &[u8] =
        b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 4 8 7 3\nf 1 5 8 4\nf 2 3 7 6\n";

    #[test]
    fn obj_cube_has_six_faces() {
        assert!(ObjImporter.sniff(CUBE_OBJ));
        let m = ObjImporter
            .load(CUBE_OBJ, &ImportOptions::default())
            .unwrap();
        assert_eq!(m.faces.len(), 6);
        assert_eq!(m.units, LengthUnit::Unknown);
        assert_eq!(m.mesh.triangle_count(), 12);
        // Four corners per face, shared within the face only.
        assert_eq!(m.mesh.positions().len(), 24);
        assert!(m.faces.iter().all(|f| (f.area - 1.0).abs() < 1e-12));
    }

    #[test]
    fn units_scale_the_model() {
        let opts = ImportOptions {
            units: Some(LengthUnit::Inch),
            ..ImportOptions::default()
        };
        let m = ObjImporter.load(CUBE_OBJ, &opts).unwrap();
        assert_eq!(m.units, LengthUnit::Inch);
        assert!(m.faces.iter().all(|f| (f.area - 25.4 * 25.4).abs() < 1e-9));
    }

    #[test]
    fn sniffing() {
        assert!(!ObjImporter.sniff(b"ISO-10303-21;"));
        assert!(ObjImporter.sniff(b"# comment\n\no cube\nv 0 0 0\n"));
        assert!(StlImporter.sniff(b"solid x\nendsolid x\n"));
        assert!(!StlImporter.sniff(b"v 0 0 0\n"));
        assert!(
            StlImporter
                .load(b"solid x\nendsolid\n", &ImportOptions::default())
                .is_err()
        );
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::Instant;

    /// About a million triangles: a 500 × 500 grid box top and bottom plus walls, as a binary
    /// STL. Run with `cargo test --release -p flatbello-import-mesh -- --ignored --nocapture`.
    #[test]
    #[ignore = "performance check, run explicitly in release"]
    fn million_triangles() {
        let n = 500;
        let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
        for (z, flip) in [(0.0f32, true), (5.0, false)] {
            for i in 0..n {
                for j in 0..n {
                    let (x0, y0) = (i as f32 * 0.2, j as f32 * 0.2);
                    let (x1, y1) = (x0 + 0.2, y0 + 0.2);
                    let mut a = [[x0, y0, z], [x1, y0, z], [x1, y1, z]];
                    let mut b = [[x0, y0, z], [x1, y1, z], [x0, y1, z]];
                    if flip {
                        a.swap(1, 2);
                        b.swap(1, 2);
                    }
                    tris.push(a);
                    tris.push(b);
                }
            }
        }
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for t in &tris {
            bytes.extend_from_slice(&[0; 12]);
            for p in t {
                for c in p {
                    bytes.extend_from_slice(&c.to_le_bytes());
                }
            }
            bytes.extend_from_slice(&[0, 0]);
        }
        let start = Instant::now();
        let model = StlImporter.load(&bytes, &ImportOptions::default()).unwrap();
        println!(
            "{} triangles, {} faces: loaded in {:.2?}",
            tris.len(),
            model.faces.len(),
            start.elapsed()
        );
        let ModelGeometry::Mesh(mesh) = &model.geometry else {
            panic!("not a mesh")
        };
        let start = Instant::now();
        let p = flatbello_extract::extract_mesh_face(
            mesh,
            FaceId(0),
            model.units,
            &flatbello_extract::ExtractOptions::default(),
        )
        .unwrap();
        println!(
            "extracted face 0 ({} edges) in {:.2?}",
            p.outer().edges().len(),
            start.elapsed()
        );
    }
}
