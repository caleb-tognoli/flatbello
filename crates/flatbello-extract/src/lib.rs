//! Face extraction: planar B-rep faces and mesh region growing with arc fitting.

pub mod fit;
pub mod mesh;
pub mod planar;

pub use mesh::extract_mesh_face;
pub use planar::{ExtractError, ExtractOptions, XAxisRule, extract_planar};
