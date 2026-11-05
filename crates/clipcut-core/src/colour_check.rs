//! Check colour: how saturated each video's footage is as recorded, as a standard conversion to
//! normal (SDR) video would show it, and as ClipCut's player shows it, at three moments of its
//! first clip on the timeline, with a report to copy (like Check playback).
//!
//! Saturation is HSV saturation of each pixel ((max − min) / max of R′G′B′, 0–100), averaged over
//! the pixels that aren't near black, with the 90th percentile; chroma is the strength of the
//! colour in Y′CbCr (√(Cb² + Cr²), 0–100). The footage's numbers are of its own signal (for
//! HDR, BT.2020 primaries, which make the same colour read less saturated than in BT.709). The
//! standard conversion is the ITU recipe with no tone mapping: HLG or PQ to light (HLG's system
//! gamma for a 1000-nit display), 203 nits as white, BT.2020 to BT.709, clipped, sRGB, the
//! colorimetric target. The player's picture is the frame as the player shows it when paused.

use filmcraft_frame::{PixelData, VideoFrame};
use filmcraft_media::FrameRequest;
use filmcraft_time::Tick;
use serde::Serialize;

use crate::{FrameFeed, MediaProbe};

/// Pixels darker than this (max R′G′B′) are left out: their hue is noise.
const DARK: f32 = 0.06;
/// The size the footage is read at (of its own), and every how many pixels are measured.
const READ_SCALE: f32 = 0.25;
const EVERY: usize = 2;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColourVideo {
    pub name: String,
    /// Plain-words findings.
    pub findings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColourReport {
    pub videos: Vec<ColourVideo>,
    /// Everything measured, to copy.
    pub text: String,
}

/// Saturation and chroma of a picture's pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Saturation {
    /// Mean HSV saturation, 0–100.
    pub mean: f64,
    /// 90th percentile of it.
    pub p90: f64,
    /// Mean chroma, 0–100.
    pub chroma: f64,
    /// Pixels measured.
    pub pixels: usize,
}

impl Saturation {
    fn words(&self) -> String {
        format!("saturation {:.1} (90% under {:.0}), chroma {:.1}", self.mean, self.p90, self.chroma)
    }
}

/// Measures R′G′B′ triples (0–1, gamma-encoded) with their chroma `(cb, cr)`.
fn measure(px: impl Iterator<Item = ([f32; 3], f32)>) -> Saturation {
    let mut sats = Vec::new();
    let mut chroma = 0f64;
    for (c, ch) in px {
        let c = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        if mx < DARK {
            continue;
        }
        sats.push((mx - mn) / mx);
        chroma += f64::from(ch);
    }
    if sats.is_empty() {
        return Saturation::default();
    }
    let n = sats.len();
    let mean = sats.iter().map(|s| f64::from(*s)).sum::<f64>() / n as f64;
    sats.sort_by(f32::total_cmp);
    let p90 = f64::from(sats.get((n * 9 / 10).min(n - 1)).copied().unwrap_or(0.0));
    Saturation { mean: mean * 100.0, p90: p90 * 100.0, chroma: chroma / n as f64 * 100.0, pixels: n }
}

/// BT.709 chroma of a gamma-encoded RGB triple.
fn chroma_709(c: [f32; 3]) -> f32 {
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (cb, cr) = ((c[2] - y) / 1.8556, (c[0] - y) / 1.5748);
    (cb * cb + cr * cr).sqrt()
}

/// The player's picture (sRGB bytes, RGBA).
pub fn measure_rgba8(rgba: &[u8], w: usize) -> Saturation {
    let rows = rgba.chunks_exact((w * 4).max(4)).step_by(EVERY);
    measure(rows.flat_map(|row| {
        row.chunks_exact(4).step_by(EVERY).map(|p| {
            let c = [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0);
            (c, chroma_709(c))
        })
    }))
}

/// HLG's inverse OETF (BT.2100): signal → scene light, 0–1.
fn hlg_to_light(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    if e <= 0.5 { e * e / 3.0 } else { (((e - C) / A).exp() + B) / 12.0 }
}

/// PQ's EOTF (ST 2084): signal → nits.
fn pq_to_nits(e: f32) -> f32 {
    let (m1, m2, c1, c2, c3) = (0.159_301_76_f32, 78.843_75_f32, 0.835_937_5_f32, 18.851_562_f32, 18.6875_f32);
    let p = e.max(0.0).powf(1.0 / m2);
    10000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
}

fn srgb_encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// The footage's R′G′B′ as a standard conversion to SDR BT.709 shows it (see the module docs);
/// SDR footage is as it is.
fn standard_sdr(c: [f32; 3], transfer: filmcraft_color::Transfer, bt2020: bool) -> [f32; 3] {
    use filmcraft_color::Transfer;
    let lin = match transfer {
        Transfer::Hlg => {
            let s = c.map(hlg_to_light);
            // the OOTF for a 1000-nit display: system gamma 1.2 on the scene's luminance
            let ys = (0.2627 * s[0] + 0.678 * s[1] + 0.0593 * s[2]).max(0.0);
            let g = 1000.0 * ys.powf(0.2);
            s.map(|v| v * g / 203.0)
        }
        Transfer::Pq => c.map(|v| pq_to_nits(v) / 203.0),
        _ => return c,
    };
    let lin = if bt2020 {
        [
            1.6605 * lin[0] - 0.5876 * lin[1] - 0.0728 * lin[2],
            -0.1246 * lin[0] + 1.1329 * lin[1] - 0.0083 * lin[2],
            -0.0182 * lin[0] - 0.1006 * lin[1] + 1.1187 * lin[2],
        ]
    } else {
        lin
    };
    lin.map(srgb_encode)
}

/// The footage's own R′G′B′ (0–1) and chroma, read from its Y′CbCr, at every [`EVERY`]-th pixel;
/// also its standard SDR conversion. `None` for pictures that aren't Y′CbCr.
fn footage(frame: &VideoFrame) -> Option<(Saturation, Saturation)> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let info = frame.color;
    let (kr, kb) = info.matrix.kr_kb();
    let kg = 1.0 - kr - kb;
    let full = info.range == filmcraft_color::Range::Full;
    let norm = |v: f32, bits: u32, luma: bool| {
        let s = (1u32 << bits.saturating_sub(8)) as f32;
        let max = ((1u64 << bits.min(16)) - 1) as f32;
        match (full, luma) {
            (true, true) => v / max,
            (true, false) => (v - max.div_euclid(2.0) - 0.5) / max,
            (false, true) => (v - 16.0 * s) / (219.0 * s),
            (false, false) => (v - 128.0 * s) / (224.0 * s),
        }
    };
    let get = |p: &[f32; 3]| {
        let (y, cb, cr) = (p[0], p[1], p[2]);
        let r = y + 2.0 * (1.0 - kr) * cr;
        let b = y + 2.0 * (1.0 - kb) * cb;
        let g = (y - kr * r - kb * b) / kg;
        ([r, g, b], (cb * cb + cr * cr).sqrt())
    };
    let samples: Vec<[f32; 3]> = match &frame.data {
        PixelData::Yuv8 { planes, chroma, .. } => {
            let (sx, sy) = chroma.shifts();
            let cw = w.div_ceil(1 << sx);
            (0..h)
                .step_by(EVERY)
                .flat_map(|y| (0..w).step_by(EVERY).map(move |x| (x, y)))
                .filter_map(|(x, y)| {
                    let c = (y >> sy) * cw + (x >> sx);
                    Some([
                        norm(f32::from(*planes[0].get(y * w + x)?), 8, true),
                        norm(f32::from(*planes[1].get(c)?), 8, false),
                        norm(f32::from(*planes[2].get(c)?), 8, false),
                    ])
                })
                .collect()
        }
        PixelData::Yuv16 { planes, chroma, bits, .. } => {
            let (sx, sy) = chroma.shifts();
            let cw = w.div_ceil(1 << sx);
            let bits = *bits;
            (0..h)
                .step_by(EVERY)
                .flat_map(|y| (0..w).step_by(EVERY).map(move |x| (x, y)))
                .filter_map(|(x, y)| {
                    let c = (y >> sy) * cw + (x >> sx);
                    Some([
                        norm(f32::from(*planes[0].get(y * w + x)?), bits, true),
                        norm(f32::from(*planes[1].get(c)?), bits, false),
                        norm(f32::from(*planes[2].get(c)?), bits, false),
                    ])
                })
                .collect()
        }
        _ => return None,
    };
    let own = measure(samples.iter().map(get));
    let bt2020 = info.primaries == filmcraft_color::Primaries::Bt2020;
    let std = measure(samples.iter().map(|p| {
        let c = standard_sdr(get(p).0.map(|v| v.clamp(0.0, 1.0)), info.transfer, bt2020);
        (c, chroma_709(c))
    }));
    Some((own, std))
}

fn ratio(a: f64, b: f64) -> String {
    if b > 0.01 { format!("×{:.2}", a / b) } else { "–".into() }
}

/// `m:ss.s`
fn at(secs: f64) -> String {
    let s = secs.max(0.0);
    format!("{}:{:04.1}", (s / 60.0).floor() as u64, s % 60.0)
}

/// Runs the colour check over `probes` (the player `feed`, when there is a timeline). `version`
/// heads the report.
pub fn colour_check(probes: &[MediaProbe], feed: Option<&FrameFeed>, version: &str) -> ColourReport {
    let mut text = format!(
        "ClipCut {version} colour check\nSaturation: 0 = grey, 100 = fully saturated (HSV, pixels not near black; 90% of them under the second number). Chroma: colour strength in Y'CbCr, 0-100.\nfootage = the file's own signal; standard = the ITU conversion to SDR BT.709 (no tone mapping); player = ClipCut's picture.\n"
    );
    let mut videos = Vec::new();
    for p in probes {
        let Some(v) = p.info.video.as_ref() else { continue };
        let mut findings = Vec::new();
        text += &format!(
            "\n{}\n  {}, {}x{}, {:?} transfer, {:?} primaries, {:?} range\n",
            p.name, v.codec, v.width, v.height, v.color.transfer, v.color.primaries, v.color.range
        );
        let mut sums = (0.0, 0.0, 0.0, 0usize);
        if p.moments.is_empty() {
            text += "  not on the timeline: only measured when it is\n";
            findings.push("Not on the timeline: put it there to compare the player's picture.".into());
        }
        for (t, media) in &p.moments {
            let frame = p.source.video_frame(FrameRequest { time: Tick::from_seconds_f64(*media), scale: READ_SCALE });
            let Some((own, std)) = frame.as_ref().ok().and_then(|f| footage(f)) else {
                text += &format!("  at {}: the footage couldn't be read\n", at(*t));
                continue;
            };
            let shown = feed.and_then(|f| f.render(f.frame_at(*t), false).ok()).map(|f| measure_rgba8(&f.rgba, f.w as usize));
            text += &format!("  at {} (footage {}):\n    footage  {}\n    standard {}\n", at(*t), at(*media), own.words(), std.words());
            if let Some(s) = shown {
                text += &format!(
                    "    player   {}\n    player vs standard: saturation {}, chroma {}\n",
                    s.words(),
                    ratio(s.mean, std.mean),
                    ratio(s.chroma, std.chroma)
                );
                sums = (sums.0 + s.mean, sums.1 + std.mean, sums.2 + s.chroma / std.chroma.max(0.01), sums.3 + 1);
            }
        }
        if sums.3 > 0 {
            let n = sums.3 as f64;
            let (player, standard) = (sums.0 / n, sums.1 / n);
            let r = if standard > 0.01 { player / standard } else { 1.0 };
            text += &format!("  on average: player vs standard saturation ×{r:.2}, chroma ×{:.2}\n", sums.2 / n);
            findings.push(format!(
                "Saturation: the player {player:.0}, a standard conversion of the footage {standard:.0} ({}).",
                if (r - 1.0).abs() < 0.05 {
                    "about the same".to_string()
                } else if r > 1.0 {
                    format!("{:.0}% more saturated", (r - 1.0) * 100.0)
                } else {
                    format!("{:.0}% less saturated", (1.0 - r) * 100.0)
                }
            ));
        }
        videos.push(ColourVideo { name: p.name.clone(), findings });
    }
    if videos.is_empty() {
        text += "\nNo videos in the project.\n";
    }
    ColourReport { videos, text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_is_unsaturated_and_pure_colours_are_full() {
        let grey = measure_rgba8(&[128, 128, 128, 255].repeat(16), 4);
        assert!(grey.mean < 0.1 && grey.chroma < 0.1 && grey.pixels > 0, "{grey:?}");
        let red = measure_rgba8(&[255, 0, 0, 255].repeat(16), 4);
        assert!((red.mean - 100.0).abs() < 0.1 && red.chroma > 40.0, "{red:?}");
        // black is left out
        assert_eq!(measure_rgba8(&[0, 0, 0, 255].repeat(16), 4).pixels, 0);
        assert_eq!(measure_rgba8(&[], 0).pixels, 0);
    }

    #[test]
    fn the_standard_conversion_follows_the_itu_recipe() {
        use filmcraft_color::Transfer;
        // SDR is as it is
        assert_eq!(standard_sdr([0.2, 0.4, 0.6], Transfer::Bt709, false), [0.2, 0.4, 0.6]);
        // HLG reference white (75% signal) lands near SDR white
        let w = standard_sdr([0.75; 3], Transfer::Hlg, true);
        assert!(w.iter().all(|v| (*v - 1.0).abs() < 0.03), "{w:?}");
        // a BT.2020 colour is more saturated in BT.709 code values
        let g = standard_sdr([0.3, 0.6, 0.3], Transfer::Hlg, true);
        assert!(g[1] > g[0] && g[1] > g[2], "{g:?}");
        assert!((pq_to_nits(1.0) - 10000.0).abs() < 1.0 && pq_to_nits(0.0) < 1e-3);
    }
}
