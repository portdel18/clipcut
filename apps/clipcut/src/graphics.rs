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
    /// The files of the latest drop on the window, which Graphics may read (see [`dropped_file`]).
    pub(crate) dropped: Mutex<Vec<PathBuf>>,
    /// Where the person chose to save Graphics' last file (see [`save_graphic_file`]).
    save_to: Mutex<Option<PathBuf>>,
}

/// Graphics saving a file of its own (a thumbnail, a rendered video, presets): the webview drops
/// page downloads, so the interface asks here. Shows the Save dialog for `name`; returns where the
/// person chose, or None when they cancelled. [`write_graphic_file`] then writes only there.
#[tauri::command]
pub(crate) async fn save_graphic_file(name: String, handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file = crate::editing::safe_name(&name);
    let ext = Path::new(&file).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let picked = blocking(move || {
        let mut d = handle.dialog().file().set_title("Save").set_file_name(&file);
        if !ext.is_empty() {
            d = d.add_filter(ext.to_uppercase(), &[ext.as_str()]);
        }
        Ok(d.blocking_save_file().and_then(|p| p.into_path().ok()))
    })
    .await?;
    *lock(&app.graphics.save_to) = picked.clone();
    Ok(picked.map(|p| p.to_string_lossy().into_owned()))
}

/// Writes the request's body to where [`save_graphic_file`] was last told to save, once.
#[tauri::command]
pub(crate) async fn write_graphic_file(request: Request<'_>, app: tauri::State<'_, App>) -> Result<(), String> {
    let path = lock(&app.graphics.save_to).take().ok_or("Choose where to save it first.")?;
    let bytes = body(&request)?;
    blocking(move || {
        std::fs::write(&path, bytes)
            .map_err(|e| format!("Couldn't save {}: {e}. Check the folder can be written to and there's space on the drive.", path.display()))
    })
    .await
}

/// The largest dropped file handed to Graphics (it is read into memory there).
const MAX_DROPPED: u64 = 1 << 30;

/// The bytes of a file just dropped on the window, for Clip Studio's drop zones: the window takes
/// drops itself (on Windows the page never sees them), so the interface asks for the file here and
/// hands it to the drop zone under the pointer. Only files from the latest drop can be read.
#[tauri::command]
pub(crate) async fn dropped_file(index: usize, app: tauri::State<'_, App>) -> Result<tauri::ipc::Response, String> {
    let path = lock(&app.graphics.dropped).get(index).cloned().ok_or("That file is no longer being dropped. Drop it again.")?;
    blocking(move || {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let len = std::fs::metadata(&path).map_err(|e| format!("Couldn't read {name}: {e}"))?.len();
        if len > MAX_DROPPED {
            return Err(format!(
                "{name} is too big to drop into Graphics ({} MB; up to 1024 MB). Choose it with the tool's own file button instead.",
                len >> 20
            ));
        }
        std::fs::read(&path).map(tauri::ipc::Response::new).map_err(|e| format!("Couldn't read {name}: {e}"))
    })
    .await
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

/// The video at `seconds` without the clips `hide`, at most `max_width` wide (8 bytes of size,
/// then RGBA, as `frame`): what Graphics shows behind the graphic being made.
#[tauri::command]
pub async fn graphic_backdrop(seconds: f64, max_width: u32, hide: Vec<u64>, app: tauri::State<'_, App>) -> Result<tauri::ipc::Response, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let f = lock(&studio).frame_without(seconds, max_width.clamp(16, 1920), &hide).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(8 + f.rgba.len());
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        out.extend_from_slice(&f.rgba);
        Ok(tauri::ipc::Response::new(out))
    })
    .await
}

/// Where a still comes from (see `Studio::still_source`).
#[derive(serde::Serialize)]
pub(crate) struct StillSource {
    item: u64,
    seconds: f64,
    name: String,
}

/// The picture a still at `seconds` comes from: clip `clip`, or the main track's clip there.
#[tauri::command]
pub(crate) fn still_source(seconds: f64, clip: Option<u64>, app: tauri::State<'_, App>) -> Result<StillSource, String> {
    let (item, seconds, name) = lock(&app.studio).still_source(seconds, clip).map_err(|e| e.to_string())?;
    Ok(StillSource { item, seconds, name })
}

/// The largest side a still is taken at (8K).
const STILL_MAX: u32 = 8192;

/// A still as a PNG: media `item`'s picture at `seconds` into it, at its own size; or, with no
/// item, the whole picture the timeline shows at `seconds` (every lane, text, graphics, captions)
/// at the video's size.
fn still(app: &App, item: Option<u64>) -> Result<Box<dyn FnOnce(f64) -> Result<Vec<u8>, String> + Send>, String> {
    let png = |f: clipcut_core::Frame| filmcraft_export::encode_png(f.rgba, f.w, f.h).map_err(|e| format!("The still couldn't be made: {e}"));
    Ok(match item {
        Some(item) => {
            let stills = lock(&app.studio).stills();
            Box::new(move |seconds| png(stills.frame(item, seconds, STILL_MAX).map_err(|e| e.to_string())?))
        }
        None => {
            let feed = lock(&app.studio).still_feed().map_err(|e| e.to_string())?;
            Box::new(move |seconds| {
                if !seconds.is_finite() {
                    return Err("the time must be a number".into());
                }
                png(feed.render(feed.frame_at(seconds), false).map_err(|e| e.to_string())?)
            })
        }
    })
}

/// A still at `seconds` as a PNG (for Thumbnails): of media `item`, or of the whole picture.
#[tauri::command]
pub(crate) async fn still_png(item: Option<u64>, seconds: f64, app: tauri::State<'_, App>) -> Result<tauri::ipc::Response, String> {
    let make = still(&app, item)?;
    blocking(move || Ok(tauri::ipc::Response::new(make(seconds)?))).await
}

/// Saves a still at `seconds` (of media `item`, or of the whole picture) as a PNG where the person
/// picks (the Save dialog, named `name`). Returns where, or None when they cancelled.
#[tauri::command]
pub(crate) async fn save_still(
    item: Option<u64>,
    seconds: f64,
    name: String,
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let make = still(&app, item)?;
    let file = crate::editing::safe_name(&name);
    blocking(move || {
        let Some(path) = handle
            .dialog()
            .file()
            .set_title("Save the still")
            .set_file_name(&file)
            .add_filter("PNG", &["png"])
            .blocking_save_file()
            .and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        let png = make(seconds)?;
        std::fs::write(&path, png)
            .map_err(|e| format!("Couldn't save {}: {e}. Check the folder can be written to and there's space on the drive.", path.display()))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}
