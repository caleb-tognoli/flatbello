//! C API shim over Open Cascade (OCCT) and its safe Rust wrapper.
//!
//! The only crate in the workspace allowed to contain `unsafe`. OCCT is linked dynamically.
//! The shim catches every C++ exception and reports it as a status code; [`Shape`] turns those
//! into [`OcctError`]s.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::ptr::{self, NonNull};

use flatbello_core::{Point3, Vec3};

mod ffi {
    use core::ffi::{c_char, c_int};

    pub const FB_OK: c_int = 0;
    pub const FB_EINVAL: c_int = -1;
    pub const FB_EREAD: c_int = -2;
    pub const FB_EEMPTY: c_int = -3;
    pub const FB_EOCCT: c_int = -4;
    pub const FB_ESIZE: c_int = -5;

    pub const FB_SURFACE_PLANE: c_int = 0;
    pub const FB_SURFACE_CYLINDER: c_int = 1;

    pub const FB_CURVE_LINE: c_int = 0;
    pub const FB_CURVE_CIRCLE: c_int = 1;
    pub const FB_CURVE_ELLIPSE: c_int = 2;
    pub const FB_CURVE_BSPLINE: c_int = 3;

    #[repr(C)]
    pub struct fb_shape {
        _private: [u8; 0],
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct fb_face_info {
        pub surface: c_int,
        pub reversed: c_int,
        pub origin: [f64; 3],
        pub normal: [f64; 3],
        pub x_dir: [f64; 3],
        pub area: f64,
        pub wire_count: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct fb_wire_info {
        pub is_outer: c_int,
        pub edge_count: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct fb_edge_info {
        pub curve: c_int,
        pub reversed: c_int,
        pub degenerated: c_int,
        pub first: f64,
        pub last: f64,
        pub start: [f64; 3],
        pub end: [f64; 3],
        pub center: [f64; 3],
        pub axis: [f64; 3],
        pub x_dir: [f64; 3],
        pub radius1: f64,
        pub radius2: f64,
        pub degree: usize,
        pub pole_count: usize,
        pub knot_count: usize,
        pub rational: c_int,
    }

    unsafe extern "C" {
        pub fn fb_occt_version() -> *const c_char;
        pub fn fb_last_error() -> *const c_char;
        pub fn fb_load_step(data: *const u8, len: usize, out: *mut *mut fb_shape) -> c_int;
        pub fn fb_load_iges(data: *const u8, len: usize, out: *mut *mut fb_shape) -> c_int;
        pub fn fb_shape_free(shape: *mut fb_shape);
        pub fn fb_shape_file_unit(shape: *const fb_shape, mm_per_unit: *mut f64) -> c_int;
        pub fn fb_face_count(shape: *mut fb_shape, count: *mut usize) -> c_int;
        pub fn fb_face_info_get(shape: *mut fb_shape, face: usize, out: *mut fb_face_info)
        -> c_int;
        pub fn fb_wire_info_get(
            shape: *mut fb_shape,
            face: usize,
            wire: usize,
            out: *mut fb_wire_info,
        ) -> c_int;
        pub fn fb_edge_info_get(
            shape: *mut fb_shape,
            face: usize,
            wire: usize,
            edge: usize,
            out: *mut fb_edge_info,
        ) -> c_int;
        pub fn fb_edge_bspline(
            shape: *mut fb_shape,
            face: usize,
            wire: usize,
            edge: usize,
            poles: *mut f64,
            pole_cap: usize,
            weights: *mut f64,
            knots: *mut f64,
            knot_cap: usize,
        ) -> c_int;
        pub fn fb_edge_sample(
            shape: *mut fb_shape,
            face: usize,
            wire: usize,
            edge: usize,
            deflection: f64,
            points: *mut f64,
            cap: usize,
            count: *mut usize,
        ) -> c_int;
        pub fn fb_tessellate(shape: *mut fb_shape, deflection: f64) -> c_int;
        pub fn fb_face_mesh(
            shape: *mut fb_shape,
            face: usize,
            nodes: *mut f64,
            node_cap: usize,
            node_count: *mut usize,
            triangles: *mut u32,
            tri_cap: usize,
            tri_count: *mut usize,
        ) -> c_int;
    }

    #[cfg(feature = "fixtures")]
    unsafe extern "C" {
        pub fn fb_make_box(
            x: f64,
            y: f64,
            z: f64,
            dx: f64,
            dy: f64,
            dz: f64,
            out: *mut *mut fb_shape,
        ) -> c_int;
        pub fn fb_make_cylinder(
            base: *const f64,
            axis: *const f64,
            radius: f64,
            height: f64,
            out: *mut *mut fb_shape,
        ) -> c_int;
        pub fn fb_make_bspline_prism(
            poles: *const f64,
            n: usize,
            degree: usize,
            height: f64,
            out: *mut *mut fb_shape,
        ) -> c_int;
        pub fn fb_fuse(a: *const fb_shape, b: *const fb_shape, out: *mut *mut fb_shape) -> c_int;
        pub fn fb_cut(a: *const fb_shape, b: *const fb_shape, out: *mut *mut fb_shape) -> c_int;
        pub fn fb_write_step(shape: *const fb_shape, path: *const c_char) -> c_int;
        pub fn fb_write_iges(shape: *const fb_shape, path: *const c_char) -> c_int;
        pub fn fb_write_stl(
            shape: *mut fb_shape,
            path: *const c_char,
            deflection: f64,
            ascii: c_int,
        ) -> c_int;
    }
}

/// A failure reported by the shim, with OCCT's message.
#[derive(Clone, Debug)]
pub enum OcctError {
    InvalidArgument(String),
    /// The file could not be read or translated.
    Read(String),
    /// The file contains no shape.
    Empty(String),
    /// OCCT raised an exception.
    Occt(String),
    Other(String),
}

impl fmt::Display for OcctError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            Self::Read(m) => write!(f, "read failed: {m}"),
            Self::Empty(m) => write!(f, "empty model: {m}"),
            Self::Occt(m) => write!(f, "OCCT error: {m}"),
            Self::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for OcctError {}

fn last_error() -> String {
    // SAFETY: fb_last_error returns a valid NUL-terminated string owned by the shim's
    // thread-local storage, which stays alive until the next shim call on this thread; it is
    // copied out immediately.
    unsafe {
        let p = ffi::fb_last_error();
        if p.is_null() {
            String::new()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Converts a shim status into a result, reading the error message on failure.
fn check(rc: c_int) -> Result<(), OcctError> {
    match rc {
        ffi::FB_OK => Ok(()),
        ffi::FB_EINVAL => Err(OcctError::InvalidArgument(last_error())),
        ffi::FB_EREAD => Err(OcctError::Read(last_error())),
        ffi::FB_EEMPTY => Err(OcctError::Empty(last_error())),
        ffi::FB_EOCCT => Err(OcctError::Occt(last_error())),
        _ => Err(OcctError::Other(last_error())),
    }
}

/// The OCCT version the shim was compiled against, such as "7.9.2".
pub fn occt_version() -> String {
    // SAFETY: fb_occt_version returns a pointer to a static NUL-terminated string literal.
    unsafe { CStr::from_ptr(ffi::fb_occt_version()) }
        .to_string_lossy()
        .into_owned()
}

fn point(a: [f64; 3]) -> Point3 {
    Point3::new(a[0], a[1], a[2])
}

fn vector(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn points(flat: &[f64]) -> Vec<Point3> {
    flat.as_chunks::<3>()
        .0
        .iter()
        .map(|[x, y, z]| Point3::new(*x, *y, *z))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Plane,
    Cylinder,
    Other,
}

/// One face. For planes, `normal` points out of the solid (the face orientation is applied).
#[derive(Clone, Copy, Debug)]
pub struct FaceInfo {
    pub surface: SurfaceKind,
    pub reversed: bool,
    pub origin: Point3,
    pub normal: Vec3,
    pub x_dir: Vec3,
    pub area: f64,
    pub wire_count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct WireInfo {
    pub is_outer: bool,
    pub edge_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveKind {
    Line,
    Circle,
    Ellipse,
    BSpline,
    Other,
}

/// One edge of a wire. `center`, `axis`, `x_dir` and the radii apply to circles and ellipses;
/// `degree` and the counts to B-splines, already trimmed to `[first, last]`.
#[derive(Clone, Copy, Debug)]
pub struct EdgeInfo {
    pub curve: CurveKind,
    pub reversed: bool,
    pub degenerated: bool,
    pub first: f64,
    pub last: f64,
    pub start: Point3,
    pub end: Point3,
    pub center: Point3,
    pub axis: Vec3,
    pub x_dir: Vec3,
    pub radius1: f64,
    pub radius2: f64,
    pub degree: usize,
    pub pole_count: usize,
    pub knot_count: usize,
    pub rational: bool,
}

/// A trimmed B-spline: knots expanded by multiplicity, weights empty unless rational.
#[derive(Clone, Debug)]
pub struct BSplineData {
    pub poles: Vec<Point3>,
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
}

/// A face's triangles, wound counter-clockwise seen from outside.
#[derive(Clone, Debug, Default)]
pub struct FaceMesh {
    pub nodes: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
}

/// An OCCT shape owned through the shim. Faces, wires and edges are addressed by index.
pub struct Shape {
    ptr: NonNull<ffi::fb_shape>,
}

// SAFETY: the shape exclusively owns its OCCT data (no other handle refers to it), so moving it
// to another thread is sound. It is not Sync: the shim fills per-face caches through `&self`.
unsafe impl Send for Shape {}

impl Drop for Shape {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from a successful shim constructor and is freed exactly once here.
        unsafe { ffi::fb_shape_free(self.ptr.as_ptr()) }
    }
}

impl Shape {
    /// Wraps the result of a shim constructor.
    fn from_raw(rc: c_int, raw: *mut ffi::fb_shape) -> Result<Shape, OcctError> {
        check(rc)?;
        NonNull::new(raw)
            .map(|ptr| Shape { ptr })
            .ok_or_else(|| OcctError::Other("shim returned no shape".into()))
    }

    pub fn load_step(bytes: &[u8]) -> Result<Shape, OcctError> {
        let mut raw = ptr::null_mut();
        // SAFETY: `bytes` is valid for `bytes.len()` reads for the duration of the call, and
        // `raw` is a valid out-pointer.
        let rc = unsafe { ffi::fb_load_step(bytes.as_ptr(), bytes.len(), &mut raw) };
        Self::from_raw(rc, raw)
    }

    pub fn load_iges(bytes: &[u8]) -> Result<Shape, OcctError> {
        let mut raw = ptr::null_mut();
        // SAFETY: as in `load_step`.
        let rc = unsafe { ffi::fb_load_iges(bytes.as_ptr(), bytes.len(), &mut raw) };
        Self::from_raw(rc, raw)
    }

    fn raw(&self) -> *mut ffi::fb_shape {
        self.ptr.as_ptr()
    }

    /// Millimetres per unit of the length unit the file declares (1 if none).
    pub fn file_mm_per_unit(&self) -> Result<f64, OcctError> {
        let mut v = 0.0;
        // SAFETY: `self.raw()` is a live shape and `v` a valid out-pointer.
        check(unsafe { ffi::fb_shape_file_unit(self.raw(), &mut v) })?;
        Ok(v)
    }

    pub fn face_count(&self) -> Result<usize, OcctError> {
        let mut n = 0;
        // SAFETY: `self.raw()` is a live shape and `n` a valid out-pointer.
        check(unsafe { ffi::fb_face_count(self.raw(), &mut n) })?;
        Ok(n)
    }

    pub fn face_info(&self, face: usize) -> Result<FaceInfo, OcctError> {
        let mut i = ffi::fb_face_info::default();
        // SAFETY: `self.raw()` is a live shape and `i` a valid out-pointer; the shim checks
        // the index.
        check(unsafe { ffi::fb_face_info_get(self.raw(), face, &mut i) })?;
        Ok(FaceInfo {
            surface: match i.surface {
                ffi::FB_SURFACE_PLANE => SurfaceKind::Plane,
                ffi::FB_SURFACE_CYLINDER => SurfaceKind::Cylinder,
                _ => SurfaceKind::Other,
            },
            reversed: i.reversed != 0,
            origin: point(i.origin),
            normal: vector(i.normal),
            x_dir: vector(i.x_dir),
            area: i.area,
            wire_count: i.wire_count,
        })
    }

    pub fn wire_info(&self, face: usize, wire: usize) -> Result<WireInfo, OcctError> {
        let mut i = ffi::fb_wire_info::default();
        // SAFETY: as in `face_info`.
        check(unsafe { ffi::fb_wire_info_get(self.raw(), face, wire, &mut i) })?;
        Ok(WireInfo {
            is_outer: i.is_outer != 0,
            edge_count: i.edge_count,
        })
    }

    pub fn edge_info(&self, face: usize, wire: usize, edge: usize) -> Result<EdgeInfo, OcctError> {
        let mut i = ffi::fb_edge_info::default();
        // SAFETY: as in `face_info`.
        check(unsafe { ffi::fb_edge_info_get(self.raw(), face, wire, edge, &mut i) })?;
        Ok(EdgeInfo {
            curve: match i.curve {
                ffi::FB_CURVE_LINE => CurveKind::Line,
                ffi::FB_CURVE_CIRCLE => CurveKind::Circle,
                ffi::FB_CURVE_ELLIPSE => CurveKind::Ellipse,
                ffi::FB_CURVE_BSPLINE => CurveKind::BSpline,
                _ => CurveKind::Other,
            },
            reversed: i.reversed != 0,
            degenerated: i.degenerated != 0,
            first: i.first,
            last: i.last,
            start: point(i.start),
            end: point(i.end),
            center: point(i.center),
            axis: vector(i.axis),
            x_dir: vector(i.x_dir),
            radius1: i.radius1,
            radius2: i.radius2,
            degree: i.degree,
            pole_count: i.pole_count,
            knot_count: i.knot_count,
            rational: i.rational != 0,
        })
    }

    /// The trimmed B-spline of a [`CurveKind::BSpline`] edge described by `info`.
    pub fn edge_bspline(
        &self,
        face: usize,
        wire: usize,
        edge: usize,
        info: &EdgeInfo,
    ) -> Result<BSplineData, OcctError> {
        let n = info.pole_count;
        let mut poles = vec![0.0; 3 * n];
        let mut weights = vec![0.0; n];
        let mut knots = vec![0.0; info.knot_count];
        // SAFETY: the buffers hold 3n, n and knot_count doubles, matching the capacities
        // passed; the shim writes no more than that and checks them against the curve.
        check(unsafe {
            ffi::fb_edge_bspline(
                self.raw(),
                face,
                wire,
                edge,
                poles.as_mut_ptr(),
                n,
                weights.as_mut_ptr(),
                knots.as_mut_ptr(),
                knots.len(),
            )
        })?;
        if !info.rational {
            weights.clear();
        }
        Ok(BSplineData {
            poles: points(&poles),
            weights,
            knots,
        })
    }

    /// Points along an edge from its first to its last parameter, within `deflection` mm.
    pub fn edge_sample(
        &self,
        face: usize,
        wire: usize,
        edge: usize,
        deflection: f64,
    ) -> Result<Vec<Point3>, OcctError> {
        let mut count = 0;
        // SAFETY: a null buffer with capacity 0 only asks for the count.
        let rc = unsafe {
            ffi::fb_edge_sample(
                self.raw(),
                face,
                wire,
                edge,
                deflection,
                ptr::null_mut(),
                0,
                &mut count,
            )
        };
        if rc != ffi::FB_ESIZE {
            check(rc)?;
        }
        let mut buf = vec![0.0; 3 * count];
        // SAFETY: `buf` holds 3 * count doubles, matching the capacity passed.
        check(unsafe {
            ffi::fb_edge_sample(
                self.raw(),
                face,
                wire,
                edge,
                deflection,
                buf.as_mut_ptr(),
                count,
                &mut count,
            )
        })?;
        buf.truncate(3 * count);
        Ok(points(&buf))
    }

    /// Meshes every face with a linear deflection in mm.
    pub fn tessellate(&self, deflection: f64) -> Result<(), OcctError> {
        // SAFETY: `self.raw()` is a live shape.
        check(unsafe { ffi::fb_tessellate(self.raw(), deflection) })
    }

    /// The triangles of a face meshed by [`Shape::tessellate`]; empty if it has none.
    pub fn face_mesh(&self, face: usize) -> Result<FaceMesh, OcctError> {
        let (mut nn, mut nt) = (0, 0);
        // SAFETY: null buffers with capacity 0 only ask for the counts.
        let rc = unsafe {
            ffi::fb_face_mesh(
                self.raw(),
                face,
                ptr::null_mut(),
                0,
                &mut nn,
                ptr::null_mut(),
                0,
                &mut nt,
            )
        };
        if rc != ffi::FB_ESIZE {
            check(rc)?;
        }
        if nn == 0 || nt == 0 {
            return Ok(FaceMesh::default());
        }
        let mut nodes = vec![0.0; 3 * nn];
        let mut tris = vec![0u32; 3 * nt];
        // SAFETY: the buffers hold 3 * nn doubles and 3 * nt indices, matching the capacities.
        check(unsafe {
            ffi::fb_face_mesh(
                self.raw(),
                face,
                nodes.as_mut_ptr(),
                nn,
                &mut nn,
                tris.as_mut_ptr(),
                nt,
                &mut nt,
            )
        })?;
        Ok(FaceMesh {
            nodes: points(&nodes),
            triangles: tris.as_chunks::<3>().0.to_vec(),
        })
    }
}

/// Fixture-only construction, used by tools/gen-fixtures.
#[cfg(feature = "fixtures")]
impl Shape {
    pub fn make_box(min: Point3, size: Vec3) -> Result<Shape, OcctError> {
        let mut raw = ptr::null_mut();
        // SAFETY: plain values and a valid out-pointer.
        let rc = unsafe { ffi::fb_make_box(min.x, min.y, min.z, size.x, size.y, size.z, &mut raw) };
        Self::from_raw(rc, raw)
    }

    pub fn make_cylinder(
        base: Point3,
        axis: Vec3,
        radius: f64,
        height: f64,
    ) -> Result<Shape, OcctError> {
        let (b, a) = ([base.x, base.y, base.z], [axis.x, axis.y, axis.z]);
        let mut raw = ptr::null_mut();
        // SAFETY: `b` and `a` are 3-element arrays alive for the call; `raw` is an out-pointer.
        let rc = unsafe { ffi::fb_make_cylinder(b.as_ptr(), a.as_ptr(), radius, height, &mut raw) };
        Self::from_raw(rc, raw)
    }

    /// A prism along +z over a clamped B-spline through `poles` (in the z = 0 plane), closed
    /// back to its start by a line.
    pub fn make_bspline_prism(
        poles: &[Point3],
        degree: usize,
        height: f64,
    ) -> Result<Shape, OcctError> {
        let flat: Vec<f64> = poles.iter().flat_map(|p| [p.x, p.y, p.z]).collect();
        let mut raw = ptr::null_mut();
        // SAFETY: `flat` holds 3 * poles.len() doubles, as the shim expects for n points.
        let rc = unsafe {
            ffi::fb_make_bspline_prism(flat.as_ptr(), poles.len(), degree, height, &mut raw)
        };
        Self::from_raw(rc, raw)
    }

    pub fn fuse(&self, other: &Shape) -> Result<Shape, OcctError> {
        let mut raw = ptr::null_mut();
        // SAFETY: both shapes are live; `raw` is an out-pointer.
        let rc = unsafe { ffi::fb_fuse(self.raw(), other.raw(), &mut raw) };
        Self::from_raw(rc, raw)
    }

    pub fn cut(&self, tool: &Shape) -> Result<Shape, OcctError> {
        let mut raw = ptr::null_mut();
        // SAFETY: both shapes are live; `raw` is an out-pointer.
        let rc = unsafe { ffi::fb_cut(self.raw(), tool.raw(), &mut raw) };
        Self::from_raw(rc, raw)
    }

    pub fn write_step(&self, path: &std::path::Path) -> Result<(), OcctError> {
        let c = path_cstring(path)?;
        // SAFETY: the shape is live and `c` is a NUL-terminated string alive for the call.
        check(unsafe { ffi::fb_write_step(self.raw(), c.as_ptr()) })
    }

    pub fn write_iges(&self, path: &std::path::Path) -> Result<(), OcctError> {
        let c = path_cstring(path)?;
        // SAFETY: as in `write_step`.
        check(unsafe { ffi::fb_write_iges(self.raw(), c.as_ptr()) })
    }

    /// Meshes the shape with `deflection` mm and writes binary (or ASCII) STL.
    pub fn write_stl(
        &self,
        path: &std::path::Path,
        deflection: f64,
        ascii: bool,
    ) -> Result<(), OcctError> {
        let c = path_cstring(path)?;
        // SAFETY: as in `write_step`.
        check(unsafe { ffi::fb_write_stl(self.raw(), c.as_ptr(), deflection, c_int::from(ascii)) })
    }
}

#[cfg(feature = "fixtures")]
fn path_cstring(path: &std::path::Path) -> Result<std::ffi::CString, OcctError> {
    let s = path
        .to_str()
        .ok_or_else(|| OcctError::InvalidArgument("path is not UTF-8".into()))?;
    std::ffi::CString::new(s).map_err(|_| OcctError::InvalidArgument("path contains NUL".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_version() {
        let v = occt_version();
        assert!(v.starts_with('7') || v.starts_with('8'), "{v}");
    }

    #[test]
    fn malformed_input_is_an_error() {
        for bytes in [&b""[..], b"garbage", b"ISO-10303-21;\nHEADER;\n"] {
            assert!(Shape::load_step(bytes).is_err());
        }
        assert!(Shape::load_iges(b"not an iges file").is_err());
    }
}
