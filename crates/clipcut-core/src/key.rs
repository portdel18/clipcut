//! Remove a colour (green screen, blue screen, any flat background) from a picture clip: the
//! engine's Ultra Key on the clip, with its colour, how strongly it keys (the effect's Relaxed,
//! Default and Aggressive settings) and how soft the edges are. The colour can be picked from the
//! video ([`Studio::pick_colour`]), as it looks without the key.

use std::sync::Arc;

use filmcraft_project::{ClipId, ParamValue, TrackItem, TrackKind};
use serde::Serialize;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

/// The engine's keyer.
const KEY: &str = "ultra_key";
/// How strongly it keys, as the interface names it, and the keyer's Setting for each.
const STRENGTHS: [(&str, &str); 3] = [("gentle", "Relaxed"), ("normal", "Default"), ("strong", "Aggressive")];

/// A clip's colour key.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    /// `#rrggbb`.
    pub color: String,
    /// `gentle`, `normal` or `strong`.
    pub strength: String,
    /// Edge softness, 0…100.
    pub soften: f64,
    /// How much of the colour's tint is taken off what stays (green spill on hair), 0…100.
    pub spill: f64,
}

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// Picture clip `i`'s colour key, when it has one.
pub(crate) fn key_of(i: &TrackItem) -> Option<KeyView> {
    let e = i.effects.iter().find(|e| e.effect == KEY && e.enabled)?;
    let color = match e.param("key_color").map(|p| &p.value) {
        Some(ParamValue::Color(c)) => hex(*c),
        _ => "#00cc33".into(),
    };
    let setting = match e.param("setting").map(|p| &p.value) {
        Some(ParamValue::Choice(k)) => *k,
        _ => 0,
    };
    let strength = match setting {
        1 => "gentle",
        2 => "strong",
        _ => "normal",
    };
    let f = |id: &str, d: f64| e.param(id).and_then(|p| p.value.as_f64()).filter(|v| v.is_finite()).unwrap_or(d);
    Some(KeyView { color, strength: strength.into(), soften: f("soften", 0.0), spill: f("spill", 50.0) })
}

/// `#rrggbb` (or `#rgb`) as a colour, or `None`.
fn parse_hex(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches('#');
    let full = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h.to_string(),
        _ => return None,
    };
    full.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("#{}", full.to_ascii_lowercase()))
}

impl Studio {
    /// The picture clips among `clips` (video and pictures; not text, sound or captions).
    pub(crate) fn pictures_of(&self, clips: &[u64]) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        Ok(clips
            .iter()
            .copied()
            .filter(|c| {
                q.find_item(ClipId(*c)).is_some_and(|(t, i)| {
                    q.track(t).is_some_and(|t| t.kind == TrackKind::Video)
                        && matches!(self.session.project.item(i.item).map(|it| &it.kind), Some(filmcraft_project::ItemKind::Media(_)))
                })
            })
            .collect())
    }

    /// Removes colour `color` (`#rrggbb`) from `clips`, keying `strength` (`gentle`, `normal`,
    /// `strong`) with edges softened by `soften` and the colour's tint taken off by `spill`
    /// (0…100); `None` puts the colour back. Calls with the same `merge` key (a dragged slider) are
    /// one undo step.
    pub fn set_key(&mut self, clips: &[u64], key: Option<(&str, &str, f64, f64)>, merge: Option<&str>) -> Result<()> {
        let pictures = self.pictures_of(clips)?;
        if pictures.is_empty() {
            return Err(Error::Invalid("select a video or picture clip to remove a colour from".into()));
        }
        let settings = match key {
            None => None,
            Some((color, strength, soften, spill)) => {
                let color = parse_hex(color).ok_or_else(|| Error::Invalid(format!("{color} is not a colour: use #rrggbb")))?;
                let setting = STRENGTHS
                    .iter()
                    .find(|(k, _)| *k == strength)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| Error::Invalid(format!("{strength} is not a strength: gentle, normal or strong")))?;
                if !soften.is_finite() || !spill.is_finite() {
                    return Err(Error::Invalid("the edge softness and the tint taken off must be numbers".into()));
                }
                Some((color, setting, soften.clamp(0.0, 100.0), spill.clamp(0.0, 100.0)))
            }
        };
        self.group_merge(if settings.is_some() { "Remove Colour" } else { "Keep Colour" }, merge, |s| {
            for c in &pictures {
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let at = q.find_item(ClipId(*c)).and_then(|(_, i)| i.effects.iter().position(|e| e.effect == KEY));
                match (&settings, at) {
                    (None, Some(index)) => {
                        s.session.execute("effects.remove", json!({"clip": c, "index": index})).map_err(engine)?;
                    }
                    (None, None) => {}
                    (Some((color, setting, soften, spill)), at) => {
                        if at.is_none() {
                            s.session.execute("effects.apply", json!({"clips": [c], "effect": KEY})).map_err(engine)?;
                        }
                        for (param, v) in [("key_color", json!(color)), ("setting", json!(setting)), ("soften", json!(soften)), ("spill", json!(spill))] {
                            s.session.execute("effects.setParam", json!({"clip": c, "effect": KEY, "param": param, "value": v})).map_err(engine)?;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// The colour (`#rrggbb`) at canvas pixel `x`, `y` of the frame at `seconds`, as it looks
    /// without colour keys (so a colour can be picked again once it is removed): the average of a
    /// small square around it.
    pub fn pick_colour(&self, seconds: f64, x: f64, y: f64) -> Result<String> {
        let info = self.info()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(Error::Invalid("the point must be numbers".into()));
        }
        let mut feed = self.frame_feed(info.width.max(16), "full")?;
        let mut p = (*feed.project).clone();
        if let Some(q) = p.sequence_mut(feed.sequence) {
            for t in q.video_tracks.iter_mut() {
                for i in t.items.iter_mut() {
                    i.effects.retain(|e| e.effect != KEY);
                }
            }
        }
        feed.project = Arc::new(p);
        if !seconds.is_finite() {
            return Err(Error::Invalid("the time must be a number".into()));
        }
        let f = feed.render(feed.frame_at(seconds), false)?;
        let (w, h) = (f.w as i64, f.h as i64);
        if w == 0 || h == 0 {
            return Err(Error::Engine("the frame is empty".into()));
        }
        // canvas pixels to frame pixels
        let sx = (x / f64::from(info.width.max(1)) * w as f64).floor() as i64;
        let sy = (y / f64::from(info.height.max(1)) * h as f64).floor() as i64;
        if sx < 0 || sy < 0 || sx >= w || sy >= h {
            return Err(Error::Invalid("pick a point on the picture".into()));
        }
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        for yy in (sy - 2).max(0)..=(sy + 2).min(h - 1) {
            for xx in (sx - 2).max(0)..=(sx + 2).min(w - 1) {
                let k = ((yy * w + xx) * 4) as usize;
                if let Some(px) = f.rgba.get(k..k + 3) {
                    for (s, v) in sum.iter_mut().zip(px) {
                        *s += u64::from(*v);
                    }
                    n += 1;
                }
            }
        }
        let n = n.max(1);
        Ok(format!("#{:02x}{:02x}{:02x}", sum[0] / n, sum[1] / n, sum[2] / n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_back() {
        assert_eq!(parse_hex("#00B140").as_deref(), Some("#00b140"));
        assert_eq!(parse_hex("0f0").as_deref(), Some("#00ff00"));
        assert!(parse_hex("green").is_none() && parse_hex("#12345").is_none() && parse_hex("#gg0000").is_none());
        assert_eq!(hex([0.0, 1.0, 0.25, 1.0]), "#00ff40");
    }
}
