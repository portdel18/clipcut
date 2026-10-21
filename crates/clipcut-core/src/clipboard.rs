//! Copy and paste clips, CapCut-style: Ctrl+C keeps the selected clips (with their linked sound),
//! Ctrl+V puts copies at the playhead. A copy plays the same media as the clip it came from, so a
//! graphic is pasted as it is, never rendered again. Lane clips (graphics, pictures, text, sound)
//! keep their spacing and lane, or go to a free lane when theirs is taken there; main-track clips
//! go into the main track after the clip under the playhead, pushing the rest along.

use filmcraft_project::{ClipId, TrackItem, TrackKind};
use filmcraft_time::Tick;

use crate::lanes::{Lane, main_links};
use crate::{Error, Result, Studio};

/// What Ctrl+C kept: each clip with its lane (index 0 = the main track's) and the time it starts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Copied {
    clips: Vec<(Lane, usize, TrackItem)>,
}

impl Studio {
    /// Copies clips `clips` and the clips linked to them (a picture's sound). Returns how many
    /// clips were copied.
    pub fn copy_clips(&mut self, clips: &[u64]) -> Result<usize> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let links: std::collections::HashSet<u64> = clips.iter().filter_map(|c| q.find_item(ClipId(*c))).filter_map(|(_, i)| i.link).collect();
        let mut out = Vec::new();
        for (lane, ts) in [(Lane::Video, &q.video_tracks), (Lane::Audio, &q.audio_tracks)] {
            for (index, t) in ts.iter().enumerate() {
                for i in &t.items {
                    if clips.contains(&i.id.0) || i.link.is_some_and(|l| links.contains(&l)) {
                        out.push((lane, index, i.clone()));
                    }
                }
            }
        }
        if out.is_empty() {
            return Err(Error::Invalid("select the clips to copy first".into()));
        }
        let n = out.len();
        self.clipboard = Copied { clips: out };
        Ok(n)
    }

    /// Whether there is something to paste.
    pub fn can_paste(&self) -> bool {
        !self.clipboard.clips.is_empty()
    }

    /// Pastes the copied clips at `seconds`. One undo step. Returns the new clips.
    pub fn paste(&mut self, seconds: f64) -> Result<Vec<u64>> {
        let at = self.frame_tick(seconds)?;
        let copied = self.clipboard.clips.clone();
        if copied.is_empty() {
            return Err(Error::Invalid("there is nothing to paste: copy clips first (Ctrl+C)".into()));
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mains = main_links(q);
        // main-track clips, and the main track's sound with them wherever it is
        let is_main = |lane: Lane, index: usize, i: &TrackItem| (lane == Lane::Video && index == 0) || i.link.is_some_and(|l| mains.contains(&l));
        let (main, lanes): (Vec<_>, Vec<_>) = copied.into_iter().partition(|(lane, index, i)| is_main(*lane, *index, i));
        // main clips go in after the clip under the playhead, so none is cut in two
        let main_at = q.video_tracks.first().and_then(|t| t.items.iter().find(|i| i.start < at && i.end() > at)).map_or(at, |i| i.end());
        let label = "Paste";
        let run = |s: &mut Studio| -> Result<Vec<u64>> {
            s.unlock_lanes()?;
            let mut made = Vec::new();
            if !lanes.is_empty() {
                made.extend(s.paste_on_lanes(&lanes, at)?);
            }
            if !main.is_empty() {
                made.extend(s.paste_on_main(&main, main_at)?);
            }
            Ok(made)
        };
        if main.is_empty() { self.group(label, run) } else { self.group_follow(label, None, run) }
    }

    /// Lane clips at `at`, keeping their spacing: each linked set on its own lane when that is
    /// free there, else on a free one.
    fn paste_on_lanes(&mut self, clips: &[(Lane, usize, TrackItem)], at: Tick) -> Result<Vec<u64>> {
        let earliest = clips.iter().map(|(_, _, i)| i.start).min().unwrap_or(at);
        let shift = Tick(at.0.saturating_sub(earliest.0));
        // lanes first (a free lane may have to be added), then the clips in one edit
        let mut placed: Vec<(Lane, usize, TrackItem)> = Vec::new();
        for (lane, index, item) in clips {
            let mut it = item.clone();
            it.start = Tick(it.start.0.saturating_add(shift.0));
            let taken = placed.iter().any(|(l, k, p)| *l == *lane && *k == *index && p.start < it.end() && p.end() > it.start);
            let dest = if !taken && *index > 0 && self.lane_is_free(*lane, *index, it.start, it.end()) {
                *index
            } else {
                let mut d = self.free_lane(*lane, it.start, it.end(), &[])?;
                // a lane free on the timeline may be one this paste just filled
                while placed.iter().any(|(l, k, p)| *l == *lane && *k == d && p.start < it.end() && p.end() > it.start) {
                    d = self.free_lane_after(*lane, d, it.start, it.end())?;
                }
                d
            };
            placed.push((*lane, dest, it));
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let tracks: Vec<(TrackKind, Vec<filmcraft_project::TrackId>)> =
            vec![(TrackKind::Video, q.video_tracks.iter().map(|t| t.id).collect()), (TrackKind::Audio, q.audio_tracks.iter().map(|t| t.id).collect())];
        let track_of = |lane: Lane, k: usize| {
            tracks.iter().find(|(kind, _)| (*kind == TrackKind::Video) == (lane == Lane::Video)).and_then(|(_, ids)| ids.get(k).copied())
        };
        let placements: Vec<(filmcraft_project::TrackId, TrackItem)> = placed.into_iter().filter_map(|(lane, k, it)| Some((track_of(lane, k)?, it))).collect();
        self.place_copies(placements, false)
    }

    /// Main-track clips (and their sound) into the main track at `at`, pushing the rest along.
    fn paste_on_main(&mut self, clips: &[(Lane, usize, TrackItem)], at: Tick) -> Result<Vec<u64>> {
        let earliest = clips.iter().filter(|(l, k, _)| *l == Lane::Video && *k == 0).map(|(_, _, i)| i.start).min();
        let earliest = earliest.or_else(|| clips.iter().map(|(_, _, i)| i.start).min()).unwrap_or(at);
        let shift = Tick(at.0.saturating_sub(earliest.0));
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (Some(v1), Some(a1)) = (q.video_tracks.first().map(|t| t.id), q.audio_tracks.first().map(|t| t.id)) else {
            return Err(Error::Invalid("the video has no main track to paste into".into()));
        };
        // the main track's sound goes back onto the main track's sound, in step with its picture
        let placements = clips
            .iter()
            .map(|(lane, _, item)| {
                let mut it = item.clone();
                it.start = Tick(it.start.0.saturating_add(shift.0));
                (if *lane == Lane::Video { v1 } else { a1 }, it)
            })
            .collect();
        let made = self.place_copies(placements, true)?;
        self.pack_main()?;
        Ok(made)
    }

    /// Puts copies of `placements` on their tracks (new ids, new links between them), inserting
    /// room for them when `insert`. Returns the new clips.
    fn place_copies(&mut self, placements: Vec<(filmcraft_project::TrackId, TrackItem)>, insert: bool) -> Result<Vec<u64>> {
        if placements.is_empty() {
            return Ok(Vec::new());
        }
        let ids = self
            .session
            .edit_sequence("Paste", |q, ctx, _| {
                let mut links = std::collections::HashMap::new();
                let mut out = Vec::with_capacity(placements.len());
                for (t, mut it) in placements {
                    it.id = ClipId(ctx.alloc());
                    if let Some(l) = it.link {
                        it.link = Some(*links.entry(l).or_insert_with(|| ctx.alloc()));
                    }
                    out.push((t, it));
                }
                Ok(if insert { filmcraft_edit::insert(q, out, ctx)? } else { filmcraft_edit::overwrite(q, out, ctx)? })
            })
            .map_err(crate::engine)?;
        Ok(ids.into_iter().map(|c| c.0).collect())
    }

    fn lane_is_free(&self, lane: Lane, index: usize, start: Tick, end: Tick) -> bool {
        self.session.active_sequence().is_some_and(|q| crate::lanes::is_free(q, lane, index, start, end, &[]))
    }

    /// The first lane after `after` free over `[start, end)`, adding one when none is.
    fn free_lane_after(&mut self, lane: Lane, after: usize, start: Tick, end: Tick) -> Result<usize> {
        let n = self.session.active_sequence().map_or(0, |q| if lane == Lane::Video { q.video_tracks.len() } else { q.audio_tracks.len() });
        if let Some(k) = (after + 1..n).find(|k| self.lane_is_free(lane, *k, start, end)) {
            return Ok(k);
        }
        self.add_lane(lane)
    }
}
