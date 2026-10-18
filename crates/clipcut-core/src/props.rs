//! A clip's properties as the interface's Properties panel shows them (volume and fades, position,
//! scale, rotation, opacity, speed, and a text clip's words and style), setting them, the
//! canvas's shape, and the sound's waveform.
//!
//! Volume is the engine's Volume effect (dB); fades are one-sided audio transitions on the clip's
//! edges, so they stay on its edges when it is trimmed or moved. Position and scale are the clip's
//! Motion effect, which also moves and sizes a whole text clip.

use filmcraft_project::{ClipId, EffectInstance, ItemKind, ParamValue, Sequence, Track, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Error, Result, Studio, engine};

/// What the Properties panel shows for a clip. Missing values don't apply to the clip's kind.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipProps {
    /// Sound clips: the Volume effect's level, dB.
    pub volume_db: Option<f64>,
    /// Sound clips: fade lengths, seconds.
    pub fade_in: f64,
    pub fade_out: f64,
    /// Music on a lane: how far it goes down under speech (dB), when it ducks.
    pub ducking: Option<f64>,
    /// Pictures and text: where the clip's centre sits, in canvas pixels.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// Percent of the clip's own size.
    pub scale: Option<f64>,
    pub rotation: Option<f64>,
    /// 0…100.
    pub opacity: Option<f64>,
    /// Percent: 100 plays at normal speed.
    pub speed: f64,
    pub text: Option<TextProps>,
    /// Picture clips: adjustments and filter, once the clip has any.
    pub colour: Option<crate::colour::ColourView>,
}

/// A text clip's words and look (its first text layer).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextProps {
    pub text: String,
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    pub size: f64,
    /// `left`, `center` or `right`.
    pub align: String,
    /// `#rrggbb`.
    pub color: String,
    pub stroke: bool,
    pub stroke_color: String,
    pub stroke_width: f64,
    pub shadow: bool,
    pub background: bool,
    pub background_color: String,
    /// How it comes in and goes out.
    pub animations: crate::animations::TextAnimations,
}

fn float(e: Option<&EffectInstance>, id: &str) -> Option<f64> {
    match e?.param(id)?.value {
        ParamValue::Float(v) if v.is_finite() => Some(v),
        _ => None,
    }
}
/// A parameter's value at media time `at`: its keyframes' value there when it is animated.
fn value_at(e: Option<&EffectInstance>, id: &str, at: Tick) -> Option<ParamValue> {
    let p = e?.param(id)?;
    Some(if p.is_animated() { p.value_at(at) } else { p.value.clone() })
}
fn float_at(e: Option<&EffectInstance>, id: &str, at: Tick) -> Option<f64> {
    match value_at(e, id, at)? {
        ParamValue::Float(v) if v.is_finite() => Some(v),
        _ => None,
    }
}
fn boolean(e: Option<&EffectInstance>, id: &str) -> bool {
    matches!(e.and_then(|e| e.param(id)).map(|p| &p.value), Some(ParamValue::Bool(true)))
}
fn text(e: Option<&EffectInstance>, id: &str) -> String {
    match e.and_then(|e| e.param(id)).map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}
fn color(e: Option<&EffectInstance>, id: &str) -> String {
    match e.and_then(|e| e.param(id)).map(|p| &p.value) {
        Some(ParamValue::Color(c)) => hex(c),
        _ => "#ffffff".into(),
    }
}
fn hex(c: &[f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}
fn effect<'a>(i: &'a TrackItem, id: &str) -> Option<&'a EffectInstance> {
    i.effects.iter().find(|e| e.effect == id)
}

/// `video`, `image`, `text` or `audio`, and the clip's properties (animated ones as they are at
/// the clip's start).
pub(crate) fn clip_props(s: &filmcraft_engine::Session, q: &Sequence, track: &Track, i: &TrackItem) -> (&'static str, ClipProps) {
    clip_props_at(s, q, track, i, i.source_in)
}

/// [`clip_props`] with animated properties as they are at media time `at`.
pub(crate) fn clip_props_at(s: &filmcraft_engine::Session, q: &Sequence, track: &Track, i: &TrackItem, at: Tick) -> (&'static str, ClipProps) {
    let mut p = ClipProps { speed: (i.speed.abs() * 100.0 * 100.0).round() / 100.0, ..Default::default() };
    if track.kind == TrackKind::Audio {
        p.volume_db = Some(float_at(effect(i, "volume"), "level", at).unwrap_or(0.0));
        p.ducking = crate::ducking::ducking_of(i);
        for t in &track.transitions {
            if t.to == Some(i.id) && t.from.is_none() {
                p.fade_in = t.duration.seconds();
            }
            if t.from == Some(i.id) && t.to.is_none() {
                p.fade_out = t.duration.seconds();
            }
        }
        return ("audio", p);
    }
    let kind = match s.project.item(i.item).map(|it| &it.kind) {
        Some(ItemKind::Graphic { .. }) => "text",
        Some(ItemKind::Media(m)) if m.info.kind == filmcraft_media::MediaKind::Still => "image",
        _ => "video",
    };
    let motion = effect(i, "motion");
    let (w, h) = (f64::from(q.settings.width), f64::from(q.settings.height));
    let (x, y) = match value_at(motion, "position", at).as_ref() {
        Some(ParamValue::Vec2(v)) => (if v.x.is_finite() { v.x } else { w / 2.0 }, if v.y.is_finite() { v.y } else { h / 2.0 }),
        _ => (w / 2.0, h / 2.0),
    };
    (p.x, p.y) = (Some(x), Some(y));
    p.scale = Some(float_at(motion, "scale", at).unwrap_or(100.0));
    p.rotation = Some(float_at(motion, "rotation", at).unwrap_or(0.0));
    p.opacity = Some(float_at(effect(i, "opacity"), "opacity", at).unwrap_or(100.0));
    p.colour = crate::colour::colour_of(&i.effects);
    if kind == "text" {
        let l = effect(i, filmcraft_project::graphic::TEXT_LAYER);
        let style = text(l, "font_style").to_ascii_lowercase();
        let align = match l.and_then(|e| e.param("align")).map(|p| &p.value) {
            Some(ParamValue::Choice(1)) => "center",
            Some(ParamValue::Choice(2)) => "right",
            _ => "left",
        };
        p.text = Some(TextProps {
            text: text(l, "text"),
            font: text(l, "font"),
            bold: style.contains("bold") || boolean(l, "faux_bold"),
            italic: style.contains("italic") || boolean(l, "faux_italic"),
            size: float(l, "size").unwrap_or(100.0),
            align: align.into(),
            color: color(l, "fill_color"),
            stroke: boolean(l, "stroke"),
            stroke_color: color(l, "stroke_color"),
            stroke_width: float(l, "stroke_width").unwrap_or(4.0),
            shadow: boolean(l, "shadow"),
            background: boolean(l, "background"),
            background_color: color(l, "background_color"),
            animations: crate::animations::animations_of(i).unwrap_or_default(),
        });
    }
    (kind, p)
}

/// A finite number from `v[key]`, or an error naming the setting.
fn num(v: &Value, key: &str) -> Result<Option<f64>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| Error::Invalid(format!("{key} must be a number"))),
    }
}

/// The friendly text keys `set_props` takes, and the layer property each one sets.
const TEXT_KEYS: &[(&str, &str)] = &[
    ("font", "font"),
    ("size", "size"),
    ("align", "align"),
    ("color", "fill_color"),
    ("stroke", "stroke"),
    ("strokeColor", "stroke_color"),
    ("strokeWidth", "stroke_width"),
    ("shadow", "shadow"),
    ("background", "background"),
    ("backgroundColor", "background_color"),
];

impl Studio {
    /// Sets properties on clips, as one undo step. `merge` folds a run of calls with the same key
    /// (a slider being dragged) into one step. Keys (all optional): `volumeDb`, `fadeIn`,
    /// `fadeOut` (seconds), `x`, `y`, `scale`, `rotation`, `opacity`, `speed` (percent), and for
    /// text `text`, `font`, `bold`, `italic`, `size`, `align`, `color`, `stroke`, `strokeColor`,
    /// `strokeWidth`, `shadow`, `background`, `backgroundColor`.
    pub fn set_props(&mut self, clips: &[u64], props: &Value, merge: Option<&str>) -> Result<()> {
        if clips.is_empty() {
            return Err(Error::Invalid("select a clip first".into()));
        }
        if !props.is_object() {
            return Err(Error::Invalid("nothing to change".into()));
        }
        let label = props.as_object().and_then(|o| o.keys().find(|k| *k != "at")).map(|k| label_for(k)).unwrap_or("Change");
        // speed moves the main track, and the captions follow it
        self.group_follow(label, merge, |s| s.apply_props(clips, props))
    }

    /// [`Studio::set_props`] without its own undo step (inside another edit).
    pub(crate) fn apply_props(&mut self, clips: &[u64], p: &Value) -> Result<()> {
        let fps = self.fps()?;
        // `at` (seconds): where an animated property gets its keyframe (a still one just changes)
        let at = match num(p, "at")? {
            Some(sec) => Some(self.frame_tick(sec)?),
            None => None,
        };
        let timed = |mut v: Value| {
            if let Some(t) = at {
                v["time"] = json!(t.0);
            }
            v
        };
        for &clip in clips {
            let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
            let (track, item) =
                q.find_item(ClipId(clip)).map(|(t, i)| (t, i.clone())).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
            let is_audio = q.track(track).is_some_and(|t| t.kind == TrackKind::Audio);
            // sound settings go to the clip's own sound: itself, or its linked sound clips
            let sound: Vec<u64> = if is_audio {
                vec![clip]
            } else {
                item.link
                    .map(|l| q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.link == Some(l)).map(|i| i.id.0).collect())
                    .unwrap_or_default()
            };
            if let Some(db) = num(p, "volumeDb")? {
                for c in &sound {
                    self.session
                        .execute("effects.setParam", timed(json!({"clip": c, "effect": "volume", "param": "level", "value": db.clamp(-60.0, 15.0)})))
                        .map_err(engine)?;
                }
            }
            for (key, edge) in [("fadeIn", "in"), ("fadeOut", "out")] {
                if let Some(sec) = num(p, key)? {
                    for c in &sound {
                        self.set_fade(*c, edge, sec, fps)?;
                    }
                }
            }
            if let Some(speed) = num(p, "speed")? {
                let speed = speed.clamp(10.0, 1000.0);
                let main = self.on_main(clip);
                self.session.execute("clip.speedDuration", json!({"clips": [clip], "speed": speed, "ripple": main})).map_err(engine)?;
                if main {
                    self.pack_main()?;
                }
            }
            if is_audio {
                continue;
            }
            let (x, y) = (num(p, "x")?, num(p, "y")?);
            if x.is_some() || y.is_some() {
                let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
                let (_, now) = q.find_item(ClipId(clip)).map(|(t, i)| (t, i.clone())).ok_or(Error::NothingOpen)?;
                let tr = q.track(track).ok_or(Error::NothingOpen)?;
                let mt = at.map_or(now.source_in, |t| now.source_time_at(t.clamp(now.start, now.end())));
                let (_, cur) = clip_props_at(&self.session, q, tr, &now, mt);
                let v = [x.or(cur.x).unwrap_or(0.0), y.or(cur.y).unwrap_or(0.0)];
                self.session.execute("effects.setParam", timed(json!({"clip": clip, "effect": "motion", "param": "position", "value": v}))).map_err(engine)?;
            }
            for (key, eff, param, lo, hi) in
                [("scale", "motion", "scale", 1.0, 2000.0), ("rotation", "motion", "rotation", -3600.0, 3600.0), ("opacity", "opacity", "opacity", 0.0, 100.0)]
            {
                if let Some(v) = num(p, key)? {
                    self.session
                        .execute("effects.setParam", timed(json!({"clip": clip, "effect": eff, "param": param, "value": v.clamp(lo, hi)})))
                        .map_err(engine)?;
                }
            }
            let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
            let is_text =
                q.find_item(ClipId(clip)).and_then(|(_, i)| self.session.project.item(i.item)).is_some_and(|it| matches!(it.kind, ItemKind::Graphic { .. }));
            if !is_text {
                continue;
            }
            // an animated text's keyframes come off while it changes, and are rebuilt for the new text
            let restyles =
                p.get("text").is_some() || p.get("bold").is_some() || p.get("italic").is_some() || TEXT_KEYS.iter().any(|(k, _)| p.get(*k).is_some());
            let held = if restyles { self.hold_animations(clip)? } else { None };
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                let t = if t.trim().is_empty() { " " } else { t };
                self.session.execute("graphics.setText", json!({"clip": clip, "layer": 0, "text": t})).map_err(engine)?;
            }
            let mut layer = serde_json::Map::new();
            for (k, id) in TEXT_KEYS {
                if let Some(v) = p.get(*k).filter(|v| !v.is_null()) {
                    layer.insert((*id).into(), v.clone());
                }
            }
            let (bold, italic) = (p.get("bold").and_then(Value::as_bool), p.get("italic").and_then(Value::as_bool));
            if bold.is_some() || italic.is_some() {
                let l = effect(&item, filmcraft_project::graphic::TEXT_LAYER);
                let style = text(l, "font_style").to_ascii_lowercase();
                let b = bold.unwrap_or(style.contains("bold"));
                let i = italic.unwrap_or(style.contains("italic"));
                let name = match (b, i) {
                    (true, true) => "Bold Italic",
                    (true, false) => "Bold",
                    (false, true) => "Italic",
                    (false, false) => "Regular",
                };
                layer.insert("font_style".into(), json!(name));
            }
            if !layer.is_empty() {
                self.session.execute("graphics.set", json!({"clip": clip, "layer": 0, "props": layer})).map_err(engine)?;
            }
            if let Some(a) = held {
                self.restore_animations(clip, a)?;
            }
        }
        Ok(())
    }

    /// A one-sided fade on a sound clip's edge, `seconds` long (0 removes it).
    fn set_fade(&mut self, clip: u64, edge: &str, seconds: f64, fps: filmcraft_time::FrameRate) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (track, item) = q.find_item(ClipId(clip)).map(|(t, i)| (t, i.clone())).ok_or(Error::NothingOpen)?;
        let tr = q.track(track).ok_or(Error::NothingOpen)?;
        let existing = tr
            .transitions
            .iter()
            .find(|t| if edge == "in" { t.to == Some(item.id) && t.from.is_none() } else { t.from == Some(item.id) && t.to.is_none() })
            .map(|t| t.id.0);
        // never longer than half the clip, so a fade in and a fade out can both fit
        let half = item.duration.seconds() / 2.0;
        let frames = (seconds.clamp(0.0, half) * fps.as_f64()).round() as i64;
        match (existing, frames) {
            (Some(t), 0) => self.session.execute("sequence.removeTransition", json!({"transitions": [t]})).map(|_| ()),
            (None, 0) => Ok(()),
            (Some(t), n) => self
                .session
                .execute("sequence.setTransition", json!({"transition": t, "frames": n, "align": if edge == "in" { "start" } else { "end" }}))
                .map(|_| ()),
            (None, n) => {
                self.session.execute("sequence.applyAudioTransition", json!({"clip": clip, "edge": edge, "effect": "constant_power", "frames": n})).map(|_| ())
            }
        }
        .map_err(engine)
    }

    /// The canvas's size (16:9 1920 × 1080, 9:16 1080 × 1920…). Clips are refitted to the new
    /// shape. One undo step.
    pub fn set_canvas(&mut self, width: u32, height: u32) -> Result<()> {
        if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
            return Err(Error::Invalid("the canvas must be between 16 and 8192 pixels on each side".into()));
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        if (q.settings.width, q.settings.height) == (width, height) {
            return Ok(());
        }
        // even sizes: H.264 needs them
        let (width, height) = (width & !1, height & !1);
        self.group("Canvas", |s| {
            s.session.execute("sequence.settings", json!({"width": width, "height": height, "scaleMotion": true})).map(|_| ()).map_err(engine)
        })
    }

    /// Fits (`fill` false: the whole picture shows) or fills (`fill` true: no bars, edges cropped)
    /// the canvas with each clip.
    pub fn fit_clips(&mut self, clips: &[u64], fill: bool) -> Result<()> {
        if clips.is_empty() {
            return Err(Error::Invalid("select a clip first".into()));
        }
        let cmd = if fill { "clip.fillFrame" } else { "clip.fitToFrame" };
        self.group(if fill { "Fill Canvas" } else { "Fit Canvas" }, |s| s.session.execute(cmd, json!({"clips": clips})).map(|_| ()).map_err(engine))
    }

    /// Font families for text: the three that come with ClipCut, then the computer's own.
    pub fn fonts(&mut self) -> Vec<String> {
        let r = self.session.execute("fonts.list", json!({"system": true})).unwrap_or(Value::Null);
        let mut v: Vec<String> =
            r.as_array().map(|a| a.iter().filter_map(|f| f.get("family").and_then(Value::as_str)).map(str::to_string).collect()).unwrap_or_default();
        let builtin = ["Inter", "Noto Serif", "JetBrains Mono"];
        v.retain(|f| !builtin.contains(&f.as_str()));
        v.sort_by_key(|f| f.to_lowercase());
        v.dedup();
        builtin.iter().map(|s| s.to_string()).chain(v).collect()
    }

    /// A reader of media `item`'s sound, for drawing its waveform on another thread.
    pub fn peaks(&self, item: u64) -> Result<Peaks> {
        let id = filmcraft_project::ItemId(item);
        let it = self.session.project.item(id).ok_or_else(|| Error::Invalid(format!("there is no media {item}")))?;
        if !it.has_audio() {
            return Err(Error::Invalid(format!("{} has no sound", it.name)));
        }
        let src = self.session.source(id).ok_or_else(|| Error::Engine(format!("{} couldn't be opened", it.name)))?;
        Ok(Peaks { src, duration: it.duration() })
    }
}

fn label_for(key: &str) -> &'static str {
    match key {
        "volumeDb" => "Volume",
        "fadeIn" => "Fade In",
        "fadeOut" => "Fade Out",
        "x" | "y" => "Position",
        "scale" => "Scale",
        "rotation" => "Rotation",
        "opacity" => "Opacity",
        "speed" => "Speed",
        "text" => "Edit Text",
        _ => "Text Style",
    }
}

/// Reads a media file's sound into a waveform.
pub struct Peaks {
    src: filmcraft_media::SharedSource,
    duration: Tick,
}

/// Waveform resolution: peaks per second.
pub const PEAKS_PER_SECOND: u32 = 100;

impl Peaks {
    /// The loudest sample (both channels) in each 1/100 s, as 0…255. At most four hours.
    pub fn compute(&self) -> Vec<u8> {
        const RATE: u32 = 48_000;
        let per = (RATE / PEAKS_PER_SECOND) as usize;
        let total = self.duration.to_units_floor(i64::from(RATE)).clamp(0, i64::from(RATE) * 4 * 3600);
        let n = usize::try_from(total).unwrap_or(0) / per;
        let mut out = Vec::with_capacity(n);
        let block = per * 200;
        let mut at: i64 = 0;
        while out.len() < n {
            let Ok(buf) = self.src.audio(at, block, RATE) else { break };
            let len = buf.channels.first().map_or(0, Vec::len);
            if len == 0 {
                break;
            }
            for k in 0..len.div_ceil(per) {
                let range = k * per..((k + 1) * per).min(len);
                let peak = buf.channels.iter().filter_map(|c| c.get(range.clone())).flat_map(|c| c.iter()).fold(0f32, |m, v| m.max(v.abs()));
                out.push((peak.clamp(0.0, 1.0) * 255.0).round() as u8);
                if out.len() >= n {
                    break;
                }
            }
            at += i64::try_from(len).unwrap_or(i64::MAX);
        }
        out
    }
}
