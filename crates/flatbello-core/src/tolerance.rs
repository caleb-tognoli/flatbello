//! Explicit tolerances. Floats are never compared with `==`; they are compared through a [`Tolerance`].

use core::f64::consts::PI;

use crate::GeometryError;
use crate::angle::signed_angle_difference;
use crate::math::{Point2, Point3};

/// Comparison tolerances. All lengths are millimetres, angles radians.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    linear: f64,
    angular: f64,
    fit: f64,
}

impl Tolerance {
    /// Lengths closer than this are equal (mm).
    pub const DEFAULT_LINEAR: f64 = 1e-3;
    /// Directions closer than this are equal: 0.5° (rad).
    pub const DEFAULT_ANGULAR: f64 = PI / 360.0;
    /// Maximum deviation allowed when fitting or flattening curves (mm).
    pub const DEFAULT_FIT: f64 = 0.01;

    /// Every value must be finite and positive, and `angular` must be below π/2.
    pub fn new(linear: f64, angular: f64, fit: f64) -> Result<Self, GeometryError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if positive(linear) && positive(angular) && angular < PI / 2.0 && positive(fit) {
            Ok(Self {
                linear,
                angular,
                fit,
            })
        } else {
            Err(GeometryError::InvalidTolerance)
        }
    }

    pub fn linear(&self) -> f64 {
        self.linear
    }

    pub fn angular(&self) -> f64 {
        self.angular
    }

    pub fn fit(&self) -> f64 {
        self.fit
    }

    pub fn lengths_equal(&self, a: f64, b: f64) -> bool {
        (a - b).abs() <= self.linear
    }

    pub fn is_zero_length(&self, a: f64) -> bool {
        a.abs() <= self.linear
    }

    pub fn points_coincide(&self, a: Point2, b: Point2) -> bool {
        a.distance(b) <= self.linear
    }

    pub fn points_coincide_3d(&self, a: Point3, b: Point3) -> bool {
        a.distance(b) <= self.linear
    }

    /// True if the angles are equal modulo 2π within the angular tolerance.
    pub fn angles_equal(&self, a: f64, b: f64) -> bool {
        signed_angle_difference(a, b).abs() <= self.angular
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            linear: Self::DEFAULT_LINEAR,
            angular: Self::DEFAULT_ANGULAR,
            fit: Self::DEFAULT_FIT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let t = Tolerance::default();
        assert!((t.linear() - 1e-3).abs() < 1e-15);
        assert!((t.angular() - 0.5_f64.to_radians()).abs() < 1e-15);
        assert!((t.fit() - 0.01).abs() < 1e-15);
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(Tolerance::new(0.0, 0.01, 0.01).is_err());
        assert!(Tolerance::new(1e-3, -0.01, 0.01).is_err());
        assert!(Tolerance::new(1e-3, 2.0, 0.01).is_err());
        assert!(Tolerance::new(1e-3, 0.01, f64::NAN).is_err());
        assert!(Tolerance::new(1e-3, 0.01, f64::INFINITY).is_err());
        assert!(Tolerance::new(1e-3, 0.01, 0.01).is_ok());
    }

    #[test]
    fn comparisons() {
        let t = Tolerance::default();
        assert!(t.lengths_equal(1.0, 1.0009));
        assert!(!t.lengths_equal(1.0, 1.0011));
        assert!(t.is_zero_length(-0.0005));
        assert!(t.points_coincide(Point2::new(0.0, 0.0), Point2::new(0.0006, 0.0006)));
        assert!(!t.points_coincide(Point2::new(0.0, 0.0), Point2::new(0.0008, 0.0008)));
        assert!(t.points_coincide_3d(Point3::ORIGIN, Point3::new(0.0, 0.0, 0.0009)));
        assert!(t.angles_equal(0.0, 2.0 * PI));
        assert!(t.angles_equal(-PI, PI));
        assert!(t.angles_equal(0.001, -0.001));
        assert!(!t.angles_equal(0.0, 0.01));
    }
}
