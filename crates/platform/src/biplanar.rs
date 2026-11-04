//! Biplanar 4:2:0 pictures (NV12, and P010 with its 10-bit samples in the high bits of 16-bit
//! words) as read back from a decoder surface, into the planar [`PixelData::Yuv8`] /
//! [`PixelData::Yuv16`] frames the rest of FilmCraft uses. Safe code, tested on every target.
//!
//! This is the one place the Windows backend copies pixels (surface to staging texture to these
//! planes). A zero-copy path would replace it, not the decoder around it.

use std::sync::Arc;

use filmcraft_frame::{Chroma, PixelData, VideoFrame, pool};

/// A mapped biplanar surface: a luma plane followed by interleaved chroma, `stride` bytes per row.
pub struct Biplanar<'a> {
    pub luma: &'a [u8],
    pub chroma: &'a [u8],
    pub stride: usize,
    /// Rows in the luma plane (the surface height, which may exceed the picture's).
    pub rows: usize,
}

/// Where the picture is in the surface and how its samples are stored.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    /// Output rectangle in luma samples: x, y, width, height.
    pub crop: (u32, u32, u32, u32),
    /// Significant bits per sample: 8 (NV12) or 10 (P010).
    pub bits: u32,
    pub color: filmcraft_color::ColorInfo,
    pub par: (u32, u32),
}

fn row(plane: &[u8], stride: usize, y: usize, x: usize, len: usize) -> Result<&[u8], String> {
    let start = y.checked_mul(stride).and_then(|s| s.checked_add(x)).ok_or("plane offset overflows")?;
    plane.get(start..start.checked_add(len).ok_or("plane offset overflows")?).ok_or_else(|| "plane row out of range".to_string())
}

/// Where the picture is in a surface, checked against it.
struct Layout {
    ox: usize,
    oy: usize,
    w: usize,
    h: usize,
    /// Bytes per sample: 1 (NV12) or 2 (P010).
    bps: usize,
    cw: usize,
    ch: usize,
    cox: usize,
    coy: usize,
}

fn layout(src: &Biplanar, g: &Geometry) -> Result<Layout, String> {
    let (cx, cy, w, h) = g.crop;
    let (ox, oy, w, h) = (cx as usize, cy as usize, w as usize, h as usize);
    if w == 0 || h == 0 || !matches!(g.bits, 8 | 10) {
        return Err(format!("cannot read a {w}x{h} picture of {} bits", g.bits));
    }
    let bps = if g.bits > 8 { 2 } else { 1 };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (cox, coy) = (ox / 2, oy / 2);
    let needs_rows = oy.checked_add(h).ok_or("picture size overflows")?;
    let needs_cols = ox.checked_add(w).and_then(|c| c.checked_mul(bps)).ok_or("picture size overflows")?;
    let needs_chroma = cox.checked_add(cw).and_then(|c| c.checked_mul(2 * bps)).ok_or("picture size overflows")?;
    if src.rows < needs_rows || src.stride < needs_cols || src.stride < needs_chroma {
        return Err(format!("decoded surface ({} rows, {} byte rows) is smaller than the {w}x{h} picture at {ox},{oy}", src.rows, src.stride));
    }
    Ok(Layout { ox, oy, w, h, bps, cw, ch, cox, coy })
}

/// [`to_frame`] at half the size each way, for draft playback: every other row of the
/// picture, with each pair of neighbouring samples across it averaged. It reads half of the
/// surface and writes a quarter of the samples, where a 4K picture read back from a hardware
/// decoder is the slow part of playing it.
// the Windows decoder's (Media Foundation); tested on every target
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn to_frame_halved(src: &Biplanar, g: &Geometry) -> Result<VideoFrame, String> {
    let Layout { ox, oy, w, h, bps, cw, ch, cox, coy } = layout(src, g)?;
    let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
    let (cw2, ch2) = (w2.div_ceil(2), h2.div_ceil(2));
    // averages sample `i` and the next one of a row of `n` (the last stands alone)
    let pair = |n: usize, i: usize| (i, if i + 1 < n { i + 1 } else { i });
    let data = if bps == 1 {
        let mut yp = pool::take_u8(w2 * h2);
        for y in 0..h2 {
            let r = row(src.luma, src.stride, oy + 2 * y, ox, w)?;
            for x in 0..w2 {
                let (a, b) = pair(w, 2 * x);
                let (Some(&a), Some(&b)) = (r.get(a), r.get(b)) else { return Err("luma row out of range".into()) };
                yp.push((u16::from(a) + u16::from(b)).div_ceil(2) as u8);
            }
        }
        let (mut u, mut v) = (pool::take_u8(cw2 * ch2), pool::take_u8(cw2 * ch2));
        for y in 0..ch2 {
            let pairs = row(src.chroma, src.stride, (coy + 2 * y).min(coy + ch - 1), cox * 2, cw * 2)?.as_chunks::<2>().0;
            for x in 0..cw2 {
                let (a, b) = pair(cw, 2 * x);
                let (Some(p), Some(q)) = (pairs.get(a), pairs.get(b)) else { return Err("chroma row out of range".into()) };
                u.push((u16::from(p[0]) + u16::from(q[0])).div_ceil(2) as u8);
                v.push((u16::from(p[1]) + u16::from(q[1])).div_ceil(2) as u8);
            }
        }
        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }
    } else {
        let shift = 16 - g.bits;
        let s = |b: &[u8; 2]| u32::from(u16::from_le_bytes(*b) >> shift);
        let mut yp = pool::take_u16(w2 * h2);
        for y in 0..h2 {
            let r = row(src.luma, src.stride, oy + 2 * y, ox * 2, w * 2)?.as_chunks::<2>().0;
            for x in 0..w2 {
                let (a, b) = pair(w, 2 * x);
                let (Some(a), Some(b)) = (r.get(a), r.get(b)) else { return Err("luma row out of range".into()) };
                yp.push((s(a) + s(b)).div_ceil(2) as u16);
            }
        }
        let (mut u, mut v) = (pool::take_u16(cw2 * ch2), pool::take_u16(cw2 * ch2));
        for y in 0..ch2 {
            let quads = row(src.chroma, src.stride, (coy + 2 * y).min(coy + ch - 1), cox * 4, cw * 4)?.as_chunks::<4>().0;
            for x in 0..cw2 {
                let (a, b) = pair(cw, 2 * x);
                let (Some(p), Some(q)) = (quads.get(a), quads.get(b)) else { return Err("chroma row out of range".into()) };
                u.push((s(&[p[0], p[1]]) + s(&[q[0], q[1]])).div_ceil(2) as u16);
                v.push((s(&[p[2], p[3]]) + s(&[q[2], q[3]])).div_ceil(2) as u16);
            }
        }
        PixelData::Yuv16 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, bits: g.bits, alpha: None }
    };
    Ok(VideoFrame { width: w2 as u32, height: h2 as u32, data, color: g.color, par: g.par, pts: filmcraft_time::Tick::ZERO })
}

/// Copy the picture at `g.crop` out of `src` into a planar 4:2:0 frame (deinterleaving chroma,
/// shifting 10-bit samples down from the high bits). Errors when the surface is smaller than the
/// rectangle needs.
pub fn to_frame(src: &Biplanar, g: &Geometry) -> Result<VideoFrame, String> {
    let Layout { ox, oy, w, h, bps, cw, ch, cox, coy } = layout(src, g)?;
    let data = if bps == 1 {
        let mut yp = pool::take_u8(w * h);
        for y in 0..h {
            yp.extend_from_slice(row(src.luma, src.stride, oy + y, ox, w)?);
        }
        let (mut u, mut v) = (pool::take_u8(cw * ch), pool::take_u8(cw * ch));
        for y in 0..ch {
            // two passes over the row: each is a simple strided copy the compiler vectorises
            let pairs = row(src.chroma, src.stride, coy + y, cox * 2, cw * 2)?.as_chunks::<2>().0;
            u.extend(pairs.iter().map(|p| p[0]));
            v.extend(pairs.iter().map(|p| p[1]));
        }
        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }
    } else {
        let shift = 16 - g.bits;
        let mut yp = pool::take_u16(w * h);
        for y in 0..h {
            yp.extend(row(src.luma, src.stride, oy + y, ox * 2, w * 2)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes([b[0], b[1]]) >> shift));
        }
        let (mut u, mut v) = (pool::take_u16(cw * ch), pool::take_u16(cw * ch));
        for y in 0..ch {
            let quads = row(src.chroma, src.stride, coy + y, cox * 4, cw * 4)?.as_chunks::<4>().0;
            u.extend(quads.iter().map(|q| u16::from_le_bytes([q[0], q[1]]) >> shift));
            v.extend(quads.iter().map(|q| u16::from_le_bytes([q[2], q[3]]) >> shift));
        }
        PixelData::Yuv16 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, bits: g.bits, alpha: None }
    };
    Ok(VideoFrame { width: w as u32, height: h as u32, data, color: g.color, par: g.par, pts: filmcraft_time::Tick::ZERO })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(crop: (u32, u32, u32, u32), bits: u32) -> Geometry {
        Geometry { crop, bits, color: filmcraft_color::ColorInfo::REC709, par: (1, 1) }
    }

    #[test]
    fn nv12_is_deinterleaved_and_cropped() {
        // 8x6 surface, stride 10; the picture is the 4x4 at (2, 2)
        let (stride, rows) = (10usize, 6usize);
        let luma: Vec<u8> = (0..stride * rows).map(|i| i as u8).collect();
        let chroma: Vec<u8> = (0..stride * rows / 2).map(|i| 100 + i as u8).collect();
        let f = to_frame(&Biplanar { luma: &luma, chroma: &chroma, stride, rows }, &geometry((2, 2, 4, 4), 8)).unwrap();
        let PixelData::Yuv8 { planes, chroma: c, .. } = &f.data else { panic!("8-bit") };
        assert_eq!(*c, Chroma::C420);
        assert_eq!(&planes[0][..], &[22, 23, 24, 25, 32, 33, 34, 35, 42, 43, 44, 45, 52, 53, 54, 55]);
        // chroma rows 1..3, pairs from byte 2: row 1 starts at 10, row 2 at 20
        assert_eq!(&planes[1][..], &[112, 114, 122, 124]);
        assert_eq!(&planes[2][..], &[113, 115, 123, 125]);
    }

    #[test]
    fn p010_shifts_down_from_the_high_bits() {
        let (stride, rows) = (8usize, 2usize);
        let mut luma = vec![0u8; stride * rows];
        for (i, s) in luma.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            *s = (((i as u16) + 1) << 6).to_le_bytes();
        }
        let mut chroma = vec![0u8; stride];
        for (i, s) in chroma.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            *s = ((500 + i as u16) << 6).to_le_bytes();
        }
        let f = to_frame(&Biplanar { luma: &luma, chroma: &chroma, stride, rows }, &geometry((0, 0, 4, 2), 10)).unwrap();
        let PixelData::Yuv16 { planes, bits, .. } = &f.data else { panic!("16-bit") };
        assert_eq!(*bits, 10);
        assert_eq!(&planes[0][..], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!((&planes[1][..], &planes[2][..]), (&[500, 502][..], &[501, 503][..]));
    }

    /// What [`to_frame_halved`] should give: the full frame, every other row, pairs averaged.
    fn halve_reference(f: &VideoFrame) -> (u32, u32, [Vec<u32>; 3]) {
        let planes: [Vec<u32>; 3] = match &f.data {
            PixelData::Yuv8 { planes, .. } => planes.clone().map(|p| p.iter().map(|v| u32::from(*v)).collect()),
            PixelData::Yuv16 { planes, .. } => planes.clone().map(|p| p.iter().map(|v| u32::from(*v)).collect()),
            _ => panic!("planar"),
        };
        let (w, h) = (f.width as usize, f.height as usize);
        let halve = |p: &[u32], pw: usize, ph: usize| -> Vec<u32> {
            let mut out = Vec::new();
            for y in (0..ph.div_ceil(2)).map(|y| 2 * y) {
                for x in (0..pw.div_ceil(2)).map(|x| 2 * x) {
                    let (a, b) = (p[y * pw + x], p[y * pw + (x + 1).min(pw - 1)]);
                    out.push((a + b).div_ceil(2));
                }
            }
            out
        };
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
        let mut c = [halve(&planes[0], w, h), halve(&planes[1], cw, ch), halve(&planes[2], cw, ch)];
        // the chroma of a halved picture is half of *its* size
        for k in 1..3 {
            c[k].truncate(w2.div_ceil(2) * h2.div_ceil(2));
        }
        (w2 as u32, h2 as u32, c)
    }

    #[test]
    fn a_halved_frame_is_every_other_row_with_pairs_averaged() {
        let mut seed = 99u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for (crop, bits) in [((0, 0, 8, 6), 8), ((2, 2, 7, 5), 8), ((0, 0, 3840 / 64, 2160 / 64), 10), ((2, 0, 9, 7), 10), ((0, 0, 1, 1), 8)] {
            let (stride, rows) = (160usize, 40usize);
            let luma: Vec<u8> = (0..stride * rows).map(|_| next() as u8).collect();
            let chroma: Vec<u8> = (0..stride * rows / 2).map(|_| next() as u8).collect();
            // P010 keeps its samples in the high bits: clear the low ones as a decoder does
            let fix =
                |v: Vec<u8>| if bits == 10 { v.chunks(2).flat_map(|c| ((u16::from_le_bytes([c[0], c[1]]) >> 6) << 6).to_le_bytes()).collect() } else { v };
            let (luma, chroma) = (fix(luma), fix(chroma));
            let src = Biplanar { luma: &luma, chroma: &chroma, stride, rows };
            let full = to_frame(&src, &geometry(crop, bits)).unwrap();
            let half = to_frame_halved(&src, &geometry(crop, bits)).unwrap();
            let (w2, h2, want) = halve_reference(&full);
            assert_eq!((half.width, half.height), (w2, h2), "{crop:?}");
            let got: [Vec<u32>; 3] = match &half.data {
                PixelData::Yuv8 { planes, .. } => planes.clone().map(|p| p.iter().map(|v| u32::from(*v)).collect()),
                PixelData::Yuv16 { planes, bits: b, .. } => {
                    assert_eq!(*b, bits);
                    planes.clone().map(|p| p.iter().map(|v| u32::from(*v)).collect())
                }
                _ => panic!("planar"),
            };
            assert_eq!(got, want, "{crop:?} at {bits} bits");
        }
        // the same refusals as the full size
        let small = Biplanar { luma: &[0u8; 64], chroma: &[0u8; 32], stride: 8, rows: 8 };
        assert!(to_frame_halved(&small, &geometry((0, 0, 9, 8), 8)).is_err());
        assert!(to_frame_halved(&small, &geometry((0, 0, 4, 4), 12)).is_err());
    }

    #[test]
    fn surfaces_smaller_than_the_picture_are_errors() {
        let luma = vec![0u8; 64];
        let chroma = vec![0u8; 32];
        let src = Biplanar { luma: &luma, chroma: &chroma, stride: 8, rows: 8 };
        for crop in [(0, 0, 9, 8), (0, 0, 8, 9), (4, 0, 8, 8), (0, 0, 0, 8), (u32::MAX, 0, 8, 8), (0, 0, 8, 8)] {
            let r = to_frame(&src, &geometry(crop, 8));
            assert_eq!(r.is_ok(), crop == (0, 0, 8, 8), "{crop:?}");
        }
        assert!(to_frame(&src, &geometry((0, 0, 4, 4), 12)).is_err());
        // a short chroma plane is an error, not a panic
        let short = Biplanar { luma: &luma, chroma: &chroma[..8], stride: 8, rows: 8 };
        assert!(to_frame(&short, &geometry((0, 0, 8, 8), 8)).is_err());
    }
}
