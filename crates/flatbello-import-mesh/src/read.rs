//! STL (binary and ASCII) and OBJ (`v` and `f` only) readers producing triangle soups.

use core::fmt;

use flatbello_core::Point3;

/// Triangles as corner positions, in file units. Corners are not shared.
#[derive(Clone, Debug, Default)]
pub struct Soup {
    pub triangles: Vec<[Point3; 3]>,
}

#[derive(Clone, Debug)]
pub enum ReadError {
    /// The file ended early or a record is malformed. `line` is 1-based, 0 for binary data.
    Malformed {
        line: usize,
        message: String,
    },
    Empty,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { line: 0, message } => f.write_str(message),
            Self::Malformed { line, message } => write!(f, "line {line}: {message}"),
            Self::Empty => f.write_str("the file contains no triangles"),
        }
    }
}

fn malformed(line: usize, message: impl Into<String>) -> ReadError {
    ReadError::Malformed {
        line,
        message: message.into(),
    }
}

/// True if `bytes` is a binary STL: its length matches the triangle count in the header.
/// (Some binary files start with "solid", so the keyword alone does not decide.)
pub fn is_binary_stl(bytes: &[u8]) -> bool {
    bytes.len() >= 84 && {
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        n.checked_mul(50)
            .and_then(|b| b.checked_add(84))
            .is_some_and(|len| len == bytes.len())
    }
}

pub fn is_ascii_stl(bytes: &[u8]) -> bool {
    let start = bytes.trim_ascii_start();
    start.len() >= 5 && start[..5].eq_ignore_ascii_case(b"solid") && !is_binary_stl(bytes)
}

pub fn read_stl(bytes: &[u8]) -> Result<Soup, ReadError> {
    if is_binary_stl(bytes) {
        read_binary_stl(bytes)
    } else {
        read_ascii_stl(bytes)
    }
}

fn read_binary_stl(bytes: &[u8]) -> Result<Soup, ReadError> {
    let f32_at = |o: usize| -> f64 {
        f64::from(f32::from_le_bytes([
            bytes[o],
            bytes[o + 1],
            bytes[o + 2],
            bytes[o + 3],
        ]))
    };
    let n = (bytes.len() - 84) / 50;
    let mut triangles = Vec::with_capacity(n);
    for t in 0..n {
        // 12 bytes of facet normal (ignored: recomputed from the winding), then 3 vertices.
        let base = 84 + t * 50 + 12;
        let p = |k: usize| {
            let o = base + k * 12;
            Point3::new(f32_at(o), f32_at(o + 4), f32_at(o + 8))
        };
        triangles.push([p(0), p(1), p(2)]);
    }
    finish(triangles)
}

fn read_ascii_stl(bytes: &[u8]) -> Result<Soup, ReadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| malformed(0, "not a text STL file"))?;
    let mut triangles = Vec::new();
    let mut corners: Vec<Point3> = Vec::with_capacity(3);
    for (i, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("vertex") => {
                let p = parse_point(&mut words).ok_or_else(|| malformed(i + 1, "bad vertex"))?;
                corners.push(p);
            }
            Some("endloop") => {
                // Polygons with more than three corners are fanned.
                if corners.len() < 3 {
                    return Err(malformed(i + 1, "a facet has fewer than 3 vertices"));
                }
                for k in 1..corners.len() - 1 {
                    triangles.push([corners[0], corners[k], corners[k + 1]]);
                }
                corners.clear();
            }
            _ => {}
        }
    }
    finish(triangles)
}

fn parse_point<'a>(words: &mut impl Iterator<Item = &'a str>) -> Option<Point3> {
    let mut v = [0.0; 3];
    for c in &mut v {
        *c = words
            .next()?
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())?;
    }
    Some(Point3::new(v[0], v[1], v[2]))
}

/// Reads `v x y z` and `f i j k ...` records; every other record is ignored. Faces may use
/// `i`, `i/t`, `i/t/n` or `i//n` and negative (relative) indices, and are fanned into triangles.
pub fn read_obj(bytes: &[u8]) -> Result<Soup, ReadError> {
    let text = String::from_utf8_lossy(bytes);
    let mut vertices: Vec<Point3> = Vec::new();
    let mut triangles = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let p = parse_point(&mut words).ok_or_else(|| malformed(i + 1, "bad vertex"))?;
                vertices.push(p);
            }
            Some("f") => {
                let corners = words
                    .map(|w| obj_index(w, vertices.len()).map(|k| vertices[k]))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| malformed(i + 1, "bad face index"))?;
                if corners.len() < 3 {
                    return Err(malformed(i + 1, "a face has fewer than 3 vertices"));
                }
                for k in 1..corners.len() - 1 {
                    triangles.push([corners[0], corners[k], corners[k + 1]]);
                }
            }
            _ => {}
        }
    }
    finish(triangles)
}

/// A 0-based vertex index from an OBJ face corner like `7`, `7/2/3` or `-1`.
fn obj_index(word: &str, count: usize) -> Option<usize> {
    let i: i64 = word.split('/').next()?.parse().ok()?;
    let k = match i {
        0 => return None,
        i if i > 0 => usize::try_from(i - 1).ok()?,
        i => count.checked_sub(usize::try_from(-i).ok()?)?,
    };
    (k < count).then_some(k)
}

fn finish(triangles: Vec<[Point3; 3]>) -> Result<Soup, ReadError> {
    if triangles.is_empty() {
        return Err(ReadError::Empty);
    }
    if !triangles.iter().flatten().all(|p| p.is_finite()) {
        return Err(malformed(0, "non-finite coordinates"));
    }
    Ok(Soup { triangles })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary(tris: &[[[f32; 3]; 3]]) -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for t in tris {
            b.extend_from_slice(&[0; 12]);
            for p in t {
                for c in p {
                    b.extend_from_slice(&c.to_le_bytes());
                }
            }
            b.extend_from_slice(&[0, 0]);
        }
        b
    }

    #[test]
    fn binary_stl() {
        let mut b = binary(&[[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]);
        // A binary file whose header starts with "solid" is still binary.
        b[..5].copy_from_slice(b"solid");
        assert!(is_binary_stl(&b) && !is_ascii_stl(&b));
        let s = read_stl(&b).unwrap();
        assert_eq!(s.triangles.len(), 1);
        assert!((s.triangles[0][1].x - 1.0).abs() < 1e-12);
        b.pop();
        assert!(!is_binary_stl(&b));
    }

    #[test]
    fn ascii_stl() {
        let text = b"solid t\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 1 1 0\n  endloop\n endfacet\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 1 0\n   vertex 0 1 0\n   vertex -1 1 0\n  endloop\n endfacet\nendsolid t\n";
        assert!(is_ascii_stl(text));
        let s = read_stl(text).unwrap();
        assert_eq!(s.triangles.len(), 3);
        assert!(read_stl(b"solid x\n facet\n outer loop\n vertex 0 0\n").is_err());
        assert!(matches!(
            read_stl(b"solid empty\nendsolid\n"),
            Err(ReadError::Empty)
        ));
    }

    #[test]
    fn obj() {
        let text = b"# cube corner\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0 1.0\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/4/1\nf -4 -3 -2\n";
        let s = read_obj(text).unwrap();
        assert_eq!(s.triangles.len(), 3);
        assert!((s.triangles[2][1].x - 1.0).abs() < 1e-12);
        assert!(read_obj(b"v 0 0 0\nf 1 2 3\n").is_err());
        assert!(read_obj(b"v 0 0 0\nf 0 1 1\n").is_err());
        assert!(matches!(read_obj(b"v 0 0 0\n"), Err(ReadError::Empty)));
    }

    #[test]
    fn rejects_non_finite() {
        let b = binary(&[[[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]);
        assert!(read_stl(&b).is_err());
    }
}
