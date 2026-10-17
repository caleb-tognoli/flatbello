//! Mesh faces: welding a triangle soup, splitting it into planar regions, and extracting a
//! region's boundary as fitted lines, arcs and circles.

use std::collections::{HashMap, VecDeque};

use flatbello_core::{
    FaceId, Frame3, GeometryError, LengthUnit, Loop, MeshGeometry, MeshPlane, Orientation, Point2,
    Point3, Profile2D, Segment, Tolerance, Vec3,
};

use crate::fit::fit_loop;
use crate::planar::{ExtractError, ExtractOptions, XAxisRule};

/// Welding tolerance relative to the bounding-box diagonal.
const WELD_RELATIVE: f64 = 1e-7;

/// Merges corners closer than 1e-7 × the bounding-box diagonal into shared vertices and drops
/// triangles that collapse. Returns positions and triangles.
pub fn weld(soup: &[[Point3; 3]]) -> (Vec<Point3>, Vec<[u32; 3]>) {
    let (mut lo, mut hi) = (
        Point3::new(f64::MAX, f64::MAX, f64::MAX),
        Point3::new(f64::MIN, f64::MIN, f64::MIN),
    );
    for p in soup.iter().flatten() {
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    let tol = (lo.distance(hi) * WELD_RELATIVE).max(f64::MIN_POSITIVE);
    let cell = |p: Point3| {
        (
            (p.x / tol).floor() as i64,
            (p.y / tol).floor() as i64,
            (p.z / tol).floor() as i64,
        )
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut positions: Vec<Point3> = Vec::new();
    let mut index_of = |p: Point3, positions: &mut Vec<Point3>| -> u32 {
        let (cx, cy, cz) = cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &i in ids {
                            if positions[i as usize].distance(p) <= tol {
                                return i;
                            }
                        }
                    }
                }
            }
        }
        let i = positions.len() as u32;
        positions.push(p);
        grid.entry((cx, cy, cz)).or_default().push(i);
        i
    };
    let mut triangles = Vec::with_capacity(soup.len());
    for t in soup {
        let ids = t.map(|p| index_of(p, &mut positions));
        if ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2] {
            triangles.push(ids);
        }
    }
    (positions, triangles)
}

/// Twice the area times the unit normal of a triangle.
fn area_normal(p: &[Point3], t: [u32; 3]) -> Vec3 {
    let [a, b, c] = t.map(|i| p[i as usize]);
    (b - a).cross(c - a)
}

/// Undirected edge key.
fn key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

fn edge_map(triangles: &[[u32; 3]]) -> HashMap<(u32, u32), Vec<u32>> {
    let mut m: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (i, t) in triangles.iter().enumerate() {
        for k in 0..3 {
            m.entry(key(t[k], t[(k + 1) % 3]))
                .or_default()
                .push(i as u32);
        }
    }
    m
}

/// Splits a welded mesh into planar regions by growing from the largest unassigned triangle.
/// A neighbour joins when its normal is within the angular tolerance of the region's fitted
/// plane and its corners lie within the linear tolerance of it; the plane is refitted as the
/// region grows. Every triangle ends up in exactly one region.
pub fn segment(positions: Vec<Point3>, triangles: Vec<[u32; 3]>, tol: &Tolerance) -> MeshGeometry {
    let edges = edge_map(&triangles);
    let normals: Vec<Vec3> = triangles
        .iter()
        .map(|t| area_normal(&positions, *t))
        .collect();
    let mut order: Vec<usize> = (0..triangles.len()).collect();
    order.sort_by(|&a, &b| {
        normals[b]
            .length()
            .total_cmp(&normals[a].length())
            .then(a.cmp(&b))
    });
    let cos_tol = tol.angular().cos();
    let mut face_of: Vec<Option<u32>> = vec![None; triangles.len()];
    let mut planes: Vec<MeshPlane> = Vec::new();
    let centroid = |t: [u32; 3]| {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        Point3::new(
            (a.x + b.x + c.x) / 3.0,
            (a.y + b.y + c.y) / 3.0,
            (a.z + b.z + c.z) / 3.0,
        )
    };

    for &seed in &order {
        if face_of[seed].is_some() {
            continue;
        }
        let id = planes.len() as u32;
        face_of[seed] = Some(id);
        // Area-weighted sums for the plane fit.
        let mut sum_n = normals[seed];
        let mut sum_area = normals[seed].length();
        let c0 = centroid(triangles[seed]);
        let mut sum_c = c0.to_vec() * sum_area;
        let plane = |sum_n: Vec3, sum_c: Vec3, sum_area: f64| {
            let n = sum_n.normalized().unwrap_or(Vec3::Z);
            let c = if sum_area > 0.0 {
                sum_c * (1.0 / sum_area)
            } else {
                c0.to_vec()
            };
            (n, Point3::new(c.x, c.y, c.z))
        };
        let mut queue = VecDeque::from([seed]);
        while let Some(t) = queue.pop_front() {
            let tri = triangles[t];
            for k in 0..3 {
                let Some(nbrs) = edges.get(&key(tri[k], tri[(k + 1) % 3])) else {
                    continue;
                };
                for &nb in nbrs {
                    let nb = nb as usize;
                    if face_of[nb].is_some() {
                        continue;
                    }
                    let (n, origin) = plane(sum_n, sum_c, sum_area);
                    let Some(nn) = normals[nb].normalized() else {
                        continue;
                    };
                    let flat = triangles[nb]
                        .iter()
                        .all(|&v| (positions[v as usize] - origin).dot(n).abs() <= tol.linear());
                    if nn.dot(n) >= cos_tol && flat {
                        face_of[nb] = Some(id);
                        let area = normals[nb].length();
                        sum_n = sum_n + normals[nb];
                        sum_area += area;
                        sum_c = sum_c + centroid(triangles[nb]).to_vec() * area;
                        queue.push_back(nb);
                    }
                }
            }
        }
        let (normal, origin) = plane(sum_n, sum_c, sum_area);
        planes.push(MeshPlane { origin, normal });
    }

    MeshGeometry {
        triangle_face: face_of.iter().map(|f| FaceId(f.unwrap_or(0))).collect(),
        positions,
        triangles,
        planes,
    }
}

/// The face's area in mm² (area of its triangles).
pub fn face_areas(mesh: &MeshGeometry) -> Vec<f64> {
    let mut areas = vec![0.0; mesh.planes.len()];
    for (t, f) in mesh.triangles.iter().zip(&mesh.triangle_face) {
        if let Some(a) = areas.get_mut(f.0 as usize) {
            *a += 0.5 * area_normal(&mesh.positions, *t).length();
        }
    }
    areas
}

/// The boundary of a face as closed vertex-index loops, following the triangles' winding. An
/// edge is on the boundary when exactly one of the face's triangles uses it.
pub fn boundary_loops(mesh: &MeshGeometry, face: FaceId) -> Vec<Vec<u32>> {
    let tris: Vec<[u32; 3]> = mesh
        .triangles
        .iter()
        .zip(&mesh.triangle_face)
        .filter(|(_, f)| **f == face)
        .map(|(t, _)| *t)
        .collect();
    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
    for t in &tris {
        for k in 0..3 {
            *count.entry(key(t[k], t[(k + 1) % 3])).or_default() += 1;
        }
    }
    // Directed boundary edges, in a deterministic order.
    let mut out_edges: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut starts: Vec<u32> = Vec::new();
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if count.get(&key(a, b)) == Some(&1) {
                out_edges.entry(a).or_default().push(b);
                starts.push(a);
            }
        }
    }
    let mut loops = Vec::new();
    for s in starts {
        let mut current = s;
        let mut chain = Vec::new();
        while let Some(next) = out_edges.get_mut(&current).and_then(|v| v.pop()) {
            chain.push(current);
            current = next;
            if current == s {
                break;
            }
        }
        if chain.len() >= 3 && current == s {
            loops.push(chain);
        }
    }
    loops
}

/// Maps loops of mesh vertices into `frame` and fits each one.
fn fit_loops(
    mesh: &MeshGeometry,
    loops: &[Vec<u32>],
    frame: &Frame3,
    tol: &Tolerance,
) -> Result<Vec<Loop>, GeometryError> {
    loops
        .iter()
        .filter_map(|l| {
            let pts: Vec<Point2> = l
                .iter()
                .map(|&v| frame.to_local_2d(mesh.positions[v as usize]))
                .collect();
            fit_loop(&pts, tol)
        })
        .map(|edges| Loop::new(edges, tol))
        .collect()
}

/// The outer loop (largest area) and the holes.
fn split_outer(mut loops: Vec<Loop>, tol: &Tolerance) -> Option<(Loop, Vec<Loop>)> {
    let i = (0..loops.len()).max_by(|&a, &b| {
        loops[a]
            .signed_area(tol)
            .abs()
            .total_cmp(&loops[b].signed_area(tol).abs())
    })?;
    let outer = loops.remove(i);
    Some((outer, loops))
}

fn longest_line(l: &Loop, tol: &Tolerance) -> Option<flatbello_core::Vec2> {
    let mut best: Option<(f64, flatbello_core::Vec2)> = None;
    for e in l.edges() {
        if let Segment::Line { a, b } = e.segment {
            let len = a.distance(b);
            if best.is_none_or(|(l, _)| len > l + tol.linear()) {
                best = Some((len, b - a));
            }
        }
    }
    best.and_then(|(_, d)| d.normalized())
}

/// Extracts a mesh face as fitted lines, arcs and circles.
pub fn extract_mesh_face(
    mesh: &MeshGeometry,
    face: FaceId,
    units: LengthUnit,
    options: &ExtractOptions,
) -> Result<Profile2D, ExtractError> {
    let tol = &options.tolerance;
    let geom = |error| ExtractError::Geometry { face, error };
    let plane = mesh
        .planes
        .get(face.0 as usize)
        .ok_or(ExtractError::NotPlanar(face))?;
    let loops = boundary_loops(mesh, face);
    if loops.is_empty() {
        return Err(ExtractError::NoWires(face));
    }
    let n = plane.normal;
    let other = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let mut frame = Frame3::from_normal(plane.origin, n, other, tol)
        .ok_or(ExtractError::DegenerateFrame(face))?;
    let mut fitted = fit_loops(mesh, &loops, &frame, tol).map_err(geom)?;
    if options.x_axis == XAxisRule::LongestStraightEdge {
        let (outer, _) = split_outer(fitted.clone(), tol).ok_or(ExtractError::NoWires(face))?;
        let outer = outer.oriented(Orientation::CounterClockwise, tol);
        if let Some(d) = longest_line(&outer, tol) {
            let hint = frame.x_axis() * d.x + frame.y_axis() * d.y;
            frame = Frame3::from_normal(plane.origin, n, hint, tol)
                .ok_or(ExtractError::DegenerateFrame(face))?;
            fitted = fit_loops(mesh, &loops, &frame, tol).map_err(geom)?;
        }
    }
    let (outer, holes) = split_outer(fitted, tol).ok_or(ExtractError::NoWires(face))?;
    let profile = Profile2D::new(outer, holes, face, frame, units, tol).map_err(geom)?;
    Ok(profile.at_origin(tol))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::TAU;

    fn tol() -> Tolerance {
        Tolerance::default()
    }

    /// A closed box made of 12 triangles, wound outward.
    fn box_soup(dx: f64, dy: f64, dz: f64) -> Vec<[Point3; 3]> {
        let p = |x: f64, y: f64, z: f64| Point3::new(x * dx, y * dy, z * dz);
        let quads = [
            [p(0., 0., 0.), p(0., 1., 0.), p(1., 1., 0.), p(1., 0., 0.)], // bottom (−z)
            [p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)], // top (+z)
            [p(0., 0., 0.), p(1., 0., 0.), p(1., 0., 1.), p(0., 0., 1.)], // front (−y)
            [p(0., 1., 0.), p(0., 1., 1.), p(1., 1., 1.), p(1., 1., 0.)], // back (+y)
            [p(0., 0., 0.), p(0., 0., 1.), p(0., 1., 1.), p(0., 1., 0.)], // left (−x)
            [p(1., 0., 0.), p(1., 1., 0.), p(1., 1., 1.), p(1., 0., 1.)], // right (+x)
        ];
        quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .collect()
    }

    #[test]
    fn welds_shared_corners() {
        let soup = box_soup(10.0, 20.0, 5.0);
        let (pos, tris) = weld(&soup);
        assert_eq!(pos.len(), 8);
        assert_eq!(tris.len(), 12);
        // A degenerate triangle disappears.
        let mut soup = soup;
        soup.push([Point3::ORIGIN, Point3::ORIGIN, Point3::new(1.0, 0.0, 0.0)]);
        assert_eq!(weld(&soup).1.len(), 12);
    }

    #[test]
    fn box_splits_into_six_faces() {
        let (pos, tris) = weld(&box_soup(10.0, 20.0, 5.0));
        let m = segment(pos, tris, &tol());
        assert_eq!(m.planes.len(), 6);
        let areas = face_areas(&m);
        let mut sorted = areas.clone();
        sorted.sort_by(f64::total_cmp);
        let want = [50.0, 50.0, 100.0, 100.0, 200.0, 200.0];
        for (a, w) in sorted.iter().zip(want) {
            assert!((a - w).abs() < 1e-9, "{sorted:?}");
        }
        // Normals point outward.
        for (plane, f) in m.planes.iter().zip(0..) {
            let centre = Point3::new(5.0, 10.0, 2.5);
            assert!((plane.origin - centre).dot(plane.normal) > 0.0, "face {f}");
        }
    }

    #[test]
    fn box_top_extracts_as_a_rectangle() {
        let (pos, tris) = weld(&box_soup(10.0, 20.0, 5.0));
        let m = segment(pos, tris, &tol());
        let top = m
            .planes
            .iter()
            .position(|p| p.normal.z > 0.99)
            .map(|i| FaceId(i as u32))
            .unwrap();
        let p =
            extract_mesh_face(&m, top, LengthUnit::Unknown, &ExtractOptions::default()).unwrap();
        let b = p.bbox(&tol());
        assert!((b.width() - 20.0).abs() < 1e-9 && (b.height() - 10.0).abs() < 1e-9);
        assert_eq!(p.outer().edges().len(), 4);
        assert!((p.frame().z_axis().z - 1.0).abs() < 1e-12);
    }

    /// A square plate face with a polygonal hole, as a triangle fan between the two loops.
    fn plate_with_hole(n: usize) -> Vec<[Point3; 3]> {
        let outer: Vec<Point3> = (0..n)
            .map(|i| {
                let a = TAU * i as f64 / n as f64;
                // Project the circle direction onto the square [−10, 10]².
                let (c, s) = (a.cos(), a.sin());
                let k = 10.0 / c.abs().max(s.abs());
                Point3::new(c * k, s * k, 0.0)
            })
            .collect();
        let inner: Vec<Point3> = (0..n)
            .map(|i| {
                let a = TAU * i as f64 / n as f64;
                Point3::new(3.0 * a.cos(), 3.0 * a.sin(), 0.0)
            })
            .collect();
        let mut soup = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            soup.push([inner[i], outer[i], outer[j]]);
            soup.push([inner[i], outer[j], inner[j]]);
        }
        soup
    }

    #[test]
    fn finds_the_hole_and_fits_a_circle() {
        let (pos, tris) = weld(&plate_with_hole(32));
        let m = segment(pos, tris, &tol());
        assert_eq!(m.planes.len(), 1);
        let loops = boundary_loops(&m, FaceId(0));
        assert_eq!(loops.len(), 2);
        let p = extract_mesh_face(
            &m,
            FaceId(0),
            LengthUnit::Unknown,
            &ExtractOptions::default(),
        )
        .unwrap();
        assert_eq!(p.holes().len(), 1);
        match p.holes()[0].edges()[0].segment {
            Segment::Circle { radius, ccw, .. } => {
                assert!((radius - 3.0).abs() < 1e-9);
                assert!(!ccw);
            }
            ref s => panic!("{s:?}"),
        }
        let b = p.bbox(&tol());
        assert!((b.width() - 20.0).abs() < 1e-9);
    }
}
