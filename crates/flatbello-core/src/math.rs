//! Points, vectors, a 4×4 matrix and an orthonormal frame. Everything is `f64`; lengths are millimetres.
//!
//! These types deliberately do not implement `PartialEq`: compare geometry through [`crate::Tolerance`].

use core::ops::{Add, Mul, Neg, Sub};

use crate::Tolerance;

/// A 2D displacement.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

/// A 2D position.
#[derive(Clone, Copy, Debug, Default)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

/// A 3D displacement.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A 3D position.
#[derive(Clone, Copy, Debug, Default)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const X: Vec2 = Vec2 { x: 1.0, y: 0.0 };
    pub const Y: Vec2 = Vec2 { x: 0.0, y: 1.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dot(self, other: Vec2) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The z component of the 3D cross product; positive when `other` is counter-clockwise from `self`.
    pub fn cross(self, other: Vec2) -> f64 {
        self.x * other.y - self.y * other.x
    }

    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The unit vector in this direction, or `None` for a zero or non-finite vector.
    pub fn normalized(self) -> Option<Vec2> {
        let len = self.length();
        (len.is_finite() && len > 0.0).then(|| self * (1.0 / len))
    }

    /// This vector rotated by +90°.
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    /// The angle from the +x axis in radians, in (-π, π].
    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }

    pub fn from_angle(angle: f64) -> Vec2 {
        let (s, c) = angle.sin_cos();
        Vec2::new(c, s)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Point2 {
    pub const ORIGIN: Point2 = Point2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn to_vec(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    pub fn distance(self, other: Point2) -> f64 {
        (other - self).length()
    }

    /// The point a fraction `t` of the way from `self` to `other`.
    pub fn lerp(self, other: Point2, t: f64) -> Point2 {
        self + (other - self) * t
    }

    /// The distance from this point to the segment `a`–`b`.
    pub fn distance_to_segment(self, a: Point2, b: Point2) -> f64 {
        let ab = b - a;
        let len2 = ab.length_squared();
        if len2 > 0.0 {
            let t = ((self - a).dot(ab) / len2).clamp(0.0, 1.0);
            self.distance(a + ab * t)
        } else {
            self.distance(a)
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const X: Vec3 = Vec3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    pub const Y: Vec3 = Vec3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    pub const Z: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, other: Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Vec3) -> Vec3 {
        Vec3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    /// The unit vector in this direction, or `None` for a zero or non-finite vector.
    pub fn normalized(self) -> Option<Vec3> {
        let len = self.length();
        (len.is_finite() && len > 0.0).then(|| self * (1.0 / len))
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Point3 {
    pub const ORIGIN: Point3 = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn to_vec(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    pub fn distance(self, other: Point3) -> f64 {
        (other - self).length()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

macro_rules! vector_ops {
    ($v:ident { $($f:ident),+ }) => {
        impl Add for $v {
            type Output = $v;
            fn add(self, o: $v) -> $v {
                $v { $($f: self.$f + o.$f),+ }
            }
        }
        impl Sub for $v {
            type Output = $v;
            fn sub(self, o: $v) -> $v {
                $v { $($f: self.$f - o.$f),+ }
            }
        }
        impl Neg for $v {
            type Output = $v;
            fn neg(self) -> $v {
                $v { $($f: -self.$f),+ }
            }
        }
        impl Mul<f64> for $v {
            type Output = $v;
            fn mul(self, s: f64) -> $v {
                $v { $($f: self.$f * s),+ }
            }
        }
    };
}

macro_rules! point_ops {
    ($p:ident, $v:ident { $($f:ident),+ }) => {
        impl Add<$v> for $p {
            type Output = $p;
            fn add(self, o: $v) -> $p {
                $p { $($f: self.$f + o.$f),+ }
            }
        }
        impl Sub<$v> for $p {
            type Output = $p;
            fn sub(self, o: $v) -> $p {
                $p { $($f: self.$f - o.$f),+ }
            }
        }
        impl Sub for $p {
            type Output = $v;
            fn sub(self, o: $p) -> $v {
                $v { $($f: self.$f - o.$f),+ }
            }
        }
    };
}

vector_ops!(Vec2 { x, y });
vector_ops!(Vec3 { x, y, z });
point_ops!(Point2, Vec2 { x, y });
point_ops!(Point3, Vec3 { x, y, z });

/// A 4×4 matrix stored column-major, acting on column vectors.
#[derive(Clone, Copy, Debug)]
pub struct Mat4 {
    cols: [[f64; 4]; 4],
}

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub const fn from_cols(cols: [[f64; 4]; 4]) -> Self {
        Self { cols }
    }

    pub const fn cols(&self) -> &[[f64; 4]; 4] {
        &self.cols
    }

    pub fn translation(v: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[3] = [v.x, v.y, v.z, 1.0];
        m
    }

    /// The element in row `row` and column `col`.
    pub fn get(&self, row: usize, col: usize) -> f64 {
        self.cols[col % 4][row % 4]
    }

    pub fn transform_point(&self, p: Point3) -> Point3 {
        let r = self.apply([p.x, p.y, p.z, 1.0]);
        Point3::new(r[0], r[1], r[2])
    }

    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        let r = self.apply([v.x, v.y, v.z, 0.0]);
        Vec3::new(r[0], r[1], r[2])
    }

    fn apply(&self, v: [f64; 4]) -> [f64; 4] {
        let mut out = [0.0; 4];
        for (col, &s) in self.cols.iter().zip(v.iter()) {
            for (o, &c) in out.iter_mut().zip(col.iter()) {
                *o += c * s;
            }
        }
        out
    }
}

impl Mul for Mat4 {
    type Output = Mat4;

    fn mul(self, rhs: Mat4) -> Mat4 {
        let mut cols = [[0.0; 4]; 4];
        for (out, col) in cols.iter_mut().zip(rhs.cols.iter()) {
            *out = self.apply(*col);
        }
        Mat4 { cols }
    }
}

/// A right-handed orthonormal frame: an origin plus unit x, y and z axes.
///
/// For a planar face, z is the outward normal and the face's 2D coordinates are the frame-local x and y.
#[derive(Clone, Copy, Debug)]
pub struct Frame3 {
    origin: Point3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

impl Frame3 {
    /// The world frame: origin at zero, axes along world x, y and z.
    pub const WORLD: Frame3 = Frame3 {
        origin: Point3::ORIGIN,
        x: Vec3::X,
        y: Vec3::Y,
        z: Vec3::Z,
    };

    /// Builds a frame with z along `normal` and x along the part of `x_hint` perpendicular to it.
    ///
    /// Returns `None` if either vector is zero or not finite, or if `x_hint` is within the angular
    /// tolerance of `normal`.
    pub fn from_normal(
        origin: Point3,
        normal: Vec3,
        x_hint: Vec3,
        tol: &Tolerance,
    ) -> Option<Frame3> {
        if !origin.is_finite() {
            return None;
        }
        let z = normal.normalized()?;
        let hint = x_hint.normalized()?;
        let perp = hint - z * hint.dot(z);
        // |perp| is the sine of the angle between the hint and the normal.
        if perp.length() <= tol.angular().sin() {
            return None;
        }
        let x = perp.normalized()?;
        let y = z.cross(x);
        Some(Frame3 { origin, x, y, z })
    }

    pub fn origin(&self) -> Point3 {
        self.origin
    }

    pub fn x_axis(&self) -> Vec3 {
        self.x
    }

    pub fn y_axis(&self) -> Vec3 {
        self.y
    }

    pub fn z_axis(&self) -> Vec3 {
        self.z
    }

    /// World coordinates → frame-local coordinates.
    pub fn to_local(&self, p: Point3) -> Point3 {
        let d = p - self.origin;
        Point3::new(d.dot(self.x), d.dot(self.y), d.dot(self.z))
    }

    /// World coordinates → frame-local x and y, dropping the distance along z.
    pub fn to_local_2d(&self, p: Point3) -> Point2 {
        let d = p - self.origin;
        Point2::new(d.dot(self.x), d.dot(self.y))
    }

    /// Frame-local coordinates → world coordinates.
    pub fn to_world(&self, p: Point3) -> Point3 {
        self.origin + self.x * p.x + self.y * p.y + self.z * p.z
    }

    /// The same frame with its origin moved so that every local 2D coordinate shifts by `offset`.
    pub fn shifted_local(&self, offset: Vec2) -> Frame3 {
        Frame3 {
            origin: self.origin - (self.x * offset.x + self.y * offset.y),
            ..*self
        }
    }

    /// The matrix mapping frame-local coordinates to world coordinates.
    pub fn local_to_world(&self) -> Mat4 {
        let (o, x, y, z) = (self.origin, self.x, self.y, self.z);
        Mat4::from_cols([
            [x.x, x.y, x.z, 0.0],
            [y.x, y.y, y.z, 0.0],
            [z.x, z.y, z.z, 0.0],
            [o.x, o.y, o.z, 1.0],
        ])
    }

    /// The matrix mapping world coordinates to frame-local coordinates.
    pub fn world_to_local(&self) -> Mat4 {
        let (x, y, z) = (self.x, self.y, self.z);
        let o = self.origin.to_vec();
        Mat4::from_cols([
            [x.x, y.x, z.x, 0.0],
            [x.y, y.y, z.y, 0.0],
            [x.z, y.z, z.z, 0.0],
            [-o.dot(x), -o.dot(y), -o.dot(z), 1.0],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= EPS
    }

    fn close3(a: Point3, b: Point3) -> bool {
        a.distance(b) <= EPS
    }

    #[test]
    fn vec2_basics() {
        let a = Vec2::new(3.0, 4.0);
        let b = Vec2::new(-1.0, 2.0);
        assert!(close(a.length(), 5.0));
        assert!(close(a.dot(b), 5.0));
        assert!(close(a.cross(b), 10.0));
        assert!(close(Vec2::X.cross(Vec2::Y), 1.0));
        assert!(close(a.perp().dot(a), 0.0));
        assert!(close(Vec2::from_angle(a.angle()).cross(a), 0.0));
        let n = a.normalized().unwrap();
        assert!(close(n.length(), 1.0));
        assert!(Vec2::ZERO.normalized().is_none());
        assert!(Vec2::new(f64::NAN, 0.0).normalized().is_none());
    }

    #[test]
    fn point_vector_arithmetic() {
        let p = Point2::new(1.0, 2.0);
        let q = Point2::new(4.0, 6.0);
        assert!(close((q - p).length(), 5.0));
        let m = p.lerp(q, 0.5);
        assert!(close(m.x, 2.5) && close(m.y, 4.0));
        assert!(close(
            Point2::new(2.0, 5.0).distance_to_segment(p, Point2::new(3.0, 2.0)),
            3.0
        ));
        assert!(close(
            Point2::new(5.0, 2.0).distance_to_segment(p, Point2::new(3.0, 2.0)),
            2.0
        ));
        assert!(close(q.distance_to_segment(p, p), 5.0));
    }

    #[test]
    fn vec3_cross_is_right_handed() {
        let z = Vec3::X.cross(Vec3::Y);
        assert!(close(z.z, 1.0) && close(z.x, 0.0) && close(z.y, 0.0));
        assert!(close(Vec3::Y.cross(Vec3::Z).x, 1.0));
    }

    #[test]
    fn frame_is_orthonormal_and_right_handed() {
        let tol = Tolerance::default();
        let f = Frame3::from_normal(
            Point3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::new(1.0, 1.0, 0.5),
            &tol,
        )
        .unwrap();
        for (a, b) in [(f.x, f.y), (f.y, f.z), (f.z, f.x)] {
            assert!(close(a.dot(b), 0.0));
        }
        for v in [f.x, f.y, f.z] {
            assert!(close(v.length(), 1.0));
        }
        assert!(close(f.x.cross(f.y).dot(f.z), 1.0));
        assert!(close(f.z.z, -1.0));
        assert!(close(f.x.y, f.x.x));
    }

    #[test]
    fn frame_rejects_degenerate_input() {
        let tol = Tolerance::default();
        let o = Point3::ORIGIN;
        assert!(Frame3::from_normal(o, Vec3::ZERO, Vec3::X, &tol).is_none());
        assert!(Frame3::from_normal(o, Vec3::Z, Vec3::ZERO, &tol).is_none());
        assert!(Frame3::from_normal(o, Vec3::Z, Vec3::new(0.0, 0.001, 1.0), &tol).is_none());
        assert!(Frame3::from_normal(o, Vec3::Z, Vec3::new(0.0, 1.0, 1.0), &tol).is_some());
    }

    #[test]
    fn frame_round_trips_and_matches_matrices() {
        let tol = Tolerance::default();
        let f = Frame3::from_normal(
            Point3::new(10.0, -5.0, 2.0),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 1.0, 0.0),
            &tol,
        )
        .unwrap();
        let p = Point3::new(7.0, 8.0, -9.0);
        let local = f.to_local(p);
        assert!(close3(f.to_world(local), p));
        assert!(close3(f.world_to_local().transform_point(p), local));
        assert!(close3(f.local_to_world().transform_point(local), p));
        let m = f.local_to_world() * f.world_to_local();
        assert!(close3(m.transform_point(p), p));
        let flat = f.to_local_2d(p);
        assert!(close(flat.x, local.x) && close(flat.y, local.y));
    }

    #[test]
    fn shifted_frame_shifts_local_coordinates() {
        let f = Frame3::WORLD.shifted_local(Vec2::new(3.0, -2.0));
        let q = f.to_local_2d(Point3::new(1.0, 1.0, 5.0));
        assert!(close(q.x, 4.0) && close(q.y, -1.0));
    }

    #[test]
    fn mat4_basics() {
        let t = Mat4::translation(Vec3::new(1.0, 2.0, 3.0));
        let p = t.transform_point(Point3::ORIGIN);
        assert!(close3(p, Point3::new(1.0, 2.0, 3.0)));
        let v = t.transform_vector(Vec3::X);
        assert!(close(v.x, 1.0) && close(v.y, 0.0));
        let tt = t * t;
        assert!(close(tt.get(2, 3), 6.0));
        assert!(close((Mat4::IDENTITY * t).get(0, 3), 1.0));
    }
}
