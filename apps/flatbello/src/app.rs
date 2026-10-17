//! Application state and the egui UI. Knows nothing about windows or the GPU: the 3D view is an
//! egui texture provided by the caller, which in turn reads back what the UI requested.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Color32, Pos2, Rect, Sense, Ui, Vec2};
use flatbello_core::{FaceId, FaceKind, LengthUnit, Point3, Profile2D, RenderMesh, Tolerance};
use flatbello_engine::XAxisRule;
use flatbello_engine::wire::{ModelSummary, Request, Response, Settings};

use crate::backend::{Backend, Crashed, Job, Reply};
use crate::camera::Camera;
use crate::preview;

const MODEL_EXTENSIONS: &[&str] = &["step", "stp", "iges", "igs", "stl", "obj"];
/// Units offered for files that do not declare any.
const UNIT_CHOICES: [(LengthUnit, &str); 4] = [
    (LengthUnit::Millimetre, "Millimetres"),
    (LengthUnit::Centimetre, "Centimetres"),
    (LengthUnit::Metre, "Metres"),
    (LengthUnit::Inch, "Inches"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    DxfR2000,
    DxfR12,
    Svg,
}

impl Format {
    fn id(self) -> &'static str {
        match self {
            Format::DxfR2000 => "dxf-r2000",
            Format::DxfR12 => "dxf-r12",
            Format::Svg => "svg",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::DxfR2000 | Format::DxfR12 => "dxf",
            Format::Svg => "svg",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Format::DxfR2000 => "DXF (R2000)",
            Format::DxfR12 => "DXF R12",
            Format::Svg => "SVG",
        }
    }
}

struct Loaded {
    name: String,
    stem: String,
    summary: ModelSummary,
}

/// A model being imported on a background thread.
struct Loading {
    name: String,
    path: PathBuf,
    result: Receiver<Reply>,
}

/// Asks which unit a unitless file (STL, OBJ) is in. `size` is the bounding box in file units.
struct UnitsPrompt {
    path: PathBuf,
    size: [f64; 3],
    choice: LengthUnit,
}

/// What the UI asks of the 3D view this frame, in physical pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewRequest {
    pub size: (u32, u32),
    /// The pixel to pick, when the pointer is over the view.
    pub pick: Option<(u32, u32)>,
}

pub struct App {
    backend: Backend,
    model: Option<Loaded>,
    loading: Option<Loading>,
    units_prompt: Option<UnitsPrompt>,
    selection: Vec<FaceId>,
    hovered: Option<FaceId>,
    pub camera: Camera,
    preview: Result<Vec<Profile2D>, String>,
    format: Format,
    native_curves: bool,
    closed_polylines: bool,
    x_axis: XAxisRule,
    one_file_per_face: bool,
    status: String,
    tolerance: Tolerance,
    /// A model arrived (or went away) and the 3D view must be rebuilt.
    pub new_mesh: bool,
    /// Hover or selection changed and the face highlight must be updated.
    pub states_dirty: bool,
    /// The view should be reset to frame the model.
    pub reset_camera: bool,
    pub view: ViewRequest,
    /// The face under the pointer in the faces list this frame.
    list_hover: Option<FaceId>,
}

impl Default for App {
    /// An app whose engine runs in this process.
    fn default() -> Self {
        App::new(Backend::in_process())
    }
}

impl App {
    pub fn new(backend: Backend) -> Self {
        App {
            backend,
            model: None,
            loading: None,
            units_prompt: None,
            selection: Vec::new(),
            hovered: None,
            camera: Camera::default(),
            preview: Ok(Vec::new()),
            format: Format::DxfR2000,
            native_curves: false,
            closed_polylines: false,
            x_axis: XAxisRule::LongestStraightEdge,
            one_file_per_face: false,
            status: "Open a STEP, IGES, STL or OBJ model, or drop one on the window.".into(),
            tolerance: Tolerance::default(),
            new_mesh: false,
            states_dirty: false,
            reset_camera: false,
            view: ViewRequest::default(),
            list_hover: None,
        }
    }
}

impl App {
    /// The open model's render mesh.
    pub fn mesh(&self) -> Option<&RenderMesh> {
        self.model.as_ref().map(|m| &m.summary.mesh)
    }

    /// The engine stopped: forget the model it held and say so.
    fn engine_crashed(&mut self, Crashed(why): Crashed, doing: &str) {
        if self.model.take().is_some() {
            self.new_mesh = true;
        }
        self.selection.clear();
        self.hovered = None;
        self.preview = Ok(Vec::new());
        self.states_dirty = true;
        self.status = format!(
            "The engine crashed while {doing} ({why}). It has been restarted; open the file again."
        );
    }

    fn settings(&self) -> Settings {
        Settings {
            tolerance: self.tolerance,
            x_axis: self.x_axis,
            native_curves: self.native_curves,
            loops_as_polylines: self.closed_polylines,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.loading.is_some()
    }

    /// Highlight bits for a face: 1 hovered, 2 selected.
    pub fn face_state(&self, f: FaceId) -> u32 {
        u32::from(self.hovered == Some(f)) | (u32::from(self.selection.contains(&f)) << 1)
    }

    /// Records the face under the pointer, as read back from the id buffer.
    pub fn set_hovered(&mut self, f: Option<FaceId>) {
        if self.hovered != f {
            self.hovered = f;
            self.states_dirty = true;
        }
    }

    /// Starts loading a model on a background thread.
    pub fn open_path(&mut self, path: PathBuf) {
        self.open_with_units(path, None);
    }

    /// Loads a model, reading unitless files in `units` (millimetres if `None`).
    fn open_with_units(&mut self, path: PathBuf, units: Option<LengthUnit>) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let result = self.backend.send(Job::Open {
            path: path.clone(),
            units,
        });
        self.status = format!("Loading {name}…");
        self.loading = Some(Loading { name, path, result });
    }

    /// Takes a finished background load, if any.
    pub fn poll_loading(&mut self) {
        let Some(loading) = &self.loading else {
            return;
        };
        let result = match loading.result.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(Crashed("no reply".into())),
        };
        let (name, path) = (loading.name.clone(), loading.path.clone());
        self.loading = None;
        match result {
            Ok(Response::Opened(summary)) => {
                let stem = Path::new(&name)
                    .file_stem()
                    .map_or_else(|| "profile".into(), |s| s.to_string_lossy().into_owned());
                let faces = summary.faces.len();
                self.status = format!("{name}: {faces} faces. Click a flat face to select it.");
                if summary.units == LengthUnit::Unknown {
                    self.units_prompt = Some(UnitsPrompt {
                        path,
                        size: extent(summary.mesh.positions()),
                        choice: LengthUnit::Millimetre,
                    });
                }
                self.model = Some(Loaded {
                    name,
                    stem,
                    summary,
                });
                self.selection.clear();
                self.hovered = None;
                self.new_mesh = true;
                self.states_dirty = true;
                self.update_preview();
            }
            Ok(Response::Error(e)) => self.status = format!("Could not open {name}: {e}"),
            Ok(_) => self.status = format!("Could not open {name}: unexpected reply"),
            Err(crash) => self.engine_crashed(crash, &format!("opening {name}")),
        }
    }

    /// Reloads the unitless model in `units`.
    fn confirm_units(&mut self, units: LengthUnit) {
        if let Some(prompt) = self.units_prompt.take() {
            self.open_with_units(prompt.path, Some(units));
        }
    }

    fn units_window(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.units_prompt else {
            return;
        };
        let mut confirmed = None;
        egui::Window::new("Units")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    "This file does not say which unit it uses. Choose the one it was made in:",
                );
                for (unit, label) in UNIT_CHOICES {
                    let mm = unit.millimetres_per_unit().unwrap_or(1.0);
                    let [x, y, z] = prompt.size.map(|v| v * mm);
                    ui.radio_value(
                        &mut prompt.choice,
                        unit,
                        format!("{label}: part is {x:.1} × {y:.1} × {z:.1} mm"),
                    );
                }
                if ui.button("Use these units").clicked() {
                    confirmed = Some(prompt.choice);
                }
            });
        if let Some(u) = confirmed {
            self.confirm_units(u);
        }
    }

    fn update_preview(&mut self) {
        self.states_dirty = true;
        if self.model.is_none() || self.selection.is_empty() {
            self.preview = Ok(Vec::new());
            return;
        }
        let request = Request::Extract {
            faces: self.selection.clone(),
            settings: self.settings(),
        };
        self.preview = match self.backend.call(request) {
            Ok(Response::Profiles(p)) => Ok(p),
            Ok(Response::Error(e)) => Err(e),
            Ok(_) => Err("unexpected reply".into()),
            Err(crash) => {
                self.engine_crashed(crash, "extracting faces");
                Ok(Vec::new())
            }
        };
    }

    fn face_kind(&self, f: FaceId) -> Option<FaceKind> {
        let m = self.model.as_ref()?;
        m.summary.faces.iter().find(|x| x.id == f).map(|x| x.kind)
    }

    /// Adds faces to the selection, as clicking them with Shift would.
    pub fn select_faces(&mut self, faces: &[FaceId]) {
        for f in faces {
            self.click_face(Some(*f), true);
        }
    }

    /// Selects `face` (or clears the selection for `None`). With `add`, toggles it instead.
    fn click_face(&mut self, face: Option<FaceId>, add: bool) {
        match face {
            Some(f) if self.face_kind(f) != Some(FaceKind::Planar) => {
                self.status = format!("Face {} is curved; only flat faces can be exported.", f.0);
                return;
            }
            Some(f) if add => {
                if let Some(i) = self.selection.iter().position(|&s| s == f) {
                    self.selection.remove(i);
                } else {
                    self.selection.push(f);
                }
            }
            Some(f) => self.selection = vec![f],
            None if !add => self.selection.clear(),
            None => return,
        }
        self.update_preview();
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Models (STEP, IGES, STL, OBJ)", MODEL_EXTENSIONS)
            .pick_file()
        {
            self.open_path(path);
        }
    }

    fn export_dialog(&mut self) {
        let Some(model) = &self.model else { return };
        let ext = self.format.extension();
        let default_name = match self.selection.as_slice() {
            [f] => format!("{}_face{}.{ext}", model.stem, f.0),
            _ => format!("{}.{ext}", model.stem),
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(self.format.label(), &[ext])
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        self.export_to(path);
    }

    /// Writes the selection to `path`, or one file per face next to it.
    fn export_to(&mut self, path: PathBuf) {
        let ext = self.format.extension();
        let jobs: Vec<(PathBuf, Vec<FaceId>)> =
            if self.one_file_per_face && self.selection.len() > 1 {
                let stem = path
                    .file_stem()
                    .map_or_else(|| "profile".into(), |s| s.to_string_lossy().into_owned());
                self.selection
                    .iter()
                    .map(|f| {
                        (
                            path.with_file_name(format!("{stem}_face{}.{ext}", f.0)),
                            vec![*f],
                        )
                    })
                    .collect()
            } else {
                vec![(path, self.selection.clone())]
            };
        let mut written = Vec::new();
        for (path, faces) in jobs {
            let request = Request::Export {
                faces,
                format: self.format.id().into(),
                settings: self.settings(),
            };
            let result = match self.backend.call(request) {
                Ok(Response::Exported(bytes)) => {
                    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
                }
                Ok(Response::Error(e)) => Err(e),
                Ok(_) => Err("unexpected reply".into()),
                Err(crash) => {
                    self.engine_crashed(crash, "exporting");
                    return;
                }
            };
            match result {
                Ok(()) => written.push(path.display().to_string()),
                Err(e) => {
                    self.status = format!("Export failed: {e}");
                    return;
                }
            }
        }
        self.status = format!("Wrote {}", written.join(", "));
    }

    /// Builds the whole UI. `view_texture` is the 3D view rendered last frame.
    pub fn ui(&mut self, ui: &mut Ui, view_texture: Option<egui::TextureId>) {
        self.list_hover = None;
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open…").clicked() {
                    self.open_dialog();
                }
                if ui
                    .add_enabled(self.model.is_some(), egui::Button::new("Fit view"))
                    .clicked()
                {
                    self.reset_camera = true;
                }
                ui.separator();
                if self.loading.is_some() {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
        });

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(250.0)
            .show(ui, |ui| self.sidebar(ui));

        egui::Panel::right("preview")
            .resizable(true)
            .default_size(380.0)
            .show(ui, |ui| {
                ui.heading("Preview");
                match &self.preview {
                    Ok(p) if p.is_empty() => {
                        ui.label("Select one or more flat faces.");
                    }
                    Ok(p) => preview::draw(ui, p, &self.tolerance),
                    Err(e) => {
                        ui.colored_label(Color32::from_rgb(240, 120, 110), e);
                    }
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport(ui, view_texture));
        let ctx = ui.ctx().clone();
        self.units_window(&ctx);
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.heading("Export");
        egui::ComboBox::from_label("Format")
            .selected_text(self.format.label())
            .show_ui(ui, |ui| {
                for f in [Format::DxfR2000, Format::DxfR12, Format::Svg] {
                    ui.selectable_value(&mut self.format, f, f.label());
                }
            });
        if self.format == Format::DxfR2000 {
            ui.checkbox(&mut self.closed_polylines, "Loops as closed polylines")
                .on_hover_text(
                    "Each outline and hole becomes one closed LWPOLYLINE; arcs become bulges.",
                );
            ui.checkbox(&mut self.native_curves, "Keep splines and ellipses")
                .on_hover_text("Off: curves other than arcs are written as polylines, which every cutter reads.");
        }
        ui.label("X axis");
        let before = self.x_axis;
        ui.radio_value(
            &mut self.x_axis,
            XAxisRule::LongestStraightEdge,
            "Longest straight edge",
        );
        ui.radio_value(&mut self.x_axis, XAxisRule::PlaneX, "Plane x direction");
        if self.x_axis != before {
            self.update_preview();
        }
        ui.add_enabled(
            self.selection.len() > 1,
            egui::Checkbox::new(&mut self.one_file_per_face, "One file per face"),
        );
        let can_export = self.model.is_some() && !self.selection.is_empty() && self.preview.is_ok();
        if ui
            .add_enabled(can_export, egui::Button::new("Export…"))
            .clicked()
        {
            self.export_dialog();
        }

        ui.separator();
        ui.heading("Faces");
        let Some(model) = &self.model else {
            ui.label("No model open.");
            return;
        };
        ui.label(&model.name);
        ui.label(
            egui::RichText::new("face  kind       area mm²")
                .monospace()
                .weak(),
        );
        let faces = model.summary.faces.clone();
        let mut clicked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for f in &faces {
                let kind = match f.kind {
                    FaceKind::Planar => "flat",
                    FaceKind::Cylindrical => "cylinder",
                    FaceKind::Other => "curved",
                };
                let text = format!("{:>4}  {kind:<8} {:>10.1}", f.id.0, f.area);
                let r = ui.selectable_label(
                    self.selection.contains(&f.id),
                    egui::RichText::new(text).monospace(),
                );
                if r.clicked() {
                    clicked = Some((f.id, ui.input(|i| i.modifiers.command || i.modifiers.shift)));
                }
                if r.hovered() {
                    self.list_hover = Some(f.id);
                }
            }
        });
        if let Some((f, add)) = clicked {
            self.click_face(Some(f), add);
        }
    }

    fn viewport(&mut self, ui: &mut Ui, texture: Option<egui::TextureId>) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        self.view.size = (
            (rect.width() * ppp).round() as u32,
            (rect.height() * ppp).round() as u32,
        );
        self.view.pick = response.hover_pos().map(|p| {
            let local = (p - rect.min) * ppp;
            (local.x.max(0.0) as u32, local.y.max(0.0) as u32)
        });
        if let Some(tex) = texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(tex, rect, uv, Color32::WHITE);
        }
        if self.model.is_none() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop a STEP or IGES file here",
                egui::FontId::proportional(18.0),
                Color32::from_gray(150),
            );
        }

        let (modifiers, scroll) = ui.input(|i| (i.modifiers, i.smooth_scroll_delta.y));
        let d: Vec2 = response.drag_delta() * ppp;
        let pan = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && modifiers.shift);
        if pan {
            self.camera
                .pan(f64::from(d.x), f64::from(d.y), f64::from(self.view.size.1));
        } else if response.dragged_by(egui::PointerButton::Primary) {
            self.camera.orbit(f64::from(d.x), f64::from(d.y));
        }
        if response.hovered() && scroll.abs() > 0.0 {
            self.camera.zoom(f64::from(scroll) / 50.0);
        }
        if response.clicked() {
            let add = modifiers.command || modifiers.shift;
            self.click_face(self.hovered, add);
        }
        if response.double_clicked() {
            self.reset_camera = true;
        }
        // Over the view, the id buffer decides the hovered face; elsewhere the faces list does.
        if self.view.pick.is_none() {
            self.set_hovered(self.list_hover);
        }
    }
}

/// Bounding-box size of `points`.
fn extent(points: &[Point3]) -> [f64; 3] {
    if points.is_empty() {
        return [0.0; 3];
    }
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in points {
        for (i, v) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[i] = lo[i].min(v);
            hi[i] = hi[i].max(v);
        }
    }
    [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbello_core::{ImportOptions, layout_side_by_side};
    use flatbello_engine::{Engine, ExtractOptions, LAYOUT_GAP};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/generated")
            .join(name)
    }

    fn wait(app: &mut App) {
        while app.is_busy() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.poll_loading();
        }
    }

    fn loaded(name: &str) -> App {
        let mut app = App::default();
        app.open_path(fixture(name));
        wait(&mut app);
        assert!(app.model.is_some(), "{}", app.status);
        app
    }

    fn faces(app: &App, kind: FaceKind) -> Vec<FaceId> {
        app.model
            .as_ref()
            .unwrap()
            .summary
            .faces
            .iter()
            .filter(|f| f.kind == kind)
            .map(|f| f.id)
            .collect()
    }

    #[test]
    fn loads_in_the_background_and_selects_faces() {
        let mut app = loaded("plate_holes.step");
        assert!(app.new_mesh);
        assert!(app.mesh().is_some_and(|m| m.triangle_count() > 0));
        let planar = faces(&app, FaceKind::Planar);
        app.click_face(Some(planar[0]), false);
        assert_eq!(app.selection, vec![planar[0]]);
        assert_eq!(app.face_state(planar[0]), 2);
        app.click_face(Some(planar[1]), true);
        assert_eq!(app.selection.len(), 2);
        assert_eq!(app.preview.as_ref().map(Vec::len), Ok(2));
        app.click_face(Some(planar[1]), true);
        assert_eq!(app.selection, vec![planar[0]]);
        app.click_face(None, false);
        assert!(app.selection.is_empty());
    }

    #[test]
    fn curved_faces_are_not_selectable() {
        let mut app = loaded("plate_holes.step");
        let cyl = faces(&app, FaceKind::Cylindrical)[0];
        app.click_face(Some(cyl), false);
        assert!(app.selection.is_empty());
        assert!(app.status.contains("curved"));
    }

    /// The same faces through a separate engine, as the CLI would extract them.
    fn engine_for(name: &str) -> (Engine, flatbello_engine::ModelId) {
        let mut e = Engine::new();
        let bytes = std::fs::read(fixture(name)).unwrap();
        let id = e.open(&bytes, name, &ImportOptions::default()).unwrap();
        (e, id)
    }

    #[test]
    fn preview_matches_export_layout() {
        let mut app = loaded("plate_slots.step");
        let planar: Vec<FaceId> = faces(&app, FaceKind::Planar).into_iter().take(2).collect();
        for f in &planar {
            app.click_face(Some(*f), true);
        }
        let (e, id) = engine_for("plate_slots.step");
        let expected = layout_side_by_side(
            &e.extract(id, &planar, &ExtractOptions::default()).unwrap(),
            LAYOUT_GAP,
            &app.tolerance,
        );
        let got = app.preview.as_ref().unwrap();
        assert_eq!(got.len(), expected.len());
        for (a, b) in got.iter().zip(&expected) {
            let (ba, bb) = (a.bbox(&app.tolerance), b.bbox(&app.tolerance));
            assert!(ba.min.distance(bb.min) < 1e-12 && ba.max.distance(bb.max) < 1e-12);
        }
    }

    #[test]
    fn export_matches_the_cli() {
        let mut app = loaded("plate_holes.step");
        let face = faces(&app, FaceKind::Planar)[0];
        app.click_face(Some(face), false);
        let dir = std::env::temp_dir().join(format!("flatbello-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("face.dxf");
        app.export_to(out.clone());
        let written = std::fs::read(&out).unwrap();
        let (e, id) = engine_for("plate_holes.step");
        let cli = e
            .export_faces(
                id,
                &[face],
                &ExtractOptions::default(),
                "dxf-r2000",
                &flatbello_core::ExportOptions::default(),
            )
            .unwrap();
        assert_eq!(written, cli);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unitless_meshes_ask_for_units_and_reload_scaled() {
        let mut app = loaded("plate_holes_medium.stl");
        let prompt = app.units_prompt.as_ref().expect("a units prompt");
        assert!((prompt.size[0] - 100.0).abs() < 1e-3 && (prompt.size[1] - 60.0).abs() < 1e-3);
        let max_area = |app: &App| {
            app.model
                .as_ref()
                .unwrap()
                .summary
                .faces
                .iter()
                .map(|f| f.area)
                .fold(0.0, f64::max)
        };
        let area_mm = max_area(&app);
        app.confirm_units(LengthUnit::Inch);
        assert!(app.units_prompt.is_none());
        wait(&mut app);
        assert_eq!(app.model.as_ref().unwrap().summary.units, LengthUnit::Inch);
        assert!(app.units_prompt.is_none(), "units are known now");
        assert!((max_area(&app) / area_mm - 25.4 * 25.4).abs() < 1e-6);
    }

    #[test]
    fn step_files_do_not_ask_for_units() {
        let app = loaded("plate_holes.step");
        assert!(app.units_prompt.is_none());
    }

    #[test]
    fn bad_files_report_errors() {
        let mut app = App::default();
        app.open_path(PathBuf::from("/definitely/missing.step"));
        wait(&mut app);
        assert!(app.model.is_none());
        assert!(app.status.starts_with("Could not open"), "{}", app.status);
    }
}
