//! Linkage, CapCut-style: clips on the lanes (B-roll, pictures, text, titles, graphics, music,
//! sound effects) follow the main-track clip they start on. Before an edit that moves the main
//! track ([`Studio::group_follow`]) each lane clip is tied to the footage under its start: the
//! main clip and the moment of its media there, or the clip's start when it begins at a cut.
//! Afterwards it goes to wherever that footage now plays, keeping its length, lane (or the next
//! free one) and everything else. When the footage is gone (the clip deleted, or that moment
//! trimmed away) the lane clip goes too, unless it runs on past the clip it started on (a music
//! bed): that stays where it is. Captions, transitions and the main track's own sound follow by
//! their own rules; a lane clip the edit itself moved is left as the edit put it.

use std::collections::HashSet;

use filmcraft_project::{Sequence, TrackId, TrackItem};
use filmcraft_time::Tick;
use serde_json::json;

use crate::lanes::{Lane, is_free, is_main_sound, lane_name, main_links};
use crate::{Result, Studio, engine};

/// A lane clip and the footage it started on, before an edit.
#[derive(Clone, Debug)]
struct Tied {
    clip: u64,
    track: TrackId,
    lane: Lane,
    index: usize,
    start: Tick,
    duration: Tick,
    /// The main-track clip under its start, and that clip as it was.
    on: TrackItem,
    /// The moment of the main clip's media under its start.
    moment: Tick,
    /// It begins at the main clip's start (within a frame): it stays at that cut.
    at_cut: bool,
    /// It runs on past the main clip it starts on.
    spans: bool,
}

/// What [`Studio::hold_lanes`] keeps for [`Studio::place_lanes`].
#[derive(Clone, Debug, Default)]
pub(crate) struct LaneHold {
    tied: Vec<Tied>,
    /// The main track's clips before the edit (a clip that is new afterwards is a piece of one).
    main: HashSet<u64>,
}

/// Timeline time at which main clip `c` plays media time `m` (`None` outside it).
fn timeline_of(c: &TrackItem, m: Tick) -> Option<Tick> {
    if c.frame_hold.is_some() || m < c.source_in || m >= c.source_out() {
        return None;
    }
    let speed = c.speed.abs().max(1e-6);
    let d = if c.reverse { c.source_out().0 - 1 - m.0 } else { m.0 - c.source_in.0 };
    Some(Tick(c.start.0.saturating_add((d as f64 / speed).round() as i64)))
}

/// The lane clips with what they start on (the main track's own sound and locked lanes aside).
fn tie(q: &Sequence) -> Vec<Tied> {
    let Some(main) = q.video_tracks.first() else { return Vec::new() };
    let frame = q.settings.frame_rate.frame_duration();
    let links = main_links(q);
    let mut out = Vec::new();
    for (lane, tracks) in [(Lane::Video, &q.video_tracks), (Lane::Audio, &q.audio_tracks)] {
        for (index, t) in tracks.iter().enumerate().skip(1) {
            if t.locked {
                continue;
            }
            for i in &t.items {
                if lane == Lane::Audio && is_main_sound(q, &links, i) {
                    continue;
                }
                let Some(on) = main.items.iter().find(|c| c.start <= i.start && i.start < c.end()) else { continue };
                out.push(Tied {
                    clip: i.id.0,
                    track: t.id,
                    lane,
                    index,
                    start: i.start,
                    duration: i.duration,
                    moment: on.source_time_at(i.start),
                    at_cut: i.start - on.start <= frame || on.frame_hold.is_some(),
                    spans: i.end() > on.end() + frame,
                    on: on.clone(),
                });
            }
        }
    }
    out
}

/// Where lane clip `t` goes now: `Some(start)`, or `None` when its footage is gone (it goes
/// too). A clip that stays where it is comes back with its own start.
fn place(q: &Sequence, t: &Tied, before: &HashSet<u64>) -> Option<Tick> {
    let main: &[TrackItem] = q.video_tracks.first().map_or(&[], |m| m.items.as_slice());
    let same = main.iter().find(|c| c.id == t.on.id);
    // pieces of it (a split) or copies the edit made
    let fresh = || main.iter().filter(|c| !before.contains(&c.id.0) && c.item == t.on.item);
    let found = if t.at_cut {
        same.map(|c| c.start + if t.on.frame_hold.is_some() { (t.start - t.on.start).min(c.duration) } else { Tick::ZERO })
            .or_else(|| fresh().find(|c| c.source_in == t.on.source_in).map(|c| c.start))
    } else {
        same.and_then(|c| timeline_of(c, t.moment))
            .or_else(|| fresh().filter_map(|c| timeline_of(c, t.moment)).min_by_key(|s| (s.0 - t.start.0).unsigned_abs()))
    };
    if found.is_some() {
        return found;
    }
    // moved off the main track (onto a lane): what was on it stays put
    let elsewhere = same.is_none() && q.all_tracks().any(|tr| tr.items.iter().any(|i| i.id == t.on.id));
    if elsewhere || t.spans { Some(t.start) } else { None }
}

impl Studio {
    /// Whether lane clips follow the main track (on unless switched off).
    pub fn linkage(&self) -> bool {
        self.linkage
    }

    /// Turns linkage on or off (the interface remembers the choice).
    pub fn set_linkage(&mut self, on: bool) {
        self.linkage = on;
    }

    /// The lane clips and the footage each starts on, before an edit of the main track; nothing
    /// when linkage is off.
    pub(crate) fn hold_lanes(&self) -> Option<LaneHold> {
        if !self.linkage {
            return None;
        }
        let q = self.session.active_sequence()?;
        let tied = tie(q);
        if tied.is_empty() {
            return None;
        }
        let main = q.video_tracks.first().map(|t| t.items.iter().map(|i| i.id.0).collect()).unwrap_or_default();
        Some(LaneHold { tied, main })
    }

    /// Puts held lane clips back over their footage as the main track now plays it, and takes
    /// away those whose footage is gone. Part of [`Studio::group_follow`].
    pub(crate) fn place_lanes(&mut self, hold: LaneHold) -> Result<()> {
        let q = self.session.active_sequence().ok_or(crate::Error::NothingOpen)?;
        let rate = q.settings.frame_rate;
        let mut moves: Vec<(&Tied, Tick)> = Vec::new();
        let mut gone: Vec<u64> = Vec::new();
        for t in &hold.tied {
            // a clip the edit itself moved, or took away, is as the edit left it
            let Some((track, now)) = q.find_item(filmcraft_project::ClipId(t.clip)) else { continue };
            if track != t.track || now.start != t.start {
                continue;
            }
            match place(q, t, &hold.main) {
                Some(at) => {
                    let at = rate.snap(at).max(Tick::ZERO);
                    if at != t.start {
                        moves.push((t, at));
                    }
                }
                None => gone.push(t.clip),
            }
        }
        if moves.is_empty() && gone.is_empty() {
            return Ok(());
        }
        // each to its own lane where that is free (of clips staying and clips already placed),
        // else to the next free lane of its kind, else a new one
        let moving: Vec<u64> = moves.iter().map(|(t, _)| t.clip).chain(gone.iter().copied()).collect();
        moves.sort_by_key(|(_, at)| *at);
        let mut placed: Vec<(Lane, usize, Tick, Tick)> = Vec::new();
        let mut out = Vec::new();
        for (t, at) in &moves {
            let end = *at + t.duration;
            let clear = |s: &Self, k: usize| {
                s.session.active_sequence().is_some_and(|q| is_free(q, t.lane, k, *at, end, &moving))
                    && !placed.iter().any(|(l, i, a, b)| *l == t.lane && *i == k && *a < end && *b > *at)
            };
            let n = self.session.active_sequence().map_or(0, |q| if t.lane == Lane::Video { q.video_tracks.len() } else { q.audio_tracks.len() });
            let dest = match std::iter::once(t.index).chain(1..n).find(|k| clear(self, *k)) {
                Some(k) => k,
                None => self.add_lane(t.lane)?,
            };
            placed.push((t.lane, dest, *at, end));
            out.push(json!({"clip": t.clip, "track": lane_name(t.lane, dest), "time": at.0}));
        }
        self.unlock_lanes()?;
        if !gone.is_empty() {
            self.session.execute("edit.clear", json!({"clips": gone})).map_err(engine)?;
        }
        if !out.is_empty() {
            self.session.execute("timeline.move", json!({"moves": out, "linked": false})).map_err(engine)?;
        }
        Ok(())
    }

    /// Runs `f` with linkage off (an edit that changes what a clip shows, not where it is).
    pub(crate) fn without_linkage<R>(&mut self, f: impl FnOnce(&mut Self) -> Result<R>) -> Result<R> {
        let was = std::mem::replace(&mut self.linkage, false);
        let r = f(self);
        self.linkage = was;
        r
    }
}
