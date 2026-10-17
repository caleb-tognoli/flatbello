//! Format registry and orchestration. UI-agnostic: takes bytes, not file paths, and runs no
//! threads; callers decide where the work happens.

use core::fmt;

use flatbello_core::{
    ExportError, ExportOptions, Exporter, Face, FaceId, ImportError, ImportOptions, Importer,
    ModelGeometry, Profile2D, RenderMesh, SelectableModel, layout_side_by_side,
};
use flatbello_export_dxf::{DxfR12Exporter, DxfR2000Exporter};
use flatbello_export_svg::SvgExporter;
use flatbello_extract::{ExtractError, extract_mesh_face, extract_planar};
use flatbello_import_mesh::{ObjImporter, StlImporter};
use flatbello_import_step::{IgesImporter, StepImporter};

pub use flatbello_extract::{ExtractOptions, XAxisRule};

/// The first exporter of each kind is the default for it: DXF R2000 before R12.
static EXPORTERS: [&dyn Exporter; 3] = [&DxfR2000Exporter, &DxfR12Exporter, &SvgExporter];
static IMPORTERS: [&dyn Importer; 4] = [&StepImporter, &IgesImporter, &StlImporter, &ObjImporter];

#[derive(Debug)]
pub enum EngineError {
    UnknownFormat(String),
    /// No importer recognises the file.
    UnrecognizedFile(String),
    UnknownModel,
    UnknownFace(FaceId),
    Import(ImportError),
    Extract(ExtractError),
    Export(ExportError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat(id) => write!(f, "unknown format \"{id}\""),
            Self::UnrecognizedFile(name) => write!(f, "\"{name}\" is not a supported model file"),
            Self::UnknownModel => f.write_str("no such model"),
            Self::UnknownFace(id) => write!(f, "no face {}", id.0),
            Self::Import(e) => write!(f, "import failed: {e}"),
            Self::Extract(e) => write!(f, "extraction failed: {e}"),
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

impl From<ImportError> for EngineError {
    fn from(e: ImportError) -> Self {
        Self::Import(e)
    }
}

impl From<ExtractError> for EngineError {
    fn from(e: ExtractError) -> Self {
        Self::Extract(e)
    }
}

/// Every compiled-in exporter, in a fixed order.
pub fn exporters() -> &'static [&'static dyn Exporter] {
    &EXPORTERS
}

pub fn exporter(id: &str) -> Option<&'static dyn Exporter> {
    EXPORTERS.iter().copied().find(|e| e.id() == id)
}

/// Every compiled-in importer, in a fixed order.
pub fn importers() -> &'static [&'static dyn Importer] {
    &IMPORTERS
}

/// The importer for a file: the one claiming its extension, if the bytes look right, or else
/// the first whose sniff test accepts the bytes.
pub fn importer_for(bytes: &[u8], filename_hint: &str) -> Option<&'static dyn Importer> {
    let ext = filename_hint
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    IMPORTERS
        .iter()
        .copied()
        .find(|i| i.extensions().contains(&ext.as_str()) && i.sniff(bytes))
        .or_else(|| IMPORTERS.iter().copied().find(|i| i.sniff(bytes)))
}

/// Imports a model without keeping it. Safe to call on a worker thread.
pub fn import(
    bytes: &[u8],
    filename_hint: &str,
    options: &ImportOptions,
) -> Result<SelectableModel, EngineError> {
    let importer = importer_for(bytes, filename_hint)
        .ok_or_else(|| EngineError::UnrecognizedFile(filename_hint.into()))?;
    Ok(importer.load(bytes, options)?)
}

/// Space between profiles exported side by side, in mm.
pub const LAYOUT_GAP: f64 = 10.0;

/// Writes `profiles` with the exporter whose id is `format`.
pub fn export(
    profiles: &[Profile2D],
    format: &str,
    options: &ExportOptions,
) -> Result<Vec<u8>, EngineError> {
    let exporter = exporter(format).ok_or_else(|| EngineError::UnknownFormat(format.into()))?;
    Ok(exporter.write(profiles, options)?)
}

/// Identifies an open model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelId(u32);

/// Holds open models and runs extraction on them.
#[derive(Default)]
pub struct Engine {
    models: Vec<Option<SelectableModel>>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Imports a model and keeps it. `filename_hint` is used only to pick the importer by extension.
    pub fn open(
        &mut self,
        bytes: &[u8],
        filename_hint: &str,
        options: &ImportOptions,
    ) -> Result<ModelId, EngineError> {
        let model = import(bytes, filename_hint, options)?;
        self.insert(model)
    }

    /// Keeps a model imported with [`import`], for example on another thread.
    pub fn insert(&mut self, model: SelectableModel) -> Result<ModelId, EngineError> {
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| {
            EngineError::Import(ImportError::Unsupported("too many models".into()))
        })?);
        self.models.push(Some(model));
        Ok(id)
    }

    pub fn close(&mut self, id: ModelId) {
        if let Some(slot) = self.models.get_mut(id.0 as usize) {
            *slot = None;
        }
    }

    pub fn model(&self, id: ModelId) -> Result<&SelectableModel, EngineError> {
        self.models
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .ok_or(EngineError::UnknownModel)
    }

    pub fn faces(&self, id: ModelId) -> Result<&[Face], EngineError> {
        Ok(&self.model(id)?.faces)
    }

    pub fn render_mesh(&self, id: ModelId) -> Result<&RenderMesh, EngineError> {
        Ok(&self.model(id)?.mesh)
    }

    /// Extracts the faces, lays them out side by side with [`LAYOUT_GAP`] and writes them with
    /// the exporter `format`. The CLI and the desktop app both export through this, so they
    /// produce identical files.
    pub fn export_faces(
        &self,
        id: ModelId,
        faces: &[FaceId],
        extract: &ExtractOptions,
        format: &str,
        options: &ExportOptions,
    ) -> Result<Vec<u8>, EngineError> {
        let profiles = self.extract(id, faces, extract)?;
        let laid = layout_side_by_side(&profiles, LAYOUT_GAP, &options.tolerance);
        export(&laid, format, options)
    }

    /// One profile per requested face, in the order given.
    pub fn extract(
        &self,
        id: ModelId,
        faces: &[FaceId],
        options: &ExtractOptions,
    ) -> Result<Vec<Profile2D>, EngineError> {
        let model = self.model(id)?;
        faces
            .iter()
            .map(|&f| match &model.geometry {
                ModelGeometry::Brep(breps) => {
                    let face = breps
                        .get(f.0 as usize)
                        .filter(|b| b.id == f)
                        .ok_or(EngineError::UnknownFace(f))?;
                    Ok(extract_planar(face, model.units, options)?)
                }
                ModelGeometry::Mesh(mesh) => {
                    if f.0 as usize >= mesh.planes.len() {
                        return Err(EngineError::UnknownFace(f));
                    }
                    Ok(extract_mesh_face(mesh, f, model.units, options)?)
                }
            })
            .collect()
    }
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
        assert!(export(&p, "dxf-r2000", &opts).is_ok());
        assert!(matches!(
            export(&p, "pdf", &opts),
            Err(EngineError::UnknownFormat(_))
        ));
        assert!(matches!(
            export(&[], "svg", &opts),
            Err(EngineError::Export(ExportError::NoProfiles))
        ));
    }

    #[test]
    fn picks_importers() {
        let step = b"ISO-10303-21;\nHEADER;";
        assert_eq!(
            importer_for(step, "part.STP").map(|i| i.name()),
            Some("STEP")
        );
        // A wrong extension falls back to sniffing.
        assert_eq!(
            importer_for(step, "part.igs").map(|i| i.name()),
            Some("STEP")
        );
        assert!(importer_for(b"hello", "notes.txt").is_none());
    }

    #[test]
    fn rejects_bad_files_and_ids() {
        let mut e = Engine::new();
        assert!(matches!(
            e.open(b"hello", "notes.txt", &ImportOptions::default()),
            Err(EngineError::UnrecognizedFile(_))
        ));
        assert!(matches!(
            e.open(
                b"ISO-10303-21;\nbroken",
                "x.step",
                &ImportOptions::default()
            ),
            Err(EngineError::Import(_))
        ));
        assert!(matches!(
            e.faces(ModelId(7)),
            Err(EngineError::UnknownModel)
        ));
    }
}
