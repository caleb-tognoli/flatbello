//! Format registry and orchestration. UI-agnostic: takes bytes, not file paths.

use core::fmt;

use flatbello_core::{ExportError, ExportOptions, Exporter, Profile2D};
use flatbello_export_dxf::DxfR12Exporter;
use flatbello_export_svg::SvgExporter;

static EXPORTERS: [&dyn Exporter; 2] = [&DxfR12Exporter, &SvgExporter];

#[derive(Debug)]
pub enum EngineError {
    UnknownFormat(String),
    Export(ExportError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat(id) => write!(f, "unknown format \"{id}\""),
            Self::Export(e) => write!(f, "export failed: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<ExportError> for EngineError {
    fn from(e: ExportError) -> Self {
        Self::Export(e)
    }
}

/// Every compiled-in exporter, in a fixed order.
pub fn exporters() -> &'static [&'static dyn Exporter] {
    &EXPORTERS
}

pub fn exporter(id: &str) -> Option<&'static dyn Exporter> {
    EXPORTERS.iter().copied().find(|e| e.id() == id)
}

/// Writes `profiles` with the exporter whose id is `format`.
pub fn export(
    profiles: &[Profile2D],
    format: &str,
    options: &ExportOptions,
) -> Result<Vec<u8>, EngineError> {
    let exporter = exporter(format).ok_or_else(|| EngineError::UnknownFormat(format.into()))?;
    Ok(exporter.write(profiles, options)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbello_core::sample;

    #[test]
    fn exporter_ids_are_unique() {
        let ids: Vec<&str> = exporters().iter().map(|e| e.id()).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "duplicate exporter id {id}");
        }
    }

    #[test]
    fn export_dispatches_by_id() {
        let p = [sample::rounded_plate().unwrap()];
        let opts = ExportOptions::default();
        let svg = export(&p, "svg", &opts).unwrap();
        let direct = flatbello_export_svg::write_svg(&p, &opts).unwrap();
        assert_eq!(svg, direct.into_bytes());
        assert!(export(&p, "dxf-r12", &opts).is_ok());
        assert!(matches!(
            export(&p, "pdf", &opts),
            Err(EngineError::UnknownFormat(_))
        ));
        assert!(matches!(
            export(&[], "svg", &opts),
            Err(EngineError::Export(ExportError::NoProfiles))
        ));
    }
}
