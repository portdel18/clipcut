//! Blur Fill: a clip that doesn't fill the frame (a vertical video in a horizontal one, or the
//! other way round) shows its own picture under itself, zoomed to cover the whole frame, blurred
//! and darkened; the clip itself can have its corners rounded. It works where the clip is placed
//! ([`crate::item_layer_ex`]), on the layer as its other effects left it, so filters show on both.
//! Not a GPU effect: [`crate::plan`] renders such a clip on the CPU.

use filmcraft_geom::Affine;
use filmcraft_project::EffectInstance;
use filmcraft_time::Tick;

use crate::Image;

/// The picture under the layer: `layer` zoomed to cover `w × h` (centred, times the effect's
/// Zoom), blurred and darkened. Worked at a quarter of the size: the blur hides it.
pub(crate) fn background(layer: &Image, w: usize, h: usize, e: &EffectInstance, t: Tick) -> Image {
    if layer.w == 0 || layer.h == 0 || w == 0 || h == 0 {
        return Image::new(w, h);
    }
    let zoom = (e.f64_at("zoom", t) / 100.0).clamp(1.0, 4.0);
    let blur = e.f64_at("blurriness", t).clamp(0.0, 200.0) as f32;
    let darken = (e.f64_at("darken", t) / 100.0).clamp(0.0, 1.0) as f32;
    let k = if w.min(h) >= 256 { 4.0 } else { 1.0 };
    let (bw, bh) = (((w as f64) / k).ceil().max(1.0) as usize, ((h as f64) / k).ceil().max(1.0) as usize);
    // as strong at any output size: Blurriness is in pixels of a 1080-line frame
    let sigma = blur * 0.5 * (w.min(h) as f32 / 1080.0) / k as f32;
    // drawn past the frame's edges by more than the blur reaches, so its soft edge stays outside
    let margin = (3.0 * sigma).ceil() as usize + 2;
    let (mw, mh) = (bw + 2 * margin, bh + 2 * margin);
    let s = (mw as f64 / layer.w as f64).max(mh as f64 / layer.h as f64) * zoom;
    let cover = Affine::translate(mw as f64 / 2.0, mh as f64 / 2.0)
        .then_apply(&Affine::scale(s, s))
        .then_apply(&Affine::translate(-(layer.w as f64) / 2.0, -(layer.h as f64) / 2.0));
    let mut small = layer.transformed(mw, mh, &cover);
    let (rx, ry) = crate::effects::gaussian_boxes(small.w, small.h, sigma, sigma);
    crate::effects::box_blur(&mut small, &rx, &ry, true);
    if darken > 0.0 {
        let keep = 1.0 - darken;
        for p in small.px.chunks_exact_mut(4) {
            p[0] *= keep;
            p[1] *= keep;
            p[2] *= keep;
        }
    }
    // back to the frame: its middle on the frame's middle, one small pixel `k` output pixels
    let (sx, sy) = (w as f64 / bw as f64, h as f64 / bh as f64);
    let up = Affine::translate(w as f64 / 2.0, h as f64 / 2.0)
        .then_apply(&Affine::scale(sx, sy))
        .then_apply(&Affine::translate(-(mw as f64) / 2.0, -(mh as f64) / 2.0));
    small.transformed(w, h, &up)
}

/// Rounds the layer's corners: Corner Rounding is the radius as a share of its shorter side
/// (50 % makes the short ends round). The edge is anti-aliased.
pub(crate) fn round_corners(layer: &mut Image, e: &EffectInstance, t: Tick) {
    let share = (e.f64_at("rounding", t) / 100.0).clamp(0.0, 0.5);
    let r = share * layer.w.min(layer.h) as f64;
    if r < 0.5 {
        return;
    }
    let (w, h) = (layer.w as f64, layer.h as f64);
    let cover = |x: f64, y: f64| -> f32 {
        // distance into the corner square, from the arc's centre
        let cx = if x < r {
            r
        } else if x > w - r {
            w - r
        } else {
            return 1.0;
        };
        let cy = if y < r {
            r
        } else if y > h - r {
            h - r
        } else {
            return 1.0;
        };
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        (r - d + 0.5).clamp(0.0, 1.0) as f32
    };
    let ri = r.ceil() as usize;
    for y in 0..layer.h {
        let fy = y as f64 + 0.5;
        if y >= ri && y + ri < layer.h {
            continue;
        }
        let row = y * layer.w * 4;
        for x in (0..layer.w).filter(|x| *x < ri || *x + ri >= layer.w) {
            let a = cover(x as f64 + 0.5, fy);
            if a < 1.0
                && let Some(p) = layer.px.get_mut(row + x * 4..row + x * 4 + 4)
            {
                for v in p {
                    *v *= a;
                }
            }
        }
    }
}
