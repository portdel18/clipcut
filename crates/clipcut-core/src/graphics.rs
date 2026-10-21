//! Graphics made in the Graphics tab (Clip Studio, shown by the app): each one is a folder of
//! numbered frames (`frame_00000.png` with transparency, or `.jpg`), maybe its sound
//! (`audio.wav`), and `graphic.json`, which holds the Clip Studio tool and its settings so the
//! graphic can be opened there again. The frames come in as an image sequence at the graphic's
//! frame rate and go on an overlay lane; updating a graphic puts the new one in its clip's place,
//! with the clip's position, size and opacity.

use std::path::{Path, PathBuf};

use filmcraft_project::{ClipId, ItemId, ItemKind, MediaRef};
use serde_json::{Value, json};

use crate::lanes::{Lane, lane_name};
use crate::{Error, Result, Studio, engine};

/// The file beside a graphic's frames that says what made it.
pub const GRAPHIC_FILE: &str = "graphic.json";
/// A graphic's sound, beside its frames.
pub const GRAPHIC_SOUND: &str = "audio.wav";

/// The engine's names for the frame rates an image sequence can have.
fn timebase(fps: f64) -> Result<&'static str> {
    const RATES: &[(f64, &str)] =
        &[(23.976, "23.976"), (24.0, "24"), (25.0, "25"), (29.97, "29.97"), (30.0, "30"), (50.0, "50"), (59.94, "59.94"), (60.0, "60")];
    RATES
        .iter()
        .find(|(r, _)| (r - fps).abs() < 0.01)
        .map(|(_, n)| *n)
        .ok_or_else(|| Error::Invalid(format!("a graphic can't be {fps} frames a second: use 24, 25, 30 or 60")))
}

/// The folder of the graphic whose frames are `path`, when it is one.
fn graphic_dir(path: &str) -> Option<PathBuf> {
    let dir = Path::new(path).parent()?;
    dir.join(GRAPHIC_FILE).is_file().then(|| dir.to_path_buf())
}

/// A graphic's `graphic.json`.
pub fn read_graphic(dir: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(dir.join(GRAPHIC_FILE)).map_err(|e| Error::Invalid(format!("this graphic's settings can't be read ({e})")))?;
    serde_json::from_str(&text).map_err(|e| Error::Invalid(format!("this graphic's settings are damaged ({e})")))
}

impl Studio {
    /// A media item's picture size.
    pub(crate) fn picture_size(&self, item: ItemId) -> Option<(u32, u32)> {
        let it = self.session.project.item(item)?;
        let ItemKind::Media(m) = &it.kind else { return None };
        m.info.video.as_ref().map(|v| (v.width, v.height))
    }

    /// Whether a clip is on a sound track.
    fn on_audio(&self, clip: u64) -> bool {
        self.session.active_sequence().is_some_and(|q| q.audio_tracks.iter().any(|t| t.items.iter().any(|i| i.id.0 == clip)))
    }

    /// The file a media item plays, if it is a file.
    fn media_path(&self, item: ItemId) -> Option<String> {
        let it = self.session.project.item(item)?;
        let ItemKind::Media(m) = &it.kind else { return None };
        let MediaRef::File { path } = &m.media else { return None };
        Some(path.clone())
    }

    /// The Clip Studio tool that made media `item` (`"lt"`, `"mp"`…), when it is a graphic. A
    /// graphic's folder never changes once made, so what it says is kept.
    pub(crate) fn graphic_tool(&self, item: ItemId) -> Option<String> {
        let dir = graphic_dir(&self.media_path(item)?)?;
        let mut known = self.graphics.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = known.get(&dir) {
            return t.clone();
        }
        let tool = read_graphic(&dir).ok().and_then(|g| g.get("tool").and_then(Value::as_str).map(str::to_string));
        known.insert(dir, tool.clone());
        tool
    }

    /// What made clip `clip`'s graphic (its `graphic.json`), to open it in Clip Studio again.
    pub fn graphic_of(&self, clip: u64) -> Result<Value> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (_, i) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        let dir = self.media_path(i.item).and_then(|p| graphic_dir(&p)).ok_or_else(|| Error::Invalid("this clip wasn't made in the Graphics tab".into()))?;
        read_graphic(&dir)
    }

    /// Imports the numbered frames that `first` starts as one clip at `fps`.
    fn import_frames(&mut self, first: &str, fps: f64) -> Result<u64> {
        let rate = timebase(fps)?;
        let media = &mut self.session.prefs.media;
        let old = std::mem::replace(&mut media.indeterminate_timebase, rate.to_string());
        let r = self.session.execute("file.import", json!({"paths": [first], "imageSequence": true}));
        self.session.prefs.media.indeterminate_timebase = old;
        let r = r.map_err(engine)?;
        r.get("items").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_u64).ok_or_else(|| {
            let why = r.get("errors").and_then(Value::as_array).map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default();
            Error::Engine(if why.is_empty() { format!("the graphic's frames ({first}) could not be read") } else { why })
        })
    }

    /// Puts the graphic in folder `dir` (its frames start at `first`, `fps` a second; its sound,
    /// if any, is [`GRAPHIC_SOUND`] there) on the timeline: at `seconds` on a free overlay lane, or
    /// in place of clip `replace` (same lane, start, position, size and opacity; the old one's
    /// sound goes too). With no video yet, it starts one. One undo step. Returns the new clips.
    pub fn add_graphic(&mut self, first: &str, fps: f64, seconds: f64, replace: Option<u64>) -> Result<Vec<u64>> {
        let dir = Path::new(first).parent().ok_or_else(|| Error::Invalid("the graphic has no folder".into()))?.to_path_buf();
        let sound = dir.join(GRAPHIC_SOUND);
        let sound = sound.is_file().then(|| sound.to_string_lossy().to_string());
        let label = if replace.is_some() { "Update Graphic" } else { "Add Graphic" };
        let start_new = self.session.active_sequence().is_none();
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Graphic".into());
        self.group_follow(label, None, |s| {
            let item = s.import_frames(first, fps)?;
            // named after its folder (Clip Studio's name for it), not its first frame's file
            s.session.execute("clip.rename", json!({"item": item, "name": name})).map_err(engine)?;
            if start_new {
                s.append_item(item)?;
                let mut made: Vec<u64> =
                    s.session.active_sequence().map(|q| q.all_tracks().flat_map(|t| t.items.iter().map(|i| i.id.0)).collect()).unwrap_or_default();
                if let Some(snd) = &sound {
                    let a = s.import(snd)?;
                    made.extend(s.add_to_lane(a, 0.0)?);
                }
                return Ok(made);
            }
            let Some(old) = replace else {
                let mut made = s.add_to_lane(item, seconds)?;
                // rendered at Clip Studio's size (1080 on the short side): the video's frame, whatever its size
                let pictures: Vec<u64> = made.iter().copied().filter(|c| !s.on_audio(*c)).collect();
                if !pictures.is_empty() {
                    s.session.execute("clip.fitToFrame", json!({"clips": pictures})).map_err(engine)?;
                }
                if let Some(snd) = &sound {
                    let a = s.import(snd)?;
                    made.extend(s.add_to_lane(a, seconds)?);
                    s.link_together(&made)?;
                }
                return Ok(made);
            };
            // in place of an older version: where it was, looking as it did
            let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
            let (track, clip) = q
                .find_item(ClipId(old))
                .map(|(t, i)| (t, i.clone()))
                .ok_or_else(|| Error::Invalid("the graphic being updated is no longer on the timeline".into()))?;
            let index = q.video_tracks.iter().position(|t| t.id == track).ok_or_else(|| Error::Invalid("only a picture clip can be updated".into()))?;
            let tr = q.track(track).ok_or(Error::NothingOpen)?;
            let (_, props) = crate::props::clip_props(&s.session, q, tr, &clip);
            let old_dir = s.media_path(clip.item).and_then(|p| graphic_dir(&p));
            // the old version's clips: its picture and its sound, linked to it (pasted copies of
            // the graphic play the same files and stay). A graphic from before they were linked:
            // the sound from its folder that starts with it (within 20 ms).
            let near = filmcraft_time::TICKS_PER_SECOND / 50;
            let gone: Vec<u64> = q
                .all_tracks()
                .flat_map(|t| t.items.iter().map(move |i| (t.kind, i)))
                .filter(|(kind, i)| {
                    i.id.0 == old
                        || match clip.link {
                            Some(l) => i.link == Some(l),
                            None => {
                                *kind == filmcraft_project::TrackKind::Audio
                                    && i.link.is_none()
                                    && (i.start.0 - clip.start.0).abs() <= near
                                    && old_dir.is_some()
                                    && s.media_path(i.item).and_then(|p| graphic_dir(&p)) == old_dir
                            }
                        }
                })
                .map(|(_, i)| i.id.0)
                .collect();
            s.unlock_lanes()?;
            if index == 0 {
                s.session.execute("edit.rippleDelete", json!({"clips": gone})).map_err(engine)?;
            } else {
                s.session.execute("edit.clear", json!({"clips": gone})).map_err(engine)?;
            }
            let p = json!({"item": item, "time": clip.start.0, "track": lane_name(Lane::Video, index), "audio": false});
            let r = s.session.execute("timeline.place", p).map_err(engine)?;
            let mut made: Vec<u64> = r.get("clips").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
            if index == 0 {
                s.pack_main()?;
            }
            // the same size: where it was and as big; another size (made for another frame shape):
            // fitted to the frame, keeping only its turn and opacity
            let same = s.picture_size(ItemId(item)) == s.picture_size(clip.item);
            if !same && !made.is_empty() {
                s.session.execute("clip.fitToFrame", json!({"clips": made})).map_err(engine)?;
            }
            let keep: &[(&str, Option<f64>)] = if same {
                &[("x", props.x), ("y", props.y), ("scale", props.scale), ("rotation", props.rotation), ("opacity", props.opacity)]
            } else {
                &[("rotation", props.rotation), ("opacity", props.opacity)]
            };
            let look: serde_json::Map<String, Value> = keep.iter().filter_map(|(k, v)| Some((k.to_string(), json!((*v)?)))).collect();
            if !look.is_empty() && !made.is_empty() {
                s.apply_props(&made, &Value::Object(look))?;
            }
            if let Some(snd) = &sound {
                let a = s.import(snd)?;
                let at = s.session.active_sequence().and_then(|q| q.find_item(ClipId(*made.first()?)).map(|(_, i)| i.start.seconds())).unwrap_or(seconds);
                made.extend(s.add_to_lane(a, at)?);
                s.link_together(&made)?;
            }
            Ok(made)
        })
    }

    /// Links a graphic's picture and its sound, so they move, trim, go, copy and update together.
    fn link_together(&mut self, clips: &[u64]) -> Result<()> {
        if clips.len() < 2 {
            return Ok(());
        }
        self.session
            .edit_sequence("Link", |q, ctx, _| {
                let l = ctx.alloc();
                for c in clips {
                    if let Some((_, i)) = q.find_item_mut(ClipId(*c)) {
                        i.link = Some(l);
                    }
                }
                Ok(())
            })
            .map_err(engine)
    }
}
