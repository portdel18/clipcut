//! The player's frames: preview sizes snapped to the renderer's fast scales (the canvas, ½, ¼,
//! ⅛), a quality setting (CapCut's playback resolution), and a frame feed that renders the
//! sequence as it is now on any thread, so playback can render ahead without holding the project.

use std::sync::Arc;

use filmcraft_engine::media_pool::PoolProvider;
use filmcraft_project::{ItemId, Project};
use filmcraft_time::{FrameRate, Tick};

use crate::{Error, Frame, Result, Studio, engine};

/// How sharp the player is: `auto` follows the player's size; the others cap it.
pub const QUALITIES: &[&str] = &["auto", "full", "half", "quarter"];

/// The scale to render the canvas `w` wide at for a player `max_width` pixels wide: the smallest
/// of 1, ½, ¼, ⅛ that still fills it (whole fractions are much faster to render than others),
/// no more than `quality` allows.
pub fn preview_scale(w: u32, max_width: u32, quality: &str) -> f32 {
    let cap = match quality {
        "half" => 0.5,
        "quarter" => 0.25,
        _ => 1.0,
    };
    let want = f64::from(max_width.max(16)) / f64::from(w.max(1));
    let mut s = 1.0f32;
    while s > 0.125 && f64::from(s / 2.0) >= want {
        s /= 2.0;
    }
    s.min(cap)
}

/// Renders frames of the sequence as it was when the feed was made, from any thread.
#[derive(Clone)]
pub struct FrameFeed {
    pub(crate) project: Arc<Project>,
    pub(crate) sequence: ItemId,
    provider: Arc<PoolProvider>,
    scale: f32,
    rate: FrameRate,
    /// Frames in the sequence.
    pub frames: i64,
}

impl FrameFeed {
    /// Frame `n` (clamped to the sequence). `draft` lets decoders take shortcuts on frames nothing
    /// else depends on: for playback at ½ size or less, never for a paused frame.
    pub fn render(&self, n: i64, draft: bool) -> Result<Frame> {
        let n = n.clamp(0, (self.frames - 1).max(0));
        let t = self.rate.tick_of(n);
        let opts = filmcraft_render::RenderOptions { scale: self.scale, captions: true, ..Default::default() };
        // as the engine's monitor draws it: a frame that is one opaque 8-bit picture as it is goes
        // straight to bytes, without the float picture in between
        let (w, h, mut rgba) = filmcraft_media::cancel::with_draft(draft && self.scale <= 0.5, || {
            filmcraft_render::render_sequence_rgba8(&self.project, self.sequence, t, opts, self.provider.as_ref())
        });
        // over black, opaque (the bytes keep the picture's alpha for monitors that show it)
        for a in rgba.iter_mut().skip(3).step_by(4) {
            *a = 255;
        }
        let w = u32::try_from(w).map_err(engine)?;
        let h = u32::try_from(h).map_err(engine)?;
        Ok(Frame { w, h, rgba })
    }

    /// The frame showing at `seconds`.
    pub fn frame_at(&self, seconds: f64) -> i64 {
        if !seconds.is_finite() {
            return 0;
        }
        self.rate.frame_at(Tick::from_seconds_f64(seconds.max(0.0)))
    }

    pub fn fps(&self) -> f64 {
        self.rate.as_f64()
    }
}

impl Studio {
    /// A feed of the sequence's frames for a player `max_width` pixels wide at `quality` (one
    /// of [`QUALITIES`]).
    pub fn frame_feed(&self, max_width: u32, quality: &str) -> Result<FrameFeed> {
        let info = self.info()?;
        let sequence = self.session.state.active_sequence.ok_or(Error::NothingOpen)?;
        let rate = self.fps()?;
        let scale = preview_scale(info.width, max_width, quality);
        let frames = rate.frame_at(Tick::from_seconds_f64(info.duration_seconds)).max(1);
        let provider = Arc::new(self.session.media.provider(self.session.project.clone(), self.session.services.clone()));
        Ok(FrameFeed { project: self.session.project.clone(), sequence, provider, scale, rate, frames })
    }

    /// The whole picture (every lane, text, graphics, captions) at the video's own size, from the
    /// original media (never proxies): a still of what the timeline shows.
    pub fn still_feed(&self) -> Result<FrameFeed> {
        let info = self.info()?;
        let mut feed = self.frame_feed(info.width.max(16), "full")?;
        feed.provider = Arc::new(self.session.media.full_res_provider(self.session.project.clone(), self.session.services.clone()));
        Ok(feed)
    }

    /// The frame at `seconds`, at most `max_width` wide, without the clips `hide` (a graphic being
    /// changed, so it isn't under its new version): the picture a graphic is designed over.
    pub fn frame_without(&self, seconds: f64, max_width: u32, hide: &[u64]) -> Result<Frame> {
        let mut feed = self.frame_feed(max_width, "auto")?;
        if !hide.is_empty() {
            let mut p = (*feed.project).clone();
            if let Some(q) = p.sequence_mut(feed.sequence) {
                for t in q.video_tracks.iter_mut() {
                    for i in t.items.iter_mut().filter(|i| hide.contains(&i.id.0)) {
                        i.enabled = false;
                    }
                }
            }
            feed.project = Arc::new(p);
        }
        if !seconds.is_finite() {
            return Err(Error::Invalid("the time must be a number".into()));
        }
        feed.render(feed.frame_at(seconds), false)
    }

    /// The paused player's frame at `seconds`, for a player `max_width` wide at `quality`.
    pub fn preview_frame(&self, seconds: f64, max_width: u32, quality: &str) -> Result<Frame> {
        if !seconds.is_finite() {
            return Err(Error::Invalid("the time must be a number".into()));
        }
        let feed = self.frame_feed(max_width, quality)?;
        feed.render(feed.frame_at(seconds), false)
    }
}

/// A video media file for the Playback check: where it is, what it is, and a source of its own
/// (the original file opened afresh, never a proxy or the player's cached one), to read and decode
/// from any thread without the studio.
pub struct MediaProbe {
    pub name: String,
    pub path: String,
    pub info: filmcraft_media::MediaInfo,
    pub source: filmcraft_media::SharedSource,
    /// Where the first clip showing it is on the timeline (start, end in seconds), to time the
    /// player there.
    pub on_timeline: Option<(f64, f64)>,
    /// Three moments of that clip (a fifth, half and four fifths of the way through): timeline
    /// seconds and the media seconds showing there, for the Colour check.
    pub moments: Vec<(f64, f64)>,
}

impl Studio {
    /// The video media files in the project, for the Playback check (at most `max`).
    pub fn media_probes(&self, max: usize) -> Vec<MediaProbe> {
        let p = &self.session.project;
        let mut items: Vec<_> = p
            .items
            .values()
            .filter_map(|i| {
                let m = i.as_media()?;
                let filmcraft_project::MediaRef::File { path } = &m.media else { return None };
                (m.info.kind == filmcraft_media::MediaKind::Movie && m.info.video.is_some())
                    .then(|| (i.created, i.id, i.name.clone(), path.clone(), m.info.clone()))
            })
            .collect();
        items.sort_by_key(|(created, ..)| *created);
        items
            .into_iter()
            .filter_map(|(_, id, name, path, info)| {
                // the file the engine actually reads (relinked, followed)
                let path = self.session.media.follow(&path).unwrap_or(path);
                // a fresh copy of the file, not the player's: its cache would hide the decoding
                let source = self.session.media.open_file(&path, &*self.session.services).ok()?;
                let first = self.session.active_sequence().and_then(|q| {
                    let c = q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|c| c.item == id && c.enabled).min_by_key(|c| c.start)?;
                    Some((c.clone(), q.settings.frame_rate))
                });
                let on_timeline = first.as_ref().map(|(c, _)| (c.start.seconds(), c.end().seconds()));
                let moments = first
                    .map(|(c, rate)| {
                        [0.2, 0.5, 0.8]
                            .iter()
                            .map(|f| {
                                let t = rate.snap(c.start + Tick((c.duration.0 as f64 * f) as i64));
                                (t.seconds(), c.source_time_at(t).seconds())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(MediaProbe { name, path, info, source, on_timeline, moments })
            })
            .take(max)
            .collect()
    }

    /// Starts making proxies of the video media `items` (all video when empty): small ProRes
    /// copies, about 960 pixels wide, in the project's folder (ClipCut's data folder while the
    /// project isn't saved). Returns the export-style job id;
    /// [`Studio::poll`] attaches them when the job finishes.
    pub fn make_proxies(&mut self, items: &[u64]) -> Result<u64> {
        let p = &self.session.project;
        let videos: Vec<(u64, u32)> = p
            .items
            .values()
            .filter_map(|i| {
                let m = i.as_media()?;
                let v = m.info.video.as_ref()?;
                (m.info.kind == filmcraft_media::MediaKind::Movie && (items.is_empty() || items.contains(&i.id.0))).then_some((i.id.0, v.width))
            })
            .collect();
        if videos.is_empty() {
            return Err(Error::Invalid("there is no video to make proxies of".into()));
        }
        // 4K and up at a quarter, the rest at half: about 960 wide either way
        let preset = if videos.iter().any(|(_, w)| *w >= 2560) { "prores_proxy_quarter" } else { "prores_proxy_half" };
        let mut q = serde_json::json!({"items": videos.iter().map(|(i, _)| i).collect::<Vec<_>>(), "preset": preset});
        // beside the project when it is saved, else in ClipCut's own folder on this computer (not
        // next to the media, which may be on a slow or removable drive)
        let dir = self
            .session
            .path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).parent())
            .or_else(|| self.session.prefs_path.as_deref().and_then(std::path::Path::parent));
        if let Some(dir) = dir {
            q["destination"] = serde_json::json!(dir.join("Proxies").to_string_lossy());
        }
        let r = self.session.execute("media.createProxies", q).map_err(engine)?;
        r.get("job").and_then(serde_json::Value::as_u64).ok_or_else(|| Error::Engine("the proxies were not started".into()))
    }

    /// Applies what background jobs finished (attaches made proxies). Returns true when something
    /// changed.
    pub fn poll(&mut self) -> bool {
        let before = self.session.project.items.values().filter(|i| i.as_media().is_some_and(|m| m.proxy.is_some())).count();
        self.session.poll_persistence();
        before != self.session.project.items.values().filter(|i| i.as_media().is_some_and(|m| m.proxy.is_some())).count()
    }

    /// Whether the player uses proxies where clips have them (exports never do).
    pub fn set_proxies(&mut self, on: bool) -> Result<()> {
        self.session.execute("media.toggleProxies", serde_json::json!({"enabled": on})).map(|_| ()).map_err(engine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_snap_to_whole_fractions() {
        assert_eq!(preview_scale(1920, 800, "auto"), 0.5);
        assert_eq!(preview_scale(1920, 960, "auto"), 0.5);
        assert_eq!(preview_scale(1920, 961, "auto"), 1.0);
        assert_eq!(preview_scale(1920, 3000, "auto"), 1.0);
        assert_eq!(preview_scale(1920, 400, "auto"), 0.25);
        assert_eq!(preview_scale(3840, 100, "auto"), 0.125);
        assert_eq!(preview_scale(1920, 1600, "half"), 0.5);
        assert_eq!(preview_scale(1920, 1600, "quarter"), 0.25);
        assert_eq!(preview_scale(1920, 300, "half"), 0.25, "a cap never makes it sharper");
        assert_eq!(preview_scale(1920, 1600, "full"), 1.0);
        assert_eq!(preview_scale(0, 0, "auto"), 1.0, "nothing to shrink");
    }
}
