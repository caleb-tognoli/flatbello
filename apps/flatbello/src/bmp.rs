//! Minimal 24-bit BMP writer for `--render-test` snapshots.

/// Encodes tightly packed RGBA8 pixels, top row first, as an uncompressed bottom-up BMP.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row = (width as usize * 3).div_ceil(4) * 4;
    let data_len = row * height as usize;
    let file_len = 54 + data_len;
    let mut out = Vec::with_capacity(file_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_len as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    for y in (0..height as usize).rev() {
        let start = out.len();
        for x in 0..width as usize {
            let i = (y * width as usize + x) * 4;
            let px = rgba.get(i..i + 3).unwrap_or(&[0, 0, 0]);
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        out.resize(start + row, 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_padding() {
        let px = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255];
        let bmp = encode(3, 1, &px);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 54 + 12);
        // Blue-green-red order, padded to 12 bytes.
        assert_eq!(&bmp[54..63], &[0, 0, 255, 0, 255, 0, 255, 0, 0]);
    }
}

/// Levels per channel of the fixed palette used by [`encode_rle8`]: 6 × 7 × 6 = 252 colours.
const LEVELS: [u32; 3] = [6, 7, 6];

fn palette_index(r: u8, g: u8, b: u8) -> u8 {
    let q = |v: u8, n: u32| (u32::from(v) * (n - 1) + 127) / 255;
    (q(r, LEVELS[0]) * LEVELS[1] * LEVELS[2] + q(g, LEVELS[1]) * LEVELS[2] + q(b, LEVELS[2])) as u8
}

/// Encodes RGBA8 pixels (top row first) as an 8-bit run-length-encoded BMP with a fixed
/// 252-colour palette. Much smaller than [`encode`] for flat UI screenshots.
pub fn encode_rle8(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    for y in (0..height as usize).rev() {
        let row: Vec<u8> = (0..width as usize)
            .map(|x| {
                let i = (y * width as usize + x) * 4;
                let px = rgba.get(i..i + 3).unwrap_or(&[0, 0, 0]);
                palette_index(px[0], px[1], px[2])
            })
            .collect();
        let mut x = 0;
        while x < row.len() {
            let mut n = 1;
            while x + n < row.len() && n < 255 && row[x + n] == row[x] {
                n += 1;
            }
            data.extend_from_slice(&[n as u8, row[x]]);
            x += n;
        }
        data.extend_from_slice(&[0, 0]);
    }
    data.extend_from_slice(&[0, 1]);

    let colors = LEVELS[0] * LEVELS[1] * LEVELS[2];
    let offset = 54 + 4 * colors;
    let mut out = Vec::with_capacity(offset as usize + data.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(offset + data.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // BI_RLE8
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&colors.to_le_bytes());
    out.extend_from_slice(&colors.to_le_bytes());
    let level = |i: u32, n: u32| (i * 255 / (n - 1)) as u8;
    for r in 0..LEVELS[0] {
        for g in 0..LEVELS[1] {
            for b in 0..LEVELS[2] {
                out.extend_from_slice(&[
                    level(b, LEVELS[2]),
                    level(g, LEVELS[1]),
                    level(r, LEVELS[0]),
                    0,
                ]);
            }
        }
    }
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod rle_tests {
    use super::*;

    #[test]
    fn runs_and_markers() {
        let white = [255u8, 255, 255, 255];
        let px: Vec<u8> = white.iter().copied().cycle().take(4 * 300).collect();
        let bmp = encode_rle8(300, 1, &px);
        let data = &bmp[54 + 4 * 252..];
        assert_eq!(data, &[255, 251, 45, 251, 0, 0, 0, 1]);
        assert_eq!(palette_index(0, 0, 0), 0);
    }
}
