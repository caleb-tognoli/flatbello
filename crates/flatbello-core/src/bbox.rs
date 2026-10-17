//! Axis-aligned 2D bounding boxes.

use crate::math::Point2;

/// An axis-aligned box. The empty box has `min` at +∞ and `max` at −∞.
#[derive(Clone, Copy, Debug)]
pub struct Bbox2 {
    pub min: Point2,
    pub max: Point2,
}

impl Bbox2 {
    pub const EMPTY: Bbox2 = Bbox2 {
        min: Point2::new(f64::INFINITY, f64::INFINITY),
        max: Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    };

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    pub fn include(&mut self, p: Point2) {
        self.min = Point2::new(self.min.x.min(p.x), self.min.y.min(p.y));
        self.max = Point2::new(self.max.x.max(p.x), self.max.y.max(p.y));
    }

    pub fn union(&self, other: &Bbox2) -> Bbox2 {
        let mut b = *self;
        if !other.is_empty() {
            b.include(other.min);
            b.include(other.max);
        }
        b
    }

    /// Width, or 0 for an empty box.
    pub fn width(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            self.max.x - self.min.x
        }
    }

    /// Height, or 0 for an empty box.
    pub fn height(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            self.max.y - self.min.y
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_and_unions() {
        let mut b = Bbox2::EMPTY;
        assert!(b.is_empty());
        assert!(b.width().abs() < 1e-15);
        b.include(Point2::new(1.0, 2.0));
        b.include(Point2::new(-1.0, 5.0));
        assert!(!b.is_empty());
        assert!((b.width() - 2.0).abs() < 1e-15 && (b.height() - 3.0).abs() < 1e-15);
        let u = b.union(&Bbox2::EMPTY);
        assert!((u.width() - 2.0).abs() < 1e-15);
        let mut c = Bbox2::EMPTY;
        c.include(Point2::new(10.0, 0.0));
        let u = b.union(&c);
        assert!((u.width() - 11.0).abs() < 1e-15 && (u.height() - 5.0).abs() < 1e-15);
    }
}
