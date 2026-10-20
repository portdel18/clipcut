//! The lanes around the magnetic main track, CapCut-style: overlay video, pictures and text above
//! it (video tracks 2 and up), music and voiceover below it (audio tracks 2 and up). Clips on lanes
//! sit where they are put: nothing closes up around them, and the main track's edits don't move
//! them (lanes are never sync-locked).
//!
//! A clip never lands on top of another one: when its place on a lane is taken it goes to the next
//! free lane of its kind, and a new lane is added when none is free.

use filmcraft_project::{ClipId, ItemId, Sequence, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{Error, Result, Studio, engine};

/// Which lanes a clip can go on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Video,
    Audio,
}

/// Lane name ("V2", "A3") for a track index (0 = the main track's).
pub(crate) fn lane_name(lane: Lane, index: usize) -> String {
    format!("{}{}", if lane == Lane::Video { 'V' } else { 'A' }, index + 1)
}

/// A lane name back to its kind and index; `None` for anything else.
pub(crate) fn parse_lane(name: &str) -> Option<(Lane, usize)> {
    let mut c = name.chars();
    let lane = match c.next()?.to_ascii_uppercase() {
        'V' => Lane::Video,
        'A' => Lane::Audio,
        _ => return None,
    };
    let n: usize = c.as_str().parse().ok()?;
    n.checked_sub(1).map(|i| (lane, i))
}

/// The links of the main track's pictures: sound with one of them belongs to the main track, on
/// whichever sound track it sits (Extract Audio keeps a main clip's sound linked on a lane).
pub(crate) fn main_links(q: &Sequence) -> std::collections::HashSet<u64> {
    q.video_tracks.first().map(|t| t.items.iter().filter_map(|i| i.link).collect()).unwrap_or_default()
}

/// Whether sound clip `c` is the main track's: on its sound track, or linked to one of its pictures.
pub(crate) fn is_main_sound(q: &Sequence, links: &std::collections::HashSet<u64>, c: &filmcraft_project::TrackItem) -> bool {
    q.audio_tracks.first().is_some_and(|t| t.items.iter().any(|i| i.id == c.id)) || c.link.is_some_and(|l| links.contains(&l))
}

/// The timeline time at which sound `a` (of the same media as picture `v`) is in step with it.
fn in_step(v: &filmcraft_project::TrackItem, a: &filmcraft_project::TrackItem) -> Tick {
    let speed = v.speed.abs().max(1e-6);
    Tick(v.start.0.saturating_add(((a.source_in.0 - v.source_in.0) as f64 / speed).round() as i64))
}

fn tracks(q: &Sequence, lane: Lane) -> &[filmcraft_project::Track] {
    if lane == Lane::Video { &q.video_tracks } else { &q.audio_tracks }
}

/// Whether `[start, end)` is clear on track `index` of `lane`, ignoring the clips in `except`.
fn is_free(q: &Sequence, lane: Lane, index: usize, start: Tick, end: Tick, except: &[u64]) -> bool {
    tracks(q, lane).get(index).is_some_and(|t| !t.locked && t.items.iter().all(|i| except.contains(&i.id.0) || i.end() <= start || i.start >= end))
}

impl Studio {
    /// Adds a file the way dropping it on the window does: a video or picture to the end of the
    /// main track, sound to a sound lane at `seconds`. Returns the media item.
    pub fn add_file(&mut self, path: &str, seconds: f64) -> Result<u64> {
        let item = self.import(path)?;
        let picture = self.session.project.item(ItemId(item)).is_some_and(|i| i.has_video());
        if picture {
            self.append_item(item)?;
        } else if self.session.active_sequence().is_none() {
            // a voice-over or a song first: a 16:9 project starts with it on a sound lane
            self.start_with_sound(item)?;
        } else {
            self.add_to_lane(item, seconds)?;
        }
        Ok(item)
    }

    /// Starts the project with a sound-only item: a 1920 × 1080, 30 fps sequence with the sound
    /// on the first sound lane at the start. Pictures, titles and graphics go over it.
    pub fn start_with_sound(&mut self, item: u64) -> Result<Vec<u64>> {
        let it = self.session.project.item(ItemId(item)).ok_or_else(|| Error::Invalid("that media is no longer in the project".into()))?;
        if !it.has_audio() {
            return Err(Error::Invalid(format!("{} has no picture or sound", it.name)));
        }
        let name = std::path::Path::new(&it.name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| it.name.clone());
        self.session.execute("file.newSequence", json!({"name": name, "width": 1920, "height": 1080, "fps": 30.0, "sampleRate": 48000})).map_err(engine)?;
        self.add_to_lane(item, 0.0)
    }

    /// Adds media to a lane at `seconds`: a picture (video or still) on the first free overlay lane,
    /// its sound (if any) on the first free sound lane; a sound-only file on the first free sound
    /// lane. One undo step. Returns the new clips.
    pub fn add_to_lane(&mut self, item: u64, seconds: f64) -> Result<Vec<u64>> {
        let t = self.frame_tick(seconds)?;
        self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let it = self.session.project.item(ItemId(item)).ok_or_else(|| Error::Invalid("that media is no longer in the project".into()))?;
        let (video, audio) = (it.has_video(), it.has_audio());
        if !video && !audio {
            return Err(Error::Invalid(format!("{} has no picture or sound", it.name)));
        }
        let still = self.session.project.resolve_media(ItemId(item)).is_some_and(|(_, m, _)| m.info.kind == filmcraft_media::MediaKind::Still);
        // a still is as long as the engine's still default; anything else plays to its end
        let len = if still { None } else { Some(it.duration()) };
        self.group(if video { "Add Overlay" } else { "Add Sound" }, |s| {
            s.unlock_lanes()?;
            let span = len.unwrap_or(Tick::from_seconds_f64(5.0));
            let mut p = json!({"item": item, "time": t.0});
            if video {
                let v = s.free_lane(Lane::Video, t, t + span, &[])?;
                p["track"] = json!(lane_name(Lane::Video, v));
                if audio {
                    let a = s.free_lane(Lane::Audio, t, t + span, &[])?;
                    p["audioTrack"] = json!(lane_name(Lane::Audio, a));
                } else {
                    p["audio"] = json!(false);
                }
                if still {
                    p["duration"] = json!(span.0);
                }
            } else {
                let a = s.free_lane(Lane::Audio, t, t + span, &[])?;
                p["track"] = json!(lane_name(Lane::Audio, a));
            }
            let r = s.session.execute("timeline.place", p).map_err(engine)?;
            Ok(r.get("clips").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default())
        })
    }

    /// Adds a text clip at `seconds`, `duration` long, on the first free overlay lane, styled by
    /// `style` (the same keys as [`Studio::set_props`]'s text ones). One undo step.
    pub fn add_text(&mut self, seconds: f64, duration: f64, text: &str, style: &Value) -> Result<u64> {
        let t = self.frame_tick(seconds)?;
        let h = self.session.active_sequence().ok_or(Error::NothingOpen)?.settings.height;
        if !duration.is_finite() || duration <= 0.0 {
            return Err(Error::Invalid("the text needs a length".into()));
        }
        let text = if text.trim().is_empty() { "Add text" } else { text };
        self.group("Add Text", |s| {
            s.unlock_lanes()?;
            let lane = s.free_lane(Lane::Video, t, t + Tick::from_seconds_f64(duration), &[])?;
            // about a tenth of the frame's height: readable on a phone, as CapCut starts
            let size = (f64::from(h) * 0.075).round().max(12.0);
            let r =
                s.session.execute("graphics.newText", json!({"text": text, "size": size, "time": t.0, "seconds": duration, "track": lane})).map_err(engine)?;
            let clip = r.get("clip").and_then(Value::as_u64).ok_or_else(|| Error::Engine("the text was not made".into()))?;
            s.session.execute("graphics.set", json!({"clip": clip, "layer": 0, "props": {"align": "center"}})).map_err(engine)?;
            if style.as_object().is_some_and(|o| !o.is_empty()) {
                s.apply_props(&[clip], style)?;
            }
            Ok(clip)
        })
    }

    /// Moves a lane clip to `lane` ("V2", "A3"…) at `seconds`, with its linked partner keeping its
    /// offset. A place that is taken sends the clip (or its partner) to the next free lane of its
    /// kind. One undo step.
    pub fn move_clip(&mut self, clip: u64, lane: &str, seconds: f64) -> Result<()> {
        let t = self.frame_tick(seconds)?;
        let (want, index) = parse_lane(lane).ok_or_else(|| Error::Invalid(format!("{lane} is not a lane")))?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (track, item) =
            q.find_item(ClipId(clip)).map(|(tr, i)| (tr, i.clone())).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let kind = q.track(track).map(|t| t.kind).ok_or(Error::NothingOpen)?;
        let own = if kind == TrackKind::Video { Lane::Video } else { Lane::Audio };
        let own_index = tracks(q, own).iter().position(|t| t.id == track).unwrap_or(0);
        if own_index == 0 {
            return Err(Error::Invalid("clips on the main track move by dragging them along it".into()));
        }
        if want != own || index == 0 {
            return Err(Error::Invalid(
                if own == Lane::Video { "pictures and text go on the lanes above the main track" } else { "sound goes on the lanes below the main track" }
                    .into(),
            ));
        }
        // the main track's sound on a lane keeps time with its picture: it only changes lane
        let main_link = item.link.is_some_and(|l| main_links(q).contains(&l));
        let shift = if main_link { Tick::ZERO } else { Tick(t.0 - item.start.0) };
        let partners: Vec<_> = item
            .link
            .map(|l| {
                q.all_tracks()
                    .flat_map(|tr| tr.items.iter().filter(move |i| i.link == Some(l) && i.id.0 != clip).map(move |i| (tr.id, i.clone())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|_| !main_link)
            .collect();
        let ids: Vec<u64> = std::iter::once(clip).chain(partners.iter().map(|(_, i)| i.id.0)).collect();
        // nothing may start before 0: the whole set stops there
        let earliest = partners.iter().map(|(_, i)| i.start).chain([item.start]).min().unwrap_or(item.start);
        let shift = if earliest.0 + shift.0 < 0 { Tick(-earliest.0) } else { shift };
        self.group("Move", |s| {
            s.unlock_lanes()?;
            let mut moves = Vec::new();
            let start = item.start + shift;
            let dest = if s.lane_free(own, index, start, start + item.duration, &ids) { index } else { s.free_lane(own, start, start + item.duration, &ids)? };
            moves.push(json!({"clip": clip, "track": lane_name(own, dest), "time": start.0}));
            for (ptrack, p) in &partners {
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let pk = q.track(*ptrack).map(|t| t.kind).unwrap_or(TrackKind::Audio);
                let pl = if pk == TrackKind::Video { Lane::Video } else { Lane::Audio };
                let pi = tracks(q, pl).iter().position(|t| t.id == *ptrack).unwrap_or(0);
                let pstart = p.start + shift;
                let pdest =
                    if pi > 0 && s.lane_free(pl, pi, pstart, pstart + p.duration, &ids) { pi } else { s.free_lane(pl, pstart, pstart + p.duration, &ids)? };
                moves.push(json!({"clip": p.id.0, "track": lane_name(pl, pdest), "time": pstart.0}));
            }
            s.session.execute("timeline.move", json!({"moves": moves, "linked": false})).map_err(engine)?;
            Ok(())
        })
    }

    /// Moves the lane clips `clips` (a box selection) together by `seconds` along the timeline,
    /// each on its own lane where it is clear there; their linked partners go with them. Clips on
    /// the main track and the main track's sound stay put. One undo step.
    pub fn move_clips(&mut self, clips: &[u64], seconds: f64) -> Result<()> {
        if !seconds.is_finite() {
            return Err(Error::Invalid("that is not a time".into()));
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mains = main_links(q);
        let mut links = std::collections::HashSet::new();
        let mut set = Vec::new();
        for &c in clips {
            let Some((track, item)) = q.find_item(ClipId(c)) else { continue };
            let Some(kind) = q.track(track).map(|t| t.kind) else { continue };
            let lane = if kind == TrackKind::Video { Lane::Video } else { Lane::Audio };
            let index = tracks(q, lane).iter().position(|t| t.id == track).unwrap_or(0);
            if index == 0 || item.link.is_some_and(|l| mains.contains(&l)) {
                continue;
            }
            // a linked pair moves once: move_clip takes the partner along
            if let Some(l) = item.link
                && !links.insert(l)
            {
                continue;
            }
            set.push((c, lane_name(lane, index), item.start));
        }
        if set.is_empty() {
            return Err(Error::Invalid("select clips on the lanes to move them together".into()));
        }
        let earliest = set.iter().map(|(_, _, t)| *t).min().unwrap_or(Tick::ZERO);
        let shift = seconds.max(-earliest.seconds());
        // the clip furthest along the way goes first, so none lands on one that has yet to move
        set.sort_by_key(|(_, _, t)| *t);
        if shift > 0.0 {
            set.reverse();
        }
        self.group("Move", |s| {
            for (c, lane, start) in &set {
                s.move_clip(*c, lane, start.seconds() + shift)?;
            }
            Ok(())
        })
    }

    /// Extract Audio, CapCut-style: clip `clip`'s own sound (or `clip` itself, when it is that
    /// sound) moves from the main track's sound to a sound lane at the same time, to see and fade
    /// it on its own. It stays linked to the picture, so the main track's edits (cuts, trims,
    /// moves, the transcript) still take both; [`Studio::unlink_sound`] parts them. Returns the
    /// sound clip. One undo step.
    pub fn extract_audio(&mut self, clip: u64) -> Result<u64> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (track, item) =
            q.find_item(ClipId(clip)).map(|(tr, i)| (tr, i.clone())).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let is_sound = q.track(track).is_some_and(|t| t.kind == TrackKind::Audio);
        let sound = if is_sound {
            item.clone()
        } else {
            let mut own: Vec<_> =
                item.link.map(|l| q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.link == Some(l)).cloned().collect()).unwrap_or_default();
            own.sort_by_key(|i| i.start);
            own.into_iter().next().ok_or_else(|| Error::Invalid("this clip has no sound of its own to extract".into()))?
        };
        let on_main = q.audio_tracks.first().is_some_and(|t| t.items.iter().any(|i| i.id == sound.id));
        if !on_main {
            let why = if sound.link.is_none() { "this sound is already a clip of its own" } else { "this sound is already on a sound lane of its own" };
            return Err(Error::Invalid(why.into()));
        }
        self.group_follow("Extract Audio", None, |s| {
            s.unlock_lanes()?;
            let dest = s.free_lane(Lane::Audio, sound.start, sound.end(), &[])?;
            s.session
                .execute(
                    "timeline.move",
                    json!({"moves": [{"clip": sound.id.0, "track": lane_name(Lane::Audio, dest), "time": sound.start.0}], "linked": false}),
                )
                .map_err(engine)?;
            Ok(sound.id.0)
        })
    }

    /// Parts clip `clip` (a picture or its sound) from the clips linked to it: each is then
    /// trimmed, moved and cut on its own, and sound on a lane leaves the transcript. One undo step.
    pub fn unlink_sound(&mut self, clip: u64) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, item) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let link = item.link.ok_or_else(|| Error::Invalid("this clip isn't linked to another".into()))?;
        self.group("Unlink", |s| {
            s.session
                .edit_sequence("Unlink", |q, _, _| {
                    for t in q.video_tracks.iter_mut().chain(q.audio_tracks.iter_mut()) {
                        for i in t.items.iter_mut().filter(|i| i.link == Some(link)) {
                            i.link = None;
                        }
                    }
                    Ok(())
                })
                .map_err(engine)
        })
    }

    /// The picture sound clip `clip` (on a lane, linked to nothing) can be linked back to: one of
    /// the same media, in step with it, with no sound of its own.
    pub(crate) fn picture_for(&self, clip: u64) -> Option<u64> {
        let q = self.session.active_sequence()?;
        let (track, a) = q.find_item(ClipId(clip))?;
        if q.track(track)?.kind != TrackKind::Audio || a.link.is_some() || q.audio_tracks.first().is_some_and(|t| t.id == track) {
            return None;
        }
        let half = Tick(q.settings.frame_rate.frame_duration().0 / 2);
        let sound_of = |l: Option<u64>| l.is_some_and(|l| q.audio_tracks.iter().flat_map(|t| &t.items).any(|i| i.link == Some(l)));
        q.video_tracks
            .iter()
            .flat_map(|t| t.items.iter())
            .find(|v| v.item == a.item && !sound_of(v.link) && (in_step(v, a).0 - a.start.0).abs() <= half.0 && v.start < a.end() && a.start < v.end())
            .map(|v| v.id.0)
    }

    /// Links sound clip `clip` back to its picture ([`Studio::picture_for`]), as Extract Audio
    /// leaves them. One undo step.
    pub fn link_sound(&mut self, clip: u64) -> Result<()> {
        let picture = self.picture_for(clip).ok_or_else(|| Error::Invalid("there is no picture of this sound in step with it to link it to".into()))?;
        self.group_follow("Link", None, |s| {
            s.session
                .edit_sequence("Link", |q, ctx, _| {
                    let old = q.find_item(ClipId(picture)).and_then(|(_, v)| v.link);
                    let l = old.unwrap_or_else(|| ctx.alloc());
                    for id in [picture, clip] {
                        if let Some((_, i)) = q.find_item_mut(ClipId(id)) {
                            i.link = Some(l);
                        }
                    }
                    Ok(())
                })
                .map_err(engine)
        })
    }

    /// Puts the main track's sound that sits on lanes back in step with its pictures, after an
    /// edit of the main track moved them (lanes don't ripple). Part of [`Studio::group_follow`].
    pub(crate) fn keep_sound_in_step(&mut self) -> Result<()> {
        let Some(q) = self.session.active_sequence() else { return Ok(()) };
        let Some(main) = q.video_tracks.first() else { return Ok(()) };
        let mut moves = Vec::new();
        for v in main.items.iter() {
            let Some(l) = v.link else { continue };
            for (k, t) in q.audio_tracks.iter().enumerate().skip(1) {
                for a in t.items.iter().filter(|a| a.link == Some(l) && a.item == v.item) {
                    let at = in_step(v, a);
                    if at != a.start {
                        moves.push((a.id.0, k, at, a.duration));
                    }
                }
            }
        }
        if moves.is_empty() {
            return Ok(());
        }
        let ids: Vec<u64> = moves.iter().map(|m| m.0).collect();
        let mut out = Vec::new();
        for (id, k, at, d) in moves {
            let at = Tick(at.0.max(0));
            let dest = if self.lane_free(Lane::Audio, k, at, at + d, &ids) { k } else { self.free_lane(Lane::Audio, at, at + d, &ids)? };
            out.push(json!({"clip": id, "track": lane_name(Lane::Audio, dest), "time": at.0}));
        }
        self.unlock_lanes()?;
        self.session.execute("timeline.move", json!({"moves": out, "linked": false})).map_err(engine)?;
        Ok(())
    }

    fn lane_free(&self, lane: Lane, index: usize, start: Tick, end: Tick, except: &[u64]) -> bool {
        self.session.active_sequence().is_some_and(|q| is_free(q, lane, index, start, end, except))
    }

    /// The first lane (never the main track's) where `[start, end)` is clear, adding one when
    /// every lane is taken.
    pub(crate) fn free_lane(&mut self, lane: Lane, start: Tick, end: Tick, except: &[u64]) -> Result<usize> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let n = tracks(q, lane).len();
        if let Some(i) = (1..n).find(|i| is_free(q, lane, *i, start, end, except)) {
            return Ok(i);
        }
        let added = self
            .session
            .execute("sequence.addTracks", if lane == Lane::Video { json!({"video": 1, "audio": 0}) } else { json!({"video": 0, "audio": 1}) })
            .map_err(engine)?;
        let key = if lane == Lane::Video { "video" } else { "audio" };
        let id = added
            .get(key)
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Engine("no lane was added".into()))?;
        self.session.execute("timeline.setTrack", json!({"track": id, "syncLock": false})).map_err(engine)?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        tracks(q, lane).iter().position(|t| t.id.0 == id).ok_or_else(|| Error::Engine("the new lane went missing".into()))
    }

    /// Lanes never follow the main track's ripple edits (CapCut keeps overlays, text and music
    /// where they are). New sequences come with sync-locked tracks; this turns that off.
    pub(crate) fn unlock_lanes(&mut self) -> Result<()> {
        let Some(q) = self.session.active_sequence() else { return Ok(()) };
        let locked: Vec<u64> = q.video_tracks.iter().skip(1).chain(q.audio_tracks.iter().skip(1)).filter(|t| t.sync_lock).map(|t| t.id.0).collect();
        for id in locked {
            self.session.execute("timeline.setTrack", json!({"track": id, "syncLock": false})).map_err(engine)?;
        }
        Ok(())
    }

    /// Whether a clip is on the main track (or is the main track's sound, on a lane too).
    pub(crate) fn on_main(&self, clip: u64) -> bool {
        let Some(q) = self.session.active_sequence() else { return false };
        if q.video_tracks.first().is_some_and(|t| t.items.iter().any(|i| i.id.0 == clip)) {
            return true;
        }
        let links = main_links(q);
        q.audio_tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id.0 == clip).is_some_and(|i| is_main_sound(q, &links, i))
    }

    /// Mutes a lane's sound or hides its picture (`on` false), or brings it back.
    pub fn set_lane(&mut self, lane: &str, on: bool) -> Result<()> {
        let (kind, index) = parse_lane(lane).ok_or_else(|| Error::Invalid(format!("{lane} is not a lane")))?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let id = tracks(q, kind).get(index).map(|t| t.id.0).ok_or_else(|| Error::Invalid(format!("there is no lane {lane}")))?;
        let p = if kind == Lane::Video { json!({"track": id, "enabled": on}) } else { json!({"track": id, "muted": !on}) };
        let label = match (kind, on) {
            (Lane::Video, true) => "Show Lane",
            (Lane::Video, false) => "Hide Lane",
            (Lane::Audio, true) => "Unmute",
            (Lane::Audio, false) => "Mute",
        };
        self.group(label, |s| s.session.execute("timeline.setTrack", p).map(|_| ()).map_err(engine))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_names_round_trip() {
        assert_eq!(lane_name(Lane::Video, 1), "V2");
        assert_eq!(parse_lane("a3"), Some((Lane::Audio, 2)));
        assert_eq!(parse_lane("V0"), None);
        assert_eq!(parse_lane("X1"), None);
        assert_eq!(parse_lane(""), None);
        assert_eq!(parse_lane("V"), None);
    }
}
