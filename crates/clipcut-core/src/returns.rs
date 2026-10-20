//! Keyframed changes that end: after a property's last keyframe it goes straight back to the value
//! it had at its first, so a zoom to 500 % between keyframes is back at 100 % for the rest of the
//! clip. The way back is one more keyframe, a frame after the last (instant on screen; for volume
//! a 33 ms ramp, no click), that ClipCut keeps itself: a clip marker lists them, the timeline never
//! shows them, and every keyframe edit takes them off first and puts them back after. A clip that
//! holds its last values (`Studio::set_hold`) has none.

use filmcraft_project::{ClipId, Label, Marker, MarkerId, MarkerKind, Sequence, TrackItem};
use filmcraft_time::Tick;

use crate::keyframes::ANIMATABLE;
use crate::{Error, Result, Studio, engine};

/// The marker listing an item's return keyframes (`motion/scale@1234;opacity/opacity@5678`), or
/// `hold` when the clip keeps its last values.
const MARK: &str = "ClipCut return";

/// What an item's marker says: whether it holds, and its return keyframes (effect, param, time).
struct Returns {
    hold: bool,
    keys: Vec<(String, String, Tick)>,
}

fn parse(comment: &str) -> Returns {
    let mut r = Returns { hold: false, keys: Vec::new() };
    for part in comment.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        if part == "hold" {
            r.hold = true;
            continue;
        }
        let Some((what, at)) = part.split_once('@') else { continue };
        let Some((eff, param)) = what.split_once('/') else { continue };
        if let Ok(t) = at.parse::<i64>() {
            r.keys.push((eff.to_string(), param.to_string(), Tick(t)));
        }
    }
    r
}

fn format(r: &Returns) -> String {
    let mut parts: Vec<String> = r.keys.iter().map(|(e, p, t)| format!("{e}/{p}@{}", t.0)).collect();
    if r.hold {
        parts.insert(0, "hold".into());
    }
    parts.join(";")
}

fn returns_of(i: &TrackItem) -> Returns {
    i.markers.iter().find(|m| m.name == MARK).map_or(Returns { hold: false, keys: Vec::new() }, |m| parse(&m.comment))
}

/// Whether the clip keeps its last values after its last keyframe.
pub(crate) fn holds(i: &TrackItem) -> bool {
    returns_of(i).hold
}

/// Whether the keyframe of `eff`/`param` at media time `t` is one of the item's return keyframes
/// (the timeline doesn't show them).
pub(crate) fn is_return(i: &TrackItem, eff: &str, param: &str, t: Tick) -> bool {
    returns_of(i).keys.iter().any(|(e, p, at)| e == eff && p == param && *at == t)
}

/// Takes item `i`'s return keyframes off (its marker keeps only whether it holds).
fn strip(i: &mut TrackItem) {
    let r = returns_of(i);
    for (eff, param, t) in &r.keys {
        if let Some(p) = i.effects.iter_mut().find(|e| e.effect == *eff).and_then(|e| e.params.get_mut(param.as_str())) {
            p.keyframes.retain(|k| k.time != *t);
        }
    }
    write(i, &Returns { hold: r.hold, keys: Vec::new() }, None);
}

/// Puts item `i`'s return keyframes back: for every animated property whose last keyframe differs
/// from its first, one a frame (`frame`, media time) after the last, with the first's value.
fn rebuild(i: &mut TrackItem, frame: Tick, alloc: &mut dyn FnMut() -> u64) {
    let hold = returns_of(i).hold;
    // ducked music's volume keyframes are the dips under speech, not a change that ends
    let ducked = crate::ducking::ducking_of(i).is_some();
    let mut keys = Vec::new();
    if !hold {
        for (_, eff, param) in ANIMATABLE {
            if ducked && *eff == "volume" {
                continue;
            }
            let Some(p) = i.effects.iter_mut().find(|e| e.effect == *eff).and_then(|e| e.params.get_mut(*param)) else { continue };
            let (Some(first), Some(last)) = (p.keyframes.first().cloned(), p.keyframes.last().cloned()) else { continue };
            if p.keyframes.len() < 2 || first.value == last.value {
                continue;
            }
            let at = last.time + frame;
            p.keyframes.push(filmcraft_project::Keyframe { time: at, value: first.value.clone(), ..last.clone() });
            keys.push((eff.to_string(), param.to_string(), at));
        }
    }
    write(i, &Returns { hold, keys }, Some(alloc));
}

fn write(i: &mut TrackItem, r: &Returns, alloc: Option<&mut dyn FnMut() -> u64>) {
    let existing = i.markers.iter().position(|m| m.name == MARK);
    if !r.hold && r.keys.is_empty() {
        if let Some(k) = existing {
            i.markers.remove(k);
        }
        return;
    }
    let comment = format(r);
    match (existing, alloc) {
        (Some(k), _) => {
            if let Some(m) = i.markers.get_mut(k) {
                m.comment = comment;
            }
        }
        (None, Some(alloc)) => i.markers.push(Marker {
            id: MarkerId(alloc()),
            start: i.source_in,
            duration: Tick::ZERO,
            name: MARK.into(),
            comment,
            kind: MarkerKind::Comment,
            color: Label::Violet,
        }),
        // stripping never adds a marker (an item without one has nothing to take off)
        (None, None) => {}
    }
}

/// Clip `clip` and the sound linked to it: the items a keyframe edit on it touches.
fn items_of(q: &Sequence, clip: u64) -> Vec<ClipId> {
    let Some((_, item)) = q.find_item(ClipId(clip)) else { return Vec::new() };
    let mut v = vec![item.id];
    if let Some(l) = item.link {
        v.extend(q.all_tracks().flat_map(|t| t.items.iter()).filter(|i| i.link == Some(l) && i.id != item.id).map(|i| i.id));
    }
    v
}

impl Studio {
    /// Inside an edit: takes the return keyframes off `clips` (and their sound), runs `f`, and puts
    /// them back to fit the keyframes `f` left.
    pub(crate) fn with_returns<T>(&mut self, clips: &[u64], f: impl FnOnce(&mut Studio) -> Result<T>) -> Result<T> {
        self.edit_returns(clips, false)?;
        let r = f(self)?;
        self.edit_returns(clips, true)?;
        Ok(r)
    }

    fn edit_returns(&mut self, clips: &[u64], put_back: bool) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let items: Vec<ClipId> = clips.iter().flat_map(|c| items_of(q, *c)).collect();
        let fd = q.settings.frame_rate.frame_duration();
        let touched =
            |i: &TrackItem| i.markers.iter().any(|m| m.name == MARK) || (put_back && i.effects.iter().any(|e| e.params.values().any(|p| p.is_animated())));
        if !items.iter().any(|id| q.find_item(*id).is_some_and(|(_, i)| touched(i))) {
            return Ok(());
        }
        self.session
            .edit_sequence(if put_back { "Keyframes" } else { "Keyframes (prepare)" }, |q, ctx, _| {
                for id in &items {
                    let Some((_, i)) = q.find_item_mut(*id) else { continue };
                    if put_back {
                        // a frame of the timeline is `speed` frames of the media
                        let frame = Tick(((fd.0 as f64) * i.speed.abs().max(1e-3)).round().max(1.0) as i64);
                        rebuild(i, frame, &mut || ctx.alloc());
                    } else {
                        strip(i);
                    }
                }
                Ok(())
            })
            .map(|_| ())
            .map_err(engine)
    }

    /// Clip `clip` keeps its last values after its last keyframe (`hold`), or goes back to the
    /// values at its first (the default). One undo step.
    pub fn set_hold(&mut self, clip: u64, hold: bool) -> Result<()> {
        if self.session.active_sequence().and_then(|q| q.find_item(ClipId(clip))).is_none() {
            return Err(Error::Invalid("that clip is no longer on the timeline".into()));
        }
        self.group(if hold { "Hold Last Keyframe" } else { "Back After Last Keyframe" }, |s| {
            s.with_returns(&[clip], |s| {
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let items = items_of(q, clip);
                s.session
                    .edit_sequence("Keyframes", |q, ctx, _| {
                        for id in &items {
                            if let Some((_, i)) = q.find_item_mut(*id) {
                                let r = returns_of(i);
                                write(i, &Returns { hold, keys: r.keys }, Some(&mut || ctx.alloc()));
                            }
                        }
                        Ok(())
                    })
                    .map(|_| ())
                    .map_err(engine)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_reads_back_and_ignores_junk() {
        let r = parse("hold;motion/scale@1234;opacity/opacity@-5;junk;x@y;a/b@c");
        assert!(r.hold);
        assert_eq!(r.keys, [("motion".to_string(), "scale".to_string(), Tick(1234)), ("opacity".into(), "opacity".into(), Tick(-5))]);
        assert_eq!(format(&r), "hold;motion/scale@1234;opacity/opacity@-5");
        assert!(!parse("").hold && parse("").keys.is_empty());
    }
}
