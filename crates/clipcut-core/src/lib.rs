//! ClipCut's engine façade: the FilmCraft engine operations the ClipCut desktop app calls.
//!
//! The app (`apps/clipcut`, Tauri) keeps one [`Studio`] and turns its interface's requests into
//! these calls: open a clip as a sequence, render a preview frame at any time and size, hand the
//! audio thread a [`AudioFeed`] that mixes the sequence, and run exports as background jobs.
//! Everything goes through the engine's own commands (`file.import`, `file.newSequence`,
//! `file.exportMedia`, `jobs.cancel`), so the result is what the engine itself would produce.
//!
//! No UI toolkit, window or audio device here: those belong to the app.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod animations;
mod beats;
mod captions;
mod colour;
mod ducking;
mod exporting;
mod graphics;
mod keyframes;
mod lanes;
mod mixer;
mod props;
mod scopes;
mod timeline;
mod transitions;
mod trimming;

use std::sync::Arc;

use filmcraft_engine::Session;
use filmcraft_engine::media_pool::PoolProvider;
use filmcraft_project::{ItemId, Project};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::{Value, json};

pub use animations::{ANIMATIONS_IN, ANIMATIONS_OUT, AnimView, TextAnimations, text_animations};
pub use captions::{CaptionStyleView, CaptionView, CaptionsView, ModelView, TranscribeJob, WordView, caption_presets, download_model, speech_models};
pub use colour::{ADJUST, ColourView, looks};
pub use ducking::DUCK_DB;
pub use exporting::ExportChoice;
pub use graphics::{GRAPHIC_FILE, GRAPHIC_SOUND, read_graphic};
pub use keyframes::{ANIMATABLE, KeyTrack};
pub use mixer::{Level, Meters, MixView};
pub use props::{ClipProps, PEAKS_PER_SECOND, Peaks, TextProps};
pub use scopes::SCOPES;
pub use timeline::{ClipView, Keep, LaneView, MediaView, ProjectView, TimelineView};
pub use transitions::{DEFAULT_SECONDS as TRANSITION_SECONDS, TRANSITIONS, TransitionPresetView, TransitionView, transition_presets, transition_previews};

/// What went wrong, in words the app can show as they are.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Engine(String),
    #[error("no clip is open yet: open a video first")]
    NothingOpen,
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn engine(e: impl std::fmt::Display) -> Error {
    Error::Engine(e.to_string())
}

/// The open sequence, as the interface shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_seconds: f64,
    /// Frames in the sequence (at `fps`).
    pub frames: i64,
    pub sample_rate: u32,
}

/// A rendered preview frame: `rgba` is `w × h × 4` bytes, straight (not premultiplied), over black.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// An export job's state.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub id: u64,
    /// 0…1.
    pub progress: f64,
    pub eta_seconds: Option<f64>,
    pub finished: bool,
    /// Set when the job failed or was cancelled.
    pub error: Option<String>,
    pub status: String,
}

/// Mixes the open sequence's sound for playback, on any thread: it holds a snapshot of the project,
/// so edits made after it was taken don't reach it (take a new one when the sequence changes).
pub struct AudioFeed {
    project: Arc<Project>,
    sequence: ItemId,
    provider: PoolProvider,
    /// Where the mixer leaves each strip's level as it plays (see [`AudioFeed::meters`]).
    live: Arc<filmcraft_render::mixer::LiveMix>,
    pub sample_rate: u32,
    /// Total samples in the sequence (per channel).
    pub length: i64,
}

impl AudioFeed {
    /// The levels the mixer leaves as this feed plays, readable from another thread.
    pub fn meters(&self) -> crate::mixer::Meters {
        let lanes = self.project.sequence(self.sequence).map(crate::mixer::strip_lanes).unwrap_or_default();
        crate::mixer::Meters { live: self.live.clone(), lanes }
    }

    /// `frames` stereo samples starting at sample `start` (sequence rate), interleaved L R L R.
    /// Past the end, or before the start, is silence.
    pub fn mix(&self, start: i64, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames.saturating_mul(2)];
        let Some(seq) = self.project.sequence(self.sequence) else { return out };
        let from = start.max(0);
        let skip = usize::try_from(from.saturating_sub(start)).unwrap_or(usize::MAX).min(frames);
        let left = usize::try_from(self.length.saturating_sub(from).max(0)).unwrap_or(0);
        let n = (frames - skip).min(left);
        if n == 0 {
            return out;
        }
        let mixed = filmcraft_render::mixer::mix_graph(&self.project, seq, from, n, &self.provider, Some(&self.live));
        let buf = filmcraft_render::audio::to_layout(mixed, filmcraft_audio_dsp::channels::Layout::Stereo, filmcraft_audio_dsp::channels::Mixdown::FrontRear);
        let (Some(l), Some(r)) = (buf.channels.first(), buf.channels.get(1).or(buf.channels.first())) else { return out };
        for i in 0..n {
            let o = (skip + i) * 2;
            if let (Some(a), Some(b), Some(dst)) = (l.get(i), r.get(i), out.get_mut(o..o + 2)) {
                dst[0] = *a;
                dst[1] = *b;
            }
        }
        out
    }
}

/// Renders media frames (not the program) for the Media panel and the timeline's filmstrips, on any
/// thread: like [`AudioFeed`] it holds a snapshot of the project, so it doesn't keep the session busy.
pub struct Thumbs {
    project: Arc<Project>,
    provider: PoolProvider,
}

impl Thumbs {
    /// Media item `item`'s picture at `seconds` into the media, at most `max_width` wide (never
    /// upscaled). Times are clamped to the media.
    pub fn frame(&self, item: u64, seconds: f64, max_width: u32) -> Result<Frame> {
        if !seconds.is_finite() {
            return Err(Error::Invalid("the time must be a number".into()));
        }
        let id = ItemId(item);
        let pi = self.project.item(id).ok_or_else(|| Error::Invalid(format!("there is no media {item}")))?;
        let (_, m, _) = self.project.resolve_media(id).ok_or_else(|| Error::Invalid(format!("{} has no picture", pi.name)))?;
        let v = m.info.video.as_ref().ok_or_else(|| Error::Invalid(format!("{} has no picture", pi.name)))?;
        let last = (pi.duration().seconds() - 1.0 / v.frame_rate.as_f64().max(1.0)).max(0.0);
        let t = Tick::from_seconds_f64(seconds.clamp(0.0, last));
        let scale = if v.width == 0 { 1.0 } else { (max_width.max(16) as f32 / v.width as f32).min(1.0) };
        let img = filmcraft_render::render_item(&self.project, id, t, scale, &self.provider)
            .ok_or_else(|| Error::Engine(format!("{} couldn't be read at {seconds:.2} s", pi.name)))?;
        let w = u32::try_from(img.w).map_err(engine)?;
        let h = u32::try_from(img.h).map_err(engine)?;
        Ok(Frame { w, h, rgba: img.over_black_rgba8() })
    }
}

/// One ClipCut window's engine session.
pub struct Studio {
    session: Session,
    /// The key and undo step of the last mergeable edit ([`Studio::set_props`] while a slider moves).
    merge: Option<(String, Arc<Project>)>,
    /// What made each graphic folder seen so far (see `graphics.rs`).
    graphics: std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Option<String>>>,
}

impl Default for Studio {
    fn default() -> Self {
        Self::new()
    }
}

impl Studio {
    pub fn new() -> Self {
        Self { session: Session::default(), merge: None, graphics: Default::default() }
    }

    /// The engine session, for operations this façade doesn't cover yet.
    pub fn session(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Opens a media file as the project's only clip, in a new sequence that matches it (size,
    /// frame rate, sample rate). Replaces whatever was open.
    pub fn open_clip(&mut self, path: &str) -> Result<SequenceInfo> {
        if path.trim().is_empty() {
            return Err(Error::Invalid("choose a file to open".into()));
        }
        let r = self.session.execute("file.import", json!({"paths": [path], "replace": true})).map_err(engine)?;
        let item = r.get("items").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_u64);
        let Some(item) = item else {
            let why = r.get("errors").and_then(Value::as_array).map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default();
            return Err(Error::Engine(if why.is_empty() { format!("{path} could not be opened") } else { why }));
        };
        let name = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sequence".into());
        self.session.execute("file.newSequence", json!({"fromItem": item, "name": name})).map_err(engine)?;
        self.info()
    }

    /// The open sequence.
    pub fn info(&self) -> Result<SequenceInfo> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let st = &q.settings;
        let fps = st.frame_rate.as_f64();
        let dur = q.duration();
        let name = self.session.state.active_sequence.and_then(|id| self.session.project.item(id)).map(|i| i.name.clone()).unwrap_or_default();
        Ok(SequenceInfo {
            name,
            width: st.width,
            height: st.height,
            fps,
            duration_seconds: dur.seconds(),
            frames: st.frame_rate.frame_at(dur),
            sample_rate: st.sample_rate,
        })
    }

    /// The program frame at `seconds`, at most `max_width` pixels wide (never upscaled). Times
    /// outside the sequence are clamped to its first or last frame.
    pub fn frame(&self, seconds: f64, max_width: u32) -> Result<Frame> {
        let info = self.info()?;
        if !seconds.is_finite() {
            return Err(Error::Invalid("the time must be a number".into()));
        }
        let last = (info.duration_seconds - 1.0 / info.fps.max(1.0)).max(0.0);
        let t = Tick::from_seconds_f64(seconds.clamp(0.0, last));
        let scale = if info.width == 0 { 1.0 } else { (max_width.max(16) as f32 / info.width as f32).min(1.0) };
        let (img, _offline) = self.session.try_render_program_with_offline_at(scale, t).map_err(engine)?;
        let w = u32::try_from(img.w).map_err(engine)?;
        let h = u32::try_from(img.h).map_err(engine)?;
        Ok(Frame { w, h, rgba: img.over_black_rgba8() })
    }

    /// A mixer for the playback thread over the sequence as it is now.
    pub fn audio_feed(&self) -> Result<AudioFeed> {
        let id = self.session.state.active_sequence.ok_or(Error::NothingOpen)?;
        let q = self.session.project.sequence(id).ok_or(Error::NothingOpen)?;
        let sample_rate = q.settings.sample_rate.max(1);
        let length = q.duration().to_units_floor(sample_rate as i64);
        let provider = self.session.media.full_res_provider(self.session.project.clone(), self.session.services.clone());
        Ok(AudioFeed { project: self.session.project.clone(), sequence: id, provider, live: Arc::default(), sample_rate, length })
    }

    /// A frame renderer for media thumbnails over the project as it is now.
    pub fn thumbs(&self) -> Thumbs {
        Thumbs { project: self.session.project.clone(), provider: self.session.media.provider(self.session.project.clone(), self.session.services.clone()) }
    }

    /// Starts exporting the whole sequence to `path` as a background job and returns its id. The
    /// format follows the extension: `.mp4` is H.264 with AAC, `.mov` ProRes, `.wav` sound only.
    pub fn export(&mut self, path: &str) -> Result<u64> {
        self.info()?;
        let format = format_for(path).ok_or_else(|| Error::Invalid(format!("{path}: save as .mp4, .mov or .wav")))?;
        // captions that show in the player are burnt into the picture too
        let r = self.session.execute("file.exportMedia", json!({"path": path, "format": format, "burnCaptions": self.burn_captions()})).map_err(engine)?;
        r.get("job").and_then(Value::as_u64).ok_or_else(|| Error::Engine("the export did not start".into()))
    }

    /// An export job's progress, or `None` for an unknown id.
    pub fn job(&self, id: u64) -> Option<JobStatus> {
        let j = self.session.jobs.iter().find(|j| j.id == id)?;
        let v = j.to_json();
        let error = v.get("result").and_then(|r| r.get("error")).and_then(Value::as_str).map(str::to_string);
        Some(JobStatus {
            id,
            progress: v.get("progress").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1.0),
            eta_seconds: v.get("etaSeconds").and_then(Value::as_f64),
            finished: v.get("finished").and_then(Value::as_bool).unwrap_or(false),
            error,
            status: v.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
    }

    /// Asks a running export to stop; it finishes with an error saying it was cancelled.
    pub fn cancel(&mut self, id: u64) -> Result<()> {
        self.session.execute("jobs.cancel", json!({"job": id})).map(|_| ()).map_err(engine)
    }
}

/// The engine's export format for a file name, by extension.
pub fn format_for(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path).extension()?.to_string_lossy().to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "m4v" => "h264",
        "mov" => "prores",
        "wav" => "wav",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_extension() {
        assert_eq!(format_for("C:\\out\\clip.MP4"), Some("h264"));
        assert_eq!(format_for("a.mov"), Some("prores"));
        assert_eq!(format_for("a.wav"), Some("wav"));
        assert_eq!(format_for("a.mkv"), None);
        assert_eq!(format_for("noext"), None);
    }

    #[test]
    fn nothing_open_is_an_error_not_a_crash() {
        let mut s = Studio::new();
        assert!(matches!(s.info(), Err(Error::NothingOpen)));
        assert!(s.frame(1.0, 640).is_err());
        assert!(s.audio_feed().is_err());
        assert!(s.export("x.mp4").is_err());
        assert!(s.open_clip("   ").is_err());
        assert!(s.open_clip("/definitely/not/here.mp4").is_err());
        assert!(s.job(99).is_none());
    }
}
