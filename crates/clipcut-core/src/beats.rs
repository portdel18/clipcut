//! Beat markers, CapCut-style: the beats of a music clip, found by the engine's beat tracker (the
//! one Remix uses), kept as markers on the clip so they move with it, shown on the timeline and
//! snapped to. A beat can also be added or removed by hand at the playhead.

use filmcraft_project::{ClipId, Label, Marker, MarkerId, MarkerKind, TrackItem, TrackKind};
use filmcraft_time::Tick;

use crate::{Error, Result, Studio, engine};

/// The name of a beat marker.
const BEAT: &str = "Beat";
/// Longest music analysed at once (seconds), so a long file can't take all the memory.
const MOST_SECONDS: f64 = 20.0 * 60.0;

/// Clip `i`'s beats as timeline seconds, in order.
pub(crate) fn beats_of(i: &TrackItem) -> Vec<f64> {
    let speed = i.speed.abs().max(1e-9);
    let mut v: Vec<f64> = i
        .markers
        .iter()
        .filter(|m| m.name == BEAT)
        .map(|m| (i.start + Tick(((m.start - i.source_in).0 as f64 / speed).round() as i64)).seconds())
        .filter(|t| *t >= i.start.seconds() - 1e-9 && *t <= i.end().seconds() + 1e-9)
        .collect();
    v.sort_by(f64::total_cmp);
    v
}

impl Studio {
    /// The sound clip `clip` (or a picture clip's own sound) on the timeline.
    fn sound_clip(&self, clip: u64) -> Result<u64> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (t, i) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        if q.track(t).is_some_and(|t| t.kind == TrackKind::Audio) {
            return Ok(clip);
        }
        i.link
            .and_then(|l| q.audio_tracks.iter().flat_map(|t| &t.items).find(|a| a.link == Some(l)))
            .map(|a| a.id.0)
            .ok_or_else(|| Error::Invalid("this clip has no sound to find beats in".into()))
    }

    /// Finds the beats of sound clip `clip` and marks them (replacing the beats it had). Returns
    /// how many, and the tempo (beats a minute). One undo step.
    pub fn mark_beats(&mut self, clip: u64) -> Result<(usize, f64)> {
        let clip = self.sound_clip(clip)?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let sr = q.settings.sample_rate.max(1);
        let (_, item) = q.find_item(ClipId(clip)).ok_or(Error::NothingOpen)?;
        let a0 = item.start.to_units_floor(i64::from(sr));
        let a1 = item.end().to_units_floor(i64::from(sr));
        let most = (MOST_SECONDS * f64::from(sr)) as i64;
        let frames = usize::try_from((a1 - a0).clamp(0, most)).unwrap_or(0);
        if frames == 0 {
            return Err(Error::Invalid("the clip is too short to have beats".into()));
        }
        let provider = self.session.media.full_res_provider(self.session.project.clone(), self.session.services.clone());
        let [l, r] = filmcraft_render::audio::clip_signal(item, a0, frames, sr, &provider)
            .ok_or_else(|| Error::Invalid("this clip has no sound to find beats in".into()))?;
        let found = filmcraft_audio_dsp::remix::analyze(&[&l, &r], sr)
            .map_err(|_| Error::Invalid("no steady beat was found in this sound: beats come from music with a clear rhythm".into()))?;
        // timeline sample → the media time the marker keeps
        let times: Vec<Tick> = found
            .beats
            .iter()
            .map(|b| Tick::from_seconds_f64((a0 + b) as f64 / f64::from(sr)))
            .filter(|t| *t >= item.start && *t < item.end())
            .map(|t| item.source_time_at(t))
            .collect();
        let (n, bpm) = (times.len(), found.bpm);
        self.group("Mark Beats", |s| {
            s.session
                .edit_sequence("Mark Beats", |q, ctx, _| {
                    if let Some((_, i)) = q.find_item_mut(ClipId(clip)) {
                        i.markers.retain(|m| m.name != BEAT);
                        for t in &times {
                            i.markers.push(beat(*t, ctx.alloc()));
                        }
                    }
                    Ok(())
                })
                .map_err(engine)
        })?;
        Ok((n, bpm))
    }

    /// Takes every beat off sound clip `clip`. One undo step.
    pub fn clear_beats(&mut self, clip: u64) -> Result<()> {
        let clip = self.sound_clip(clip)?;
        self.group("Clear Beats", |s| {
            s.session
                .edit_sequence("Clear Beats", |q, _, _| {
                    if let Some((_, i)) = q.find_item_mut(ClipId(clip)) {
                        i.markers.retain(|m| m.name != BEAT);
                    }
                    Ok(())
                })
                .map_err(engine)
        })
    }

    /// Adds a beat to sound clip `clip` at `seconds` on the timeline, or takes away the one within
    /// a frame of it. Returns true when a beat was added. One undo step.
    pub fn toggle_beat(&mut self, clip: u64, seconds: f64) -> Result<bool> {
        let clip = self.sound_clip(clip)?;
        let t = self.frame_tick(seconds)?;
        let frame = self.fps()?.frame_duration();
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, item) = q.find_item(ClipId(clip)).ok_or(Error::NothingOpen)?;
        if t < item.start || t >= item.end() {
            return Err(Error::Invalid("move the playhead over the clip first".into()));
        }
        let mt = item.source_time_at(t);
        let near = item.markers.iter().find(|m| m.name == BEAT && (m.start - mt).0.abs() < frame.0).map(|m| m.id);
        self.group(if near.is_some() { "Remove Beat" } else { "Add Beat" }, |s| {
            s.session
                .edit_sequence("Beat", |q, ctx, _| {
                    if let Some((_, i)) = q.find_item_mut(ClipId(clip)) {
                        match near {
                            Some(id) => i.markers.retain(|m| m.id != id),
                            None => i.markers.push(beat(mt, ctx.alloc())),
                        }
                    }
                    Ok(())
                })
                .map_err(engine)
        })?;
        Ok(near.is_none())
    }
}

fn beat(at: Tick, id: u64) -> Marker {
    Marker { id: MarkerId(id), start: at, duration: Tick::ZERO, name: BEAT.into(), comment: String::new(), kind: MarkerKind::Comment, color: Label::Mango }
}
