//! Reconstructs lines, arcs and circles from closed polygons (mesh face boundaries).
//!
//! Greedy segmentation: from a corner, each run grows as long as a line or an arc fits all its
//! points within the fit tolerance; the longer fit wins and ties go to the line. Circles are
//! fitted algebraically (Kåsa) and refined with a few Gauss–Newton steps.

use core::f64::consts::{PI, TAU};

use flatbello_core::{Edge, Point2, Segment, Tolerance};

/// The largest turn between consecutive chords of an arc. Bigger turns are corners.
const MAX_ARC_STEP: f64 = 50.0 * PI / 180.0;
/// Arcs need at least this many points, so that two lines meeting at a corner never fit one.
const MIN_ARC_POINTS: usize = 4;
/// Tessellated arcs have nearly equal chords. A run mixing long and short chords is a line
/// followed by an arc, even if one flat circle happens to fit it.
const MAX_CHORD_RATIO: f64 = 4.0;
/// Whole loops need at least this many points to become a circle.
const MIN_CIRCLE_POINTS: usize = 6;
const GAUSS_NEWTON_STEPS: usize = 6;

#[derive(Clone, Copy, Debug)]
pub struct Circle {
    pub center: Point2,
    pub radius: f64,
}

/// Solves a 3×3 linear system by Gaussian elimination with partial pivoting.
fn solve3(mut m: [[f64; 3]; 3], mut v: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let pivot = (col..3).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[pivot][col].abs() < 1e-300 {
            return None;
        }
        m.swap(col, pivot);
        v.swap(col, pivot);
        let pivot_row = m[col];
        for row in col + 1..3 {
            let f = m[row][col] / pivot_row[col];
            for (x, p) in m[row].iter_mut().zip(pivot_row).skip(col) {
                *x -= f * p;
            }
            v[row] -= f * v[col];
        }
    }
    let mut x = [0.0; 3];
    for row in (0..3).rev() {
        let s: f64 = (row + 1..3).map(|k| m[row][k] * x[k]).sum();
        x[row] = (v[row] - s) / m[row][row];
    }
    x.iter().all(|c| c.is_finite()).then_some(x)
}

/// Least-squares circle through `pts`: Kåsa's algebraic fit, refined by Gauss–Newton on the
/// geometric distances.
pub fn fit_circle(pts: &[Point2]) -> Option<Circle> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    // Work relative to the centroid for conditioning.
    let (mx, my) = pts
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p.x / n, y + p.y / n));
    let mut m = [[0.0; 3]; 3];
    let mut v = [0.0; 3];
    for p in pts {
        let (x, y) = (p.x - mx, p.y - my);
        let z = x * x + y * y;
        let row = [x, y, 1.0];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += row[i] * row[j];
            }
            v[i] -= row[i] * z;
        }
    }
    let [d, e, f] = solve3(m, v)?;
    let (mut a, mut b) = (-d / 2.0, -e / 2.0);
    let r2 = a * a + b * b - f;
    if r2.is_nan() || r2 <= 0.0 {
        return None;
    }
    let mut r = r2.sqrt();

    for _ in 0..GAUSS_NEWTON_STEPS {
        let mut jtj = [[0.0; 3]; 3];
        let mut jtr = [0.0; 3];
        for p in pts {
            let (dx, dy) = (p.x - mx - a, p.y - my - b);
            let dist = dx.hypot(dy);
            if dist <= 0.0 {
                continue;
            }
            let res = dist - r;
            let j = [-dx / dist, -dy / dist, -1.0];
            for i in 0..3 {
                for k in 0..3 {
                    jtj[i][k] += j[i] * j[k];
                }
                jtr[i] -= j[i] * res;
            }
        }
        let Some([da, db, dr]) = solve3(jtj, jtr) else {
            break;
        };
        a += da;
        b += db;
        r += dr;
        if da.abs() + db.abs() + dr.abs() < 1e-14 * (1.0 + r) {
            break;
        }
    }
    (r > 0.0 && r.is_finite()).then(|| Circle {
        center: Point2::new(a + mx, b + my),
        radius: r,
    })
}

/// The largest distance from `pts` to the circle.
fn circle_error(c: &Circle, pts: &[Point2]) -> f64 {
    pts.iter()
        .map(|p| (p.distance(c.center) - c.radius).abs())
        .fold(0.0, f64::max)
}

/// The largest distance from `pts` to the chord between the first and last point.
fn line_error(pts: &[Point2]) -> f64 {
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    pts.iter()
        .map(|p| p.distance_to_segment(a, b))
        .fold(0.0, f64::max)
}

/// The signed angle turned going from `a` to `b` around `c`.
fn turn(c: Point2, a: Point2, b: Point2) -> f64 {
    let (u, v) = (a - c, b - c);
    u.cross(v).atan2(u.dot(v))
}

/// If `pts` is an arc within `fit`, its circle, direction, sweep and error. Every step around
/// the centre must turn the same way by at most [`MAX_ARC_STEP`].
fn arc_fit(pts: &[Point2], fit: f64) -> Option<(Circle, bool, f64)> {
    if pts.len() < MIN_ARC_POINTS {
        return None;
    }
    let (lo, hi) = pts
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .fold((f64::MAX, 0.0_f64), |(lo, hi), d| (lo.min(d), hi.max(d)));
    if hi > MAX_CHORD_RATIO * lo {
        return None;
    }
    let c = fit_circle(pts)?;
    let err = circle_error(&c, pts);
    if err > fit {
        return None;
    }
    let steps: Vec<f64> = pts.windows(2).map(|w| turn(c.center, w[0], w[1])).collect();
    let ccw = steps[0] > 0.0;
    if !steps
        .iter()
        .all(|&s| (s > 0.0) == ccw && s.abs() <= MAX_ARC_STEP)
    {
        return None;
    }
    let sweep: f64 = steps.iter().map(|s| s.abs()).sum();
    (sweep < TAU - 1e-6).then_some((c, ccw, err))
}

/// The point of the circle nearest `p`.
fn project(c: &Circle, p: Point2) -> Point2 {
    (p - c.center)
        .normalized()
        .map_or(p, |u| c.center + u * c.radius)
}

/// The intersection of two circles nearest `p`, if they intersect.
fn nearest_intersection(a: &Circle, b: &Circle, p: Point2) -> Option<Point2> {
    let d = b.center - a.center;
    let dist = d.length();
    if dist <= 0.0 || dist > a.radius + b.radius || dist < (a.radius - b.radius).abs() {
        return None;
    }
    let along = (a.radius * a.radius - b.radius * b.radius + dist * dist) / (2.0 * dist);
    let h = (a.radius * a.radius - along * along).max(0.0).sqrt();
    let u = d * (1.0 / dist);
    let base = a.center + u * along;
    let (p1, p2) = (base + u.perp() * h, base - u.perp() * h);
    Some(if p1.distance(p) <= p2.distance(p) {
        p1
    } else {
        p2
    })
}

/// Removes repeated points and points within `linear` of the line through their neighbours.
pub fn simplify(points: &[Point2], tol: &Tolerance) -> Vec<Point2> {
    let mut pts: Vec<Point2> = Vec::with_capacity(points.len());
    for &p in points {
        if pts.last().is_none_or(|q| !tol.points_coincide(*q, p)) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && tol.points_coincide(pts[0], pts[pts.len() - 1]) {
        pts.pop();
    }
    loop {
        let n = pts.len();
        if n <= 3 {
            return pts;
        }
        let keep: Vec<bool> = (0..n)
            .map(|i| {
                let (a, p, b) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
                p.distance_to_segment(a, b) > tol.linear()
            })
            .collect();
        if keep.iter().all(|&k| k) {
            return pts;
        }
        // Drop at most every other point per pass, so neighbours stay to judge by.
        let mut out = Vec::with_capacity(n);
        let mut dropped_prev = false;
        for (i, p) in pts.iter().enumerate() {
            if keep[i] || dropped_prev {
                out.push(*p);
                dropped_prev = false;
            } else {
                dropped_prev = true;
            }
        }
        pts = out;
    }
}

enum Run {
    Line {
        end: usize,
        err: f64,
    },
    Arc {
        end: usize,
        circle: Circle,
        ccw: bool,
        err: f64,
    },
}

impl Run {
    fn end(&self) -> usize {
        match self {
            Run::Line { end, .. } | Run::Arc { end, .. } => *end,
        }
    }
}

/// The longest run starting at `start` in the closed sequence `ring` (which repeats its first
/// point at the end).
fn longest_run(ring: &[Point2], start: usize, fit: f64) -> Run {
    let last = ring.len() - 1;
    let mut line_end = start + 1;
    let mut line_err = 0.0;
    while line_end < last {
        let e = line_error(&ring[start..=line_end + 1]);
        if e > fit {
            break;
        }
        line_end += 1;
        line_err = e;
    }
    let mut best_arc: Option<(usize, Circle, bool, f64)> = None;
    let mut end = start + MIN_ARC_POINTS - 1;
    while end <= last {
        match arc_fit(&ring[start..=end], fit) {
            Some((c, ccw, err)) => best_arc = Some((end, c, ccw, err)),
            None if best_arc.is_some() => break,
            None => {}
        }
        // Without an arc after a few points, more points will not create one.
        if best_arc.is_none() && end > start + MIN_ARC_POINTS {
            break;
        }
        end += 1;
    }
    match best_arc {
        Some((end, circle, ccw, err)) if end > line_end => Run::Arc {
            end,
            circle,
            ccw,
            err,
        },
        _ => Run::Line {
            end: line_end,
            err: line_err,
        },
    }
}

/// Fits a closed polygon with lines, arcs or one circle. The result chains end to end within
/// the linear tolerance, in the polygon's direction. Returns `None` for fewer than 3 points.
pub fn fit_loop(points: &[Point2], tol: &Tolerance) -> Option<Vec<Edge>> {
    let fit = tol.fit();
    let pts = simplify(points, tol);
    let n = pts.len();
    if n < 3 {
        return None;
    }

    if n >= MIN_CIRCLE_POINTS
        && let Some(c) = fit_circle(&pts)
    {
        let err = circle_error(&c, &pts);
        let mut ring = pts.clone();
        ring.push(pts[0]);
        let steps: Vec<f64> = ring
            .windows(2)
            .map(|w| turn(c.center, w[0], w[1]))
            .collect();
        let ccw = steps[0] > 0.0;
        let monotonic = steps
            .iter()
            .all(|&s| (s > 0.0) == ccw && s.abs() <= MAX_ARC_STEP);
        if err <= fit && monotonic {
            return Some(vec![Edge::fitted(
                Segment::Circle {
                    center: c.center,
                    radius: c.radius,
                    ccw,
                },
                err,
            )]);
        }
    }

    // Start where no run can continue through: at the sharpest corner if there is one, or else at
    // the start of the longest chord (a straight side between two arcs).
    // turn() measures the angle at a vertex: a straight continuation is ±π.
    let deflection = |i: usize| PI - turn(pts[i], pts[(i + n - 1) % n], pts[(i + 1) % n]).abs();
    let sharpest = (0..n)
        .max_by(|&a, &b| deflection(a).total_cmp(&deflection(b)))
        .unwrap_or(0);
    let corner = if deflection(sharpest) > MAX_ARC_STEP {
        sharpest
    } else {
        (0..n)
            .max_by(|&a, &b| {
                let la = pts[a].distance(pts[(a + 1) % n]);
                let lb = pts[b].distance(pts[(b + 1) % n]);
                la.total_cmp(&lb).then(b.cmp(&a))
            })
            .unwrap_or(0)
    };
    let mut ring: Vec<Point2> = pts[corner..]
        .iter()
        .chain(&pts[..corner])
        .copied()
        .collect();
    ring.push(ring[0]);

    let mut runs = Vec::new();
    let mut i = 0;
    while i < ring.len() - 1 {
        let run = longest_run(&ring, i, fit);
        i = run.end();
        runs.push(run);
    }

    // Where segments meet, find a point on both: between two arcs, the intersection of their
    // circles nearest the vertex; next to one arc, the vertex moved onto it. Two arcs whose
    // circles miss each other each end at their own nearest point, bridged by a short line.
    let starts: Vec<usize> = std::iter::once(0)
        .chain(runs.iter().map(Run::end))
        .collect();
    let circle_of = |r: &Run| match r {
        Run::Arc { circle, .. } => Some(*circle),
        Run::Line { .. } => None,
    };
    let count = runs.len();
    // For joint j: where run j - 1 ends and where run j starts.
    let mut ends: Vec<(Point2, Point2)> = Vec::with_capacity(count);
    for &k in &starts[..count] {
        let j = ends.len();
        let v = ring[k];
        let before = circle_of(&runs[(j + count - 1) % count]);
        let after = circle_of(&runs[j]);
        ends.push(match (before, after) {
            (Some(a), Some(b)) => match nearest_intersection(&a, &b, v) {
                Some(p) => (p, p),
                None => (project(&a, v), project(&b, v)),
            },
            (Some(c), None) | (None, Some(c)) => (project(&c, v), project(&c, v)),
            (None, None) => (v, v),
        });
    }

    let mut edges = Vec::with_capacity(count);
    for (k, run) in runs.iter().enumerate() {
        let (end_before, a) = ends[k];
        if end_before.distance(a) > tol.linear() {
            edges.push(Edge::fitted(
                Segment::Line {
                    a: end_before,
                    b: a,
                },
                end_before.distance(a),
            ));
        }
        let b = ends[(k + 1) % count].0;
        edges.push(match run {
            Run::Line { err, .. } => Edge::fitted(Segment::Line { a, b }, *err),
            Run::Arc {
                circle, ccw, err, ..
            } => Edge::fitted(
                Segment::Arc {
                    center: circle.center,
                    radius: circle.radius,
                    start_angle: (a - circle.center).angle(),
                    end_angle: (b - circle.center).angle(),
                    ccw: *ccw,
                },
                *err,
            ),
        });
    }
    Some(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbello_core::{Loop, Vec2};

    fn tol() -> Tolerance {
        Tolerance::default()
    }

    fn polygon_circle(c: Point2, r: f64, n: usize, phase: f64) -> Vec<Point2> {
        (0..n)
            .map(|i| c + Vec2::from_angle(phase + TAU * i as f64 / n as f64) * r)
            .collect()
    }

    /// A rounded rectangle sampled the way a tessellator would: corners only on arcs, `k`
    /// chords per quarter arc, straight sides as single chords.
    fn rounded_rect(w: f64, h: f64, r: f64, k: usize) -> Vec<Point2> {
        let mut pts = Vec::new();
        let corners = [
            (Point2::new(w - r, r), -PI / 2.0),
            (Point2::new(w - r, h - r), 0.0),
            (Point2::new(r, h - r), PI / 2.0),
            (Point2::new(r, r), PI),
        ];
        for (c, a0) in corners {
            for i in 0..=k {
                pts.push(c + Vec2::from_angle(a0 + PI / 2.0 * i as f64 / k as f64) * r);
            }
        }
        pts
    }

    #[test]
    fn solves_linear_systems() {
        let x = solve3(
            [[2.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 4.0]],
            [3.0, 5.0, 5.0],
        )
        .unwrap();
        assert!(
            (x[0] - 1.0).abs() < 1e-12 && (x[1] - 1.0).abs() < 1e-12 && (x[2] - 1.0).abs() < 1e-12
        );
        assert!(
            solve3(
                [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]],
                [1.0, 2.0, 3.0]
            )
            .is_none()
        );
    }

    #[test]
    fn fits_circles_exactly() {
        let pts = polygon_circle(Point2::new(12.0, -3.0), 4.5, 7, 0.3);
        let c = fit_circle(&pts).unwrap();
        assert!(c.center.distance(Point2::new(12.0, -3.0)) < 1e-9);
        assert!((c.radius - 4.5).abs() < 1e-9);
        // A short arc of a large circle.
        let arc: Vec<Point2> = (0..6)
            .map(|i| Point2::new(0.0, 0.0) + Vec2::from_angle(0.02 * i as f64) * 1000.0)
            .collect();
        let c = fit_circle(&arc).unwrap();
        assert!((c.radius - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn polygon_of_a_circle_becomes_a_circle() {
        let pts = polygon_circle(Point2::new(5.0, 5.0), 3.0, 24, 0.1);
        let edges = fit_loop(&pts, &tol()).unwrap();
        assert_eq!(edges.len(), 1);
        match edges[0].segment {
            Segment::Circle {
                center,
                radius,
                ccw,
            } => {
                assert!(center.distance(Point2::new(5.0, 5.0)) < 1e-9);
                assert!((radius - 3.0).abs() < 1e-9);
                assert!(ccw);
            }
            ref s => panic!("{s:?}"),
        }
        let rev: Vec<Point2> = pts.iter().rev().copied().collect();
        assert!(matches!(
            fit_loop(&rev, &tol()).unwrap()[0].segment,
            Segment::Circle { ccw: false, .. }
        ));
    }

    #[test]
    fn rectangle_stays_four_lines() {
        let pts = [
            Point2::new(0.0, 0.0),
            Point2::new(5.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(10.0, 4.0),
            Point2::new(0.0, 4.0),
        ];
        let edges = fit_loop(&pts, &tol()).unwrap();
        assert_eq!(edges.len(), 4);
        assert!(
            edges
                .iter()
                .all(|e| matches!(e.segment, Segment::Line { .. }))
        );
        assert!(Loop::new(edges, &tol()).is_ok());
    }

    #[test]
    fn rounded_rectangle_becomes_four_lines_and_four_arcs() {
        for k in [3, 6, 20] {
            let pts = rounded_rect(40.0, 20.0, 5.0, k);
            let edges = fit_loop(&pts, &tol()).unwrap();
            let arcs: Vec<f64> = edges
                .iter()
                .filter_map(|e| match e.segment {
                    Segment::Arc { radius, .. } => Some(radius),
                    _ => None,
                })
                .collect();
            let lines = edges
                .iter()
                .filter(|e| matches!(e.segment, Segment::Line { .. }))
                .count();
            assert_eq!((lines, arcs.len()), (4, 4), "k = {k}: {edges:?}");
            assert!(arcs.iter().all(|r| (r - 5.0).abs() < 1e-9));
            let l = Loop::new(edges, &tol()).unwrap();
            let area = 40.0 * 20.0 - (4.0 - PI) * 25.0;
            // The fitted arcs are exact, so only the chords' end points remain.
            assert!((l.signed_area(&tol()) - area).abs() < 1e-6, "k = {k}");
        }
    }

    #[test]
    fn slot_keeps_its_arcs_clockwise() {
        // A slot traversed clockwise, as a hole is.
        let mut pts = Vec::new();
        for i in 0..=8 {
            pts.push(
                Point2::new(55.0, 20.0) + Vec2::from_angle(PI / 2.0 - PI * i as f64 / 8.0) * 4.0,
            );
        }
        for i in 0..=8 {
            pts.push(
                Point2::new(25.0, 20.0) + Vec2::from_angle(-PI / 2.0 - PI * i as f64 / 8.0) * 4.0,
            );
        }
        let edges = fit_loop(&pts, &tol()).unwrap();
        let arcs: Vec<bool> = edges
            .iter()
            .filter_map(|e| match e.segment {
                Segment::Arc { ccw, .. } => Some(ccw),
                _ => None,
            })
            .collect();
        assert_eq!(arcs, vec![false, false], "{edges:?}");
        assert_eq!(edges.len(), 4);
        assert!(Loop::new(edges, &tol()).is_ok());
    }

    #[test]
    fn simplify_drops_collinear_and_repeated_points() {
        let pts = [
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
            Point2::new(0.0, 2.0),
            Point2::new(0.0, 0.0),
        ];
        assert_eq!(simplify(&pts, &tol()).len(), 4);
    }

    #[test]
    fn too_few_points() {
        assert!(fit_loop(&[Point2::ORIGIN, Point2::new(1.0, 0.0)], &tol()).is_none());
    }
}
