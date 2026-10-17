//! Planar (rational) B-splines.

use crate::SegmentError;
use crate::math::Point2;

/// Subdivision depth limit when flattening one knot span.
const MAX_DEPTH: u32 = 12;
/// Every knot span is first cut into this many pieces, so small features are not skipped.
const INITIAL_PIECES: usize = 4;

/// A B-spline curve in the plane, optionally rational.
///
/// `knots.len()` must equal `control_points.len() + degree + 1`. `weights` is either empty
/// (non-rational) or holds one positive weight per control point.
#[derive(Clone, Debug)]
pub struct Spline {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub control_points: Vec<Point2>,
    pub weights: Vec<f64>,
}

impl Spline {
    pub fn is_rational(&self) -> bool {
        !self.weights.is_empty()
    }

    pub fn validate(&self) -> Result<(), SegmentError> {
        let n = self.control_points.len();
        let p = self.degree;
        if p == 0 {
            return Err(SegmentError::InvalidSpline("degree must be at least 1"));
        }
        if n < p + 1 {
            return Err(SegmentError::InvalidSpline("too few control points"));
        }
        if self.knots.len() != n + p + 1 {
            return Err(SegmentError::InvalidSpline(
                "knot count must be points + degree + 1",
            ));
        }
        if !self.weights.is_empty() && self.weights.len() != n {
            return Err(SegmentError::InvalidSpline(
                "weight count must match control points",
            ));
        }
        if !self.control_points.iter().all(|c| c.is_finite())
            || !self.knots.iter().all(|k| k.is_finite())
            || !self.weights.iter().all(|w| w.is_finite())
        {
            return Err(SegmentError::NonFinite);
        }
        if self.knots.windows(2).any(|w| w[1] < w[0]) {
            return Err(SegmentError::InvalidSpline("knots must not decrease"));
        }
        if self.weights.iter().any(|&w| w <= 0.0) {
            return Err(SegmentError::InvalidSpline("weights must be positive"));
        }
        let (a, b) = self.domain();
        if b <= a {
            return Err(SegmentError::InvalidSpline("empty parameter domain"));
        }
        Ok(())
    }

    /// The parameter range `[knots[degree], knots[n]]`.
    pub fn domain(&self) -> (f64, f64) {
        let n = self.control_points.len();
        let a = self.knots.get(self.degree).copied().unwrap_or(0.0);
        let b = self.knots.get(n).copied().unwrap_or(a);
        (a, b)
    }

    /// Evaluates the curve with de Boor's algorithm in homogeneous coordinates.
    /// `t` is clamped to the domain. An invalid spline yields an unspecified point.
    pub fn point_at(&self, t: f64) -> Point2 {
        let n = self.control_points.len();
        let p = self.degree;
        if n == 0 || p == 0 || self.knots.len() != n + p + 1 {
            return self
                .control_points
                .first()
                .copied()
                .unwrap_or(Point2::ORIGIN);
        }
        let (a, b) = self.domain();
        let t = t.clamp(a, b);

        // The knot span [knots[k], knots[k+1]) holding t, with k in [p, n-1] and a non-empty span.
        let mut k = p;
        while k + 1 < n && self.knots[k + 1] <= t {
            k += 1;
        }
        while k > p && self.knots[k] >= self.knots[k + 1] {
            k -= 1;
        }

        let weight = |i: usize| self.weights.get(i).copied().unwrap_or(1.0);
        let mut d: Vec<[f64; 3]> = (0..=p)
            .map(|j| {
                let i = j + k - p;
                let c = self.control_points[i];
                let w = weight(i);
                [c.x * w, c.y * w, w]
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = j + k - p;
                let den = self.knots[i + p + 1 - r] - self.knots[i];
                let alpha = if den > 0.0 {
                    (t - self.knots[i]) / den
                } else {
                    0.0
                };
                let (prev, cur) = (d[j - 1], d[j]);
                for c in 0..3 {
                    d[j][c] = (1.0 - alpha) * prev[c] + alpha * cur[c];
                }
            }
        }
        let [x, y, w] = d[p];
        Point2::new(x / w, y / w)
    }

    pub fn start_point(&self) -> Point2 {
        self.point_at(self.domain().0)
    }

    pub fn end_point(&self) -> Point2 {
        self.point_at(self.domain().1)
    }

    /// The same curve traversed in the opposite direction, over the same parameter domain.
    pub fn reversed(&self) -> Spline {
        let (first, last) = (
            self.knots.first().copied().unwrap_or(0.0),
            self.knots.last().copied().unwrap_or(0.0),
        );
        Spline {
            degree: self.degree,
            knots: self.knots.iter().rev().map(|k| first + last - k).collect(),
            control_points: self.control_points.iter().rev().copied().collect(),
            weights: self.weights.iter().rev().copied().collect(),
        }
    }

    pub fn translated(&self, dx: f64, dy: f64) -> Spline {
        Spline {
            control_points: self
                .control_points
                .iter()
                .map(|c| Point2::new(c.x + dx, c.y + dy))
                .collect(),
            ..self.clone()
        }
    }

    /// Points along the curve, start and end included, such that the polyline through them
    /// stays within about `fit` of the curve.
    pub fn flatten(&self, fit: f64) -> Vec<Point2> {
        let (a, b) = self.domain();
        let mut out = vec![self.point_at(a)];
        let spans: Vec<(f64, f64)> = self
            .knots
            .windows(2)
            .map(|w| (w[0].max(a), w[1].min(b)))
            .filter(|(s, e)| e > s)
            .collect();
        for (s, e) in spans {
            for i in 0..INITIAL_PIECES {
                let t0 = s + (e - s) * i as f64 / INITIAL_PIECES as f64;
                let t1 = s + (e - s) * (i + 1) as f64 / INITIAL_PIECES as f64;
                self.subdivide(
                    t0,
                    t1,
                    self.point_at(t0),
                    self.point_at(t1),
                    fit,
                    0,
                    &mut out,
                );
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn subdivide(
        &self,
        t0: f64,
        t1: f64,
        p0: Point2,
        p1: Point2,
        fit: f64,
        depth: u32,
        out: &mut Vec<Point2>,
    ) {
        let tm = 0.5 * (t0 + t1);
        let pm = self.point_at(tm);
        let q1 = self.point_at(0.5 * (t0 + tm));
        let q3 = self.point_at(0.5 * (tm + t1));
        let err = [pm, q1, q3]
            .iter()
            .map(|q| q.distance_to_segment(p0, p1))
            .fold(0.0, f64::max);
        if err > fit && depth < MAX_DEPTH {
            self.subdivide(t0, tm, p0, pm, fit, depth + 1, out);
            self.subdivide(tm, t1, pm, p1, fit, depth + 1, out);
        } else {
            out.push(p1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-12
    }

    /// A clamped quadratic Bézier from (0,0) via (1,2) to (2,0).
    fn quadratic() -> Spline {
        Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![
                Point2::new(0.0, 0.0),
                Point2::new(1.0, 2.0),
                Point2::new(2.0, 0.0),
            ],
            weights: vec![],
        }
    }

    /// The exact rational quadratic quarter circle of radius 1 from (1,0) to (0,1).
    fn quarter_circle() -> Spline {
        Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![
                Point2::new(1.0, 0.0),
                Point2::new(1.0, 1.0),
                Point2::new(0.0, 1.0),
            ],
            weights: vec![1.0, core::f64::consts::FRAC_1_SQRT_2, 1.0],
        }
    }

    #[test]
    fn evaluates_bezier() {
        let s = quadratic();
        assert!(s.validate().is_ok());
        assert!(close(s.start_point(), Point2::new(0.0, 0.0)));
        assert!(close(s.end_point(), Point2::new(2.0, 0.0)));
        assert!(close(s.point_at(0.5), Point2::new(1.0, 1.0)));
    }

    #[test]
    fn rational_quarter_circle_lies_on_circle() {
        let s = quarter_circle();
        assert!(s.validate().is_ok());
        for i in 0..=10 {
            let p = s.point_at(i as f64 / 10.0);
            assert!((p.to_vec().length() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn multi_span_uniform_cubic() {
        // A clamped cubic with an interior knot; its ends interpolate the end control points.
        let s = Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
            control_points: (0..5)
                .map(|i| Point2::new(i as f64, (i % 2) as f64))
                .collect(),
            weights: vec![],
        };
        assert!(s.validate().is_ok());
        assert!(close(s.start_point(), Point2::new(0.0, 0.0)));
        assert!(close(s.end_point(), Point2::new(4.0, 0.0)));
        // Continuity across the interior knot.
        assert!(s.point_at(0.5 - 1e-9).distance(s.point_at(0.5 + 1e-9)) < 1e-6);
    }

    #[test]
    fn reversal_traverses_backwards() {
        let s = quarter_circle();
        let r = s.reversed();
        assert!(r.validate().is_ok());
        assert!(close(r.start_point(), s.end_point()));
        assert!(close(r.end_point(), s.start_point()));
        assert!(close(r.point_at(0.3), s.point_at(0.7)));
    }

    #[test]
    fn flatten_stays_within_fit() {
        let s = quarter_circle();
        let fit = 1e-4;
        let pts = s.flatten(fit);
        assert!(close(pts[0], s.start_point()));
        assert!(close(*pts.last().unwrap(), s.end_point()));
        for w in pts.windows(2) {
            let mid = w[0].lerp(w[1], 0.5);
            // The chord midpoint is the farthest point from a circular arc.
            assert!(1.0 - mid.to_vec().length() <= fit);
        }
    }

    #[test]
    fn rejects_bad_data() {
        let mut s = quadratic();
        s.knots.pop();
        assert!(s.validate().is_err());
        let mut s = quadratic();
        s.knots = vec![0.0, 0.0, 1.0, 0.5, 1.0, 1.0];
        assert!(s.validate().is_err());
        let mut s = quarter_circle();
        s.weights[1] = 0.0;
        assert!(s.validate().is_err());
        let mut s = quadratic();
        s.degree = 0;
        assert!(s.validate().is_err());
        let mut s = quadratic();
        s.knots = vec![1.0; 6];
        assert!(s.validate().is_err());
    }
}
