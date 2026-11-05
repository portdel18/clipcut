//! Speed ramps, CapCut's curve speed: a clip that speeds up and slows down along its footage
//! (fast, then slow motion on the moment, then fast again). A ramp is points over the clip's
//! footage, each a place in it (0 = where the clip starts, 1 = where it ends) and a speed there (1 =
//! normal), eased smoothly from one to the next. It is the engine's Time Remapping Speed keyframed
//! at those media times ([`filmcraft_project::ramp`]) on the picture and its sound, so it plays,
//! exports, splits and trims with the clip. The clip keeps the same footage: its length becomes the
//! time the ramp takes to play it, and the main track moves along.

use filmcraft_project::{ClipId, Interpolation, Keyframe, ParamValue, TrackItem};
use filmcraft_time::Tick;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

/// The engine effect and parameter a ramp is keyed on.
const REMAP: &str = "time_remap";
const SPEED: &str = "speed";
/// Most points a ramp has.
const MOST_POINTS: usize = 32;

/// Clip `i`'s ramp as points `[place in its footage (0…1), speed (1 = normal)]`; empty for none.
pub(crate) fn ramp_of(i: &TrackItem) -> Vec<[f64; 2]> {
    let Some(p) = i.ramp() else { return Vec::new() };
    let (a, b) = (i.source_in.0 as f64, i.source_out().0 as f64);
    let len = (b - a).max(1.0);
    p.keyframes.iter().map(|k| [round((k.time.0 as f64 - a) / len, 1e4), round(k.value.as_f64().unwrap_or(100.0) / 100.0, 1e3)]).collect()
}

fn round(v: f64, by: f64) -> f64 {
    (v * by).round() / by
}

/// `points` checked: finite, in order of place, speeds within the engine's limits.
fn clean(points: &[[f64; 2]]) -> Result<Vec<[f64; 2]>> {
    if points.len() > MOST_POINTS {
        return Err(Error::Invalid(format!("a speed curve has at most {MOST_POINTS} points")));
    }
    let lo = filmcraft_project::ramp::RAMP_MIN / 100.0;
    let hi = filmcraft_project::ramp::RAMP_MAX / 100.0;
    let mut v: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for [at, speed] in points {
        if !(at.is_finite() && speed.is_finite()) {
            return Err(Error::Invalid("a point of the speed curve isn't a number".into()));
        }
        v.push([at.clamp(-10.0, 10.0), speed.clamp(lo, hi)]);
    }
    v.sort_by(|a, b| a[0].total_cmp(&b[0]));
    v.dedup_by(|b, a| (b[0] - a[0]).abs() < 1e-6);
    if v.len() == 1 {
        return Err(Error::Invalid("a speed curve needs two points or more".into()));
    }
    Ok(v)
}

/// Puts ramp `points` over media `[a, b]` on `i` (playing at its own speed 1).
fn put_ramp(i: &mut TrackItem, points: &[[f64; 2]], a: Tick, b: Tick) -> Result<()> {
    if i.effect(REMAP).is_none() {
        // a sound clip has no Time Remapping of its own: it gets one (intrinsic, so nothing else
        // in the sound's chain sees it)
        let def = filmcraft_project::effect::find_effect(REMAP).ok_or_else(|| Error::Engine("the engine has no Time Remapping".into()))?;
        i.effects.push(def.instance());
    }
    let p = i.effect_mut(REMAP).and_then(|e| e.param_mut(SPEED)).ok_or_else(|| Error::Engine("the engine's Time Remapping has no Speed".into()))?;
    let len = (b - a).0 as f64;
    p.value = ParamValue::Float(100.0);
    p.keyframes = points
        .iter()
        .map(|[at, speed]| {
            let mut k = Keyframe::new(Tick(a.0.saturating_add((at * len).round() as i64)), ParamValue::Float(speed * 100.0));
            // eased from point to point, as a drawn curve is
            k.interp = Interpolation::Bezier;
            k
        })
        .collect();
    p.keyframes.dedup_by_key(|k| k.time);
    i.speed = 1.0;
    Ok(())
}

/// Takes `i`'s ramp off, keeping its footage: it plays at the one speed that fits it in its length.
fn drop_ramp(i: &mut TrackItem) {
    if i.ramp().is_none() {
        return;
    }
    let (a, b) = (i.source_in, i.source_out());
    if let Some(p) = i.effect_mut(REMAP).and_then(|e| e.param_mut(SPEED)) {
        p.keyframes.clear();
        p.value = ParamValue::Float(100.0);
    }
    if i.duration > Tick::ZERO && b > a {
        i.speed = (b - a).0 as f64 / i.duration.0 as f64;
    }
}

impl Studio {
    /// Clip `clip` and the clips linked to it (its sound).
    fn with_partners(&self, clip: u64) -> Vec<u64> {
        let Some(q) = self.session.active_sequence() else { return vec![clip] };
        let link = q.find_item(ClipId(clip)).and_then(|(_, i)| i.link);
        let mut v: Vec<u64> = vec![clip];
        if let Some(l) = link {
            v.extend(q.all_tracks().flat_map(|t| t.items.iter()).filter(|i| i.link == Some(l)).map(|i| i.id.0));
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Takes the ramps off `clips` and their linked clips, each keeping its footage (before a
    /// speed or reverse edit). Inside an edit.
    pub(crate) fn drop_ramps(&mut self, clips: &[u64]) -> Result<()> {
        let all: Vec<u64> = clips.iter().flat_map(|c| self.with_partners(*c)).collect();
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        if !all.iter().any(|c| q.find_item(ClipId(*c)).is_some_and(|(_, i)| i.ramp().is_some())) {
            return Ok(());
        }
        self.session
            .edit_sequence("Speed", |q, _, _| {
                for c in &all {
                    if let Some((_, i)) = q.find_item_mut(ClipId(*c)) {
                        drop_ramp(i);
                    }
                }
                Ok(())
            })
            .map_err(engine)
    }

    /// Gives clip `clip` (and its sound) the speed ramp `points` (`[place in its footage 0…1,
    /// speed (1 = normal)]`, two or more), or none (empty: back to normal speed). It keeps the same
    /// footage, so its length changes; the main track moves along. One undo step.
    pub fn set_ramp(&mut self, clip: u64, points: &[[f64; 2]]) -> Result<()> {
        let points = clean(points)?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, it) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        if it.frame_hold.is_some() {
            return Err(Error::Invalid("a freeze frame doesn't move, so it has no speed to ramp".into()));
        }
        if it.reverse {
            return Err(Error::Invalid("speed curves play forwards: turn Reverse off first".into()));
        }
        if crate::timing::s_is_still(&self.session, it) {
            return Err(Error::Invalid("pictures and stills don't move: choose a video or sound clip for a speed curve".into()));
        }
        let (a, b) = (it.source_in, it.source_out());
        if b <= a || it.duration <= Tick::ZERO {
            return Err(Error::Invalid("this clip has no footage to ramp".into()));
        }
        // how long the footage plays with the ramp
        let length = if points.is_empty() {
            b - a
        } else {
            let mut probe = it.clone();
            put_ramp(&mut probe, &points, a, b)?;
            probe.ramp_time_between(a, b).ok_or_else(|| Error::Invalid("this clip can't take a speed curve".into()))?
        };
        if length <= Tick::ZERO {
            return Err(Error::Invalid("this clip has no footage to ramp".into()));
        }
        let speed = (b - a).0 as f64 / length.0 as f64 * 100.0;
        let partners = self.with_partners(clip);
        let main = self.on_main(clip);
        let label = if points.is_empty() { "Normal Speed" } else { "Speed Curve" };
        self.group_follow(label, None, |s| {
            s.drop_ramps(&[clip])?;
            // the clip's new length (its sound with it), the main track moving along
            s.session.execute("clip.speedDuration", json!({"clips": [clip], "speed": speed, "ripple": main})).map_err(engine)?;
            if !points.is_empty() {
                s.session
                    .edit_sequence(label, |q, _, _| {
                        for c in &partners {
                            if let Some((_, i)) = q.find_item_mut(ClipId(*c))
                                && !i.reverse
                                && i.frame_hold.is_none()
                            {
                                // a hostile clip is left as it is rather than stopping the edit
                                let _ = put_ramp(i, &points, a, b);
                            }
                        }
                        Ok(())
                    })
                    .map_err(engine)?;
            }
            s.fit_pitch(&[clip])?;
            if main {
                s.pack_main()?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_are_checked() {
        assert!(clean(&[]).unwrap().is_empty());
        assert!(clean(&[[0.5, 1.0]]).is_err(), "one point is no curve");
        assert!(clean(&[[0.0, f64::NAN], [1.0, 1.0]]).is_err());
        assert!(clean(&vec![[0.0, 1.0]; MOST_POINTS + 1]).is_err());
        // sorted, speeds held to the limits, places on top of each other merged
        let c = clean(&[[1.0, 1e9], [0.0, 0.0], [0.0, 2.0]]).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0][0], 0.0);
        assert_eq!(c[1], [1.0, filmcraft_project::ramp::RAMP_MAX / 100.0]);
        assert!(c[0][1] >= filmcraft_project::ramp::RAMP_MIN / 100.0);
    }
}
