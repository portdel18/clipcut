//! Editing the open sequence the way ClipCut's timeline does: one **magnetic main track** (video
//! track 1 with its linked sound on audio track 1) whose clips always sit back to back from 0,
//! CapCut-style split, split-and-delete-left/right, ripple delete, ripple trims, reordering, and
//! undo/redo.
//!
//! Every action is one engine edit group (`Session::edit_group`), so it is one undo step and it
//! rolls back whole if any part fails. Times come in as seconds and are snapped to the sequence's
//! frames before they reach the engine, whose own commands (`timeline.razor`, `timeline.trim`,
//! `edit.rippleDelete`, `timeline.move`, `timeline.place`) do the editing.

use filmcraft_engine::Session;
use filmcraft_project::{ItemId, ItemKind, MediaRef};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::{Value, json};

use crate::props::{ClipProps, clip_props};
use crate::{Error, Result, Studio, engine};

/// A clip on the timeline, in seconds.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    pub id: u64,
    /// `V1`, `V2`… or `A1`, `A2`…
    pub track: String,
    pub start: f64,
    pub end: f64,
    /// Where the clip starts and ends in its media.
    pub source_in: f64,
    pub source_out: f64,
    /// The whole media's length: how far the clip's edges can be dragged out.
    pub media_duration: f64,
    pub name: String,
    /// Clips that move together (a video clip and its own sound) share this.
    pub link: Option<u64>,
    /// The media item the clip plays.
    pub item: u64,
    /// `video`, `image`, `text` or `audio`.
    pub kind: &'static str,
    pub props: ClipProps,
    /// The Clip Studio tool that made it (`"lt"`, `"mp"`…), for a graphic from the Graphics tab.
    pub graphic: Option<String>,
    /// Its keyframed properties.
    pub keys: Vec<crate::keyframes::KeyTrack>,
}

/// A track as a lane of the timeline: `V1` and `A1` are the main track's picture and sound.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneView {
    pub id: String,
    /// `video` or `audio`.
    pub kind: &'static str,
    /// False when the lane's picture is hidden or its sound muted.
    pub on: bool,
    pub clips: usize,
}

/// A media file in the project (the Media panel).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaView {
    pub item: u64,
    pub name: String,
    pub path: String,
    pub duration: f64,
    /// `video`, `audio` or `image`.
    pub kind: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Everything the timeline and Media panel draw, after any change.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineView {
    pub has_sequence: bool,
    pub fps: f64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub clips: Vec<ClipView>,
    /// Every track, top to bottom of their kind: video `V1`, `V2`…, then audio `A1`, `A2`….
    pub lanes: Vec<LaneView>,
    /// The caption track (`C1`) and how much of the main track's speech has a transcript.
    pub captions: crate::captions::CaptionsView,
    /// The transitions on the main track's cuts.
    pub transitions: Vec<crate::transitions::TransitionView>,
    pub media: Vec<MediaView>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// What Undo / Redo would do, for their tooltips ("Undo Split").
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub project: ProjectView,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub name: String,
    pub path: Option<String>,
    /// Changed since it was last saved.
    pub dirty: bool,
}

/// Which side Q and W keep when they split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Q: remove from the clip's start up to the playhead.
    Right,
    /// W: remove from the playhead to the clip's end.
    Left,
}

/// The folder beside a saved project that holds its render previews.
pub const PREVIEWS_DIR: &str = "ClipCut Previews";

fn secs(t: Tick) -> f64 {
    t.seconds()
}

impl Studio {
    pub(crate) fn fps(&self) -> Result<filmcraft_time::FrameRate> {
        Ok(self.session.active_sequence().ok_or(Error::NothingOpen)?.settings.frame_rate)
    }

    /// `seconds` on the sequence's frame grid, never negative.
    pub(crate) fn frame_tick(&self, seconds: f64) -> Result<Tick> {
        if !seconds.is_finite() {
            return Err(Error::Invalid("the time must be a number".into()));
        }
        let rate = self.fps()?;
        Ok(rate.snap(Tick::from_seconds_f64(seconds.max(0.0))))
    }

    /// The timeline and media, as the interface draws them.
    pub fn timeline(&self) -> TimelineView {
        let s = &self.session;
        let mut clips = Vec::new();
        let mut lanes = Vec::new();
        let (mut fps, mut duration, mut width, mut height) = (30.0, 0.0, 1920, 1080);
        if let Some(q) = s.active_sequence() {
            fps = q.settings.frame_rate.as_f64();
            duration = secs(q.duration());
            (width, height) = (q.settings.width, q.settings.height);
            for (prefix, tracks) in [("V", &q.video_tracks), ("A", &q.audio_tracks)] {
                for (n, t) in tracks.iter().enumerate() {
                    let on = if prefix == "V" { t.enabled } else { !t.muted };
                    lanes.push(LaneView { id: format!("{prefix}{}", n + 1), kind: if prefix == "V" { "video" } else { "audio" }, on, clips: t.items.len() });
                    for i in &t.items {
                        let (kind, props) = clip_props(s, q, t, i);
                        clips.push(ClipView {
                            id: i.id.0,
                            track: format!("{prefix}{}", n + 1),
                            start: secs(i.start),
                            end: secs(i.end()),
                            source_in: secs(i.source_in),
                            source_out: secs(i.source_out()),
                            media_duration: s.project.item(i.item).map(|m| secs(m.duration())).unwrap_or(0.0),
                            name: i.name.clone(),
                            link: i.link,
                            item: i.item.0,
                            kind,
                            props,
                            graphic: if prefix == "V" { self.graphic_tool(i.item) } else { None },
                            keys: crate::keyframes::keys_of(q, t, i),
                        });
                    }
                }
            }
        }
        let mut media: Vec<(u64, MediaView)> = s
            .project
            .items
            .values()
            .filter_map(|it| {
                let ItemKind::Media(m) = &it.kind else { return None };
                let MediaRef::File { path } = &m.media else { return None };
                let kind = match m.info.kind {
                    filmcraft_media::MediaKind::AudioOnly => "audio",
                    filmcraft_media::MediaKind::Still | filmcraft_media::MediaKind::ImageSequence => "image",
                    _ => "video",
                };
                let (w, h) = m.info.video.as_ref().map(|v| (v.width, v.height)).unwrap_or((0, 0));
                Some((
                    it.created,
                    MediaView { item: it.id.0, name: it.name.clone(), path: path.clone(), duration: secs(m.duration()), kind, width: w, height: h },
                ))
            })
            .collect();
        media.sort_by_key(|(created, m)| (*created, m.item));
        TimelineView {
            has_sequence: s.active_sequence().is_some(),
            fps,
            duration,
            width,
            height,
            clips,
            lanes,
            captions: self.captions_view(),
            transitions: self.transitions_view(),
            media: media.into_iter().map(|(_, m)| m).collect(),
            can_undo: s.history.can_undo(),
            can_redo: s.history.can_redo(),
            undo_label: s.history.undo.last().map(|h| h.0.clone()),
            redo_label: s.history.redo.last().map(|h| h.0.clone()),
            project: ProjectView { name: s.project.name.clone(), path: s.path.clone(), dirty: s.revision != s.saved_revision },
        }
    }

    /// Imports a media file into the project. The first clip makes the sequence (matching it);
    /// later ones go to the end of the main track. Returns the media item.
    pub fn add_clip(&mut self, path: &str) -> Result<u64> {
        let item = self.import(path)?;
        self.append_item(item)?;
        Ok(item)
    }

    /// Imports a media file into the project's media without putting it on the timeline.
    pub fn import(&mut self, path: &str) -> Result<u64> {
        if path.trim().is_empty() {
            return Err(Error::Invalid("choose a file to add".into()));
        }
        let r = self.session.execute("file.import", json!({"paths": [path]})).map_err(engine)?;
        let item = r.get("items").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_u64).ok_or_else(|| {
            let why = r.get("errors").and_then(Value::as_array).map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default();
            Error::Engine(if why.is_empty() { format!("{path} could not be opened") } else { why })
        })?;
        Ok(item)
    }

    /// Puts a media item at the end of the main track (making the sequence if there is none).
    pub fn append_item(&mut self, item: u64) -> Result<()> {
        let it = self.session.project.item(ItemId(item)).ok_or_else(|| Error::Invalid("that media is no longer in the project".into()))?;
        if !it.has_video() {
            return Err(Error::Invalid(format!("{} has no picture. Sound and music tracks arrive in the next update.", it.name)));
        }
        if self.session.active_sequence().is_none() {
            let name = self.session.project.item(ItemId(item)).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into());
            let name = std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name);
            self.session.execute("file.newSequence", json!({"fromItem": item, "name": name})).map_err(engine)?;
            return Ok(());
        }
        let end = self.main_end()?;
        self.session.execute("timeline.place", json!({"item": item, "track": "V1", "audioTrack": "A1", "time": end.0})).map_err(engine)?;
        Ok(())
    }

    /// Puts a media item on the main track at position `index` (0 = first; past the end appends).
    /// One undo step.
    pub fn insert_item(&mut self, item: u64, index: usize) -> Result<()> {
        self.group_follow("Add Clip", None, |s| {
            let before = s.main_order()?;
            s.append_item(item)?;
            let mut order = s.main_order()?;
            let Some(new) = order.iter().position(|c| !before.contains(c)) else { return Ok(()) };
            let id = order.remove(new);
            order.insert(index.min(order.len()), id);
            s.pack(&order, "Add Clip")
        })
    }

    /// Runs `f` as one undo step called `label`, however many engine edits it makes; if it fails,
    /// everything it changed is undone.
    pub(crate) fn group<R>(&mut self, label: &str, f: impl FnOnce(&mut Self) -> Result<R>) -> Result<R> {
        self.group_merge(label, None, f)
    }

    /// [`Studio::group`] that folds into the previous step when that one was made by the same
    /// `merge` key and nothing happened since (a slider dragged through many values is one step).
    pub(crate) fn group_merge<R>(&mut self, label: &str, merge: Option<&str>, f: impl FnOnce(&mut Self) -> Result<R>) -> Result<R> {
        let top = self.session.history.undo.last().map(|e| e.1.clone());
        let fold = match (merge, &self.merge, &top) {
            (Some(k), Some((key, snap)), Some(t)) => k == key && std::sync::Arc::ptr_eq(snap, t) && self.session.history.redo.is_empty(),
            _ => false,
        };
        self.merge = None;
        let before = self.session.history.undo.len();
        // a folded step keeps the state from before the first change of the run
        let mark = if fold { before.saturating_sub(1) } else { before };
        let redo = self.session.history.redo.clone();
        let limit = self.session.history.limit;
        // keep every step, so the first one can be found afterwards
        self.session.history.limit = usize::MAX;
        let r = f(self);
        self.session.history.limit = limit;
        match r {
            Ok(v) => {
                let h = &mut self.session.history;
                if h.undo.len() > before
                    && let Some(first) = h.undo.get(mark).map(|e| e.1.clone())
                {
                    h.undo.truncate(mark);
                    h.undo.push((label.to_string(), first));
                    if h.undo.len() > limit {
                        h.undo.remove(0);
                    }
                    h.merge_key = None;
                }
                if let (Some(k), Some(top)) = (merge, self.session.history.undo.last()) {
                    self.merge = Some((k.to_string(), top.1.clone()));
                }
                Ok(v)
            }
            Err(e) => {
                while self.session.history.undo.len() > before {
                    if self.session.execute("edit.undo", json!({})).is_err() {
                        break;
                    }
                }
                self.session.history.redo = redo;
                Err(e)
            }
        }
    }

    /// Where the main track ends.
    fn main_end(&self) -> Result<Tick> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        Ok(q.video_tracks.first().and_then(|t| t.items.iter().map(|i| i.end()).max()).unwrap_or(Tick::ZERO))
    }

    /// Clips under `t` (strictly inside, so a cut lands within them): of `among` when given, else
    /// those on the main track.
    fn clips_under(&self, t: Tick, among: &[u64]) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let tracks: Vec<_> = if among.is_empty() { q.video_tracks.first().into_iter().collect() } else { q.all_tracks().collect() };
        Ok(tracks
            .iter()
            .flat_map(|tr| tr.items.iter())
            .filter(|i| (among.is_empty() || among.contains(&i.id.0)) && i.start < t && i.end() > t)
            .map(|i| i.id.0)
            .collect())
    }

    /// S: cuts the selected clips (or the main track) at `seconds`. Returns how many clips were cut.
    pub fn split(&mut self, seconds: f64, selection: &[u64]) -> Result<usize> {
        let t = self.frame_tick(seconds)?;
        let targets = self.clips_under(t, selection)?;
        if targets.is_empty() {
            return Err(Error::Invalid("move the playhead inside a clip to split it".into()));
        }
        // a cut inside a transition's clip moves it to the piece at the cut
        self.group_follow("Split", None, |st| {
            st.session
                .edit_group("Split", |s| {
                    for c in &targets {
                        // A linked partner may already have been cut along with an earlier target
                        if s.active_sequence().and_then(|q| q.find_item(filmcraft_project::ClipId(*c))).is_some_and(|(_, i)| i.start < t && i.end() > t) {
                            s.execute("timeline.razor", json!({"time": t.0, "clip": c}))?;
                        }
                    }
                    Ok(())
                })
                .map_err(engine)?;
            st.pack_main()
        })?;
        Ok(targets.len())
    }

    /// Q / W: cuts at `seconds` and removes the part before (Q) or after (W) the cut, closing the
    /// gap. Returns where the playhead belongs afterwards (Q moves it to the cut's new place).
    pub fn split_keep(&mut self, seconds: f64, selection: &[u64], keep: Keep) -> Result<f64> {
        let t = self.frame_tick(seconds)?;
        let targets = self.clips_under(t, selection)?;
        if targets.is_empty() {
            return Err(Error::Invalid("move the playhead inside a clip first".into()));
        }
        let mut playhead = t;
        let label = if keep == Keep::Right { "Delete to the Left" } else { "Delete to the Right" };
        let main_ids: Vec<u64> = self
            .session
            .active_sequence()
            .map(|q| q.video_tracks.first().into_iter().chain(q.audio_tracks.first()).flat_map(|t| t.items.iter().map(|i| i.id.0)).collect())
            .unwrap_or_default();
        self.group_follow(label, None, |st| {
            st.unlock_lanes()?;
            st.session
                .edit_group(label, |s| {
                    let mut remove = Vec::new();
                    for c in &targets {
                        let cid = filmcraft_project::ClipId(*c);
                        let Some((track, item)) = s.active_sequence().and_then(|q| q.find_item(cid)).map(|(tr, i)| (tr, i.clone())) else { continue };
                        if item.start < t && item.end() > t {
                            s.execute("timeline.razor", json!({"time": t.0, "clip": c}))?;
                        }
                        // the piece on this clip's side of the cut, on its track
                        let q = s.active_sequence().ok_or(filmcraft_engine::EngineError::NoSequence)?;
                        let tr = q.track(track).ok_or(filmcraft_engine::EngineError::NoSequence)?;
                        let piece = tr
                            .items
                            .iter()
                            .find(|i| if keep == Keep::Right { i.end() == t && i.start >= item.start } else { i.start == t && i.end() <= item.end() });
                        if let Some(p) = piece {
                            if keep == Keep::Right && main_ids.contains(c) {
                                playhead = playhead.min(p.start);
                            }
                            if !remove.contains(&p.id.0) {
                                remove.push(p.id.0);
                            }
                        }
                    }
                    // main-track pieces close up; lane pieces leave their place empty
                    let (main, lanes): (Vec<u64>, Vec<u64>) = remove.iter().partition(|c| main_ids.contains(c));
                    if !lanes.is_empty() {
                        s.execute("edit.clear", json!({"clips": lanes}))?;
                    }
                    if !main.is_empty() {
                        s.execute("edit.rippleDelete", json!({"clips": main}))?;
                    }
                    Ok(())
                })
                .map_err(engine)?;
            st.pack_main()
        })?;
        Ok(secs(playhead))
    }

    /// Delete: removes clips and closes the gaps they leave (ripple delete), CapCut-style.
    pub fn delete(&mut self, clips: &[u64]) -> Result<()> {
        if clips.is_empty() {
            return Err(Error::Invalid("select a clip to delete".into()));
        }
        // the main track closes up; clips on lanes leave their place empty, as in CapCut
        let (main, lanes): (Vec<u64>, Vec<u64>) = clips.iter().partition(|c| self.on_main(**c));
        self.group_follow("Delete", None, |s| {
            s.unlock_lanes()?;
            if !lanes.is_empty() {
                s.session.execute("edit.clear", json!({"clips": lanes})).map_err(engine)?;
            }
            if !main.is_empty() {
                s.session.execute("edit.rippleDelete", json!({"clips": main})).map_err(engine)?;
            }
            s.pack_main()
        })
    }

    /// Drags a clip's edge by `delta_seconds` (snapped to frames). Later clips follow, so the main
    /// track stays closed. Returns the change actually made (the engine stops at the media's
    /// ends and at one frame of length).
    pub fn trim(&mut self, clip: u64, edge_in: bool, delta_seconds: f64) -> Result<f64> {
        let rate = self.fps()?;
        if !delta_seconds.is_finite() {
            return Err(Error::Invalid("the trim must be a number".into()));
        }
        let frames = (delta_seconds * rate.as_f64()).round() as i64;
        if frames == 0 {
            return Ok(0.0);
        }
        // the main track ripples; a lane clip's edge just moves
        let mode = if self.on_main(clip) { "ripple" } else { "regular" };
        self.group_follow("Trim", None, |s| {
            s.unlock_lanes()?;
            let r = s
                .session
                .execute("timeline.trim", json!({"clip": clip, "edge": if edge_in { "in" } else { "out" }, "mode": mode, "deltaFrames": frames}))
                .map_err(engine)?;
            s.pack_main()?;
            Ok(r.get("delta").and_then(Value::as_i64).map(|d| secs(Tick(d))).unwrap_or(0.0))
        })
    }

    /// Moves a main-track clip to position `to` (0 = first) among the main-track clips; the
    /// others close up around it.
    pub fn reorder(&mut self, clip: u64, to: usize) -> Result<()> {
        let mut order = self.main_order()?;
        let from = order.iter().position(|c| *c == clip).ok_or_else(|| Error::Invalid("only clips on the main track can be reordered".into()))?;
        let id = order.remove(from);
        order.insert(to.min(order.len()), id);
        self.group_follow("Reorder", None, |s| s.pack(&order, "Reorder"))
    }

    /// Main-track clip ids in timeline order.
    fn main_order(&self) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mut items: Vec<_> = q.video_tracks.first().map(|t| t.items.iter().map(|i| (i.start, i.id.0)).collect()).unwrap_or_default();
        items.sort();
        Ok(items.into_iter().map(|(_, id)| id).collect())
    }

    /// Closes any gap on the main track (the magnetic rule).
    pub(crate) fn pack_main(&mut self) -> Result<()> {
        let order = self.main_order()?;
        self.pack(&order, "Close Gaps")
    }

    /// Lays `order` out back to back from 0 on the main track, moving linked sound along. One
    /// undo step; nothing happens when everything is already in place.
    fn pack(&mut self, order: &[u64], label: &str) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let Some(main) = q.video_tracks.first() else { return Ok(()) };
        let mut at = Tick::ZERO;
        let mut moves = Vec::new();
        for id in order {
            let Some(i) = main.items.iter().find(|i| i.id.0 == *id) else { continue };
            if i.start != at {
                moves.push(json!({"clip": id, "track": main.id.0, "time": at.0}));
            }
            at += i.duration;
        }
        if moves.is_empty() {
            return Ok(());
        }
        // The engine moves the whole set at once, so a clip never lands on one that hasn't moved
        // yet; linked sound follows each clip.
        let moves = Value::Array(moves);
        self.session.edit_group(label, |s: &mut Session| s.execute("timeline.move", json!({"moves": moves, "linked": true})).map(|_| ())).map_err(engine)
    }

    pub fn undo(&mut self) -> Result<()> {
        if !self.session.history.can_undo() {
            return Err(Error::Invalid("nothing to undo".into()));
        }
        self.session.execute("edit.undo", json!({})).map(|_| ()).map_err(engine)
    }

    pub fn redo(&mut self) -> Result<()> {
        if !self.session.history.can_redo() {
            return Err(Error::Invalid("nothing to redo".into()));
        }
        self.session.execute("edit.redo", json!({})).map(|_| ()).map_err(engine)
    }

    /* ---------- projects ---------- */

    /// Saves to `path` and makes it the project's file.
    pub fn save_as(&mut self, path: &str) -> Result<()> {
        // Render previews go in "ClipCut Previews" beside the project (the engine's default folder
        // carries FilmCraft's name). Relative, so it moves with the project's folder.
        let scratch = &self.session.project.settings.scratch;
        if scratch.video_previews.is_none() || scratch.audio_previews.is_none() {
            let project = std::sync::Arc::make_mut(&mut self.session.project);
            project.settings.scratch.video_previews.get_or_insert_with(|| PREVIEWS_DIR.to_string());
            project.settings.scratch.audio_previews.get_or_insert_with(|| PREVIEWS_DIR.to_string());
        }
        self.session.execute("file.saveAs", json!({"path": path})).map(|_| ()).map_err(engine)
    }

    /// Saves to the project's file (after [`Studio::save_as`] or [`Studio::open_project`]).
    pub fn save(&mut self) -> Result<()> {
        self.session.execute("file.save", json!({})).map(|_| ()).map_err(engine)
    }

    /// Opens a saved project and its first sequence.
    pub fn open_project(&mut self, path: &str) -> Result<()> {
        self.session.execute("file.open", json!({"path": path})).map_err(engine)?;
        if self.session.active_sequence().is_none() {
            let first = self.session.project.sequences().map(|i| i.id).min();
            if let Some(id) = first {
                self.session.state.active_sequence = Some(id);
            }
        }
        Ok(())
    }

    /// Starts an empty project.
    pub fn new_project(&mut self, name: &str) -> Result<()> {
        self.session.execute("file.newProject", json!({"name": name})).map(|_| ()).map_err(engine)
    }
}
