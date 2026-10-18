//! Colour, CapCut-style: a picture clip's adjustments (exposure, contrast, white balance,
//! saturation…) and a filter look, all on the engine's Lumetri Color effect, which the clip gets
//! the first time it is adjusted and loses on Reset.

use std::collections::BTreeMap;

use filmcraft_project::{ClipId, EffectInstance, ParamKind, ParamValue, TrackKind, find_effect};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Error, Frame, Result, Studio, engine};

const LUMETRI: &str = "lumetri";

/// The adjustments: name, the Lumetri parameter, its range and its neutral value.
pub const ADJUST: &[(&str, &str, f64, f64, f64)] = &[
    ("exposure", "exposure", -5.0, 5.0, 0.0),
    ("contrast", "contrast", -100.0, 100.0, 0.0),
    ("highlights", "highlights", -100.0, 100.0, 0.0),
    ("shadows", "shadows", -100.0, 100.0, 0.0),
    ("whites", "whites", -100.0, 100.0, 0.0),
    ("blacks", "blacks", -100.0, 100.0, 0.0),
    ("temperature", "temperature", -100.0, 100.0, 0.0),
    ("tint", "tint", -100.0, 100.0, 0.0),
    ("saturation", "saturation", 0.0, 200.0, 100.0),
    ("vibrance", "vibrance", -100.0, 100.0, 0.0),
    ("fade", "faded_film", 0.0, 100.0, 0.0),
    ("sharpen", "sharpen", -100.0, 100.0, 0.0),
    ("vignette", "vignette_amount", -5.0, 5.0, 0.0),
];

/// A picture clip's colour, as the Adjust section and the Filters tab show it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColourView {
    /// Every [`ADJUST`] name and its value.
    pub values: BTreeMap<&'static str, f64>,
    /// The filter look, if any (one of [`looks`]).
    pub look: Option<String>,
    /// The look's strength, percent.
    pub intensity: f64,
}

/// The filter looks the engine's Lumetri has (its "Look" choices, without None).
pub fn looks() -> Vec<&'static str> {
    let Some(def) = find_effect(LUMETRI) else { return Vec::new() };
    match def.param("look").map(|p| &p.kind) {
        Some(ParamKind::Choice(opts)) => opts.iter().skip(1).copied().collect(),
        _ => Vec::new(),
    }
}

fn float(e: &EffectInstance, id: &str) -> Option<f64> {
    match e.param(id).map(|p| &p.value) {
        Some(ParamValue::Float(v)) if v.is_finite() => Some(*v),
        _ => None,
    }
}

/// A clip's colour, when it has been adjusted (a Lumetri effect).
pub(crate) fn colour_of(effects: &[EffectInstance]) -> Option<ColourView> {
    let e = effects.iter().find(|e| e.effect == LUMETRI)?;
    let values = ADJUST.iter().map(|(name, param, _, _, neutral)| (*name, float(e, param).unwrap_or(*neutral))).collect();
    let look = match e.param("look").map(|p| &p.value) {
        Some(ParamValue::Choice(k)) if *k > 0 => looks().get(*k as usize - 1).map(|s| s.to_string()),
        _ => None,
    };
    Some(ColourView { values, look, intensity: float(e, "look_intensity").unwrap_or(100.0) })
}

impl Studio {
    /// The picture clips among `clips` (colour has no meaning for sound).
    fn pictures(&self, clips: &[u64]) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let v: Vec<u64> =
            clips.iter().copied().filter(|c| q.find_item(ClipId(*c)).and_then(|(t, _)| q.track(t)).is_some_and(|t| t.kind == TrackKind::Video)).collect();
        if v.is_empty() {
            return Err(Error::Invalid("select a picture clip first".into()));
        }
        Ok(v)
    }

    /// Gives the clips a Lumetri effect where they have none.
    fn ensure_lumetri(&mut self, clips: &[u64]) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let bare: Vec<u64> =
            clips.iter().copied().filter(|c| q.find_item(ClipId(*c)).is_some_and(|(_, i)| !i.effects.iter().any(|e| e.effect == LUMETRI))).collect();
        if !bare.is_empty() {
            self.session.execute("effects.apply", json!({"clips": bare, "effect": LUMETRI})).map_err(engine)?;
        }
        Ok(())
    }

    /// Sets adjustments (`values`: [`ADJUST`] name → value) on the picture clips among `clips`.
    /// Calls with the same `merge` key (a dragged slider) are one undo step.
    pub fn set_colour(&mut self, clips: &[u64], values: &Value, merge: Option<&str>) -> Result<()> {
        let clips = self.pictures(clips)?;
        let obj = values.as_object().ok_or_else(|| Error::Invalid("nothing to change".into()))?;
        let mut set = Vec::new();
        for (k, v) in obj {
            let &(_, param, lo, hi, _) = ADJUST.iter().find(|(n, ..)| n == k).ok_or_else(|| Error::Invalid(format!("there is no adjustment called “{k}”")))?;
            let v = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| Error::Invalid(format!("{k} must be a number")))?;
            set.push((param, v.clamp(lo, hi)));
        }
        let key = merge.map(|m| format!("colour:{m}"));
        self.group_merge("Adjust Colour", key.as_deref(), |s| {
            s.ensure_lumetri(&clips)?;
            for c in &clips {
                for (param, v) in &set {
                    s.session.execute("effects.setParam", json!({"clip": c, "effect": LUMETRI, "param": param, "value": v})).map_err(engine)?;
                }
            }
            Ok(())
        })
    }

    /// Puts filter `look` (one of [`looks`], or none) on the picture clips among `clips` at
    /// `intensity` percent. One undo step.
    pub fn set_look(&mut self, clips: &[u64], look: Option<&str>, intensity: f64, merge: Option<&str>) -> Result<()> {
        let clips = self.pictures(clips)?;
        let index = match look {
            None => 0,
            Some(name) => looks()
                .iter()
                .position(|l| l.eq_ignore_ascii_case(name))
                .map(|i| i + 1)
                .ok_or_else(|| Error::Invalid(format!("there is no filter called “{name}”")))?,
        };
        if !intensity.is_finite() {
            return Err(Error::Invalid("the filter's strength must be a number".into()));
        }
        let key = merge.map(|m| format!("look:{m}"));
        self.group_merge("Filter", key.as_deref(), |s| {
            s.ensure_lumetri(&clips)?;
            for c in &clips {
                s.session.execute("effects.setParam", json!({"clip": c, "effect": LUMETRI, "param": "look", "value": index})).map_err(engine)?;
                s.session
                    .execute("effects.setParam", json!({"clip": c, "effect": LUMETRI, "param": "look_intensity", "value": intensity.clamp(0.0, 200.0)}))
                    .map_err(engine)?;
            }
            Ok(())
        })
    }

    /// Takes every adjustment and filter off the clips (their Lumetri effect goes). One undo step.
    pub fn reset_colour(&mut self, clips: &[u64]) -> Result<()> {
        let clips = self.pictures(clips)?;
        self.group("Reset Colour", |s| {
            for c in &clips {
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let Some(index) = q.find_item(ClipId(*c)).and_then(|(_, i)| i.effects.iter().position(|e| e.effect == LUMETRI)) else { continue };
                s.session.execute("effects.remove", json!({"clip": c, "index": index})).map_err(engine)?;
            }
            Ok(())
        })
    }

    /// Gives every clip on the main track `clip`'s colour (adjustments and filter). Returns how
    /// many clips changed. One undo step.
    pub fn colour_everywhere(&mut self, clip: u64) -> Result<usize> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, from) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let grade = from
            .effects
            .iter()
            .find(|e| e.effect == LUMETRI)
            .cloned()
            .ok_or_else(|| Error::Invalid("adjust this clip first: there is nothing to copy".into()))?;
        let targets: Vec<ClipId> = q.video_tracks.first().map(|t| t.items.iter().map(|i| i.id).filter(|id| id.0 != clip).collect()).unwrap_or_default();
        if targets.is_empty() {
            return Err(Error::Invalid("there are no other clips on the main track".into()));
        }
        let n = targets.len();
        self.group("Colour Everywhere", |s| {
            s.session
                .edit_sequence("Colour Everywhere", |q, _, _| {
                    for t in q.video_tracks.iter_mut().take(1) {
                        for i in t.items.iter_mut().filter(|i| targets.contains(&i.id)) {
                            if let Some(e) = i.effects.iter_mut().find(|e| e.effect == LUMETRI) {
                                *e = grade.clone();
                            } else {
                                let pos = i.effects.iter().position(|e| e.def().is_some_and(|d| d.intrinsic)).unwrap_or(i.effects.len());
                                i.effects.insert(pos, grade.clone());
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(engine)
        })?;
        Ok(n)
    }

    /// Clip `clip`'s own picture at `seconds` on the timeline (no effects), at most `w` wide,
    /// with each filter look on it in turn, for the Filters tab: the first frame has no look.
    pub fn look_previews(&self, clip: u64, seconds: f64, w: u32) -> Result<Vec<Frame>> {
        let t = self.frame_tick(seconds)?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, item) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let mt = item.source_time_at(t.clamp(item.start, (item.end() - filmcraft_time::Tick(1)).max(item.start)));
        let thumbs = self.thumbs();
        let base = thumbs.frame(item.item.0, mt.seconds(), w.clamp(32, 320))?;
        // the frame's sRGB pixels as the engine's linear-light picture
        let mut img = filmcraft_render::Image::new(base.w as usize, base.h as usize);
        for (d, s) in img.px.chunks_mut(4).zip(base.rgba.chunks(4)) {
            for k in 0..3 {
                d[k] = (f32::from(s[k]) / 255.0).powf(2.2);
            }
            d[3] = 1.0;
        }
        let def = find_effect(LUMETRI).ok_or_else(|| Error::Engine("the engine has no Lumetri Color".into()))?;
        let rate = q.settings.frame_rate.as_f64();
        let working = filmcraft_color::WorkingSpace::parse(&q.settings.working_space).unwrap_or_default();
        let mut out = vec![base];
        for k in 1..=looks().len() {
            let mut e = def.instance();
            e.params.insert("look".into(), filmcraft_project::Param::new(ParamValue::Choice(k as u32)));
            let cx = filmcraft_render::effects::FxCtx {
                t: filmcraft_time::Tick::ZERO,
                px_scale: 1.0,
                seconds: 0.0,
                timecode: "",
                clip_name: "",
                project: None,
                env: None,
                frame_rate: rate,
                working,
            };
            let mut one = img.clone();
            filmcraft_render::effects::apply(&mut one, &e, &cx);
            out.push(Frame { w: u32::try_from(one.w).map_err(engine)?, h: u32::try_from(one.h).map_err(engine)?, rgba: one.over_black_rgba8() });
        }
        Ok(out)
    }
}
