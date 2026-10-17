//! Face extraction: planar B-rep faces and mesh region growing with arc fitting.

pub mod planar;

pub use planar::{ExtractError, ExtractOptions, XAxisRule, extract_planar};
