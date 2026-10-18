//! The Graphics tab's commands: Clip Studio (in `ui/graphics`, shown in a frame over the editor)
//! renders a graphic and hands its frames here one by one; they go into a folder beside the
//! project (`Graphics/<name>/frame_00000.png`…, its sound, and `graphic.json` with the tool and
//! its settings), then onto the timeline through [`Studio::add_graphic`].
//!
//! Frames arrive as raw request bodies (no JSON), with the job, the frame's number and its type
//! in the request's headers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use clipcut_core::{GRAPHIC_FILE, GRAPHIC_SOUND, TimelineView};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};

use crate::{App, blocking, lock};

/// Graphics being rendered: job → its folder.
#[derive(Default)]
pub(crate) struct Jobs {
    next: AtomicU64,
    dirs: Mutex<HashMap<u64, PathBuf>>,
}

/// Where graphics go: `Graphics` beside the project, or in ClipCut's projects folder before the
/// project has been saved.
fn graphics_root(handle: &tauri::AppHandle, app: &App) -> PathBuf {
    let path = lock(&app.studio).timeline().project.path;
    match path.as_deref().map(Path::new).and_then(Path::parent) {
        Some(dir) => dir.join("Graphics"),
        None => crate::editing::projects_root(handle).join("Graphics"),
    }
}

fn header<'a>(req: &'a Request<'_>, name: &str) -> Result<&'a str, String> {
    req.headers().get(name).and_then(|v| v.to_str().ok()).ok_or_else(|| format!("the graphic's {name} is missing"))
}

fn job_dir(app: &App, job: u64) -> Result<PathBuf, String> {
    lock(&app.graphics.dirs).get(&job).cloned().ok_or_else(|| "that graphic is no longer being made: render it again".to_string())
}

fn body(req: &Request<'_>) -> Result<Vec<u8>, String> {
    match req.body() {
        InvokeBody::Raw(b) if !b.is_empty() => Ok(b.clone()),
        _ => Err("the graphic's frame was empty".into()),
    }
}

/// Starts a graphic: makes its folder, named after it. Returns the job.
#[tauri::command]
pub fn graphic_begin(name: String, handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<u64, String> {
    let root = graphics_root(&handle, &app);
    let base = crate::editing::safe_name(&name);
    let dir = (1..10_000)
        .map(|n| if n == 1 { root.join(&base) } else { root.join(format!("{base} {n}")) })
        .find(|d| !d.exists())
        .ok_or("too many graphics with this name")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("the graphic's folder can't be made ({}): {e}", dir.display()))?;
    let job = app.graphics.next.fetch_add(1, Ordering::Relaxed) + 1;
    lock(&app.graphics.dirs).insert(job, dir);
    Ok(job)
}

/// One frame: the body is the picture (PNG or JPEG); headers `job`, `index`, `ext`.
#[tauri::command]
pub fn graphic_frame(request: Request<'_>, app: tauri::State<'_, App>) -> Result<(), String> {
    let job: u64 = header(&request, "job")?.parse().map_err(|_| "the graphic's job is not a number")?;
    let index: u32 = header(&request, "index")?.parse().map_err(|_| "the frame's number is not a number")?;
    let ext = match header(&request, "ext")? {
        "png" => "png",
        "jpg" => "jpg",
        other => return Err(format!("a graphic's frames are PNG or JPEG, not {other}")),
    };
    let dir = job_dir(&app, job)?;
    std::fs::write(dir.join(format!("frame_{index:05}.{ext}")), body(&request)?).map_err(|e| format!("a frame couldn't be saved: {e}"))
}

/// The graphic's sound (WAV); header `job`.
#[tauri::command]
pub fn graphic_audio(request: Request<'_>, app: tauri::State<'_, App>) -> Result<(), String> {
    let job: u64 = header(&request, "job")?.parse().map_err(|_| "the graphic's job is not a number")?;
    let dir = job_dir(&app, job)?;
    std::fs::write(dir.join(GRAPHIC_SOUND), body(&request)?).map_err(|e| format!("the graphic's sound couldn't be saved: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicResult {
    timeline: TimelineView,
    clips: Vec<u64>,
}

/// Every frame is in: saves what made it (`info`: Clip Studio's tool, settings, size, frame rate,
/// frame type) and puts it on the timeline at `seconds`, or in place of clip `replace`.
#[tauri::command]
pub async fn graphic_finish(
    job: u64,
    info: serde_json::Value,
    seconds: f64,
    replace: Option<u64>,
    app: tauri::State<'_, App>,
) -> Result<GraphicResult, String> {
    let dir = lock(&app.graphics.dirs).remove(&job).ok_or("that graphic is no longer being made: render it again")?;
    let ext = match info.get("ext").and_then(|v| v.as_str()) {
        Some("jpg") => "jpg",
        _ => "png",
    };
    let fps = info.get("fps").and_then(serde_json::Value::as_f64).unwrap_or(30.0);
    let text = serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(GRAPHIC_FILE), text).map_err(|e| format!("the graphic's settings couldn't be saved: {e}"))?;
    let first = dir.join(format!("frame_00000.{ext}")).to_string_lossy().to_string();
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let clips = s.add_graphic(&first, fps, seconds, replace).map_err(|e| e.to_string())?;
        Ok(GraphicResult { timeline: s.timeline(), clips })
    })
    .await
}

/// Stops a graphic part way (cancelled or failed): its folder goes.
#[tauri::command]
pub fn graphic_abort(job: u64, app: tauri::State<'_, App>) {
    if let Some(dir) = lock(&app.graphics.dirs).remove(&job) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// What made clip `clip`'s graphic, to open it in Clip Studio again.
#[tauri::command]
pub fn graphic_info(clip: u64, app: tauri::State<'_, App>) -> Result<serde_json::Value, String> {
    lock(&app.studio).graphic_of(clip).map_err(|e| e.to_string())
}
