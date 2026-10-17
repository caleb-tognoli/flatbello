//! The worker protocol: requests and responses in a hand-written binary encoding, sent as
//! frames of a little-endian `u32` length followed by that many bytes.
//!
//! The app runs the engine in a child process (`--worker`) so that a crash inside OCCT on a
//! broken file takes down only the worker. [`serve`] is the worker's loop.

use core::fmt;
use std::io::{self, Read, Write};

use flatbello_core::{
    Edge, ExportOptions, Face, FaceId, FaceKind, Frame3, LengthUnit, Loop, Point2, Point3,
    Profile2D, Provenance, RenderMesh, Segment, Spline, Tolerance, Vec2, Vec3,
};

use crate::{Engine, EngineError, ExtractOptions, ModelId, XAxisRule};

/// Frames larger than this are refused, so a corrupted length cannot exhaust memory.
pub const MAX_FRAME: usize = 1 << 31;

#[derive(Clone, Debug)]
pub enum WireError {
    Truncated,
    Invalid(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("message ended early"),
            Self::Invalid(what) => write!(f, "invalid message: {what}"),
        }
    }
}

impl std::error::Error for WireError {}

/// The settings every extraction and export request carries.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub tolerance: Tolerance,
    pub x_axis: XAxisRule,
    pub native_curves: bool,
    pub loops_as_polylines: bool,
    pub snap_hole_sizes: bool,
}

impl Settings {
    pub fn extract(&self) -> ExtractOptions {
        ExtractOptions {
            tolerance: self.tolerance,
            x_axis: self.x_axis,
            snap_hole_sizes: self.snap_hole_sizes,
        }
    }

    pub fn export(&self) -> ExportOptions {
        ExportOptions {
            tolerance: self.tolerance,
            native_curves: self.native_curves,
            loops_as_polylines: self.loops_as_polylines,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Request {
    /// Imports a model, replacing the open one.
    Open {
        bytes: Vec<u8>,
        hint: String,
        units: Option<LengthUnit>,
    },
    /// Profiles of the open model's faces, laid out side by side as they would be exported.
    Extract {
        faces: Vec<FaceId>,
        settings: Settings,
    },
    Export {
        faces: Vec<FaceId>,
        format: String,
        settings: Settings,
    },
    /// Test hook: the worker aborts, as it would on a crash inside OCCT.
    Crash,
}

/// What the app keeps of an open model: enough to draw and pick it.
#[derive(Clone, Debug)]
pub struct ModelSummary {
    pub faces: Vec<Face>,
    /// The outward normal of every planar face, by face index.
    pub normals: Vec<Option<Vec3>>,
    pub mesh: RenderMesh,
    pub units: LengthUnit,
}

#[derive(Clone, Debug)]
pub enum Response {
    Opened(ModelSummary),
    Profiles(Vec<Profile2D>),
    Exported(Vec<u8>),
    Error(String),
}

// --- Encoding -----------------------------------------------------------------------------

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f64) {
        self.buf.extend_from_slice(&(v as f32).to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.len(b.len());
        self.buf.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn p2(&mut self, p: Point2) {
        self.f64(p.x);
        self.f64(p.y);
    }
    fn v2(&mut self, v: Vec2) {
        self.f64(v.x);
        self.f64(v.y);
    }
    fn p3(&mut self, p: Point3) {
        self.f64(p.x);
        self.f64(p.y);
        self.f64(p.z);
    }
    fn v3(&mut self, v: Vec3) {
        self.f64(v.x);
        self.f64(v.y);
        self.f64(v.z);
    }
    fn units(&mut self, u: LengthUnit) {
        self.u8(match u {
            LengthUnit::Millimetre => 0,
            LengthUnit::Centimetre => 1,
            LengthUnit::Metre => 2,
            LengthUnit::Inch => 3,
            LengthUnit::Foot => 4,
            LengthUnit::Unknown => 5,
        });
    }
    fn faces(&mut self, faces: &[FaceId]) {
        self.len(faces.len());
        for f in faces {
            self.u32(f.0);
        }
    }
    fn settings(&mut self, s: &Settings) {
        self.f64(s.tolerance.linear());
        self.f64(s.tolerance.angular());
        self.f64(s.tolerance.fit());
        self.u8(match s.x_axis {
            XAxisRule::LongestStraightEdge => 0,
            XAxisRule::PlaneX => 1,
        });
        self.bool(s.native_curves);
        self.bool(s.loops_as_polylines);
        self.bool(s.snap_hole_sizes);
    }
    fn segment(&mut self, s: &Segment) {
        match s {
            Segment::Line { a, b } => {
                self.u8(0);
                self.p2(*a);
                self.p2(*b);
            }
            Segment::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => {
                self.u8(1);
                self.p2(*center);
                self.f64(*radius);
                self.f64(*start_angle);
                self.f64(*end_angle);
                self.bool(*ccw);
            }
            Segment::Circle {
                center,
                radius,
                ccw,
            } => {
                self.u8(2);
                self.p2(*center);
                self.f64(*radius);
                self.bool(*ccw);
            }
            Segment::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                end,
                ccw,
            } => {
                self.u8(3);
                self.p2(*center);
                self.v2(*major_axis);
                self.f64(*ratio);
                self.f64(*start);
                self.f64(*end);
                self.bool(*ccw);
            }
            Segment::Spline(sp) => {
                self.u8(4);
                self.len(sp.degree);
                self.len(sp.knots.len());
                sp.knots.iter().for_each(|k| self.f64(*k));
                self.len(sp.control_points.len());
                sp.control_points.iter().for_each(|p| self.p2(*p));
                self.len(sp.weights.len());
                sp.weights.iter().for_each(|w| self.f64(*w));
            }
            Segment::Polyline { points } => {
                self.u8(5);
                self.len(points.len());
                points.iter().for_each(|p| self.p2(*p));
            }
        }
    }
    fn loop_(&mut self, l: &Loop) {
        self.len(l.edges().len());
        for e in l.edges() {
            self.segment(&e.segment);
            match e.provenance {
                Provenance::Exact => self.u8(0),
                Provenance::Fitted { max_deviation } => {
                    self.u8(1);
                    self.f64(max_deviation);
                }
            }
        }
    }
    fn profile(&mut self, p: &Profile2D) {
        self.u32(p.face().0);
        self.units(p.units());
        let f = p.frame();
        self.p3(f.origin());
        self.v3(f.z_axis());
        self.v3(f.x_axis());
        self.loop_(p.outer());
        self.len(p.holes().len());
        p.holes().iter().for_each(|h| self.loop_(h));
    }
    fn mesh(&mut self, m: &RenderMesh) {
        // Display only, so f32 is plenty and halves the transfer.
        self.len(m.positions().len());
        for (p, n) in m.positions().iter().zip(m.normals()) {
            for v in [p.x, p.y, p.z, n.x, n.y, n.z] {
                self.f32(v);
            }
        }
        m.face_ids().iter().for_each(|f| self.u32(f.0));
        self.len(m.indices().len());
        m.indices().iter().for_each(|i| self.u32(*i));
    }
}

pub fn encode_request(r: &Request) -> Vec<u8> {
    let mut w = Writer::default();
    match r {
        Request::Open { bytes, hint, units } => {
            w.u8(0);
            w.bytes(bytes);
            w.str(hint);
            match units {
                Some(u) => {
                    w.bool(true);
                    w.units(*u);
                }
                None => w.bool(false),
            }
        }
        Request::Extract { faces, settings } => {
            w.u8(1);
            w.faces(faces);
            w.settings(settings);
        }
        Request::Export {
            faces,
            format,
            settings,
        } => {
            w.u8(2);
            w.faces(faces);
            w.str(format);
            w.settings(settings);
        }
        Request::Crash => w.u8(3),
    }
    w.buf
}

pub fn encode_response(r: &Response) -> Vec<u8> {
    let mut w = Writer::default();
    match r {
        Response::Opened(s) => {
            w.u8(0);
            w.len(s.faces.len());
            for (f, n) in s.faces.iter().zip(&s.normals) {
                w.u32(f.id.0);
                w.u8(match f.kind {
                    FaceKind::Planar => 0,
                    FaceKind::Cylindrical => 1,
                    FaceKind::Other => 2,
                });
                w.f64(f.area);
                match n {
                    Some(n) => {
                        w.bool(true);
                        w.v3(*n);
                    }
                    None => w.bool(false),
                }
            }
            w.mesh(&s.mesh);
            w.units(s.units);
        }
        Response::Profiles(ps) => {
            w.u8(1);
            w.len(ps.len());
            ps.iter().for_each(|p| w.profile(p));
        }
        Response::Exported(b) => {
            w.u8(2);
            w.bytes(b);
        }
        Response::Error(m) => {
            w.u8(3);
            w.str(m);
        }
    }
    w.buf
}

// --- Decoding -----------------------------------------------------------------------------

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

type R<T> = Result<T, WireError>;

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        let s = self.data.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> R<[u8; N]> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> R<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::Invalid("bool")),
        }
    }
    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    /// A count, checked against the bytes left so that a bad count cannot allocate wildly.
    fn len(&mut self, min_item_size: usize) -> R<usize> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_item_size.max(1)) > self.data.len() - self.pos {
            return Err(WireError::Truncated);
        }
        Ok(n)
    }
    fn f64(&mut self) -> R<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> R<f64> {
        Ok(f64::from(f32::from_le_bytes(self.array()?)))
    }
    fn bytes(&mut self) -> R<Vec<u8>> {
        let n = self.len(1)?;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> R<String> {
        String::from_utf8(self.bytes()?).map_err(|_| WireError::Invalid("utf-8"))
    }
    fn p2(&mut self) -> R<Point2> {
        Ok(Point2::new(self.f64()?, self.f64()?))
    }
    fn v2(&mut self) -> R<Vec2> {
        Ok(Vec2::new(self.f64()?, self.f64()?))
    }
    fn p3(&mut self) -> R<Point3> {
        Ok(Point3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    fn v3(&mut self) -> R<Vec3> {
        Ok(Vec3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    fn units(&mut self) -> R<LengthUnit> {
        Ok(match self.u8()? {
            0 => LengthUnit::Millimetre,
            1 => LengthUnit::Centimetre,
            2 => LengthUnit::Metre,
            3 => LengthUnit::Inch,
            4 => LengthUnit::Foot,
            5 => LengthUnit::Unknown,
            _ => return Err(WireError::Invalid("unit")),
        })
    }
    fn faces(&mut self) -> R<Vec<FaceId>> {
        let n = self.len(4)?;
        (0..n).map(|_| self.u32().map(FaceId)).collect()
    }
    fn settings(&mut self) -> R<Settings> {
        let tolerance = Tolerance::new(self.f64()?, self.f64()?, self.f64()?)
            .map_err(|_| WireError::Invalid("tolerance"))?;
        let x_axis = match self.u8()? {
            0 => XAxisRule::LongestStraightEdge,
            1 => XAxisRule::PlaneX,
            _ => return Err(WireError::Invalid("x-axis rule")),
        };
        Ok(Settings {
            tolerance,
            x_axis,
            native_curves: self.bool()?,
            loops_as_polylines: self.bool()?,
            snap_hole_sizes: self.bool()?,
        })
    }
    fn segment(&mut self) -> R<Segment> {
        Ok(match self.u8()? {
            0 => Segment::Line {
                a: self.p2()?,
                b: self.p2()?,
            },
            1 => Segment::Arc {
                center: self.p2()?,
                radius: self.f64()?,
                start_angle: self.f64()?,
                end_angle: self.f64()?,
                ccw: self.bool()?,
            },
            2 => Segment::Circle {
                center: self.p2()?,
                radius: self.f64()?,
                ccw: self.bool()?,
            },
            3 => Segment::Ellipse {
                center: self.p2()?,
                major_axis: self.v2()?,
                ratio: self.f64()?,
                start: self.f64()?,
                end: self.f64()?,
                ccw: self.bool()?,
            },
            4 => {
                let degree = self.u32()? as usize;
                let nk = self.len(8)?;
                let knots = (0..nk).map(|_| self.f64()).collect::<R<_>>()?;
                let nc = self.len(16)?;
                let control_points = (0..nc).map(|_| self.p2()).collect::<R<_>>()?;
                let nw = self.len(8)?;
                let weights = (0..nw).map(|_| self.f64()).collect::<R<_>>()?;
                Segment::Spline(Spline {
                    degree,
                    knots,
                    control_points,
                    weights,
                })
            }
            5 => {
                let n = self.len(16)?;
                Segment::Polyline {
                    points: (0..n).map(|_| self.p2()).collect::<R<_>>()?,
                }
            }
            _ => return Err(WireError::Invalid("segment")),
        })
    }
    fn loop_(&mut self, tol: &Tolerance) -> R<Loop> {
        let n = self.len(2)?;
        let mut edges = Vec::with_capacity(n);
        for _ in 0..n {
            let segment = self.segment()?;
            let provenance = match self.u8()? {
                0 => Provenance::Exact,
                1 => Provenance::Fitted {
                    max_deviation: self.f64()?,
                },
                _ => return Err(WireError::Invalid("provenance")),
            };
            edges.push(Edge {
                segment,
                provenance,
            });
        }
        Loop::new(edges, tol).map_err(|_| WireError::Invalid("loop"))
    }
    fn profile(&mut self, tol: &Tolerance) -> R<Profile2D> {
        let face = FaceId(self.u32()?);
        let units = self.units()?;
        let (origin, z, x) = (self.p3()?, self.v3()?, self.v3()?);
        let frame = Frame3::from_normal(origin, z, x, tol).ok_or(WireError::Invalid("frame"))?;
        let outer = self.loop_(tol)?;
        let n = self.len(2)?;
        let holes = (0..n).map(|_| self.loop_(tol)).collect::<R<_>>()?;
        Profile2D::new(outer, holes, face, frame, units, tol)
            .map_err(|_| WireError::Invalid("profile"))
    }
    fn mesh(&mut self) -> R<RenderMesh> {
        let n = self.len(28)?;
        let (mut positions, mut normals) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for _ in 0..n {
            positions.push(Point3::new(self.f32()?, self.f32()?, self.f32()?));
            normals.push(Vec3::new(self.f32()?, self.f32()?, self.f32()?));
        }
        let face_ids = (0..n).map(|_| self.u32().map(FaceId)).collect::<R<_>>()?;
        let ni = self.len(4)?;
        let indices = (0..ni).map(|_| self.u32()).collect::<R<_>>()?;
        RenderMesh::new(positions, normals, face_ids, indices)
            .map_err(|_| WireError::Invalid("mesh"))
    }
}

pub fn decode_request(data: &[u8]) -> Result<Request, WireError> {
    let mut r = Reader::new(data);
    Ok(match r.u8()? {
        0 => Request::Open {
            bytes: r.bytes()?,
            hint: r.str()?,
            units: if r.bool()? { Some(r.units()?) } else { None },
        },
        1 => Request::Extract {
            faces: r.faces()?,
            settings: r.settings()?,
        },
        2 => Request::Export {
            faces: r.faces()?,
            format: r.str()?,
            settings: r.settings()?,
        },
        3 => Request::Crash,
        _ => return Err(WireError::Invalid("request")),
    })
}

/// Decodes a response. Profiles are rebuilt and revalidated with the default tolerance.
pub fn decode_response(data: &[u8]) -> Result<Response, WireError> {
    let mut r = Reader::new(data);
    let tol = Tolerance::default();
    Ok(match r.u8()? {
        0 => {
            let n = r.len(14)?;
            let (mut faces, mut normals) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for _ in 0..n {
                let id = FaceId(r.u32()?);
                let kind = match r.u8()? {
                    0 => FaceKind::Planar,
                    1 => FaceKind::Cylindrical,
                    2 => FaceKind::Other,
                    _ => return Err(WireError::Invalid("face kind")),
                };
                let area = r.f64()?;
                faces.push(Face { id, kind, area });
                normals.push(if r.bool()? { Some(r.v3()?) } else { None });
            }
            let mesh = r.mesh()?;
            Response::Opened(ModelSummary {
                faces,
                normals,
                mesh,
                units: r.units()?,
            })
        }
        1 => {
            let n = r.len(2)?;
            Response::Profiles((0..n).map(|_| r.profile(&tol)).collect::<R<_>>()?)
        }
        2 => Response::Exported(r.bytes()?),
        3 => Response::Error(r.str()?),
        _ => return Err(WireError::Invalid("response")),
    })
}

// --- Framing and the worker loop ------------------------------------------------------------

pub fn write_frame(out: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let n = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    out.write_all(&n.to_le_bytes())?;
    out.write_all(payload)?;
    out.flush()
}

/// Reads one frame, or `None` at a clean end of input.
pub fn read_frame(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0; n];
    input.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Answers requests against one open model at a time.
#[derive(Default)]
pub struct Service {
    engine: Engine,
    model: Option<ModelId>,
}

impl Service {
    pub fn handle(&mut self, request: Request) -> Response {
        match self.try_handle(request) {
            Ok(r) => r,
            Err(e) => Response::Error(e.to_string()),
        }
    }

    fn try_handle(&mut self, request: Request) -> Result<Response, EngineError> {
        match request {
            Request::Open { bytes, hint, units } => {
                let options = flatbello_core::ImportOptions {
                    units,
                    ..Default::default()
                };
                let id = self.engine.open(&bytes, &hint, &options)?;
                if let Some(old) = self.model.replace(id) {
                    self.engine.close(old);
                }
                let m = self.engine.model(id)?;
                Ok(Response::Opened(ModelSummary {
                    faces: m.faces.clone(),
                    normals: m.faces.iter().map(|f| m.face_normal(f.id)).collect(),
                    mesh: m.mesh.clone(),
                    units: m.units,
                }))
            }
            Request::Extract { faces, settings } => {
                let id = self.model.ok_or(EngineError::UnknownModel)?;
                let profiles = self.engine.extract(id, &faces, &settings.extract())?;
                Ok(Response::Profiles(flatbello_core::layout_side_by_side(
                    &profiles,
                    crate::LAYOUT_GAP,
                    &settings.tolerance,
                )))
            }
            Request::Export {
                faces,
                format,
                settings,
            } => {
                let id = self.model.ok_or(EngineError::UnknownModel)?;
                Ok(Response::Exported(self.engine.export_faces(
                    id,
                    &faces,
                    &settings.extract(),
                    &format,
                    &settings.export(),
                )?))
            }
            Request::Crash => std::process::abort(),
        }
    }
}

/// The worker's loop: answers framed requests from `input` on `output` until input ends.
pub fn serve(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::default();
    while let Some(frame) = read_frame(&mut input)? {
        let response = match decode_request(&frame) {
            Ok(request) => service.handle(request),
            Err(e) => Response::Error(e.to_string()),
        };
        write_frame(&mut output, &encode_response(&response))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbello_core::sample;

    #[test]
    fn requests_round_trip() {
        let settings = Settings {
            x_axis: XAxisRule::PlaneX,
            native_curves: true,
            ..Settings::default()
        };
        let reqs = [
            Request::Open {
                bytes: vec![1, 2, 3],
                hint: "part.stl".into(),
                units: Some(LengthUnit::Inch),
            },
            Request::Extract {
                faces: vec![FaceId(3), FaceId(9)],
                settings: settings.clone(),
            },
            Request::Export {
                faces: vec![FaceId(1)],
                format: "svg".into(),
                settings,
            },
            Request::Crash,
        ];
        for r in reqs {
            let back = decode_request(&encode_request(&r)).unwrap();
            assert_eq!(format!("{r:?}"), format!("{back:?}"));
        }
    }

    #[test]
    fn profiles_round_trip_exactly() {
        let p = vec![
            sample::rounded_plate().unwrap(),
            sample::slotted_plate().unwrap(),
        ];
        let Response::Profiles(back) =
            decode_response(&encode_response(&Response::Profiles(p.clone()))).unwrap()
        else {
            panic!("wrong response");
        };
        let opts = ExportOptions::default();
        assert_eq!(
            crate::export(&p, "dxf-r2000", &opts).unwrap(),
            crate::export(&back, "dxf-r2000", &opts).unwrap()
        );
    }

    #[test]
    fn truncated_and_garbage_messages_are_errors() {
        let full = encode_response(&Response::Profiles(vec![sample::rounded_plate().unwrap()]));
        for cut in [0, 1, 5, full.len() / 2, full.len() - 1] {
            assert!(decode_response(&full[..cut]).is_err());
        }
        assert!(decode_request(&[9]).is_err());
        // A huge count with no data behind it is refused before allocating.
        let mut bogus = vec![1u8];
        bogus.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_response(&bogus).is_err());
    }

    #[test]
    fn serve_answers_framed_requests() {
        let mut input = Vec::new();
        let step = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/generated/plate_holes.step"),
        )
        .unwrap();
        for r in [
            Request::Open {
                bytes: step,
                hint: "plate_holes.step".into(),
                units: None,
            },
            Request::Export {
                faces: vec![FaceId(0)],
                format: "svg".into(),
                settings: Settings::default(),
            },
            Request::Extract {
                faces: vec![FaceId(999)],
                settings: Settings::default(),
            },
        ] {
            write_frame(&mut input, &encode_request(&r)).unwrap();
        }
        let mut output = Vec::new();
        serve(input.as_slice(), &mut output).unwrap();
        let mut out = output.as_slice();
        let responses: Vec<Response> = std::iter::from_fn(|| read_frame(&mut out).unwrap())
            .map(|f| decode_response(&f).unwrap())
            .collect();
        assert!(matches!(&responses[0], Response::Opened(s) if !s.faces.is_empty()));
        assert!(matches!(&responses[1], Response::Exported(b) if b.starts_with(b"<?xml")));
        assert!(matches!(&responses[2], Response::Error(_)));
    }
}
