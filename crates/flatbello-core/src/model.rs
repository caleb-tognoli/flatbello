//! Imported models: selectable faces and the render mesh.

use crate::GeometryError;
use crate::math::{Point3, Vec3};

/// Identifies a face within one model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct FaceId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceKind {
    Planar,
    Cylindrical,
    Other,
}

/// A selectable face.
///
/// The handle to the importer's underlying geometry is designed with the first importer (M2).
#[derive(Clone, Debug)]
pub struct Face {
    pub id: FaceId,
    pub kind: FaceKind,
}

/// The length unit a source file uses. Internally everything is millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthUnit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
    /// The file does not say (typical for STL and OBJ).
    Unknown,
}

impl LengthUnit {
    /// How many millimetres one unit is, or `None` if the unit is unknown.
    pub fn millimetres_per_unit(self) -> Option<f64> {
        match self {
            LengthUnit::Millimetre => Some(1.0),
            LengthUnit::Centimetre => Some(10.0),
            LengthUnit::Metre => Some(1000.0),
            LengthUnit::Inch => Some(25.4),
            LengthUnit::Foot => Some(304.8),
            LengthUnit::Unknown => None,
        }
    }
}

/// Triangles for display and picking. Vertices are not shared between faces, so every vertex
/// carries exactly one face id.
#[derive(Clone, Debug, Default)]
pub struct RenderMesh {
    positions: Vec<Point3>,
    normals: Vec<Vec3>,
    face_ids: Vec<FaceId>,
    indices: Vec<u32>,
}

impl RenderMesh {
    /// Checks that the per-vertex arrays have equal length, the indices form whole triangles,
    /// and every index refers to a vertex.
    pub fn new(
        positions: Vec<Point3>,
        normals: Vec<Vec3>,
        face_ids: Vec<FaceId>,
        indices: Vec<u32>,
    ) -> Result<Self, GeometryError> {
        if normals.len() != positions.len() || face_ids.len() != positions.len() {
            return Err(GeometryError::InvalidMesh(
                "per-vertex arrays differ in length",
            ));
        }
        if !indices.len().is_multiple_of(3) {
            return Err(GeometryError::InvalidMesh(
                "index count is not a multiple of 3",
            ));
        }
        if indices.iter().any(|&i| i as usize >= positions.len()) {
            return Err(GeometryError::InvalidMesh("index out of range"));
        }
        if !positions.iter().all(|p| p.is_finite()) || !normals.iter().all(|n| n.is_finite()) {
            return Err(GeometryError::InvalidMesh("non-finite vertex data"));
        }
        Ok(Self {
            positions,
            normals,
            face_ids,
            indices,
        })
    }

    pub fn positions(&self) -> &[Point3] {
        &self.positions
    }

    pub fn normals(&self) -> &[Vec3] {
        &self.normals
    }

    pub fn face_ids(&self) -> &[FaceId] {
        &self.face_ids
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// What an importer produces: the faces a user can pick, a mesh to draw, and the source units.
#[derive(Clone, Debug)]
pub struct SelectableModel {
    pub faces: Vec<Face>,
    pub mesh: RenderMesh,
    pub units: LengthUnit,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri() -> (Vec<Point3>, Vec<Vec3>, Vec<FaceId>) {
        (
            vec![
                Point3::ORIGIN,
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            vec![Vec3::Z; 3],
            vec![FaceId(0); 3],
        )
    }

    #[test]
    fn validates_mesh() {
        let (p, n, f) = tri();
        let m = RenderMesh::new(p.clone(), n.clone(), f.clone(), vec![0, 1, 2]).unwrap();
        assert_eq!(m.triangle_count(), 1);
        assert!(RenderMesh::new(p.clone(), n.clone(), f.clone(), vec![0, 1]).is_err());
        assert!(RenderMesh::new(p.clone(), n.clone(), f.clone(), vec![0, 1, 3]).is_err());
        assert!(RenderMesh::new(p, n[..2].to_vec(), f, vec![0, 1, 2]).is_err());
    }

    #[test]
    fn unit_scales() {
        assert_eq!(LengthUnit::Unknown.millimetres_per_unit(), None);
        let inch = LengthUnit::Inch.millimetres_per_unit().unwrap();
        assert!((inch - 25.4).abs() < 1e-12);
    }
}
