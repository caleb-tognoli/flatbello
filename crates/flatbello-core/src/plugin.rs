//! The importer and exporter interfaces implemented by format crates.

use core::fmt;

use crate::model::{LengthUnit, SelectableModel};
use crate::profile::Profile2D;
use crate::{GeometryError, Tolerance};

#[derive(Clone, Debug, Default)]
pub struct ImportOptions {
    /// The unit to assume when the file does not declare one.
    pub units: Option<LengthUnit>,
    pub tolerance: Tolerance,
}

#[derive(Clone, Debug)]
pub enum ImportError {
    /// The bytes are not a valid file of this format.
    Malformed(String),
    /// The file is valid but uses a feature this importer does not support.
    Unsupported(String),
    /// The geometry library reported a failure.
    Backend(String),
    Geometry(GeometryError),
}

/// Reads one model format.
pub trait Importer: Send + Sync {
    /// A human-readable name, such as "STEP".
    fn name(&self) -> &'static str;
    /// Lower-case file extensions without the dot.
    fn extensions(&self) -> &'static [&'static str];
    /// A cheap check that the bytes look like this format.
    fn sniff(&self, bytes: &[u8]) -> bool;
    fn load(&self, bytes: &[u8], options: &ImportOptions) -> Result<SelectableModel, ImportError>;
}

#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    /// Ellipses and splines that a format cannot represent are flattened within `tolerance.fit()`.
    pub tolerance: Tolerance,
}

#[derive(Clone, Debug)]
pub enum ExportError {
    NoProfiles,
    Geometry(GeometryError),
}

/// Writes profiles in one 2D format. Output is deterministic: the same input always gives the
/// same bytes. Profiles are written where they are; position them before writing.
pub trait Exporter: Send + Sync {
    /// A stable identifier, such as "dxf-r12".
    fn id(&self) -> &'static str;
    /// A human-readable name, such as "DXF R12".
    fn name(&self) -> &'static str;
    /// The file extension without the dot.
    fn extension(&self) -> &'static str;
    fn write(
        &self,
        profiles: &[Profile2D],
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ExportError>;
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "malformed file: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::Backend(m) => write!(f, "geometry backend error: {m}"),
            Self::Geometry(e) => write!(f, "{e}"),
        }
    }
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProfiles => f.write_str("nothing to export"),
            Self::Geometry(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImportError {}
impl std::error::Error for ExportError {}

impl From<GeometryError> for ImportError {
    fn from(e: GeometryError) -> Self {
        Self::Geometry(e)
    }
}

impl From<GeometryError> for ExportError {
    fn from(e: GeometryError) -> Self {
        Self::Geometry(e)
    }
}
