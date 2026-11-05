//! Voiceover: type a script, pick a voice, and it goes on a sound lane at the playhead as a clip.
//!
//! The engine's text to speech ([`filmcraft_tts`]): built-in voices (an original formant
//! synthesizer: no download, robotic), and natural voices (Kokoro-82M, Apache-2.0, with CMUdict),
//! downloaded once into ClipCut's models folder beside the speech models, pinned and checked by
//! SHA-256. The speaking runs off the studio ([`speak`]); the WAV is written beside the project
//! ([`Studio::voiceover_dir`]) and placed like imported music ([`Studio::add_voiceover`]).

use std::path::{Path, PathBuf};

use filmcraft_tts as tts;
use serde::Serialize;

use crate::{Error, Result, Studio};

/// A voice the Voiceover panel offers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrationVoice {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `female` or `male`.
    pub gender: &'static str,
    /// A natural voice (downloaded), else a built-in one.
    pub natural: bool,
    /// Usable now.
    pub installed: bool,
}

/// The voices, built-in first; natural ones are `installed` once downloaded into `models_dir`.
pub fn voiceover_voices(models_dir: &Path) -> Vec<NarrationVoice> {
    tts::voices_in(Some(models_dir))
        .into_iter()
        .map(|v| NarrationVoice {
            id: v.id.into(),
            name: v.name.into(),
            description: v.description.into(),
            gender: match v.gender {
                tts::Gender::Female => "female",
                tts::Gender::Male => "male",
            },
            natural: v.engine != "basic",
            installed: v.installed,
        })
        .collect()
}

/// How much the natural voices download is, bytes (and how much of it is still missing).
pub fn natural_voices_size(models_dir: &Path) -> (u64, u64) {
    (tts::catalog::size(), tts::catalog::missing_bytes(models_dir))
}

/// Downloads the natural voices into `models_dir` (each file checked against its SHA-256).
/// `progress(done, total)` returning `false` stops it.
pub fn download_voices(models_dir: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<()> {
    let files: Vec<filmcraft_speech::models::ModelFile> =
        tts::catalog::FILES.iter().map(|f| filmcraft_speech::models::ModelFile { name: f.name, url: f.url, sha256: f.sha256, size: f.size }).collect();
    let mut cb = |done: u64, total: u64, _file: &str| progress(done, total);
    filmcraft_speech::models::download_files(&tts::catalog::package_dir(models_dir), &files, &mut cb).map_err(|e| match e {
        filmcraft_speech::SpeechError::Cancelled => Error::Invalid("stopped".into()),
        e => Error::Engine(format!("the voices couldn't be downloaded ({e}). Check the internet connection and try again")),
    })
}

/// Speaks `text` with voice `voice` at `pace` (0.5…2, 1 = normal), on any thread: the WAV's bytes
/// (mono 32-bit float at the voice's rate) and its length in seconds.
pub fn speak(models_dir: &Path, voice: &str, text: &str, pace: f64) -> Result<(Vec<u8>, f64)> {
    let text = text.trim();
    tts::check_text(text).map_err(|e| Error::Invalid(words(&e)))?;
    if !pace.is_finite() {
        return Err(Error::Invalid("the speed must be a number".into()));
    }
    let v = tts::voice_in(voice, Some(models_dir)).map_err(|e| Error::Invalid(words(&e)))?;
    let params = tts::Params { pace: pace.clamp(tts::MIN_PACE, tts::MAX_PACE), ..Default::default() };
    let audio = v.synthesize(text, &params).map_err(|e| Error::Invalid(words(&e)))?;
    let bytes = filmcraft_export::write_wav(&audio.samples, 1, audio.sample_rate, filmcraft_export::SampleFormat::Float32).map_err(Error::Engine)?;
    Ok((bytes, audio.seconds()))
}

/// The engine's text to speech errors in the panel's words.
fn words(e: &tts::TtsError) -> String {
    match e {
        tts::TtsError::NotInstalled => "download the natural voices first (once)".into(),
        tts::TtsError::Empty => "type what the voice should say".into(),
        e => e.to_string(),
    }
}

impl Studio {
    /// Where voiceovers are saved: `Voiceovers` beside the project, or in ClipCut's own folder
    /// while the project isn't saved (as proxies are).
    pub fn voiceover_dir(&self) -> Option<PathBuf> {
        let base = self.session.path.as_deref().and_then(|p| Path::new(p).parent()).or_else(|| self.session.prefs_path.as_deref().and_then(Path::parent))?;
        Some(base.join("Voiceovers"))
    }

    /// Adds the voiceover file `path` to a sound lane at `seconds` (a free lane, as imported music
    /// goes). Returns the new clip's ids.
    pub fn add_voiceover(&mut self, path: &str, seconds: f64) -> Result<Vec<u64>> {
        let item = self.import(path)?;
        self.add_to_lane(item, seconds)
    }
}

/// A file name for the next voiceover in `dir`: `Voiceover 1.wav`, `Voiceover 2.wav`… (the first
/// not taken).
pub fn next_voiceover_path(dir: &Path) -> PathBuf {
    (1..10_000u32).map(|n| dir.join(format!("Voiceover {n}.wav"))).find(|p| !p.exists()).unwrap_or_else(|| dir.join("Voiceover.wav"))
}
