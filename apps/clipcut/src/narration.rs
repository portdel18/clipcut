//! The Audio tab's Voiceover: voices, the natural voices' download (progress as `voice-progress`
//! events, `0…1`; `cancel_voices` stops it), a listen-first preview, and adding the voiceover to a
//! sound lane. The speaking runs off the studio; only placing the clip holds it.

use std::sync::atomic::{AtomicBool, Ordering};

use clipcut_core::{NarrationVoice, TimelineView};
use tauri::Emitter;
use tauri::ipc::Response;

use crate::captioning::models_dir;
use crate::{App, blocking, lock};

/// Asks a running voices download to stop.
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voices {
    voices: Vec<NarrationVoice>,
    /// The natural voices' download in bytes, and how much of it is still missing.
    size: u64,
    missing: u64,
}

#[tauri::command]
pub fn voiceover_voices(handle: tauri::AppHandle) -> Voices {
    let dir = models_dir(&handle);
    let (size, missing) = clipcut_core::natural_voices_size(&dir);
    Voices { voices: clipcut_core::voiceover_voices(&dir), size, missing }
}

#[tauri::command]
pub fn cancel_voices() {
    CANCEL.store(true, Ordering::Relaxed);
}

/// Downloads the natural voices once (about 336 MB, each file checked).
#[tauri::command]
pub async fn download_voices(handle: tauri::AppHandle) -> Result<(), String> {
    CANCEL.store(false, Ordering::Relaxed);
    blocking(move || {
        let dir = models_dir(&handle);
        clipcut_core::download_voices(&dir, &mut |done, total| {
            let _ = handle.emit("voice-progress", if total > 0 { done as f64 / total as f64 } else { 0.0 });
            !CANCEL.load(Ordering::Relaxed)
        })
        .map_err(|e| e.to_string())
    })
    .await
}

/// The script spoken, as WAV bytes, to listen to before adding it.
#[tauri::command]
pub async fn preview_voiceover(text: String, voice: String, pace: f64, handle: tauri::AppHandle) -> Result<Response, String> {
    blocking(move || {
        let dir = models_dir(&handle);
        let (wav, _) = speak_guarded(&dir, &voice, &text, pace)?;
        Ok(Response::new(wav))
    })
    .await
}

#[derive(serde::Serialize)]
pub struct Added {
    timeline: TimelineView,
    clips: Vec<u64>,
}

/// Speaks the script, saves it as `Voiceover <n>.wav` beside the project and puts it on a sound
/// lane at `seconds`.
#[tauri::command]
pub async fn add_voiceover(
    text: String,
    voice: String,
    pace: f64,
    seconds: f64,
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
) -> Result<Added, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        let models = models_dir(&handle);
        let dir = lock(&studio).voiceover_dir().unwrap_or_else(|| models.with_file_name("Voiceovers"));
        let (wav, _) = speak_guarded(&models, &voice, &text, pace)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("the voiceover can't be saved in {} ({e})", dir.display()))?;
        let path = clipcut_core::next_voiceover_path(&dir);
        std::fs::write(&path, wav).map_err(|e| format!("the voiceover can't be saved as {} ({e})", path.display()))?;
        lock(&player).stop();
        let mut s = lock(&studio);
        let clips = s.add_voiceover(&path.to_string_lossy(), seconds).map_err(|e| e.to_string())?;
        Ok(Added { timeline: s.timeline(), clips })
    })
    .await
}

/// [`clipcut_core::speak`], with a failure inside the voice reported instead of taking the app down.
fn speak_guarded(dir: &std::path::Path, voice: &str, text: &str, pace: f64) -> Result<(Vec<u8>, f64), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clipcut_core::speak(dir, voice, text, pace)))
        .map_err(|_| "the voice couldn't speak this script; please try again".to_string())?
        .map_err(|e| e.to_string())
}
