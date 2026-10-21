//! Precise trimming: a clip's edges to the playhead, frame by frame nudges of lane clips, slips
//! (other frames of the media in the same place) and rolls (a cut between two main-track clips
//! moved, the video keeping its length).

use filmcraft_project::ClipId;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

impl Studio {
    /// Trims clip `clip`'s start (`edge_in`) or end to `seconds`: the main track closes up, a
    /// lane clip keeps its place. Returns the change made (seconds).
    pub fn trim_to(&mut self, clip: u64, edge_in: bool, seconds: f64) -> Result<f64> {
        let t = self.frame_tick(seconds)?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, i) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        if t <= i.start || t >= i.end() {
            return Err(Error::Invalid("move the playhead inside the clip first".into()));
        }
        let delta = if edge_in { t - i.start } else { t - i.end() };
        self.trim(clip, edge_in, delta.seconds())
    }

    /// Moves lane clips `frames` frames later (earlier when negative). Main-track clips stay
    /// back to back: reorder them, or roll the cut between two.
    pub fn nudge(&mut self, clips: &[u64], frames: i64) -> Result<()> {
        if clips.is_empty() {
            return Err(Error::Invalid("select a clip first".into()));
        }
        if clips.iter().any(|c| self.on_main(*c)) {
            return Err(Error::Invalid("clips on the main track stay back to back: drag one to reorder it, or move a cut".into()));
        }
        let rate = self.fps()?;
        let step = rate.frame_duration();
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mut moves = Vec::new();
        for c in clips {
            let (tid, i) = q.find_item(ClipId(*c)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
            let to = i.start + filmcraft_time::Tick(step.0.saturating_mul(frames));
            if to < filmcraft_time::Tick::ZERO {
                return Err(Error::Invalid("the clip is already at the start".into()));
            }
            moves.push(json!({"clip": c, "track": tid.0, "time": to.0}));
        }
        self.group("Nudge", |s| {
            s.unlock_lanes()?;
            s.session.execute("timeline.move", json!({"moves": moves, "linked": true})).map(|_| ()).map_err(engine)
        })
    }

    /// Slips clip `clip` by `frames`: it shows media that many frames later (earlier when
    /// negative), in the same place and length. Returns the frames slipped (the media's ends
    /// stop it).
    pub fn slip(&mut self, clip: u64, frames: i64) -> Result<i64> {
        let rate = self.fps()?;
        let source_in = |s: &Self| s.session.active_sequence().and_then(|q| q.find_item(ClipId(clip)).map(|(_, i)| i.source_in));
        let before = source_in(self).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        // what the clip shows changes, not where it is: the lanes stay
        self.without_linkage(|st| {
            st.group_follow("Slip", None, |s| {
                s.session.execute("timeline.slip", json!({"clip": clip, "deltaFrames": frames})).map_err(engine)?;
                let moved = rate.frame_at(source_in(s).unwrap_or(before) - before);
                if moved == 0 && frames != 0 {
                    return Err(Error::Invalid(format!("the clip already shows the media's {} frames", if frames < 0 { "first" } else { "last" })));
                }
                Ok(moved)
            })
        })
    }

    /// Moves the cut after main-track clip `from` by `frames` (later when positive): one clip
    /// grows as the other shrinks, so the video keeps its length. One undo step.
    pub fn roll(&mut self, from: u64, frames: i64) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let main = q.video_tracks.first().ok_or(Error::NothingOpen)?;
        let mut items: Vec<_> = main.items.iter().collect();
        items.sort_by_key(|i| i.start);
        let k = items.iter().position(|i| i.id.0 == from).ok_or_else(|| Error::Invalid("rolls move a cut on the main track".into()))?;
        let (a, b) = (items.get(k).ok_or(Error::NothingOpen)?, items.get(k + 1).ok_or_else(|| Error::Invalid("this clip has no clip after it".into()))?);
        if a.end() != b.start {
            return Err(Error::Invalid("these clips don't meet".into()));
        }
        let (left, right, cut) = (a.id.0, b.id.0, a.end());
        self.group_follow("Move Cut", None, |s| {
            s.session.execute("timeline.roll", json!({"left": left, "right": right, "deltaFrames": frames, "followLinks": true})).map_err(engine)?;
            let now = s.session.active_sequence().and_then(|q| q.find_item(ClipId(left)).map(|(_, i)| i.end()));
            if frames != 0 && now == Some(cut) {
                let (short, side) = if frames > 0 { ("first", "end") } else { ("second", "start") };
                return Err(Error::Invalid(format!("the cut can't move that way: the {short} clip already shows its media's {side}")));
            }
            Ok(())
        })
    }
}
