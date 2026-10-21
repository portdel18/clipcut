//! CapCut-style captions and transcript cutting.
//!
//! The speech on the main track is transcribed by Whisper (pure Rust, on the CPU) away from the
//! session, so the window stays usable and the run can be cancelled: [`Studio::transcribe_job`]
//! gathers the sound, [`TranscribeJob::run`] transcribes it on any thread, and
//! [`Studio::finish_captions`] stores the transcripts and lays the words out as captions on the
//! first caption track (`C1`), one undo step.
//!
//! Captions follow the speech. Each caption remembers the media time it was said at (in its cue
//! id, `cc:<media item>:<from>:<to>`, which the project file keeps), and after every main-track
//! edit (cuts, trims, reorders, speed) [`Studio::group_follow`] puts every caption back over its
//! words wherever they now play, shortens it when part of them was cut, and drops it when all of
//! them were.
//!
//! Transcript cutting deletes words from the main track: [`Studio::cut_words`],
//! [`Studio::remove_fillers`] and [`Studio::remove_pauses`].

use std::path::Path;
use std::sync::Arc;

use filmcraft_edit::transcript::{self as tx, CaptionRules, SeqWord};
use filmcraft_project::caption::{Caption, CaptionAnchor, CaptionTrack};
use filmcraft_project::transcript::Transcript;
use filmcraft_project::{ClipId, ItemId, Sequence, TrackItem};
use filmcraft_speech::{Options, Transcriber, models};
use filmcraft_time::{Tick, TimeRange};
use serde::Serialize;
use serde_json::{Value, json};

use crate::lanes::{is_main_sound, main_links};
use crate::{Error, Result, Studio, engine};

/// The caption track as the timeline and the Captions tab show it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsView {
    /// False when there is no caption track yet.
    pub exists: bool,
    /// Shown in the player and burnt into exports.
    pub on: bool,
    pub style: CaptionStyleView,
    pub items: Vec<CaptionView>,
    /// Media items on the main track that have a transcript, and that don't.
    pub transcribed: usize,
    pub untranscribed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionView {
    pub id: u64,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionStyleView {
    pub font: String,
    /// Pixels at 1080 lines.
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
    pub background: bool,
    pub background_color: String,
    /// How solid the box is, 0–100.
    pub background_opacity: f64,
    pub outline: f64,
    pub outline_color: String,
    /// `top`, `middle` or `bottom`.
    pub position: String,
    /// `left`, `center` or `right`.
    pub align: String,
    /// Distance from the edge it sits at, as a share of the frame's height (0–0.45).
    pub margin: f64,
    /// Line height, as a multiple of the size.
    pub line_spacing: f64,
}

/// A word of the main track's speech, in timeline time.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordView {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub speaker: Option<String>,
}

/// A speech model the Captions tab offers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Download size, bytes.
    pub size: u64,
    pub installed: bool,
}

/// How captions are cut from the words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub max_chars: usize,
    pub lines: usize,
}

impl Layout {
    /// Short single lines, CapCut-style: about five words across a wide frame, three or four
    /// on a tall one.
    pub fn for_canvas(width: u32, height: u32) -> Self {
        if height > width { Self { max_chars: 18, lines: 2 } } else { Self { max_chars: 32, lines: 1 } }
    }
}

/// Where a caption's words are in their media: media item and media-time range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    item: u64,
    from: Tick,
    to: Tick,
}

impl Anchor {
    const PREFIX: &'static str = "cc:";
    fn parse(s: Option<&str>) -> Option<Self> {
        let mut p = s?.strip_prefix(Self::PREFIX)?.split(':');
        let item = p.next()?.parse().ok()?;
        let from = Tick(p.next()?.parse().ok()?);
        let to = Tick(p.next()?.parse().ok()?);
        (to > from).then_some(Self { item, from, to })
    }
    fn id(&self) -> String {
        format!("{}{}:{}:{}", Self::PREFIX, self.item, self.from.0, self.to.0)
    }
}

/// A caption about to be put back: its text and where its words are.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    id: ClipId,
    start: Tick,
    end: Tick,
    text: String,
    speaker: Option<String>,
    settings: String,
    anchor: Option<Anchor>,
    /// It still says exactly the words heard under it (so it may be rewritten when some go).
    auto: bool,
}

/// The main track's sound clips (audio track 1, and its pictures' sound extracted to lanes), in
/// order.
fn main_sound(q: &Sequence) -> Vec<&TrackItem> {
    let links = main_links(q);
    let mut v: Vec<&TrackItem> = q
        .audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|i| i.enabled && !i.reverse && i.speed > 0.0 && i.frame_hold.is_none() && is_main_sound(q, &links, i))
        .collect();
    v.sort_by_key(|i| i.start);
    v
}

/// Sound on the lanes below the main track (extracted from a main-track clip, say): captions over
/// words that play only there stay on them.
fn lane_sound(q: &Sequence) -> Vec<&TrackItem> {
    let mut v: Vec<&TrackItem> =
        q.audio_tracks.iter().skip(1).flat_map(|t| t.items.iter()).filter(|i| i.enabled && !i.reverse && i.speed > 0.0 && i.frame_hold.is_none()).collect();
    v.sort_by_key(|i| i.start);
    v
}

/// Timeline time at which clip `c` plays media time `m`.
fn timeline_at(c: &TrackItem, m: Tick) -> Tick {
    let speed = c.speed.abs().max(1e-6);
    let d = (m.0 - c.source_in.0) as f64 / speed;
    Tick(c.start.0.saturating_add(d.round() as i64))
}

/// Where the words of `[start, end)` (timeline) are in their media, from the main-track sound
/// clip under the middle of it.
fn anchor_at(q: &Sequence, start: Tick, end: Tick) -> Option<Anchor> {
    let mid = Tick(start.0 + (end.0 - start.0) / 2);
    let c = main_sound(q).into_iter().find(|c| c.start <= mid && c.end() > mid)?;
    let (a, b) = (start.max(c.start), end.min(c.end()));
    let a = Anchor { item: c.item.0, from: c.source_time_at(a), to: c.source_time_at(b) };
    (a.to > a.from).then_some(a)
}

fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The main track's words, in order: heard on its sound track, or on lanes from its pictures'
/// extracted sound (sound unlinked from them is a clip of its own and isn't in the transcript).
pub(crate) fn main_words(q: &Sequence, p: &filmcraft_project::Project) -> Vec<SeqWord> {
    let links = main_links(q);
    let main: std::collections::HashSet<ClipId> =
        q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| is_main_sound(q, &links, i)).map(|i| i.id).collect();
    tx::sequence_words(q, &p.transcripts).into_iter().filter(|w| main.contains(&w.clip)).collect()
}

/// Transcribes media on any thread (see [`Studio::transcribe_job`]).
pub struct TranscribeJob {
    transcriber: Arc<dyn Transcriber>,
    /// Media item, its source and length.
    sources: Vec<(u64, filmcraft_media::SharedSource, Tick)>,
    options: Options,
}

/// The longest sound transcribed in one go: six hours.
const MAX_SECONDS: i64 = 6 * 3600;

impl TranscribeJob {
    /// How many media files it will transcribe.
    pub fn len(&self) -> usize {
        self.sources.len()
    }
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Transcribes every file. `progress(fraction, status)` returns false to cancel.
    pub fn run(&self, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<Vec<(u64, Transcript)>> {
        let sr = filmcraft_speech::SAMPLE_RATE;
        let total: f64 = self.sources.iter().map(|(_, _, d)| d.seconds().max(0.1)).sum::<f64>().max(0.1);
        let mut done = 0.0;
        let mut out = Vec::new();
        for (n, (item, src, dur)) in self.sources.iter().enumerate() {
            let share = dur.seconds().max(0.1) / total;
            let label = if self.sources.len() > 1 { format!("file {} of {}", n + 1, self.sources.len()) } else { String::new() };
            // the sound, 16 kHz mono, read in 30 s blocks (the first tenth of this file's share)
            let len = dur.to_units_floor(i64::from(sr)).clamp(0, MAX_SECONDS * i64::from(sr));
            let mut mono: Vec<f32> = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
            let block = i64::from(sr) * 30;
            let mut at = 0;
            while at < len {
                let k = (len - at).min(block);
                let buf = src.audio(at, usize::try_from(k).unwrap_or(0), sr).map_err(|e| Error::Engine(format!("the sound couldn't be read: {e}")))?;
                mono.extend(filmcraft_speech::downmix(&buf.channels));
                at += k;
                let f = done + share * 0.1 * (at as f64 / len.max(1) as f64);
                if !progress(f as f32, format!("Reading the sound {label}").trim_end()) {
                    return Err(Error::Invalid("cancelled".into()));
                }
            }
            let base = done + share * 0.1;
            let mut cb = |f: f32, _status: &str| progress((base + share * 0.9 * f64::from(f.clamp(0.0, 1.0))) as f32, format!("Listening {label}").trim_end());
            let t = self.transcriber.transcribe(&mono, &self.options, &mut cb).map_err(|e| match e {
                filmcraft_speech::SpeechError::Cancelled => Error::Invalid("cancelled".into()),
                e => Error::Engine(e.to_string()),
            })?;
            out.push((*item, t));
            done += share;
        }
        progress(1.0, "Done");
        Ok(out)
    }
}

/// The speech models, smallest first, and which are downloaded into `models_dir`.
pub fn speech_models(models_dir: &Path) -> Vec<ModelView> {
    models::catalogue()
        .iter()
        .map(|m| ModelView {
            id: m.id.into(),
            name: m.name.into(),
            description: m.description.into(),
            size: m.size(),
            installed: models::installed(models_dir, m),
        })
        .collect()
}

/// Downloads a speech model into `models_dir` (checked against its published checksums).
/// `progress(done, total)` returns false to cancel.
pub fn download_model(models_dir: &Path, id: &str, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<()> {
    let m = models::find(id).ok_or_else(|| Error::Invalid(format!("there is no speech model called {id}")))?;
    let mut cb = |done: u64, total: u64, _file: &str| progress(done, total);
    models::download(models_dir, m, &mut cb).map_err(|e| match e {
        filmcraft_speech::SpeechError::Cancelled => Error::Invalid("cancelled".into()),
        e => Error::Engine(format!("the speech model couldn't be downloaded ({e}). Check the internet connection and try again")),
    })
}

/// Caption looks the Captions tab offers (the keys of [`Studio::set_caption_style`]).
pub fn caption_presets() -> Value {
    json!([
        {"name": "Clean", "style": {"font": "Inter", "size": 60, "color": "#ffffff", "background": false, "outline": 4, "outlineColor": "#000000", "position": "bottom"}},
        {"name": "Box", "style": {"font": "Inter", "size": 54, "color": "#ffffff", "background": true, "backgroundColor": "#000000c0", "outline": 0, "position": "bottom"}},
        {"name": "Yellow", "style": {"font": "Inter", "size": 64, "color": "#ffd400", "background": false, "outline": 5, "outlineColor": "#000000", "position": "bottom"}},
        {"name": "Big", "style": {"font": "Inter", "size": 84, "color": "#ffffff", "background": false, "outline": 6, "outlineColor": "#000000", "position": "middle"}},
        {"name": "Top", "style": {"font": "Inter", "size": 54, "color": "#ffffff", "background": true, "backgroundColor": "#1b1b1fd0", "outline": 0, "position": "top"}},
        {"name": "Serif", "style": {"font": "Noto Serif", "size": 58, "color": "#f4f0e6", "background": false, "outline": 3, "outlineColor": "#000000", "position": "bottom"}}
    ])
}

impl Studio {
    fn caption_track(&self) -> Option<&CaptionTrack> {
        self.session.active_sequence()?.caption_tracks.first()
    }

    /// The caption track, for the timeline view.
    pub(crate) fn captions_view(&self) -> CaptionsView {
        let Some(q) = self.session.active_sequence() else { return CaptionsView::default() };
        let mut items: Vec<u64> = main_sound(q).iter().map(|c| c.item.0).collect();
        items.sort_unstable();
        items.dedup();
        let transcribed = items.iter().filter(|i| self.session.project.transcripts.contains_key(&ItemId(**i))).count();
        let mut v = CaptionsView { transcribed, untranscribed: items.len() - transcribed, ..Default::default() };
        let Some(t) = q.caption_tracks.first() else { return v };
        let st = &t.style;
        v.exists = true;
        v.on = t.enabled;
        v.style = CaptionStyleView {
            font: st.font.clone(),
            size: f64::from(st.size),
            color: hex(st.color),
            background: st.background,
            background_color: hex(st.background_color),
            background_opacity: (f64::from(st.background_color[3]) / 2.55).round(),
            outline: f64::from(st.outline),
            outline_color: hex(st.outline_color),
            position: match st.anchor {
                CaptionAnchor::Top => "top",
                CaptionAnchor::Middle => "middle",
                CaptionAnchor::Bottom => "bottom",
            }
            .into(),
            align: match st.align {
                filmcraft_project::caption::CaptionAlign::Left => "left",
                filmcraft_project::caption::CaptionAlign::Right => "right",
                _ => "center",
            }
            .into(),
            margin: f64::from(st.margin),
            line_spacing: f64::from(st.line_spacing),
        };
        v.items = t
            .captions
            .iter()
            .map(|c| CaptionView { id: c.id.0, start: c.start.seconds(), end: c.end().seconds(), text: c.text.clone(), speaker: c.speaker.clone() })
            .collect();
        v
    }

    /// The main track's words in timeline time (the transcript panel).
    pub fn words(&self) -> Vec<WordView> {
        let Some(q) = self.session.active_sequence() else { return Vec::new() };
        main_words(q, &self.session.project)
            .into_iter()
            .map(|w| WordView { text: w.text, start: w.start.seconds(), end: w.end.seconds(), speaker: w.speaker })
            .collect()
    }

    /// Instead of a speech model, hears every clip saying `transcript` (JSON as a project file
    /// stores one): for tests, CI and agents checking captions without downloading a model.
    pub fn hear(&mut self, transcript: &str) -> Result<()> {
        let t: Transcript = serde_json::from_str(transcript).map_err(|e| Error::Invalid(format!("that transcript can't be read: {e}")))?;
        self.session.transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: t, id: "test".into() }));
        Ok(())
    }

    /// Gets ready to transcribe the main track's speech: each media file on it once (all of them
    /// with `again`, else only those without a transcript), with model `model` from `models_dir`.
    /// `language` is an ISO 639-1 code, or `None` to detect it.
    pub fn transcribe_job(&self, models_dir: &Path, model: &str, language: Option<String>, again: bool) -> Result<TranscribeJob> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mut items: Vec<ItemId> = main_sound(q).iter().map(|c| c.item).collect();
        items.sort_unstable();
        items.dedup();
        if items.is_empty() {
            return Err(Error::Invalid("there is no speech on the main track to caption: add a clip with sound first".into()));
        }
        items.retain(|i| again || !self.session.project.transcripts.contains_key(i));
        let transcriber = match &self.session.transcriber {
            Some(t) => t.clone(),
            None => filmcraft_speech::load(models_dir, model).map_err(|e| match e {
                filmcraft_speech::SpeechError::NotInstalled(_) => Error::Invalid("the speech model isn't downloaded yet".into()),
                e => Error::Engine(e.to_string()),
            })?,
        };
        let mut sources = Vec::new();
        for i in items {
            let Some(it) = self.session.project.item(i) else { continue };
            if !it.has_audio() {
                continue;
            }
            let src = self.session.source(i).ok_or_else(|| Error::Engine(format!("{} couldn't be opened", it.name)))?;
            sources.push((i.0, src, it.duration()));
        }
        let language = language.filter(|l| !l.trim().is_empty() && l != "auto");
        Ok(TranscribeJob { transcriber, sources, options: Options { language, diarize: true, max_speakers: 6 } })
    }

    /// Stores transcripts and (re)makes the captions from every transcript on the main track,
    /// laid out for the canvas, as one undo step. Returns how many captions there are.
    pub fn finish_captions(&mut self, results: Vec<(u64, Transcript)>) -> Result<usize> {
        self.group("Auto Captions", |s| {
            for (item, t) in &results {
                let v = serde_json::to_value(t).map_err(engine)?;
                s.session.execute("transcript.set", json!({"item": item, "transcript": v})).map_err(engine)?;
            }
            s.lay_out_captions()
        })
    }

    /// Replaces the caption track's captions with ones laid out from the words (keeping its
    /// style), making the track if there is none.
    fn lay_out_captions(&mut self) -> Result<usize> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let words = main_words(q, &self.session.project);
        if words.is_empty() {
            return Err(Error::Invalid("no speech was heard on the main track".into()));
        }
        let rate = q.settings.frame_rate;
        let layout = Layout::for_canvas(q.settings.width, q.settings.height);
        let rules = CaptionRules {
            max_chars: layout.max_chars,
            lines: layout.lines,
            min_duration: Tick::from_seconds_f64(0.7),
            max_duration: Tick::from_seconds_f64(4.0),
            gap_frames: 0,
            break_pause: Tick::from_seconds_f64(0.8),
        };
        // a caption never runs across a cut: each clip's words are laid out on their own
        let mut blocks = Vec::new();
        let mut from = 0;
        for k in 1..=words.len() {
            if words.get(k).is_none_or(|w| words.get(k - 1).is_some_and(|p| p.clip != w.clip)) {
                if let Some(run) = words.get(from..k) {
                    blocks.extend(tx::caption_blocks(run, &rules, rate));
                }
                from = k;
            }
        }
        let fresh = q.caption_tracks.is_empty();
        if fresh {
            self.session.execute("captions.newTrack", json!({"format": "Subtitle", "name": "Captions"})).map_err(engine)?;
            let presets = caption_presets();
            if let Some(o) = presets.get(0).and_then(|p| p.get("style")).and_then(Value::as_object) {
                self.session.execute("captions.setStyle", style_params(o)).map_err(engine)?;
            }
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let held: Vec<Held> = blocks
            .iter()
            .map(|b| Held {
                id: ClipId(0),
                start: b.start,
                end: b.end,
                text: b.text.clone(),
                speaker: b.speaker.clone(),
                settings: String::new(),
                anchor: anchor_at(q, b.start, b.end),
                auto: true,
            })
            .collect();
        let n = held.len();
        self.write_captions("Auto Captions", held)?;
        Ok(n)
    }

    /// The captions with where their words are, before an edit that moves the main track
    /// ([`Studio::group_follow`]). `None` when there are none.
    pub(crate) fn hold_captions(&self) -> Option<Vec<Held>> {
        let q = self.session.active_sequence()?;
        let t = q.caption_tracks.first()?;
        if t.captions.is_empty() || t.locked {
            return None;
        }
        let words = main_words(q, &self.session.project);
        Some(
            t.captions
                .iter()
                .map(|c| Held {
                    id: c.id,
                    start: c.start,
                    end: c.end(),
                    text: c.text.clone(),
                    speaker: c.speaker.clone(),
                    settings: c.settings.clone(),
                    anchor: Anchor::parse(c.cue_id.as_deref()).or_else(|| anchor_at(q, c.start, c.end())),
                    auto: same_words(&c.text, &heard(&words, c.start, c.end())),
                })
                .collect(),
        )
    }

    /// Puts held captions back over their words as the main track now plays them.
    pub(crate) fn place_captions(&mut self, held: Vec<Held>) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let rate = q.settings.frame_rate;
        let layout = Layout::for_canvas(q.settings.width, q.settings.height);
        let sound = main_sound(q);
        let lanes = lane_sound(q);
        let mut placed: Vec<Held> = Vec::new();
        for h in held {
            let Some(a) = h.anchor else {
                // a caption over no speech stays where it is
                placed.push(h);
                continue;
            };
            // every place its words still play (a clip used twice shows them twice)
            let mut pieces: Vec<Held> = Vec::new();
            // words that no longer play on the main track but do on a lane are where the lane
            // plays them, as they are (the transcript is the main track's)
            let on_main = sound.iter().any(|c| c.item.0 == a.item && c.source_in < a.to && c.source_out() > a.from);
            let from_lane = !on_main && lanes.iter().any(|c| c.item.0 == a.item && c.source_in < a.to && c.source_out() > a.from);
            let auto = h.auto && !from_lane;
            for c in if from_lane { &lanes } else { &sound }.iter().filter(|c| c.item.0 == a.item) {
                let (si, so) = (c.source_in, c.source_out());
                let (from, to) = (a.from.max(si), a.to.min(so));
                if to <= from {
                    continue;
                }
                let (start, end) = (rate.snap(timeline_at(c, from)), rate.snap(timeline_at(c, to)));
                if end > start {
                    pieces.push(Held { start, end, anchor: Some(Anchor { item: a.item, from, to }), auto, ..h.clone() });
                }
            }
            // pieces that now play one after the other (a word cut out of the middle) are one
            // caption again
            pieces.sort_by_key(|p| p.start);
            for p in pieces {
                if let Some(last) = placed.last_mut()
                    && last.id == p.id
                    && last.anchor.is_some_and(|l| l.item == a.item)
                    && (p.start - last.end).0.abs() <= rate.frame_duration().0
                {
                    last.end = p.end;
                    last.anchor = last.anchor.map(|l| Anchor {
                        item: l.item,
                        from: l.from.min(p.anchor.map_or(l.from, |x| x.from)),
                        to: l.to.max(p.anchor.map_or(l.to, |x| x.to)),
                    });
                } else {
                    placed.push(p);
                }
            }
        }
        // a caption that still said just its words says what is left of them
        let words = main_words(q, &self.session.project);
        placed.retain_mut(|h| {
            if !h.auto || h.anchor.is_none() {
                return true;
            }
            let left = heard(&words, h.start, h.end);
            if left.trim().is_empty() {
                return false;
            }
            if !same_words(&h.text, &left) {
                h.text = wrap(&left, layout);
            }
            true
        });
        placed.sort_by_key(|h| h.start);
        let current: Vec<(Tick, Tick, &str)> =
            q.caption_tracks.first().map(|t| t.captions.iter().map(|c| (c.start, c.end(), c.text.as_str())).collect()).unwrap_or_default();
        let next: Vec<(Tick, Tick, &str)> = placed.iter().map(|h| (h.start, h.end, h.text.as_str())).collect();
        if current == next {
            return Ok(());
        }
        self.write_captions("Captions Follow", placed)
    }

    /// Writes `held` as the caption track's captions (sorted, no overlaps, each at least a frame),
    /// each carrying where its words are.
    fn write_captions(&mut self, label: &str, mut held: Vec<Held>) -> Result<()> {
        held.sort_by_key(|h| h.start);
        self.session
            .edit_sequence(label, |q, ctx, _| {
                let min = q.settings.frame_rate.frame_duration();
                let Some(t) = q.caption_tracks.first_mut() else { return Ok(()) };
                let mut used = std::collections::HashSet::new();
                let mut out: Vec<Caption> = Vec::with_capacity(held.len());
                for (k, h) in held.iter().enumerate() {
                    let end = held.get(k + 1).map_or(h.end, |n| h.end.min(n.start));
                    if end - h.start < min {
                        continue;
                    }
                    // the first copy keeps its id (selection and links survive)
                    let id = if h.id.0 != 0 && used.insert(h.id) { h.id } else { ClipId(ctx.alloc()) };
                    out.push(Caption {
                        id,
                        start: h.start,
                        duration: end - h.start,
                        text: h.text.clone(),
                        speaker: h.speaker.clone(),
                        cue_id: h.anchor.map(|a| a.id()),
                        settings: h.settings.clone(),
                    });
                }
                t.captions = out;
                Ok(())
            })
            .map_err(engine)
    }

    /// [`Studio::group`] for an edit that moves the main track: the captions follow its speech
    /// and the transitions stay on their cuts.
    pub(crate) fn group_follow<R>(&mut self, label: &str, merge: Option<&str>, f: impl FnOnce(&mut Self) -> Result<R>) -> Result<R> {
        let held = self.hold_captions();
        let transitions = self.hold_transitions();
        self.group_merge(label, merge, |s| {
            let r = f(s)?;
            s.keep_sound_in_step()?;
            s.place_transitions(transitions)?;
            if let Some(h) = held {
                s.place_captions(h)?;
            }
            Ok(r)
        })
    }

    /* ---------- editing captions ---------- */

    pub fn set_caption_text(&mut self, id: u64, text: &str) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return self.delete_captions(&[id]);
        }
        self.group("Edit Caption", |s| s.session.execute("captions.setText", json!({"caption": id, "text": text})).map(|_| ()).map_err(engine))
    }

    pub fn delete_captions(&mut self, ids: &[u64]) -> Result<()> {
        if ids.is_empty() {
            return Err(Error::Invalid("select a caption first".into()));
        }
        self.group("Delete Caption", |s| s.session.execute("captions.delete", json!({"captions": ids})).map(|_| ()).map_err(engine))
    }

    /// Moves a caption to `[start, end)` seconds; it then belongs to the words there.
    pub fn retime_caption(&mut self, id: u64, start: f64, end: f64) -> Result<()> {
        let (a, b) = (self.frame_tick(start)?, self.frame_tick(end)?);
        let mut held = self.hold_captions().ok_or_else(|| Error::Invalid("there are no captions".into()))?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let min = q.settings.frame_rate.frame_duration();
        let h = held.iter_mut().find(|h| h.id.0 == id).ok_or_else(|| Error::Invalid("that caption is no longer there".into()))?;
        if b - a < min {
            return Err(Error::Invalid("a caption must be at least a frame long".into()));
        }
        (h.start, h.end, h.anchor) = (a, b, anchor_at(q, a, b));
        // it may not cover its neighbours: they keep their place, it gives way
        let me = h.clone();
        let others: Vec<&Held> = held.iter().filter(|o| o.id.0 != id).collect();
        if others.iter().any(|o| o.start < me.end && o.end > me.start) {
            return Err(Error::Invalid("captions can't overlap: move it into a free place".into()));
        }
        self.group("Move Caption", |s| s.write_captions("Move Caption", held))
    }

    /// The caption track's look: `font`, `size` (pixels at 1080 lines), `color`, `background`,
    /// `backgroundColor`, `outline`, `outlineColor` and `position` (`top`, `middle`, `bottom`).
    /// Changes the captions' look: any of `font`, `size`, `color`, `background`,
    /// `backgroundColor`, `backgroundOpacity` (0–100), `outline`, `outlineColor`, `position`,
    /// `align`, `margin` (share of the height) and `lineSpacing`. A `merge` key (a slider being
    /// dragged) folds the changes it names into one undo step.
    pub fn set_caption_style(&mut self, style: &Value) -> Result<()> {
        let o = style.as_object().ok_or_else(|| Error::Invalid("nothing to change".into()))?;
        let track = self.caption_track().ok_or_else(|| Error::Invalid("make captions first".into()))?;
        let mut o = o.clone();
        // the box's colour and how solid it is go to the engine as one #rrggbbaa
        if o.contains_key("backgroundOpacity") || o.get("backgroundColor").and_then(Value::as_str).is_some_and(|c| c.len() == 7) {
            let cur = track.style.background_color;
            let rgb = o
                .get("backgroundColor")
                .and_then(Value::as_str)
                .filter(|c| c.len() >= 7)
                .map(|c| c[1..7].to_string())
                .unwrap_or_else(|| hex(cur)[1..].to_string());
            let alpha =
                o.get("backgroundOpacity").and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(cur[3], |v| (v.clamp(0.0, 100.0) * 2.55).round() as u8);
            o.insert("backgroundColor".into(), json!(format!("#{rgb}{alpha:02x}")));
        }
        let merge = o.get("merge").and_then(Value::as_str).map(|m| format!("capstyle:{m}"));
        let p = style_params(&o);
        self.group_merge("Caption Style", merge.as_deref(), |s| s.session.execute("captions.setStyle", p).map(|_| ()).map_err(engine))
    }

    /// Shows or hides the captions (in the player and in exports).
    pub fn set_captions_on(&mut self, on: bool) -> Result<()> {
        let t = self.caption_track().ok_or_else(|| Error::Invalid("make captions first".into()))?.id.0;
        self.group(if on { "Show Captions" } else { "Hide Captions" }, |s| {
            s.session.execute("captions.setTrack", json!({"track": t, "enabled": on})).map(|_| ()).map_err(engine)
        })
    }

    /// Writes the captions to a subtitle file (`.srt` or `.vtt`, by its extension).
    pub fn export_captions(&mut self, path: &str) -> Result<usize> {
        self.caption_track().filter(|t| !t.captions.is_empty()).ok_or_else(|| Error::Invalid("there are no captions to save".into()))?;
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let format = if ext == "vtt" { "vtt" } else { "srt" };
        let r = self.session.execute("captions.export", json!({"path": path, "format": format})).map_err(engine)?;
        Ok(r.get("captions").and_then(Value::as_u64).unwrap_or(0) as usize)
    }

    /// Whether exports should have the captions burnt in.
    pub(crate) fn burn_captions(&self) -> bool {
        self.caption_track().is_some_and(|t| t.enabled && !t.captions.is_empty())
    }

    /* ---------- transcript cutting ---------- */

    /// Cuts words `from..=to` (indices into [`Studio::words`]) out of the main track; it closes
    /// up and the captions follow. One undo step.
    pub fn cut_words(&mut self, from: usize, to: usize) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let words = main_words(q, &self.session.project);
        let (a, b) = (from.min(to), from.max(to));
        let r = tx::word_range(&words, a, b, q.settings.frame_rate).ok_or_else(|| Error::Invalid("those words are no longer on the timeline".into()))?;
        let n = b - a + 1;
        self.cut_ranges(vec![r], if n == 1 { "Cut Word" } else { "Cut Words" }).map(|_| ())
    }

    /// Cuts "um", "uh" and the like out of the main track. Returns how many went.
    pub fn remove_fillers(&mut self) -> Result<usize> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let words = main_words(q, &self.session.project);
        let fillers: Vec<String> = tx::DEFAULT_FILLERS.iter().map(|s| s.to_string()).collect();
        let hits = tx::find_fillers(&words, &fillers);
        let ranges = tx::filler_ranges(&words, &hits, q.settings.frame_rate);
        let n = hits.len();
        if n == 0 {
            return Ok(0);
        }
        self.cut_ranges(ranges, "Remove Filler Words")?;
        Ok(n)
    }

    /// Shortens every pause between words longer than `min_seconds`, keeping `keep_seconds` of it
    /// on each side. Returns how many were shortened and the seconds saved.
    pub fn remove_pauses(&mut self, min_seconds: f64, keep_seconds: f64) -> Result<(usize, f64)> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        if !min_seconds.is_finite() || !keep_seconds.is_finite() {
            return Err(Error::Invalid("the pause lengths must be numbers".into()));
        }
        let words = main_words(q, &self.session.project);
        let ranges =
            tx::find_pauses(&words, Tick::from_seconds_f64(min_seconds.max(0.1)), Tick::from_seconds_f64(keep_seconds.max(0.0)), q.settings.frame_rate);
        let saved: f64 = ranges.iter().map(|r| r.duration.seconds()).sum();
        let n = ranges.len();
        if n == 0 {
            return Ok((0, 0.0));
        }
        self.cut_ranges(ranges, "Remove Pauses")?;
        Ok((n, saved))
    }

    /// Cuts timeline ranges out of the main track (its pictures and sound), right to left; the
    /// main track closes up, lanes stay, captions follow. Returns the time removed.
    fn cut_ranges(&mut self, ranges: Vec<TimeRange>, label: &str) -> Result<Tick> {
        let ranges = tx::merge_ranges(ranges);
        if ranges.is_empty() {
            return Ok(Tick::ZERO);
        }
        self.group_follow(label, None, |s| {
            s.unlock_lanes()?;
            let mut removed = Tick::ZERO;
            for r in ranges.iter().rev() {
                let (a, b) = (r.start, r.start + r.duration);
                for t in [b, a] {
                    for c in s.main_clips_cut_by(t)? {
                        s.session.execute("timeline.razor", json!({"time": t.0, "clip": c})).map_err(engine)?;
                    }
                }
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let inside: Vec<u64> =
                    q.video_tracks.first().map(|tr| tr.items.iter().filter(|i| i.start >= a && i.end() <= b).map(|i| i.id.0).collect()).unwrap_or_default();
                if !inside.is_empty() {
                    s.session.execute("edit.rippleDelete", json!({"clips": inside})).map_err(engine)?;
                    removed += r.duration;
                }
                s.pack_main()?;
            }
            Ok(removed)
        })
    }

    /// Main-track picture clips with `t` strictly inside them.
    fn main_clips_cut_by(&self, t: Tick) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        Ok(q.video_tracks.first().map(|tr| tr.items.iter().filter(|i| i.start < t && i.end() > t).map(|i| i.id.0).collect()).unwrap_or_default())
    }
}

/// The main-track words heard within `[start, end)` of the timeline (by their middles).
pub(crate) fn heard(words: &[SeqWord], start: Tick, end: Tick) -> String {
    words
        .iter()
        .filter(|w| {
            let mid = w.start.0 + (w.end.0 - w.start.0) / 2;
            mid >= start.0 && mid < end.0
        })
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether two captions say the same words (line breaks and spacing aside).
pub(crate) fn same_words(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// Words broken into at most `layout.lines` lines of about `layout.max_chars`.
pub(crate) fn wrap(text: &str, layout: Layout) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in text.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + w.len() > layout.max_chars && lines.len() + 1 < layout.lines.max(1) {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    lines.push(cur);
    lines.join("\n")
}

/// The Captions tab's style keys as `captions.setStyle` params.
fn style_params(o: &serde_json::Map<String, Value>) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in o {
        match k.as_str() {
            "position" => {
                p.insert("anchor".into(), v.clone());
            }
            "font" | "size" | "color" | "background" | "backgroundColor" | "outline" | "outlineColor" | "align" | "margin" | "lineSpacing" => {
                p.insert(k.clone(), v.clone());
            }
            _ => {}
        }
    }
    Value::Object(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_round_trip_and_refuse_junk() {
        let a = Anchor { item: 7, from: Tick(100), to: Tick(900) };
        assert_eq!(Anchor::parse(Some(&a.id())), Some(a));
        for bad in ["", "cc:", "cc:7:9", "cc:x:1:2", "cc:7:5:5", "cc:7:9:1", "intro", "cc:-1:1:2"] {
            assert_eq!(Anchor::parse(Some(bad)), None, "{bad}");
        }
        assert_eq!(Anchor::parse(None), None);
    }

    #[test]
    fn captions_wrap_into_their_lines() {
        let two = Layout { max_chars: 10, lines: 2 };
        assert_eq!(wrap("hello there my friend", two), "hello\nthere my friend");
        assert_eq!(wrap("one two", Layout { max_chars: 32, lines: 1 }), "one two");
        assert_eq!(wrap("", two), "");
        assert!(same_words("Hello\nthere", "Hello there"));
        assert!(!same_words("Hello there", "Hello there you"));
    }

    #[test]
    fn layouts_suit_the_canvas() {
        assert_eq!(Layout::for_canvas(1920, 1080).lines, 1);
        assert!(Layout::for_canvas(1080, 1920).max_chars < Layout::for_canvas(1920, 1080).max_chars);
    }
}
