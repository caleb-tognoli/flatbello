//! Error types for geometry construction and validation.

use core::fmt;

/// Why a single segment is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentError {
    /// A coordinate, radius or angle is NaN or infinite.
    NonFinite,
    /// The segment is shorter than the linear tolerance, or an arc is a full turn.
    Degenerate,
    /// The spline data is inconsistent; the message says how.
    InvalidSpline(&'static str),
}

/// Errors from building or validating geometry.
#[derive(Clone, Debug)]
pub enum GeometryError {
    InvalidTolerance,
    /// The axes given for a frame are zero, not finite, or parallel.
    DegenerateFrame,
    EmptyLoop,
    InvalidSegment {
        edge: usize,
        error: SegmentError,
    },
    /// A closed curve (circle) must be the only edge of its loop.
    ClosedCurveNotAlone {
        edge: usize,
    },
    /// The end of `edge` does not meet the start of the next edge.
    Gap {
        edge: usize,
        distance: f64,
    },
    /// The loop encloses no area.
    ZeroArea,
    HoleOutsideOuter {
        hole: usize,
    },
    NestedHoles {
        outer_hole: usize,
        inner_hole: usize,
    },
    InvalidMesh(&'static str),
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => f.write_str("non-finite value"),
            Self::Degenerate => f.write_str("degenerate segment"),
            Self::InvalidSpline(why) => write!(f, "invalid spline: {why}"),
        }
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTolerance => f.write_str("tolerances must be finite and positive"),
            Self::DegenerateFrame => f.write_str("frame axes are degenerate"),
            Self::EmptyLoop => f.write_str("loop has no edges"),
            Self::InvalidSegment { edge, error } => write!(f, "edge {edge}: {error}"),
            Self::ClosedCurveNotAlone { edge } => {
                write!(
                    f,
                    "edge {edge} is a closed curve but the loop has other edges"
                )
            }
            Self::Gap { edge, distance } => {
                write!(f, "gap of {distance} mm after edge {edge}")
            }
            Self::ZeroArea => f.write_str("loop encloses no area"),
            Self::HoleOutsideOuter { hole } => {
                write!(f, "hole {hole} is not inside the outer loop")
            }
            Self::NestedHoles {
                outer_hole,
                inner_hole,
            } => {
                write!(f, "hole {inner_hole} lies inside hole {outer_hole}")
            }
            Self::InvalidMesh(why) => write!(f, "invalid mesh: {why}"),
        }
    }
}

impl std::error::Error for SegmentError {}
impl std::error::Error for GeometryError {}
