//! Plain-data B-rep description of a model's planar faces, filled by B-rep importers.
//!
//! Importers copy what extraction needs out of the geometry library when a model is loaded, so
//! extraction runs in pure Rust and no library handles outlive the import.

use crate::math::{Point3, Vec3};
use crate::model::FaceId;

/// The 3D curve of an edge, over the edge's own parameter range and in its natural direction.
#[derive(Clone, Debug)]
pub enum Curve3 {
    Line {
        start: Point3,
        end: Point3,
    },
    /// The arc `center + radius·(cos t·x_dir + sin t·(axis × x_dir))` for t from `first` to
    /// `last` (first < last). A full circle has `last − first` = 2π.
    Circle {
        center: Point3,
        axis: Vec3,
        x_dir: Vec3,
        radius: f64,
        first: f64,
        last: f64,
    },
    /// The arc `center + major·cos t·x_dir + minor·sin t·(axis × x_dir)` for t in [first, last].
    Ellipse {
        center: Point3,
        axis: Vec3,
        x_dir: Vec3,
        major_radius: f64,
        minor_radius: f64,
        first: f64,
        last: f64,
    },
    /// A B-spline already trimmed to the edge, with knots expanded by multiplicity. `weights` is
    /// empty for a non-rational curve.
    BSpline {
        degree: usize,
        knots: Vec<f64>,
        poles: Vec<Point3>,
        weights: Vec<f64>,
    },
    /// Any other curve, sampled from start to end within `max_deviation` mm.
    Sampled {
        points: Vec<Point3>,
        max_deviation: f64,
    },
}

/// An edge as used by a wire. `reversed` means the wire runs the curve from its end to its start.
#[derive(Clone, Debug)]
pub struct BrepEdge {
    pub curve: Curve3,
    pub reversed: bool,
}

/// A closed chain of edges bounding a face.
#[derive(Clone, Debug)]
pub struct BrepWire {
    pub edges: Vec<BrepEdge>,
    pub is_outer: bool,
}

/// The plane of a planar face. `normal` already points out of the solid: the face's orientation
/// has been applied.
#[derive(Clone, Copy, Debug)]
pub struct BrepPlane {
    pub origin: Point3,
    pub normal: Vec3,
    pub x_dir: Vec3,
}

/// A face's boundary data. Only planar faces carry a plane and wires.
#[derive(Clone, Debug)]
pub struct BrepFace {
    pub id: FaceId,
    pub plane: Option<BrepPlane>,
    pub wires: Vec<BrepWire>,
}
