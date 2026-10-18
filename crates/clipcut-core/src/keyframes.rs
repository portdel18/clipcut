//! Keyframes: a clip's position, scale, rotation, opacity and volume changing over time.
//!
//! The engine keeps keyframes per parameter, in media time; the timeline shows them in timeline
//! seconds. A property with keyframes is animated: [`Studio::set_props`] with `at` then writes a
//! keyframe there instead of changing the one value. [`Studio::toggle_keyframe`] adds a keyframe
//! at a time (with the value the property has there) or removes the one there; removing the last
//! leaves the property still at that value. Volume keyframes go on the clip's own sound.

use filmcraft_project::{ClipId, Sequence, Track, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::json;

use crate::props::{ClipProps, clip_props_at};
use crate::{Error, Result, Studio, engine};

/// The properties that take keyframes: name, and the engine effect and parameter behind it.
pub const ANIMATABLE: &[(&str, &str, &str)] = &[
    ("position", "motion", "position"),
    ("scale", "motion", "scale"),
    ("rotation", "motion", "rotation"),
    ("opacity", "opacity", "opacity"),
    ("volume", "volume", "level"),
];

/// One property's keyframes on a clip.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyTrack {
    /// `position`, `scale`, `rotation`, `opacity` or `volume`.
    pub prop: &'static str,
    /// Timeline seconds, in order.
    pub times: Vec<f64>,
    /// Its keyframes ease in and out (rather than changing at a steady rate).
    pub smooth: bool,
}

/// Timeline time at which clip `c` plays media time `m`.
fn timeline_at(c: &TrackItem, m: Tick) -> Tick {
    let speed = c.speed.abs().max(1e-6);
    Tick(c.start.0.saturating_add(((m.0 - c.source_in.0) as f64 / speed).round() as i64))
}

/// The keyframes of `item`'s own parameters (no sound for a picture: see [`keys_of`]).
fn own_keys(track: &Track, item: &TrackItem) -> Vec<KeyTrack> {
    ANIMATABLE
        .iter()
        .filter(|(prop, ..)| (*prop == "volume") == (track.kind == TrackKind::Audio))
        .filter_map(|(prop, eff, param)| {
            let p = item.effects.iter().find(|e| e.effect == *eff)?.param(param)?;
            if !p.is_animated() {
                return None;
            }
            Some(KeyTrack {
                prop,
                times: p.keyframes.iter().map(|k| timeline_at(item, k.time).seconds()).collect(),
                smooth: p.keyframes.iter().any(|k| k.interp != filmcraft_project::Interpolation::Linear),
            })
        })
        .collect()
}

/// A clip's sound clips: itself on a sound track, else its linked sound.
fn sound_of<'a>(q: &'a Sequence, track: &Track, item: &'a TrackItem) -> Vec<&'a TrackItem> {
    if track.kind == TrackKind::Audio {
        return vec![item];
    }
    let Some(link) = item.link else { return Vec::new() };
    q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.link == Some(link)).collect()
}

/// Every keyframed property of a clip, its sound's volume included.
pub(crate) fn keys_of(q: &Sequence, track: &Track, item: &TrackItem) -> Vec<KeyTrack> {
    let mut keys = own_keys(track, item);
    if track.kind != TrackKind::Audio
        && let Some(snd) = sound_of(q, track, item).first()
        && let Some(t) = q.audio_tracks.iter().find(|t| t.items.iter().any(|i| i.id == snd.id))
    {
        keys.extend(own_keys(t, snd));
    }
    keys
}

impl Studio {
    fn find(&self, clip: u64) -> Result<(&Sequence, &Track, &TrackItem)> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (tid, item) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let track = q.track(tid).ok_or(Error::NothingOpen)?;
        Ok((q, track, item))
    }

    /// Clip `clip`'s properties as they are at `seconds` (animated ones from their keyframes).
    pub fn props_at(&self, clip: u64, seconds: f64) -> Result<ClipProps> {
        let t = self.frame_tick(seconds)?;
        let (q, track, item) = self.find(clip)?;
        let mt = |i: &TrackItem| i.source_time_at(t.clamp(i.start, (i.end() - Tick(1)).max(i.start)));
        let (_, mut p) = clip_props_at(&self.session, q, track, item, mt(item));
        if track.kind != TrackKind::Audio
            && let Some(snd) = sound_of(q, track, item).first()
            && let Some(st) = q.audio_tracks.iter().find(|t| t.items.iter().any(|i| i.id == snd.id))
        {
            p.volume_db = clip_props_at(&self.session, q, st, snd, mt(snd)).1.volume_db;
        }
        Ok(p)
    }

    /// Adds a keyframe for `prop` of clip `clip` at `seconds` (the value it has there), or removes
    /// the one there. Returns whether one was added. One undo step.
    pub fn toggle_keyframe(&mut self, clip: u64, prop: &str, seconds: f64) -> Result<bool> {
        let &(_, eff, param) = ANIMATABLE.iter().find(|(p, ..)| *p == prop).ok_or_else(|| Error::Invalid(format!("“{prop}” can't take keyframes")))?;
        let t = self.frame_tick(seconds)?;
        let (q, track, item) = self.find(clip)?;
        if t < item.start || t >= item.end() {
            return Err(Error::Invalid("move the playhead over the clip to set a keyframe".into()));
        }
        let targets: Vec<u64> = if prop == "volume" {
            sound_of(q, track, item).iter().map(|i| i.id.0).collect()
        } else if track.kind == TrackKind::Audio {
            Vec::new()
        } else {
            vec![clip]
        };
        if targets.is_empty() {
            return Err(Error::Invalid(if prop == "volume" { "this clip has no sound".into() } else { format!("a sound clip has no {prop}") }));
        }
        let before = keys_of(q, track, item).iter().find(|k| k.prop == prop).map_or(0, |k| k.times.len());
        let label = format!("Keyframe {}", capital(prop));
        self.group(&label, |s| {
            for c in &targets {
                s.session.execute("effects.addKeyframe", json!({"clip": c, "effect": eff, "param": param, "time": t.0})).map_err(engine)?;
            }
            Ok(())
        })?;
        let (q, track, item) = self.find(clip)?;
        let after = keys_of(q, track, item).iter().find(|k| k.prop == prop).map_or(0, |k| k.times.len());
        Ok(after > before)
    }

    /// Makes every keyframe of clip `clip` (and its sound) ease in and out (`smooth`), or change at
    /// a steady rate. One undo step.
    pub fn set_easing(&mut self, clip: u64, smooth: bool) -> Result<()> {
        let (q, track, item) = self.find(clip)?;
        let mut items = vec![item.id.0];
        if track.kind != TrackKind::Audio {
            items.extend(sound_of(q, track, item).iter().map(|i| i.id.0));
        }
        let mut ops = Vec::new();
        for c in items {
            let Some((_, i)) = q.find_item(ClipId(c)) else { continue };
            for (_, eff, param) in ANIMATABLE {
                let Some(p) = i.effects.iter().find(|e| e.effect == *eff).and_then(|e| e.param(param)) else { continue };
                for k in &p.keyframes {
                    ops.push(
                        json!({"clip": c, "effect": eff, "param": param, "mediaTime": k.time.0, "interpolation": if smooth { "autoBezier" } else { "linear" }}),
                    );
                }
            }
        }
        if ops.is_empty() {
            return Err(Error::Invalid("this clip has no keyframes yet".into()));
        }
        self.group(if smooth { "Smooth Keyframes" } else { "Steady Keyframes" }, |s| {
            for op in ops {
                s.session.execute("effects.setInterpolation", op).map_err(engine)?;
            }
            Ok(())
        })
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
