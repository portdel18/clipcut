//! Split at scene changes: for a clip that is already edited (a broadcast segment, someone else's
//! video), the engine's scene detector ([`filmcraft_render::scene`]) compares each frame with the
//! one before and finds where a new shot starts. The frames are read off the studio lock
//! ([`SceneScan::run`], a source of its own at a small size), then the clip and its linked sound
//! are cut at every change in one undo step through ClipCut's own split edit, so captions, linkage
//! and transitions follow as they do for any cut. (The engine's Scene Edit Detection command cuts
//! with a plain razor, outside that.)

use filmcraft_media::{FrameRequest, SharedSource};
use filmcraft_project::{ClipId, MediaRef, TrackKind};
use filmcraft_render::scene::{SceneDetector, SceneOptions};
use filmcraft_time::Tick;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

/// Longest side of the frames compared (the detector needs no detail).
const ANALYSIS_WIDTH: f64 = 256.0;
/// Most frames one scan reads (about an hour at 60 fps).
const MAX_FRAMES: i64 = 216_000;

/// A clip's frames to compare, read on any thread.
pub struct SceneScan {
    source: SharedSource,
    /// Timeline and media time of every sequence frame of the clip.
    frames: Vec<(Tick, Tick)>,
    scale: f32,
    name: String,
}

impl Studio {
    /// The frames of video clip `clip` to look for scene changes in (with a source of its own, so
    /// the player's isn't disturbed).
    pub fn scene_scan(&self, clip: u64) -> Result<SceneScan> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (track, it) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip isn't on the timeline any more".into()))?;
        if q.track(track).is_none_or(|t| t.kind != TrackKind::Video) {
            return Err(Error::Invalid("choose a video clip to split at its scene changes".into()));
        }
        let (_, media, _) = self.session.project.resolve_media(it.item).ok_or_else(|| Error::Invalid(format!("{} has no video to look at", it.name)))?;
        let width = media.info.video.as_ref().map(|v| v.width).ok_or_else(|| Error::Invalid(format!("{} has no video to look at", it.name)))?;
        // a fresh copy of the file, not the player's (its cache and decoder stay as they are)
        let source = match &media.media {
            MediaRef::File { path } => {
                let path = self.session.media.follow(path).unwrap_or_else(|_| path.clone());
                self.session.media.open_file(&path, &*self.session.services).ok()
            }
            _ => None,
        }
        .or_else(|| self.session.source(it.item))
        .ok_or_else(|| Error::Invalid(format!("{}'s file can't be read: is its drive connected?", it.name)))?;
        let fd = q.settings.frame_rate.frame_duration();
        let n = (it.duration.0 / fd.0.max(1)).clamp(1, MAX_FRAMES);
        let frames = (0..n)
            .map(|j| {
                let t = it.start + Tick(fd.0.saturating_mul(j));
                (t, it.source_time_at(t))
            })
            .collect();
        Ok(SceneScan { source, frames, scale: (ANALYSIS_WIDTH / f64::from(width.max(1))).min(1.0) as f32, name: it.name.clone() })
    }

    /// Cuts clip `clip` (and its linked sound) at each of `cuts` (timeline ticks, as
    /// [`SceneScan::run`] gives them) that falls inside it, in one undo step. Returns how many
    /// cuts were made.
    pub fn split_at_scenes(&mut self, clip: u64, cuts: &[i64]) -> Result<usize> {
        let rate = self.fps()?;
        let (start, end) = {
            let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
            let (_, it) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip isn't on the timeline any more".into()))?;
            (it.start, it.end())
        };
        // on the frame grid, inside the clip as it is now (it may have been trimmed meanwhile)
        let mut at: Vec<Tick> = cuts.iter().map(|t| rate.snap(Tick(*t))).filter(|t| *t > start && *t < end).collect();
        at.sort();
        at.dedup();
        if at.is_empty() {
            return Ok(0);
        }
        let label = "Split at Scene Changes";
        self.group_follow(label, None, |st| {
            st.session
                .edit_group(label, |s| {
                    // the latest first: the piece before each cut keeps the clip's id for the next
                    for t in at.iter().rev() {
                        s.execute("timeline.razor", json!({"time": t.0, "clip": clip}))?;
                    }
                    Ok(())
                })
                .map_err(engine)?;
            st.pack_main()
        })?;
        Ok(at.len())
    }
}

impl SceneScan {
    /// How many frames [`SceneScan::run`] reads.
    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// Reads the clip's frames and returns where each new shot starts (timeline ticks).
    /// `progress(fraction)` is called as it goes; returning `false` stops it with an error.
    pub fn run(&self, progress: &mut dyn FnMut(f64) -> bool) -> Result<Vec<i64>> {
        let mut det = SceneDetector::new();
        let n = self.frames.len().max(1) as f64;
        for (k, (_, mt)) in self.frames.iter().enumerate() {
            if k % 8 == 0 && !progress(k as f64 / n) {
                return Err(Error::Invalid("stopped: nothing was changed".into()));
            }
            // quick decoding is enough to tell shots apart
            let f = filmcraft_media::cancel::with_draft(true, || self.source.video_frame(FrameRequest { time: *mt, scale: self.scale }))
                .map_err(|e| Error::Invalid(format!("{}: frame {} can't be read ({e})", self.name, k + 1)))?;
            det.push_rgba(&f.to_rgba8(), f.width as usize, f.height as usize);
        }
        progress(1.0);
        Ok(det.cuts(&SceneOptions::default()).into_iter().filter_map(|i| self.frames.get(i)).map(|(t, _)| t.0).collect())
    }
}
