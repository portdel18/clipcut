//! The Captions tab's commands: speech models (downloaded once into ClipCut's own data folder),
//! auto-captions run off the session with progress and Cancel, caption edits and style,
//! transcript cutting, and subtitle files.
//!
//! Long work reports progress as `caption-progress` events (`{fraction, status}`), and stops when
//! `cancel_captions` is called.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use clipcut_core::{ModelView, TimelineView, WordView};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::{App, blocking, lock};

/// Where speech models live: `<app data>\models` (`%APPDATA%\io.github.portdel18.clipcut\models`).
pub(crate) fn models_dir(handle: &tauri::AppHandle) -> PathBuf {
    handle.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("ClipCut")).join("models")
}

/// Asks running caption work to stop.
pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Clone, serde::Serialize)]
struct Progress {
    fraction: f32,
    status: String,
}

fn report(handle: &tauri::AppHandle, fraction: f32, status: &str) -> bool {
    let _ = handle.emit("caption-progress", Progress { fraction: fraction.clamp(0.0, 1.0), status: status.to_string() });
    !CANCEL.load(Ordering::Relaxed)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Models {
    models: Vec<ModelView>,
    dir: String,
}

#[tauri::command]
pub fn speech_models(handle: tauri::AppHandle) -> Models {
    let dir = models_dir(&handle);
    Models { models: clipcut_core::speech_models(&dir), dir: dir.to_string_lossy().into_owned() }
}

#[tauri::command]
pub fn cancel_captions() {
    CANCEL.store(true, Ordering::Relaxed);
}

/// Makes captions from the main track's speech: downloads `model` first if needed, transcribes
/// what has no transcript yet (everything with `again`), then lays out the captions.
#[tauri::command]
pub async fn auto_captions(
    model: String,
    language: Option<String>,
    again: bool,
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
) -> Result<TimelineView, String> {
    CANCEL.store(false, Ordering::Relaxed);
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let dir = models_dir(&handle);
        let uses_model = lock(&studio).session().transcriber.is_none();
        let info = clipcut_core::speech_models(&dir).into_iter().find(|m| m.id == model).ok_or_else(|| format!("there is no speech model called {model}"))?;
        if uses_model && !info.installed {
            let mb = info.size as f64 / 1e6;
            clipcut_core::download_model(&dir, &model, &mut |done, total| {
                let f = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                report(&handle, f * 0.3, &format!("Downloading the speech model ({:.0} of {mb:.0} MB, once only)", done as f64 / 1e6))
            })
            .map_err(|e| cap(&e.to_string()))?;
        }
        let start = if uses_model && !info.installed { 0.3 } else { 0.0 };
        report(&handle, start, "Getting the sound ready");
        // the session is only held to gather the sound and to store the result
        let job = lock(&studio).transcribe_job(&dir, &model, language, again).map_err(|e| cap(&e.to_string()))?;
        let results = job.run(&mut |f, status| report(&handle, start + (1.0 - start) * f * 0.97, status)).map_err(|e| cap(&e.to_string()))?;
        report(&handle, 0.98, "Making the captions");
        let mut s = lock(&studio);
        s.finish_captions(results).map_err(|e| cap(&e.to_string()))?;
        report(&handle, 1.0, "Done");
        Ok(s.timeline())
    })
    .await
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The main track's words, for the transcript panel.
#[tauri::command]
pub fn words(app: tauri::State<'_, App>) -> Vec<WordView> {
    lock(&app.studio).words()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    timeline: TimelineView,
    removed: usize,
    seconds: f64,
}

async fn studio_edit<T: Send + 'static>(
    app: &tauri::State<'_, App>,
    f: impl FnOnce(&mut clipcut_core::Studio) -> clipcut_core::Result<T> + Send + 'static,
) -> Result<(T, TimelineView), String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let r = f(&mut s).map_err(|e| e.to_string())?;
        Ok((r, s.timeline()))
    })
    .await
}

#[tauri::command]
pub async fn cut_words(from: usize, to: usize, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    studio_edit(&app, move |s| s.cut_words(from, to)).await.map(|(_, v)| v)
}

#[tauri::command]
pub async fn remove_fillers(app: tauri::State<'_, App>) -> Result<Cleaned, String> {
    let (n, timeline) = studio_edit(&app, |s| s.remove_fillers()).await?;
    Ok(Cleaned { timeline, removed: n, seconds: 0.0 })
}

#[tauri::command]
pub async fn remove_pauses(min_seconds: f64, keep_seconds: f64, app: tauri::State<'_, App>) -> Result<Cleaned, String> {
    let ((n, seconds), timeline) = studio_edit(&app, move |s| s.remove_pauses(min_seconds, keep_seconds)).await?;
    Ok(Cleaned { timeline, removed: n, seconds })
}

/// Find and replace in the transcript: everywhere, or at the `only`-th place found.
#[tauri::command]
pub async fn replace_words(find: String, with: String, case: bool, only: Option<usize>, app: tauri::State<'_, App>) -> Result<Cleaned, String> {
    let (r, timeline) = studio_edit(&app, move |s| s.replace_words(&find, &with, case, only)).await?;
    Ok(Cleaned { timeline, removed: r.count, seconds: 0.0 })
}

#[tauri::command]
pub async fn set_caption_text(id: u64, text: String, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    studio_edit(&app, move |s| s.set_caption_text(id, &text)).await.map(|(_, v)| v)
}

#[tauri::command]
pub async fn delete_captions(ids: Vec<u64>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    studio_edit(&app, move |s| s.delete_captions(&ids)).await.map(|(_, v)| v)
}

#[tauri::command]
pub async fn retime_caption(id: u64, start: f64, end: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    studio_edit(&app, move |s| s.retime_caption(id, start, end)).await.map(|(_, v)| v)
}

#[tauri::command]
pub async fn set_caption_style(style: serde_json::Value, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    studio_edit(&app, move |s| s.set_caption_style(&style)).await.map(|(_, v)| v)
}

#[tauri::command]
pub async fn set_captions_on(on: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    studio_edit(&app, move |s| s.set_captions_on(on)).await.map(|(_, v)| v)
}

#[tauri::command]
pub fn caption_presets() -> serde_json::Value {
    clipcut_core::caption_presets()
}

/// Asks where to save the subtitle file (`.srt`, or `.vtt` for the web).
#[tauri::command]
pub async fn pick_captions_path(name: String, handle: tauri::AppHandle) -> Result<Option<String>, String> {
    blocking(move || {
        let stem: String = name.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
        let f = handle
            .dialog()
            .file()
            .set_title("Save the captions")
            .set_file_name(format!("{}.srt", if stem.trim().is_empty() { "Captions" } else { stem.trim() }))
            .add_filter("SubRip subtitles", &["srt"])
            .add_filter("WebVTT subtitles", &["vtt"])
            .blocking_save_file();
        Ok(f.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub async fn export_captions(path: String, app: tauri::State<'_, App>) -> Result<usize, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).export_captions(&path).map_err(|e| e.to_string())).await
}

/// `ClipCut --caption-test <clip> <models dir> <model> <report.json> <srt out>`: downloads the model
/// if needed, captions the clip's speech with it and writes the subtitles and what happened (CI
/// runs it on Windows with real speech).
pub fn caption_test(input: &str, dir: &str, model: &str, report: &str, srt: &str) -> i32 {
    let t0 = std::time::Instant::now();
    let mut steps = serde_json::Map::new();
    let result = (|| -> Result<(), String> {
        let dir = std::path::Path::new(dir);
        let mut last = 0u64;
        for attempt in 1..=3 {
            match clipcut_core::download_model(dir, model, &mut |done, _| {
                if done >= last + 50_000_000 {
                    last = done;
                    eprintln!("downloaded {} MB", done / 1_000_000);
                }
                true
            }) {
                Ok(()) => break,
                Err(e) if attempt < 3 => eprintln!("download attempt {attempt}: {e}"),
                Err(e) => return Err(format!("download: {e}")),
            }
        }
        steps.insert("downloadSeconds".into(), serde_json::json!(t0.elapsed().as_secs_f64()));
        let mut s = clipcut_core::Studio::new();
        s.add_clip(input).map_err(|e| format!("open: {e}"))?;
        let t1 = std::time::Instant::now();
        let job = s.transcribe_job(dir, model, Some("en".into()), false).map_err(|e| format!("prepare: {e}"))?;
        let results = job.run(&mut |_, _| true).map_err(|e| format!("transcribe: {e}"))?;
        steps.insert("transcribeSeconds".into(), serde_json::json!(t1.elapsed().as_secs_f64()));
        let n = s.finish_captions(results).map_err(|e| format!("captions: {e}"))?;
        let words: Vec<String> = s.words().into_iter().map(|w| w.text).collect();
        steps.insert("words".into(), serde_json::json!(words.join(" ")));
        steps.insert(
            "captions".into(),
            serde_json::json!(s.timeline().captions.items.iter().map(|c| format!("{:.2}-{:.2} {}", c.start, c.end, c.text)).collect::<Vec<_>>()),
        );
        s.export_captions(srt).map_err(|e| format!("srt: {e}"))?;
        if n == 0 || words.len() < 3 {
            return Err(format!("only {} words were heard", words.len()));
        }
        Ok(())
    })();
    steps.insert("ok".into(), serde_json::json!(result.is_ok()));
    steps.insert("error".into(), serde_json::json!(result.as_ref().err()));
    steps.insert("totalSeconds".into(), serde_json::json!(t0.elapsed().as_secs_f64()));
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(steps)).unwrap_or_default();
    let wrote = std::fs::write(report, &text).is_ok();
    println!("{text}");
    if result.is_ok() && wrote { 0 } else { 1 }
}
