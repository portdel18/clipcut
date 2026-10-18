//! Text animations, CapCut-style: how a text clip comes in and goes out (fade, rise, slide, pop,
//! zoom, typewriter, word by word). They are keyframes on the clip's text layer, at its start and
//! end, kept there by the engine's Responsive Design – Time (the intro and outro play the same
//! when the clip is trimmed); the clip's own position, scale and opacity stay free for the user.
//! Which animations a clip has is kept in a clip marker, so they can be shown, changed and
//! rebuilt after the text changes.

use filmcraft_geom::Vec2;
use filmcraft_project::graphic::TEXT_LAYER;
use filmcraft_project::{ClipId, Interpolation, ItemKind, Keyframe, Label, Marker, MarkerId, MarkerKind, Param, ParamValue, TrackItem};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Error, Result, Studio, engine};

/// The marker that holds a text clip's animations (`in=pop:0.5;out=fade:0.4`).
const MARK: &str = "ClipCut animation";

/// The ways in: id and name.
pub const ANIMATIONS_IN: &[(&str, &str)] = &[
    ("fade", "Fade in"),
    ("rise", "Rise"),
    ("drop", "Drop"),
    ("left", "Slide left"),
    ("right", "Slide right"),
    ("pop", "Pop"),
    ("zoom", "Zoom in"),
    ("type", "Typewriter"),
    ("words", "Word by word"),
];
/// The ways out: id and name.
pub const ANIMATIONS_OUT: &[(&str, &str)] = &[
    ("fade", "Fade out"),
    ("rise", "Rise"),
    ("sink", "Sink"),
    ("left", "Slide left"),
    ("right", "Slide right"),
    ("shrink", "Shrink"),
    ("zoom", "Zoom out"),
    ("type", "Erase"),
    ("words", "Word by word"),
];

/// How long an animation lasts when none is given, and the limits.
pub const DEFAULT_SECONDS: f64 = 0.5;
const MIN_SECONDS: f64 = 0.1;
const MAX_SECONDS: f64 = 5.0;

/// The layer parameters animations write.
const PARAMS: &[&str] = &["position", "scale", "opacity", "text"];

/// One animation on a text clip.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimView {
    pub id: String,
    pub name: String,
    pub seconds: f64,
}

/// A text clip's animations in and out.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TextAnimations {
    #[serde(rename = "in")]
    pub enter: Option<AnimView>,
    #[serde(rename = "out")]
    pub leave: Option<AnimView>,
}

impl TextAnimations {
    fn is_empty(&self) -> bool {
        self.enter.is_none() && self.leave.is_none()
    }
}

/// Every animation, for the Animation section: `{in: [{id, name}], out: [...]}`.
pub fn text_animations() -> Value {
    let list = |l: &[(&str, &str)]| l.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>();
    json!({"in": list(ANIMATIONS_IN), "out": list(ANIMATIONS_OUT), "seconds": DEFAULT_SECONDS})
}

fn view(side: &[(&str, &str)], id: &str, seconds: f64) -> Option<AnimView> {
    let (id, name) = side.iter().find(|(i, _)| *i == id)?;
    Some(AnimView { id: id.to_string(), name: name.to_string(), seconds })
}

fn parse(comment: &str) -> TextAnimations {
    let mut a = TextAnimations::default();
    for part in comment.split(';') {
        let Some((side, rest)) = part.split_once('=') else { continue };
        let Some((id, secs)) = rest.split_once(':') else { continue };
        let secs = secs.trim().parse::<f64>().ok().filter(|s| s.is_finite()).unwrap_or(DEFAULT_SECONDS).clamp(MIN_SECONDS, MAX_SECONDS);
        match side.trim() {
            "in" => a.enter = view(ANIMATIONS_IN, id.trim(), secs),
            "out" => a.leave = view(ANIMATIONS_OUT, id.trim(), secs),
            _ => {}
        }
    }
    a
}

fn format(a: &TextAnimations) -> String {
    let mut parts = Vec::new();
    if let Some(v) = &a.enter {
        parts.push(format!("in={}:{}", v.id, v.seconds));
    }
    if let Some(v) = &a.leave {
        parts.push(format!("out={}:{}", v.id, v.seconds));
    }
    parts.join(";")
}

/// A text clip's animations, when it has any.
pub(crate) fn animations_of(i: &TrackItem) -> Option<TextAnimations> {
    let a = parse(&i.markers.iter().find(|m| m.name == MARK)?.comment);
    if a.is_empty() { None } else { Some(a) }
}

/// The clip's length in media time.
fn media_length(i: &TrackItem) -> Tick {
    Tick((i.duration.0 as f64 * i.speed.abs().max(1e-9)).round() as i64)
}

/// Takes the animation keyframes off `i`'s text layer: each parameter keeps the value it has
/// between the intro and the outro.
fn clear(i: &mut TrackItem) {
    let Some(meta) = i.graphic.as_deref_mut() else { return };
    if meta.has_responsive_time() {
        // the hold between them: the intro's end, or the outro's start
        let hold = if meta.intro > Tick::ZERO { meta.design_in + meta.intro } else { meta.design_in + meta.design_duration - meta.outro };
        if let Some(e) = i.effects.iter_mut().find(|e| e.effect == TEXT_LAYER) {
            for id in PARAMS {
                if let Some(p) = e.params.get_mut(*id)
                    && p.is_animated()
                {
                    p.value = p.value_at(hold);
                    p.keyframes.clear();
                }
            }
        }
    }
    meta.intro = Tick::ZERO;
    meta.outro = Tick::ZERO;
    meta.design_in = Tick::ZERO;
    meta.design_duration = Tick::ZERO;
}

fn key(time: Tick, value: ParamValue, interp: Interpolation) -> Keyframe {
    Keyframe { time, value, interp, out_influence: 1.0 / 3.0, in_influence: 1.0 / 3.0 }
}

/// Prefixes of `text` for a typewriter (`words`: a word at a time), from nothing to all of it.
fn prefixes(text: &str, words: bool, most: usize) -> Vec<String> {
    let mut cuts: Vec<usize> = if words {
        let mut v: Vec<usize> = text.char_indices().filter(|(k, c)| c.is_whitespace() && *k > 0).map(|(k, _)| k).collect();
        v.dedup_by(|a, b| text.get(*b..*a).is_some_and(|s| s.chars().all(char::is_whitespace)));
        v
    } else {
        text.char_indices().map(|(k, _)| k).skip(1).collect()
    };
    cuts.push(text.len());
    // no more steps than frames to show them
    if cuts.len() > most.max(1) {
        let n = cuts.len();
        cuts = (1..=most.max(1)).filter_map(|k| cuts.get(k * n / most.max(1) - 1).copied()).collect();
    }
    std::iter::once(String::new()).chain(cuts.iter().filter_map(|c| text.get(..*c).map(str::to_string))).collect()
}

/// The keyframes for one side, as `(param, keys)`. `t0`…`t1` is the animation's span; `toward`
/// is true for the way in (from the effect to the plain text).
struct Base {
    position: Vec2,
    scale: f64,
    opacity: f64,
    text: String,
    canvas: (f64, f64),
}

fn side_keys(id: &str, enter: bool, t0: Tick, t1: Tick, b: &Base, frames: usize) -> Vec<(&'static str, Vec<Keyframe>)> {
    let smooth = Interpolation::AutoBezier;
    let at = |f: f64| Tick(t0.0 + ((t1.0 - t0.0) as f64 * f).round() as i64);
    // `away` is the effect's far end, `here` the plain text; the way out runs here → away
    let pair = |away: ParamValue, here: ParamValue| {
        if enter { vec![key(t0, away, smooth), key(t1, here, smooth)] } else { vec![key(t0, here, smooth), key(t1, away, smooth)] }
    };
    let f = ParamValue::Float;
    let fade = || ("opacity", pair(f(0.0), f(b.opacity)));
    let (w, h) = b.canvas;
    let slide = |dx: f64, dy: f64| ("position", pair(ParamValue::Vec2(Vec2::new(b.position.x + dx, b.position.y + dy)), ParamValue::Vec2(b.position)));
    let typing = |words: bool| {
        let steps = prefixes(&b.text, words, frames);
        let n = steps.len().saturating_sub(1).max(1);
        let keys = steps
            .into_iter()
            .enumerate()
            .map(|(k, s)| {
                let frac = k as f64 / n as f64;
                key(at(if enter { frac } else { 1.0 - frac }), ParamValue::Text(s), Interpolation::Hold)
            })
            .collect::<Vec<_>>();
        let mut keys = keys;
        keys.sort_by_key(|k| k.time);
        ("text", keys)
    };
    match (id, enter) {
        ("fade", _) => vec![fade()],
        ("rise", true) => vec![slide(0.0, h * 0.1), fade()],
        ("rise", false) => vec![slide(0.0, -h * 0.1), fade()],
        ("drop", true) => vec![slide(0.0, -h * 0.1), fade()],
        ("sink", false) => vec![slide(0.0, h * 0.1), fade()],
        // slide left: moves leftwards (in from the right, out to the left)
        ("left", true) => vec![slide(w * 0.2, 0.0), fade()],
        ("left", false) => vec![slide(-w * 0.2, 0.0), fade()],
        ("right", true) => vec![slide(-w * 0.2, 0.0), fade()],
        ("right", false) => vec![slide(w * 0.2, 0.0), fade()],
        ("pop", true) => vec![
            ("scale", vec![key(t0, f(0.0), smooth), key(at(0.7), f(b.scale * 1.12), smooth), key(t1, f(b.scale), smooth)]),
            ("opacity", vec![key(t0, f(0.0), smooth), key(at(0.3), f(b.opacity), smooth)]),
        ],
        ("shrink", false) => vec![
            ("scale", vec![key(t0, f(b.scale), smooth), key(at(0.3), f(b.scale * 1.08), smooth), key(t1, f(0.0), smooth)]),
            ("opacity", vec![key(at(0.7), f(b.opacity), smooth), key(t1, f(0.0), smooth)]),
        ],
        ("zoom", true) => vec![("scale", pair(f(b.scale * 1.6), f(b.scale))), fade()],
        ("zoom", false) => vec![("scale", pair(f(b.scale * 1.6), f(b.scale))), fade()],
        ("type", _) => vec![typing(false)],
        ("words", _) => vec![typing(true)],
        _ => Vec::new(),
    }
}

/// Puts animations `a` on text clip `i` (a graphic `canvas` big, at `fps`), replacing any it has.
fn build(i: &mut TrackItem, a: &TextAnimations, canvas: (u32, u32), fps: f64, alloc: &mut dyn FnMut() -> u64) {
    clear(i);
    i.markers.retain(|m| m.name != MARK);
    if a.is_empty() {
        return;
    }
    let start = i.source_in;
    let length = media_length(i);
    let frame = Tick::from_seconds_f64(1.0 / fps.max(1.0));
    // both fit in the clip, the way out ending on its last frame
    let span = (length - frame).max(Tick(1));
    let (mut li, mut lo) = (a.enter.as_ref().map_or(0.0, |v| v.seconds), a.leave.as_ref().map_or(0.0, |v| v.seconds));
    if li + lo > span.seconds() {
        let k = span.seconds() / (li + lo);
        (li, lo) = (li * k, lo * k);
    }
    let (li, lo) = (Tick::from_seconds_f64(li), Tick::from_seconds_f64(lo));
    let Some(e) = i.effects.iter_mut().find(|e| e.effect == TEXT_LAYER) else { return };
    let Some(spec) = filmcraft_project::graphic::eval_layer(e, start, canvas) else { return };
    let mut position = spec.transform.position;
    // pops and zooms grow from the middle of the text: the layer turns about its centre
    let ids = [a.enter.as_ref().map(|v| v.id.as_str()), a.leave.as_ref().map(|v| v.id.as_str())];
    if ids.iter().flatten().any(|id| matches!(*id, "pop" | "shrink" | "zoom")) {
        let lb = filmcraft_render::graphic_clip::layer_local_bounds(&spec);
        let centre = Vec2::new(f64::from(lb[0] + lb[2]) / 2.0, f64::from(lb[1] + lb[3]) / 2.0);
        position = filmcraft_render::graphic_clip::layer_matrix(&spec).apply(centre);
        for (id, v) in [("anchor", centre), ("position", position)] {
            e.params.insert(id.into(), Param::new(ParamValue::Vec2(v)));
        }
    }
    let float = |id: &str, def: f64| match e.params.get(id).map(|p| &p.value) {
        Some(ParamValue::Float(v)) if v.is_finite() => *v,
        _ => def,
    };
    let text = match e.params.get("text").map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    };
    let base = Base { position, scale: float("scale", 100.0), opacity: float("opacity", 100.0), text, canvas: (f64::from(canvas.0), f64::from(canvas.1)) };
    let frames = |len: Tick| usize::try_from(len.0 / frame.0.max(1)).unwrap_or(1).max(1);
    let mut keys: Vec<(&'static str, Vec<Keyframe>)> = Vec::new();
    if let Some(v) = &a.enter {
        keys.extend(side_keys(&v.id, true, start, start + li, &base, frames(li)));
    }
    if let Some(v) = &a.leave {
        let end = start + span;
        keys.extend(side_keys(&v.id, false, end - lo, end, &base, frames(lo)));
    }
    for (id, ks) in keys {
        let Some(def) = filmcraft_project::find_effect(TEXT_LAYER).and_then(|d| d.param(id)) else { continue };
        let p = e.params.entry(id.to_string()).or_insert_with(|| Param::new(def.default.clone()));
        // the plain value stays what the text shows between the animations
        if id == "position" {
            p.value = ParamValue::Vec2(base.position);
        }
        p.keyframes.extend(ks);
        p.keyframes.sort_by_key(|k| k.time);
        p.keyframes.dedup_by_key(|k| k.time);
    }
    let meta = i.graphic.get_or_insert_with(Default::default);
    meta.design_in = start;
    meta.design_duration = length;
    meta.intro = li;
    meta.outro = if lo > Tick::ZERO { lo + frame } else { Tick::ZERO };
    i.markers.push(Marker {
        id: MarkerId(alloc()),
        start,
        duration: Tick::ZERO,
        name: MARK.into(),
        comment: format(a),
        kind: MarkerKind::Comment,
        color: Label::Violet,
    });
}

impl Studio {
    /// The text clips among `clips` (animations are for text).
    fn text_clips(&self, clips: &[u64]) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let v: Vec<u64> = clips
            .iter()
            .copied()
            .filter(|c| {
                q.find_item(ClipId(*c)).and_then(|(_, i)| self.session.project.item(i.item)).is_some_and(|it| matches!(it.kind, ItemKind::Graphic { .. }))
            })
            .collect();
        if v.is_empty() {
            return Err(Error::Invalid("select a text clip first".into()));
        }
        Ok(v)
    }

    /// Rebuilds each clip's animations from `wanted` (clip → animations; `None` keeps the ones it
    /// has), as one sequence edit.
    fn rebuild_animations(&mut self, label: &str, wanted: Vec<(u64, Option<TextAnimations>)>) -> Result<()> {
        let fps = self.fps()?.as_f64();
        let canvases: Vec<(u64, (u32, u32))> = {
            let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
            wanted
                .iter()
                .filter_map(|(c, _)| {
                    let (_, i) = q.find_item(ClipId(*c))?;
                    match self.session.project.item(i.item).map(|x| &x.kind) {
                        Some(ItemKind::Graphic { width, height, .. }) => Some((*c, (*width, *height))),
                        _ => None,
                    }
                })
                .collect()
        };
        self.session
            .edit_sequence(label, |q, ctx, _| {
                for (c, a) in &wanted {
                    let Some(canvas) = canvases.iter().find(|(id, _)| id == c).map(|(_, s)| *s) else { continue };
                    let Some((_, i)) = q.find_item_mut(ClipId(*c)) else { continue };
                    let Some(a) = a.clone().or_else(|| animations_of(i)) else { continue };
                    build(i, &a, canvas, fps, &mut || ctx.alloc());
                }
                Ok(())
            })
            .map_err(engine)
    }

    /// Takes clip `clip`'s animation keyframes off (its text plain), returning its animations so
    /// [`Studio::rebuild_animations`] can put them back after the text changes.
    pub(crate) fn hold_animations(&mut self, clip: u64) -> Result<Option<TextAnimations>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let Some(a) = q.find_item(ClipId(clip)).and_then(|(_, i)| animations_of(i)) else { return Ok(None) };
        self.rebuild_animations("Edit Text", vec![(clip, Some(TextAnimations::default()))])?;
        Ok(Some(a))
    }

    pub(crate) fn restore_animations(&mut self, clip: u64, a: TextAnimations) -> Result<()> {
        self.rebuild_animations("Edit Text", vec![(clip, Some(a))])
    }

    /// Sets how the text clips among `clips` come in (`enter`) or go out: animation `id` (one of
    /// [`ANIMATIONS_IN`] or [`ANIMATIONS_OUT`], or none), `seconds` long. One undo step; calls
    /// with the same `merge` key (a dragged length) fold into one.
    pub fn set_text_animation(&mut self, clips: &[u64], enter: bool, id: Option<&str>, seconds: f64, merge: Option<&str>) -> Result<()> {
        let clips = self.text_clips(clips)?;
        if !seconds.is_finite() {
            return Err(Error::Invalid("the animation's length must be a number".into()));
        }
        let seconds = (seconds.clamp(MIN_SECONDS, MAX_SECONDS) * 100.0).round() / 100.0;
        let side = if enter { ANIMATIONS_IN } else { ANIMATIONS_OUT };
        let v = match id {
            None => None,
            Some(id) => Some(view(side, id, seconds).ok_or_else(|| Error::Invalid(format!("there is no animation called “{id}”")))?),
        };
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let wanted: Vec<(u64, Option<TextAnimations>)> = clips
            .iter()
            .map(|c| {
                let mut a = q.find_item(ClipId(*c)).and_then(|(_, i)| animations_of(i)).unwrap_or_default();
                if enter {
                    a.enter = v.clone();
                } else {
                    a.leave = v.clone();
                }
                (*c, Some(a))
            })
            .collect();
        let key = merge.map(|m| format!("anim:{enter}:{m}"));
        self.group_merge("Text Animation", key.as_deref(), |s| s.rebuild_animations("Text Animation", wanted))
    }
}
