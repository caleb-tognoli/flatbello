//! Core types for Flatbello: math, `Tolerance`, segments, loops, profiles and
//! the `Importer`/`Exporter` traits. No I/O.
//!
//! All geometry is `f64` and all lengths are millimetres.

pub mod angle;
pub mod bbox;
pub mod brep;
pub mod error;
pub mod format;
pub mod math;
pub mod model;
pub mod plugin;
pub mod profile;
pub mod sample;
pub mod segment;
pub mod spline;
pub mod tolerance;

pub use bbox::Bbox2;
pub use brep::{BrepEdge, BrepFace, BrepPlane, BrepWire, Curve3};
pub use error::{GeometryError, SegmentError};
pub use math::{Frame3, Mat4, Point2, Point3, Vec2, Vec3};
pub use model::{Face, FaceId, FaceKind, LengthUnit, ModelGeometry, RenderMesh, SelectableModel};
pub use plugin::{ExportError, ExportOptions, Exporter, ImportError, ImportOptions, Importer};
pub use profile::{Loop, Orientation, Profile2D, layout_side_by_side};
pub use segment::{Edge, Provenance, Segment};
pub use spline::Spline;
pub use tolerance::Tolerance;
