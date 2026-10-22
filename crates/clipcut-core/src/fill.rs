//! Background fill, CapCut's Canvas › Blur: a clip that doesn't fill the frame (a vertical video in
//! a horizontal one, or a horizontal one in a vertical video) shows its own picture under itself,
//! zoomed to cover the frame, blurred and darkened, and the clip can have rounded corners. It is the
//! engine's Blur Fill effect on the clip (`filmcraft_render::blur_fill`), so filters and colour
//! show on both, and it follows every edit of the clip.

use filmcraft_project::{ClipId, TrackItem};
use serde::Serialize;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

/// The engine's effect.
const FILL: &str = "blur_fill";

/// A clip's background fill.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillView {
    /// How soft the background is, 0…200 (pixels of a 1080-line frame).
    pub blur: f64,
    /// How much darker it is, 0…100 %.
    pub darken: f64,
    /// How far it is zoomed beyond covering the frame, 100…400 %.
    pub zoom: f64,
    /// The clip's corner radius, 0…50 % of its shorter side.
    pub rounding: f64,
}

/// Picture clip `i`'s background fill, when it has one.
pub(crate) fn fill_of(i: &TrackItem) -> Option<FillView> {
    let e = i.effects.iter().find(|e| e.effect == FILL && e.enabled)?;
    let f = |id: &str, d: f64| e.param(id).and_then(|p| p.value.as_f64()).filter(|v| v.is_finite()).unwrap_or(d);
    Some(FillView { blur: f("blurriness", 60.0), darken: f("darken", 25.0), zoom: f("zoom", 100.0), rounding: f("rounding", 0.0) })
}

impl Studio {
    /// Gives `clips` (their pictures) a background fill of `blur`, `darken`, `zoom` and
    /// `rounding` (see [`FillView`]); `None` takes it off. Calls with the same `merge` key (a
    /// dragged slider) are one undo step.
    pub fn set_fill(&mut self, clips: &[u64], fill: Option<(f64, f64, f64, f64)>, merge: Option<&str>) -> Result<()> {
        let pictures = self.pictures_of(clips)?;
        if pictures.is_empty() {
            return Err(Error::Invalid("select a video or picture clip to fill the frame behind".into()));
        }
        let values = match fill {
            None => None,
            Some((blur, darken, zoom, rounding)) => {
                if ![blur, darken, zoom, rounding].iter().all(|v| v.is_finite()) {
                    return Err(Error::Invalid("the background's settings must be numbers".into()));
                }
                Some([
                    ("blurriness", blur.clamp(0.0, 200.0)),
                    ("darken", darken.clamp(0.0, 100.0)),
                    ("zoom", zoom.clamp(100.0, 400.0)),
                    ("rounding", rounding.clamp(0.0, 50.0)),
                ])
            }
        };
        self.group_merge(if values.is_some() { "Background Fill" } else { "No Background Fill" }, merge, |s| {
            for c in &pictures {
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let at = q.find_item(ClipId(*c)).and_then(|(_, i)| i.effects.iter().position(|e| e.effect == FILL));
                match (&values, at) {
                    (None, Some(index)) => {
                        s.session.execute("effects.remove", json!({"clip": c, "index": index})).map_err(engine)?;
                    }
                    (None, None) => {}
                    (Some(values), at) => {
                        if at.is_none() {
                            s.session.execute("effects.apply", json!({"clips": [c], "effect": FILL})).map_err(engine)?;
                        }
                        for (param, v) in values {
                            s.session.execute("effects.setParam", json!({"clip": c, "effect": FILL, "param": param, "value": v})).map_err(engine)?;
                        }
                    }
                }
            }
            Ok(())
        })
    }
}
