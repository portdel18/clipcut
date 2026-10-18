//! Transitions between main-track clips, CapCut-style: one per cut, centred on it, with the sound
//! crossfading under it.
//!
//! A transition joins a main-track clip to the one after it. The picture uses the engine's own
//! transitions (a curated set, [`TRANSITIONS`]), over the cut, half on each side, playing the
//! clips' media past their edges (the engine holds the nearest frame where a clip has none), so
//! adding a transition never changes the timeline's length or moves the captions. The clips'
//! linked sound on `A1` crossfades (constant power) over the same span when both have media to
//! spare past the cut; without it a crossfade would only dip the sound, so the sound cuts.
//!
//! Transitions follow the main track: every edit that moves it ([`Studio::group_follow`]) puts
//! each transition back on its cut afterwards, shortens it to fit its clips, and drops it when its
//! two clips no longer meet.

use std::collections::HashMap;

use filmcraft_project::{
    ClipId, EffectInstance, EffectKind, Param, ParamValue, Sequence, Track, TrackItem, Transition, TransitionAlign, TransitionId, vtransition::find_transition,
};
use filmcraft_render::Image;
use filmcraft_time::Tick;
use serde::Serialize;

use crate::{Error, Frame, Result, Studio};

/// One of the transitions ClipCut offers: an engine transition and the choices it is set to.
#[derive(Clone, Copy, Debug)]
pub struct TransitionPreset {
    pub id: &'static str,
    pub name: &'static str,
    effect: &'static str,
    /// Choice parameters (`direction` and the like) and the option picked.
    choices: &'static [(&'static str, u32)],
}

// the engine's Direction options: From North, From East, From South, From West
const FROM_EAST: u32 = 1;
const FROM_SOUTH: u32 = 2;
const FROM_WEST: u32 = 3;

/// The transitions in the Transitions tab, in its order.
pub const TRANSITIONS: &[TransitionPreset] = &[
    TransitionPreset { id: "mix", name: "Mix", effect: "cross_dissolve", choices: &[] },
    TransitionPreset { id: "black", name: "Through black", effect: "dip_to_black", choices: &[] },
    TransitionPreset { id: "white", name: "Through white", effect: "dip_to_white", choices: &[] },
    TransitionPreset { id: "flash", name: "Flash", effect: "flash", choices: &[] },
    TransitionPreset { id: "blur", name: "Blur", effect: "blur_dissolve", choices: &[] },
    TransitionPreset { id: "zoom", name: "Zoom", effect: "cross_zoom", choices: &[] },
    TransitionPreset { id: "push-left", name: "Push left", effect: "push", choices: &[("direction", FROM_EAST)] },
    TransitionPreset { id: "push-right", name: "Push right", effect: "push", choices: &[("direction", FROM_WEST)] },
    TransitionPreset { id: "push-up", name: "Push up", effect: "push", choices: &[("direction", FROM_SOUTH)] },
    TransitionPreset { id: "slide", name: "Slide", effect: "slide", choices: &[("direction", FROM_EAST)] },
    TransitionPreset { id: "whip", name: "Whip", effect: "whip", choices: &[("direction", FROM_EAST)] },
    TransitionPreset { id: "wipe", name: "Wipe", effect: "linear_wipe", choices: &[] },
    TransitionPreset { id: "circle", name: "Circle", effect: "iris_round", choices: &[] },
    TransitionPreset { id: "glitch", name: "Glitch", effect: "glitch", choices: &[] },
    TransitionPreset { id: "light-leak", name: "Light leak", effect: "light_leak", choices: &[] },
    TransitionPreset { id: "spin", name: "3D spin", effect: "spin_3d", choices: &[("direction", FROM_EAST)] },
];

/// A new transition's length when none is given, as in CapCut.
pub const DEFAULT_SECONDS: f64 = 0.5;
/// The longest a transition may be.
pub const MAX_SECONDS: f64 = 5.0;

fn preset(id: &str) -> Result<&'static TransitionPreset> {
    TRANSITIONS.iter().find(|p| p.id == id).ok_or_else(|| Error::Invalid(format!("there is no transition called “{id}”")))
}

impl TransitionPreset {
    fn instance(&self) -> Result<EffectInstance> {
        let def =
            find_transition(self.effect, EffectKind::VideoTransition).ok_or_else(|| Error::Engine(format!("the engine has no {} transition", self.name)))?;
        let mut e = def.instance();
        for (k, v) in self.choices {
            e.params.insert((*k).to_string(), Param::new(ParamValue::Choice(*v)));
        }
        Ok(e)
    }

    /// Whether `e` is this preset (its effect, with its choices).
    fn is(&self, e: &EffectInstance) -> bool {
        e.effect == self.effect && self.choices.iter().all(|(k, v)| matches!(e.params.get(*k).map(|p| &p.value), Some(ParamValue::Choice(c)) if c == v))
    }
}

/// The preset a transition is, if it is one of ClipCut's.
fn preset_of(e: &EffectInstance) -> Option<&'static TransitionPreset> {
    TRANSITIONS.iter().find(|p| p.is(e))
}

/// A transition, as the timeline and the Transitions tab show it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionView {
    pub id: u64,
    /// The clip before the cut and the one after it.
    pub from: u64,
    pub to: u64,
    /// The cut, and the span it covers (seconds).
    pub at: f64,
    pub start: f64,
    pub end: f64,
    /// [`TRANSITIONS`]' id, or `"other"` for one made elsewhere (FilmCraft).
    pub preset: String,
    pub name: String,
    /// The longest it can be at this cut.
    pub max: f64,
}

/// A transition offered in the Transitions tab.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPresetView {
    pub id: &'static str,
    pub name: &'static str,
}

pub fn transition_presets() -> Vec<TransitionPresetView> {
    TRANSITIONS.iter().map(|p| TransitionPresetView { id: p.id, name: p.name }).collect()
}

/// A transition as it was before an edit (see [`Studio::place_transitions`]).
#[derive(Clone, Debug)]
pub(crate) struct HeldTransition(Transition);

/// The main track's clips in timeline order.
fn main_items(q: &Sequence) -> Vec<&TrackItem> {
    let mut v: Vec<&TrackItem> = q.video_tracks.first().map(|t| t.items.iter().collect()).unwrap_or_default();
    v.sort_by_key(|i| i.start);
    v
}

/// The clip right after `from` on the main track, when the two meet.
fn next_after<'a>(items: &[&'a TrackItem], from: u64) -> Option<&'a TrackItem> {
    let k = items.iter().position(|i| i.id.0 == from)?;
    let (a, b) = (items.get(k)?, items.get(k + 1)?);
    (a.end() == b.start).then_some(*b)
}

/// The longest a transition between `a` and `b` may be: each keeps at least half of itself
/// clear, so the transitions at both ends of a short clip never overlap.
fn max_len(a: &TrackItem, b: &TrackItem) -> Tick {
    a.duration.min(b.duration).min(Tick::from_seconds_f64(MAX_SECONDS))
}

/// What the main track's transitions (and their sound) are, to tell whether an edit changed them.
fn signature(t: Option<&Track>) -> Vec<(Option<u64>, Option<u64>, i64, i64, String)> {
    t.map(|t| t.transitions.iter().map(|x| (x.from.map(|c| c.0), x.to.map(|c| c.0), x.start.0, x.duration.0, format!("{:?}", x.effect.params))).collect())
        .unwrap_or_default()
}

/// How much media each main-track sound clip has before its start and after its end, in
/// timeline time (see [`lay_out`]).
pub(crate) type Spare = HashMap<ClipId, (Tick, Tick)>;

fn spare_of(project: &filmcraft_project::Project, q: &Sequence) -> Spare {
    let Some(a1) = q.audio_tracks.first() else { return Spare::new() };
    a1.items
        .iter()
        .filter_map(|i| {
            let media = project.item(i.item)?.duration();
            let speed = i.speed.abs().max(1e-6);
            let at = |t: Tick| Tick::from_seconds_f64(t.seconds().max(0.0) / speed);
            Some((i.id, (at(i.source_in), at(media - i.source_out()))))
        })
        .collect()
}

/// Lays the main track's transitions out as `wanted` (each on its cut, fitting its clips, whole
/// frames) and the sound crossfades under them on `A1`,
/// where `spare` says both sounds go on past the cut. Returns false when nothing needed to change.
fn lay_out(q: &mut Sequence, wanted: Vec<Transition>, spare: &Spare, alloc: &mut dyn FnMut() -> u64) -> bool {
    let rate = q.settings.frame_rate;
    let items: Vec<TrackItem> = main_items(q).into_iter().cloned().collect();
    let refs: Vec<&TrackItem> = items.iter().collect();
    let mut video: Vec<Transition> = Vec::new();
    for mut t in wanted {
        let (Some(from), Some(to)) = (t.from, t.to) else { continue };
        let Some(a) = refs.iter().find(|i| i.id == from) else { continue };
        let Some(b) = next_after(&refs, from.0).filter(|b| b.id == to) else { continue };
        // one transition per cut
        if video.iter().any(|v| v.from == t.from) {
            continue;
        }
        let n = rate.frame_at(t.duration.min(max_len(a, b)));
        if n < 2 {
            continue;
        }
        // an odd number of frames has its extra one after the cut, as Premiere centres it
        t.start = b.start - rate.tick_of(n / 2);
        t.duration = rate.tick_of(n);
        t.align = TransitionAlign::default();
        video.push(t);
    }
    video.sort_by_key(|t| t.start);
    // the sound: a crossfade between the cut's linked sound clips, where they meet on A1
    let crossfade = find_transition("constant_power", EffectKind::AudioTransition).map(|d| d.instance());
    let old_audio: Vec<Transition> = q.audio_tracks.first().map(|t| t.transitions.clone()).unwrap_or_default();
    let mut audio: Vec<Transition> = Vec::new();
    if let (Some(fade), Some(a1)) = (crossfade, q.audio_tracks.first()) {
        let linked = |c: filmcraft_project::ClipId| -> Option<&TrackItem> {
            let link = items.iter().find(|i| i.id == c)?.link?;
            a1.items.iter().find(|i| i.link == Some(link))
        };
        for v in &video {
            let (Some(x), Some(y)) = (v.from.and_then(linked), v.to.and_then(linked)) else { continue };
            let half = v.duration.mul_ratio(1, 2);
            let room = |c: &TrackItem, after: bool| spare.get(&c.id).is_some_and(|(before, past)| if after { *past >= half } else { *before >= half });
            if x.end() != y.start || !room(x, true) || !room(y, false) {
                continue;
            }
            let id = old_audio.iter().find(|o| o.from == Some(x.id) && o.to == Some(y.id)).map_or_else(|| TransitionId(alloc()), |o| o.id);
            audio.push(Transition {
                id,
                effect: fade.clone(),
                start: v.start,
                duration: v.duration,
                from: Some(x.id),
                to: Some(y.id),
                align: TransitionAlign::default(),
                reverse: false,
            });
        }
    }
    let before = (signature(q.video_tracks.first()), signature(q.audio_tracks.first()));
    let mut v1 = q.video_tracks.first().cloned();
    let mut a1 = q.audio_tracks.first().cloned();
    if let Some(t) = v1.as_mut() {
        t.transitions = video;
    }
    if let Some(t) = a1.as_mut() {
        t.transitions = audio;
    }
    if (signature(v1.as_ref()), signature(a1.as_ref())) == before {
        return false;
    }
    if let (Some(t), Some(slot)) = (v1, q.video_tracks.first_mut()) {
        *slot = t;
    }
    if let (Some(t), Some(slot)) = (a1, q.audio_tracks.first_mut()) {
        *slot = t;
    }
    true
}

impl Studio {
    /// The main track's transitions.
    pub(crate) fn transitions_view(&self) -> Vec<TransitionView> {
        let Some(q) = self.session.active_sequence() else { return Vec::new() };
        let items = main_items(q);
        q.video_tracks
            .first()
            .map(|t| {
                t.transitions
                    .iter()
                    .filter_map(|x| {
                        let (from, to) = (x.from?, x.to?);
                        let a = items.iter().find(|i| i.id == from)?;
                        let b = next_after(&items, from.0).filter(|b| b.id == to)?;
                        let p = preset_of(&x.effect);
                        Some(TransitionView {
                            id: x.id.0,
                            from: from.0,
                            to: to.0,
                            at: b.start.seconds(),
                            start: x.start.seconds(),
                            end: x.end().seconds(),
                            preset: p.map_or("other", |p| p.id).to_string(),
                            name: p.map_or_else(|| x.effect.def().map_or("Transition", |d| d.name), |p| p.name).to_string(),
                            max: max_len(a, b).seconds(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The main track's transitions as they are, before an edit that moves it.
    pub(crate) fn hold_transitions(&self) -> Vec<HeldTransition> {
        self.session
            .active_sequence()
            .and_then(|q| q.video_tracks.first())
            .map(|t| t.transitions.iter().cloned().map(HeldTransition).collect())
            .unwrap_or_default()
    }

    /// After an edit that moved the main track: every transition (the engine's, and those held
    /// from before that the edit dropped) goes back on its cut when its two clips still meet.
    pub(crate) fn place_transitions(&mut self, held: Vec<HeldTransition>) -> Result<()> {
        let Some(q) = self.session.active_sequence() else { return Ok(()) };
        let mut wanted: Vec<Transition> = q.video_tracks.first().map(|t| t.transitions.clone()).unwrap_or_default();
        for HeldTransition(h) in held {
            if !wanted.iter().any(|w| w.id == h.id) {
                wanted.push(h);
            }
        }
        self.write_transitions("Transitions Follow", wanted)
    }

    fn write_transitions(&mut self, label: &str, wanted: Vec<Transition>) -> Result<()> {
        // check first, so an edit that leaves them be adds no step
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let spare = spare_of(&self.session.project, q);
        {
            let mut probe = q.clone();
            let mut n = 0u64;
            if !lay_out(&mut probe, wanted.clone(), &spare, &mut || {
                n += 1;
                n
            }) {
                return Ok(());
            }
        }
        self.session
            .edit_sequence(label, |q, ctx, _| {
                lay_out(q, wanted, &spare, &mut || ctx.alloc());
                Ok(())
            })
            .map_err(crate::engine)
    }

    /// The cut after main-track clip `from`: the clip after it, and the transition there.
    fn cut_after(&self, from: u64) -> Result<(TrackItem, TrackItem, Option<Transition>)> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let items = main_items(q);
        let a = items.iter().find(|i| i.id.0 == from).ok_or_else(|| Error::Invalid("transitions go between clips on the main track".into()))?;
        let b = next_after(&items, from).ok_or_else(|| Error::Invalid("this clip has no clip after it to go into".into()))?;
        let tr = q.video_tracks.first().and_then(|t| t.transitions.iter().find(|x| x.from == Some(a.id) && x.to == Some(b.id)).cloned());
        Ok(((*a).clone(), b.clone(), tr))
    }

    fn transition_len(&self, seconds: f64, a: &TrackItem, b: &TrackItem) -> Result<Tick> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(Error::Invalid("a transition's length must be more than zero".into()));
        }
        let rate = self.fps()?;
        let len = rate.snap(Tick::from_seconds_f64(seconds.min(MAX_SECONDS)));
        let min = rate.frame_duration() + rate.frame_duration();
        let max = max_len(a, b);
        if max < min {
            return Err(Error::Invalid("these clips are too short for a transition".into()));
        }
        Ok(len.max(min).min(max))
    }

    /// Puts transition `preset` on the cut after main-track clip `from` (replacing the one there),
    /// `seconds` long (or as long as the one it replaces, or half a second). One undo step.
    pub fn set_transition(&mut self, from: u64, preset_id: &str, seconds: Option<f64>) -> Result<()> {
        let p = preset(preset_id)?;
        let (a, b, old) = self.cut_after(from)?;
        let len = self.transition_len(seconds.or_else(|| old.as_ref().map(|o| o.duration.seconds())).unwrap_or(DEFAULT_SECONDS), &a, &b)?;
        let effect = p.instance()?;
        let label = if old.is_some() { "Change Transition" } else { "Add Transition" };
        self.group(label, |s| {
            let mut wanted = s.hold_transitions().into_iter().map(|h| h.0).filter(|t| t.from != Some(a.id)).collect::<Vec<_>>();
            let id = old.as_ref().map(|o| o.id);
            let spare = s.session.active_sequence().map(|q| spare_of(&s.session.project, q)).unwrap_or_default();
            s.session
                .edit_sequence(label, |q, ctx, _| {
                    let id = id.unwrap_or_else(|| TransitionId(ctx.alloc()));
                    wanted.push(Transition {
                        id,
                        effect,
                        start: b.start,
                        duration: len,
                        from: Some(a.id),
                        to: Some(b.id),
                        align: TransitionAlign::default(),
                        reverse: false,
                    });
                    lay_out(q, wanted, &spare, &mut || ctx.alloc());
                    Ok(())
                })
                .map_err(crate::engine)
        })
    }

    /// Changes the length of the transition after clip `from`. Calls with the same `merge` key
    /// (a slider dragged through many lengths) are one undo step.
    pub fn set_transition_length(&mut self, from: u64, seconds: f64, merge: Option<&str>) -> Result<()> {
        let (a, b, old) = self.cut_after(from)?;
        let mut old = old.ok_or_else(|| Error::Invalid("there is no transition here".into()))?;
        old.duration = self.transition_len(seconds, &a, &b)?;
        let key = merge.map(|m| format!("transition-length:{from}:{m}"));
        self.group_merge("Transition Length", key.as_deref(), |s| {
            let mut wanted: Vec<Transition> = s.hold_transitions().into_iter().map(|h| h.0).filter(|t| t.id != old.id).collect();
            wanted.push(old);
            s.write_transitions("Transition Length", wanted)
        })
    }

    /// Removes the transitions after clips `from` (a plain cut again).
    pub fn remove_transitions(&mut self, from: &[u64]) -> Result<()> {
        if from.is_empty() {
            return Err(Error::Invalid("select a transition first".into()));
        }
        self.group("Remove Transition", |s| {
            let wanted: Vec<Transition> = s.hold_transitions().into_iter().map(|h| h.0).filter(|t| !t.from.is_some_and(|f| from.contains(&f.0))).collect();
            s.write_transitions("Remove Transition", wanted)
        })
    }

    /// Puts `preset` on every cut of the main track, `seconds` long (fitted to each). Returns how
    /// many cuts have it. One undo step.
    pub fn transition_everywhere(&mut self, preset_id: &str, seconds: f64) -> Result<usize> {
        let p = preset(preset_id)?;
        let effect = p.instance()?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let items = main_items(q);
        let rate = q.settings.frame_rate;
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(Error::Invalid("a transition's length must be more than zero".into()));
        }
        let len = rate.snap(Tick::from_seconds_f64(seconds.min(MAX_SECONDS)));
        let old: Vec<Transition> = q.video_tracks.first().map(|t| t.transitions.clone()).unwrap_or_default();
        let spare = spare_of(&self.session.project, q);
        let cuts: Vec<(ClipId, ClipId, Tick)> = items.iter().filter_map(|a| next_after(&items, a.id.0).map(|b| (a.id, b.id, b.start))).collect();
        if cuts.is_empty() {
            return Err(Error::Invalid("add a second clip to the main track first: transitions go between clips".into()));
        }
        self.group("Transition Everywhere", |s| {
            s.session
                .edit_sequence("Transition Everywhere", |q, ctx, _| {
                    let wanted = cuts
                        .iter()
                        .map(|(a, b, at)| {
                            let id = old.iter().find(|o| o.from == Some(*a)).map_or_else(|| TransitionId(ctx.alloc()), |o| o.id);
                            Transition {
                                id,
                                effect: effect.clone(),
                                start: *at,
                                duration: len,
                                from: Some(*a),
                                to: Some(*b),
                                align: TransitionAlign::default(),
                                reverse: false,
                            }
                        })
                        .collect();
                    lay_out(q, wanted, &spare, &mut || ctx.alloc());
                    Ok(())
                })
                .map_err(crate::engine)
        })?;
        Ok(self.transitions_view().len())
    }
}

/// Two made-up pictures, `w × h`: the clip before the cut (warm, a disc) and the one after it
/// (cool, stripes), for the Transitions tab's previews.
fn sample_pictures(w: usize, h: usize) -> (Image, Image) {
    let mut a = Image::new(w, h);
    let mut b = Image::new(w, h);
    let (wf, hf) = (w.max(1) as f32, h.max(1) as f32);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / wf, y as f32 / hf);
            let d = ((u - 0.5) * wf / hf).hypot(v - 0.5);
            let disc = if d < 0.26 { 1.0 } else { 0.0 };
            let pa = [0.85 - 0.35 * v + 0.15 * disc, 0.45 - 0.2 * v + 0.45 * disc, 0.12 + 0.3 * disc, 1.0];
            let stripe = if ((u * 6.0 + v * 2.0) as i32) % 2 == 0 { 0.12 } else { 0.0 };
            let pb = [0.08 + stripe, 0.38 + 0.25 * u + stripe, 0.62 + 0.25 * v + stripe, 1.0];
            // those are sRGB; the engine's pictures are linear light
            let lin = |c: [f32; 4]| [c[0].clamp(0.0, 1.0).powf(2.2), c[1].clamp(0.0, 1.0).powf(2.2), c[2].clamp(0.0, 1.0).powf(2.2), c[3]];
            let k = (y * w + x) * 4;
            if let (Some(sa), Some(sb)) = (a.px.get_mut(k..k + 4), b.px.get_mut(k..k + 4)) {
                sa.copy_from_slice(&lin(pa));
                sb.copy_from_slice(&lin(pb));
            }
        }
    }
    (a, b)
}

/// Each transition's preview: `frames` pictures `w × h` from start to end, transition by
/// transition in [`TRANSITIONS`]' order.
pub fn transition_previews(w: u32, h: u32, frames: u32) -> Result<Vec<Frame>> {
    let (w, h, frames) = (w.clamp(16, 320), h.clamp(16, 320), frames.clamp(2, 24));
    let (a, b) = sample_pictures(w as usize, h as usize);
    let mut out = Vec::with_capacity(TRANSITIONS.len() * frames as usize);
    for p in TRANSITIONS {
        let e = p.instance()?;
        for k in 0..frames {
            let t = k as f32 / (frames - 1) as f32;
            // pixel sizes (blur, borders) are meant for a 1920-wide frame
            let img = filmcraft_render::transitions::apply_scaled(&e, &a, &b, t, w as f32 / 1920.0);
            out.push(Frame { w, h, rgba: img.over_black_rgba8() });
        }
    }
    Ok(out)
}
