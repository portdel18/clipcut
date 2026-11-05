//! Markers with notes: a mark on the timeline at a moment, with a name, notes and a colour (for
//! "fix the colour here", "cut to the wide shot", a chapter of the video). They are the engine's
//! sequence markers. Like the lanes' clips, a marker over the main track follows that footage
//! through edits that move it ([`Studio::group_follow`]); when its footage is gone it stays where
//! it was, so no note is ever lost. Named markers also give YouTube chapters ([`Studio::chapters`]).

use filmcraft_project::{HexColor, Label, Marker, MarkerId, MarkerKind, Sequence};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::Value;

use crate::{Error, Result, Studio, engine};

/// Longest name and notes kept (characters).
const MOST_NAME: usize = 200;
const MOST_NOTES: usize = 10_000;
/// Most markers in a video.
const MOST_MARKERS: usize = 10_000;
/// A new marker's colour.
pub const MARKER_COLOUR: &str = "#4d9cff";

/// A marker as the interface shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerView {
    pub id: u64,
    /// Seconds on the timeline.
    pub time: f64,
    pub name: String,
    pub notes: String,
    /// `#rrggbb`.
    pub colour: String,
}

/// The sequence's markers, in time order.
pub(crate) fn markers_of(q: &Sequence) -> Vec<MarkerView> {
    let mut v: Vec<MarkerView> = q
        .markers
        .iter()
        .map(|m| MarkerView {
            id: m.id.0,
            time: m.start.seconds(),
            name: m.name.clone(),
            notes: m.comment.clone(),
            colour: HexColor(m.rgb()).hex().to_lowercase(),
        })
        .collect();
    v.sort_by(|a, b| a.time.total_cmp(&b.time));
    v
}

fn clip_text(s: &str, most: usize) -> String {
    s.chars().take(most).collect()
}

fn colour(c: &str) -> Result<HexColor> {
    HexColor::parse(c).ok_or_else(|| Error::Invalid(format!("{c} isn't a colour: use #rrggbb")))
}

/// YouTube chapters from the named markers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapters {
    /// One line a chapter, `0:00 Name`, to paste into the video's description.
    pub text: String,
    /// What YouTube would refuse, in plain words (empty when it takes them).
    pub problems: Vec<String>,
}

/// `m:ss` or `h:mm:ss`, as YouTube writes times.
fn stamp(secs: f64) -> String {
    let s = secs.max(0.0).floor() as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// A marker over the main track before an edit: the footage under it.
#[derive(Clone, Debug)]
pub(crate) struct Tie {
    marker: MarkerId,
    start: Tick,
    /// The main clip under it, and that clip as it was.
    on: filmcraft_project::TrackItem,
    /// The moment of the clip's media under it.
    moment: Tick,
    /// It sits on the clip's first frame: it stays at that cut.
    at_cut: bool,
}

impl Studio {
    /// Adds a marker at `seconds` (on the frame grid) with `name`, `notes` and `colour`
    /// (`#rrggbb`; blue by default). One undo step. Returns its id.
    pub fn add_marker(&mut self, seconds: f64, name: &str, notes: &str, colour_hex: Option<&str>) -> Result<u64> {
        let t = self.frame_tick(seconds)?;
        let c = colour(colour_hex.unwrap_or(MARKER_COLOUR))?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        if q.markers.len() >= MOST_MARKERS {
            return Err(Error::Invalid(format!("a video has at most {MOST_MARKERS} markers")));
        }
        let (name, notes) = (clip_text(name.trim(), MOST_NAME), clip_text(notes, MOST_NOTES));
        let id = self
            .session
            .edit_sequence("Add Marker", |q, ctx, _| {
                let id = MarkerId(ctx.alloc());
                q.markers.push(Marker {
                    url: String::new(),
                    author: String::new(),
                    custom_color: Some(c),
                    id,
                    start: t,
                    duration: Tick::ZERO,
                    name,
                    comment: notes,
                    kind: MarkerKind::Comment,
                    color: Label::Blue,
                });
                q.markers.sort_by_key(|m| m.start);
                Ok(id)
            })
            .map_err(engine)?;
        Ok(id.0)
    }

    /// Changes marker `id`: any of `name`, `notes`, `colour` (`#rrggbb`) and `seconds` (moves it).
    /// One undo step; calls with the same `merge` key (typing in its notes) make one step.
    pub fn edit_marker(&mut self, id: u64, p: &Value, merge: Option<&str>) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        if !q.markers.iter().any(|m| m.id.0 == id) {
            return Err(Error::Invalid("that marker is no longer on the timeline".into()));
        }
        let text = |k: &str| p.get(k).map(|v| v.as_str().map(str::to_string).ok_or_else(|| Error::Invalid(format!("{k} must be text")))).transpose();
        let name = text("name")?.map(|n| clip_text(n.trim(), MOST_NAME));
        let notes = text("notes")?.map(|n| clip_text(&n, MOST_NOTES));
        let c = text("colour")?.map(|c| colour(&c)).transpose()?;
        let at = match p.get("seconds") {
            Some(v) => Some(self.frame_tick(v.as_f64().ok_or_else(|| Error::Invalid("seconds must be a number".into()))?)?),
            None => None,
        };
        self.group_merge("Edit Marker", merge, |s| {
            s.session
                .edit_sequence("Edit Marker", |q, _, _| {
                    if let Some(m) = q.markers.iter_mut().find(|m| m.id.0 == id) {
                        if let Some(n) = name {
                            m.name = n;
                        }
                        if let Some(n) = notes {
                            m.comment = n;
                        }
                        if let Some(c) = c {
                            m.custom_color = Some(c);
                        }
                        if let Some(t) = at {
                            m.start = t;
                        }
                    }
                    q.markers.sort_by_key(|m| m.start);
                    Ok(())
                })
                .map_err(engine)
        })
    }

    /// Takes marker `id` away. One undo step.
    pub fn delete_marker(&mut self, id: u64) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        if !q.markers.iter().any(|m| m.id.0 == id) {
            return Err(Error::Invalid("that marker is no longer on the timeline".into()));
        }
        self.session
            .edit_sequence("Delete Marker", |q, _, _| {
                q.markers.retain(|m| m.id.0 != id);
                Ok(())
            })
            .map_err(engine)
    }

    /// YouTube chapters from the markers that have a name, with what YouTube would refuse
    /// (the first must be at 0:00, three or more, each 10 seconds or longer).
    pub fn chapters(&self) -> Result<Chapters> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let named: Vec<MarkerView> = markers_of(q).into_iter().filter(|m| !m.name.trim().is_empty()).collect();
        let mut problems = Vec::new();
        if named.is_empty() {
            problems.push("Give markers a name: each named marker is a chapter".into());
        } else {
            if named.first().is_some_and(|m| m.time >= 1.0) {
                problems.push("The first chapter must start at 0:00: put a named marker at the start".into());
            }
            if named.len() < 3 {
                problems.push(format!("YouTube needs three chapters or more; there {} {}", if named.len() == 1 { "is" } else { "are" }, named.len()));
            }
            let end = q.duration().seconds();
            let short: Vec<String> = named
                .iter()
                .enumerate()
                .filter(|(k, m)| named.get(k + 1).map_or(end, |n| n.time) - m.time < 10.0)
                .map(|(_, m)| format!("“{}”", m.name.trim()))
                .collect();
            if !short.is_empty() {
                problems.push(format!("Chapters must be 10 seconds or longer: {}", short.join(", ")));
            }
        }
        let text = named.iter().map(|m| format!("{} {}", stamp(m.time), m.name.trim().replace('\n', " "))).collect::<Vec<_>>().join("\n");
        Ok(Chapters { text, problems })
    }

    /// The markers over the main track and the footage under each, before an edit.
    pub(crate) fn hold_markers(&self) -> Vec<Tie> {
        let Some(q) = self.session.active_sequence() else { return Vec::new() };
        let Some(main) = q.video_tracks.first() else { return Vec::new() };
        let frame = q.settings.frame_rate.frame_duration();
        q.markers
            .iter()
            .filter_map(|m| {
                let on = main.items.iter().find(|c| c.start <= m.start && m.start < c.end())?;
                Some(Tie {
                    marker: m.id,
                    start: m.start,
                    moment: on.source_time_at(m.start),
                    at_cut: m.start - on.start < frame || on.frame_hold.is_some(),
                    on: on.clone(),
                })
            })
            .collect()
    }

    /// Puts held markers back over their footage as the main track now plays it; a marker whose
    /// footage is gone, or that the edit itself moved, stays where it is. Inside an edit.
    pub(crate) fn place_markers(&mut self, held: Vec<Tie>) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let rate = q.settings.frame_rate;
        let main: &[filmcraft_project::TrackItem] = q.video_tracks.first().map_or(&[], |m| m.items.as_slice());
        let mut moves: Vec<(MarkerId, Tick)> = Vec::new();
        for t in &held {
            if !q.markers.iter().any(|m| m.id == t.marker && m.start == t.start) {
                continue;
            }
            let same = main.iter().find(|c| c.id == t.on.id);
            let pieces = || main.iter().filter(|c| c.id != t.on.id && c.item == t.on.item);
            let found = if t.at_cut {
                same.map(|c| c.start).or_else(|| pieces().find(|c| c.source_in == t.on.source_in).map(|c| c.start))
            } else {
                same.and_then(|c| crate::linkage::timeline_of(c, t.moment))
                    .or_else(|| pieces().filter_map(|c| crate::linkage::timeline_of(c, t.moment)).min_by_key(|s| (s.0 - t.start.0).unsigned_abs()))
            };
            if let Some(at) = found.map(|at| rate.snap(at).max(Tick::ZERO))
                && at != t.start
            {
                moves.push((t.marker, at));
            }
        }
        if moves.is_empty() {
            return Ok(());
        }
        self.session
            .edit_sequence("Markers Follow", |q, _, _| {
                for (id, at) in &moves {
                    if let Some(m) = q.markers.iter_mut().find(|m| m.id == *id) {
                        m.start = *at;
                    }
                }
                q.markers.sort_by_key(|m| m.start);
                Ok(())
            })
            .map_err(engine)
    }

    /// The clip under marker `id` on the main track (for tests and the interface).
    pub fn marker_clip(&self, id: u64) -> Option<u64> {
        let q = self.session.active_sequence()?;
        let m = q.markers.iter().find(|m| m.id.0 == id)?;
        q.video_tracks.first()?.items.iter().find(|c| c.start <= m.start && m.start < c.end()).map(|c| c.id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_stamps() {
        assert_eq!(stamp(0.0), "0:00");
        assert_eq!(stamp(83.9), "1:23");
        assert_eq!(stamp(3725.0), "1:02:05");
        assert_eq!(stamp(-4.0), "0:00");
    }

    #[test]
    fn names_and_colours_are_checked() {
        assert!(colour("#12ab34").is_ok());
        assert!(colour("blue").is_err());
        assert_eq!(clip_text(&"x".repeat(500), MOST_NAME).len(), MOST_NAME);
    }
}
