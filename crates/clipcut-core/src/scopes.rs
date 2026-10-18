//! Scopes of the program frame, drawn by the engine's scopes: a luma waveform, an RGB parade, a
//! vectorscope and a histogram, each as a picture with its scale lines, for the player's Scopes.

use filmcraft_scopes::{Matrix, ParadeType, Params, Signal, WaveformType, paint};

use crate::{Error, Frame, Result, Studio};

/// The scopes, by name: `waveform`, `parade`, `vectorscope`, `histogram`.
pub const SCOPES: &[&str] = &["waveform", "parade", "vectorscope", "histogram"];

const BG: [u8; 4] = [16, 16, 18, 255];
const LINE: [u8; 4] = [70, 68, 76, 255];

/// A picture `w × h` of the scope's dark background.
fn canvas(w: usize, h: usize) -> Vec<u8> {
    BG.iter().copied().cycle().take(w * h * 4).collect()
}

fn put(px: &mut [u8], w: usize, x: usize, y: usize, c: [u8; 4]) {
    if let Some(p) = px.get_mut((y * w + x) * 4..(y * w + x) * 4 + 4) {
        p.copy_from_slice(&c);
    }
}

/// Draws `src` (`sw × sh` RGBA with coverage in alpha) over `dst` at `(x0, y0)`.
fn over(dst: &mut [u8], dw: usize, src: &[u8], sw: usize, sh: usize, x0: usize, y0: usize) {
    for y in 0..sh {
        for x in 0..sw {
            let Some(s) = src.get((y * sw + x) * 4..(y * sw + x) * 4 + 4) else { continue };
            let a = u32::from(s[3]);
            if a == 0 {
                continue;
            }
            if let Some(d) = dst.get_mut(((y0 + y) * dw + x0 + x) * 4..((y0 + y) * dw + x0 + x) * 4 + 4) {
                for k in 0..3 {
                    d[k] = ((u32::from(s[k]) * a + u32::from(d[k]) * (255 - a)) / 255) as u8;
                }
            }
        }
    }
}

/// Lines at 0, 25, 50, 75 and 100 % across a `w × h` waveform.
fn levels(px: &mut [u8], w: usize, h: usize) {
    for k in 0..=4 {
        let y = ((h - 1) as f32 * (1.0 - k as f32 / 4.0)).round() as usize;
        for x in (0..w).step_by(if k % 2 == 0 { 1 } else { 3 }) {
            put(px, w, x, y, LINE);
        }
    }
}

impl Studio {
    /// Scope `kind` (one of [`SCOPES`]) of the program frame at `seconds`.
    pub fn scope(&self, seconds: f64, kind: &str) -> Result<Frame> {
        let f = self.frame(seconds, 480)?;
        let s = Signal::from_rgba8(f.w as usize, f.h as usize, &f.rgba);
        let p = Params::default();
        let (w, h, px) = match kind {
            "waveform" => {
                let wf = filmcraft_scopes::waveform(&s, WaveformType::Luma, &p);
                let y = wf.trace("Y").ok_or_else(|| Error::Engine("the waveform has no luma".into()))?;
                let (gw, gh, g) = paint::grids(&[(y, [0.55, 1.0, 0.65])], paint::waveform_reference(wf.per_column), 2.2);
                let mut px = canvas(gw, gh);
                levels(&mut px, gw, gh);
                over(&mut px, gw, &g, gw, gh, 0, 0);
                (gw, gh, px)
            }
            "parade" => {
                let wf = filmcraft_scopes::parade(&s, ParadeType::Rgb, &p);
                let parts: Vec<(usize, usize, Vec<u8>)> = [("R", [1.0, 0.35, 0.3]), ("G", [0.4, 1.0, 0.45]), ("B", [0.4, 0.55, 1.0])]
                    .iter()
                    .filter_map(|(n, c)| wf.trace(n).map(|g| paint::grids(&[(g, *c)], paint::waveform_reference(wf.per_column), 2.2)))
                    .collect();
                let gap = 8;
                let (gw, gh) = parts.first().map(|(w, h, _)| (*w, *h)).unwrap_or((0, 0));
                let w = (gw + gap) * parts.len().max(1) - gap;
                let mut px = canvas(w, gh);
                for (k, (pw, ph, g)) in parts.iter().enumerate() {
                    let x0 = k * (gw + gap);
                    let mut part = canvas(*pw, *ph);
                    levels(&mut part, *pw, *ph);
                    over(&mut part, *pw, g, *pw, *ph, 0, 0);
                    over(&mut px, w, &part, *pw, *ph, x0, 0);
                }
                (w, gh, px)
            }
            "vectorscope" => {
                let v = filmcraft_scopes::vectorscope_yuv(&s, &p);
                let (vw, vh, g) = paint::vectorscope(&v, [0.85, 0.95, 0.85], 2.0, true);
                let mut px = canvas(vw, vh);
                // the outer circle, the centre cross, and the six colour-bar targets at 75 %
                let c = (vw as f32 - 1.0) / 2.0;
                for a in 0..720 {
                    let r = (a as f32 / 2.0).to_radians();
                    put(&mut px, vw, (c + c * r.cos()).round() as usize, (c + c * r.sin()).round() as usize, LINE);
                }
                for d in 0..vw {
                    put(&mut px, vw, d, c as usize, LINE);
                    put(&mut px, vw, c as usize, d, LINE);
                }
                for t in filmcraft_scopes::targets(Matrix::Bt709, 0.75) {
                    if let Some((x, y)) = v.cell(t.cb, t.cr) {
                        for dy in 0..9 {
                            for dx in 0..9 {
                                if dx == 0 || dy == 0 || dx == 8 || dy == 8 {
                                    put(&mut px, vw, (x + dx).saturating_sub(4), (y + dy).saturating_sub(4), [160, 150, 120, 255]);
                                }
                            }
                        }
                    }
                }
                over(&mut px, vw, &g, vw, vh, 0, 0);
                (vw, vh, px)
            }
            "histogram" => {
                let hi = filmcraft_scopes::histogram(&s, &p);
                let (w, h) = (filmcraft_scopes::BINS, 160);
                let mut px = canvas(w, h);
                levels(&mut px, w, h);
                let top = hi.r.iter().chain(&hi.g).chain(&hi.b).copied().max().unwrap_or(1).max(1) as f32;
                for (bins, c) in [(&hi.r, [255u8, 80, 70, 110]), (&hi.g, [90, 255, 110, 110]), (&hi.b, [90, 130, 255, 110])] {
                    for (x, n) in bins.iter().enumerate().take(w) {
                        // square-root scale, so dark and bright tails stay visible
                        let bar = ((*n as f32 / top).sqrt() * (h - 1) as f32).round() as usize;
                        for y in (h - bar)..h {
                            if let Some(d) = px.get_mut((y * w + x) * 4..(y * w + x) * 4 + 4) {
                                for k in 0..3 {
                                    d[k] = d[k].saturating_add((u32::from(c[k]) * u32::from(c[3]) / 255) as u8);
                                }
                            }
                        }
                    }
                }
                (w, h, px)
            }
            other => return Err(Error::Invalid(format!("there is no scope called “{other}”"))),
        };
        Ok(Frame { w: u32::try_from(w).map_err(crate::engine)?, h: u32::try_from(h).map_err(crate::engine)?, rgba: px })
    }
}
