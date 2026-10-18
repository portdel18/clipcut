//! ClipCut desktop app: a Tauri window whose interface (`ui/`, HTML and JS) calls the commands
//! below, which drive the FilmCraft engine through `clipcut-core`.
//!
//! `ClipCut <file>` starts a new project with that clip; otherwise the last project reopens. `ClipCut --self-test <in> <out> <report.json>`
//! opens, previews, mixes and exports without a window and writes what happened to the report
//! (`--caption-test` does the same for auto-captions with a real speech model):
//! the Windows build runs it in CI, so the packaged engine is tested on Windows itself.

// No console window behind the app in release builds on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod captioning;
mod editing;
mod graphics;
mod playback;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use clipcut_core::{JobStatus, Studio};
use tauri::ipc::Response;
use tauri_plugin_dialog::DialogExt;

/// Video, audio and still formats the engine imports (the picker's filter; anything else can still
/// be dropped on the window and gets a clear error if the engine can't read it).
pub(crate) const OPEN_EXTENSIONS: &[&str] =
    &["mp4", "mov", "m4v", "mkv", "webm", "avi", "mxf", "mts", "m2ts", "ts", "mpg", "mpeg", "wav", "mp3", "m4a", "aac", "flac", "png", "jpg", "jpeg"];

pub(crate) struct App {
    pub(crate) studio: Arc<Mutex<Studio>>,
    pub(crate) player: Arc<Mutex<playback::Player>>,
    /// Files named on the command line, added by the interface once it has loaded (once: a
    /// reload of the page doesn't add them again).
    pub(crate) startup_files: Mutex<Vec<String>>,
    /// Waveforms already worked out, by media item.
    pub(crate) peaks: Arc<Mutex<std::collections::HashMap<u64, Arc<Vec<u8>>>>>,
    /// Graphics from the Graphics tab being rendered.
    pub(crate) graphics: graphics::Jobs,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `f` off the main thread (rendering and decoding must not freeze the window).
pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("the task stopped unexpectedly: {e}"))?
}

/// The frame at `seconds`, at most `max_width` wide, as 8 bytes of header (width, height: u32
/// little-endian) followed by RGBA pixels.
#[tauri::command]
async fn frame(seconds: f64, max_width: u32, app: tauri::State<'_, App>) -> Result<Response, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let f = lock(&studio).frame(seconds, max_width.clamp(16, 7680)).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(8 + f.rgba.len());
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        out.extend_from_slice(&f.rgba);
        Ok(Response::new(out))
    })
    .await
}

/// Media item `item`'s picture at `seconds` into the media (thumbnails and filmstrips), in the same
/// layout as [`frame`]. The session stays free while it renders.
#[tauri::command]
async fn thumb(item: u64, seconds: f64, max_width: u32, app: tauri::State<'_, App>) -> Result<Response, String> {
    let th = lock(&app.studio).thumbs();
    blocking(move || {
        let f = th.frame(item, seconds, max_width.clamp(16, 1920)).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(8 + f.rgba.len());
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        out.extend_from_slice(&f.rgba);
        Ok(Response::new(out))
    })
    .await
}

#[tauri::command]
fn play(from: f64, app: tauri::State<'_, App>) -> Result<(), String> {
    let (feed, end) = {
        let s = lock(&app.studio);
        (s.audio_feed().map_err(|e| e.to_string())?, s.info().map_err(|e| e.to_string())?.duration_seconds)
    };
    lock(&app.player).play(feed, from, end);
    Ok(())
}

#[tauri::command]
fn pause(app: tauri::State<'_, App>) -> playback::State {
    let mut p = lock(&app.player);
    p.stop();
    p.state()
}

#[tauri::command]
fn playback(app: tauri::State<'_, App>) -> playback::State {
    lock(&app.player).state()
}

#[tauri::command]
fn seek(seconds: f64, app: tauri::State<'_, App>) {
    let mut p = lock(&app.player);
    p.stop();
    p.set_position(seconds);
}

#[tauri::command]
async fn pick_export_path(name: String, handle: tauri::AppHandle) -> Result<Option<String>, String> {
    blocking(move || {
        let stem: String = name.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
        let f = handle
            .dialog()
            .file()
            .set_title("Export video")
            .set_file_name(format!("{}.mp4", if stem.trim().is_empty() { "ClipCut export" } else { stem.trim() }))
            .add_filter("MP4 video (H.264)", &["mp4"])
            .add_filter("QuickTime (ProRes)", &["mov"])
            .add_filter("Sound only (WAV)", &["wav"])
            .blocking_save_file();
        Ok(f.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
fn export(path: String, app: tauri::State<'_, App>) -> Result<u64, String> {
    lock(&app.studio).export(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_status(job: u64, app: tauri::State<'_, App>) -> Option<JobStatus> {
    lock(&app.studio).job(job)
}

#[tauri::command]
fn cancel_export(job: u64, app: tauri::State<'_, App>) -> Result<(), String> {
    lock(&app.studio).cancel(job).map_err(|e| e.to_string())
}

/// Shows `path` in the system's file manager.
#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    let r = if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(p).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn()
    };
    r.map(|_| ()).map_err(|e| format!("couldn't open the folder: {e}"))
}

/// The GPU export renderer (as in FilmCraft's CLI): exports composite on the GPU where they can and
/// fall back to the CPU reference renderer elsewhere.
struct GpuFrameRenderer(filmcraft_gpu::ExportRenderer);

impl filmcraft_export::FrameRenderer for GpuFrameRenderer {
    fn render(
        &mut self,
        project: &filmcraft_project::Project,
        seq: filmcraft_project::ItemId,
        t: filmcraft_time::Tick,
        opts: filmcraft_render::RenderOptions,
        sources: &dyn filmcraft_render::SourceProvider,
    ) -> Option<filmcraft_render::Image> {
        self.0.render(project, seq, t, opts, sources)
    }
}

fn register_engine_accelerators() {
    // Hardware video decoding (Media Foundation on Windows), with FilmCraft's decoders behind it
    let _ = filmcraft_platform::register();
    filmcraft_export::register_frame_renderer(|| {
        filmcraft_gpu::ExportRenderer::new().map(|r| Box::new(GpuFrameRenderer(r)) as Box<dyn filmcraft_export::FrameRenderer>)
    });
}

/// `--self-test <in> <out> <report.json>`: the engine path of the packaged app, without a window.
fn self_test(input: &str, output: &str, report: &str) -> i32 {
    let t0 = std::time::Instant::now();
    let mut steps = serde_json::Map::new();
    let result = (|| -> Result<(), String> {
        let mut s = Studio::new();
        let info = s.open_clip(input).map_err(|e| format!("open: {e}"))?;
        steps.insert("info".into(), serde_json::to_value(&info).unwrap_or_default());
        let f = s.frame(info.duration_seconds / 2.0, 640).map_err(|e| format!("frame: {e}"))?;
        let lit = f.rgba.chunks(4).filter(|p| p.iter().take(3).map(|v| u32::from(*v)).sum::<u32>() > 60).count();
        steps.insert("frame".into(), serde_json::json!({"w": f.w, "h": f.h, "litPixels": lit}));
        let feed = s.audio_feed().map_err(|e| format!("audio: {e}"))?;
        let peak = feed.mix(feed.length / 2, 4096).iter().fold(0f32, |m, v| m.max(v.abs()));
        steps.insert("audioPeak".into(), serde_json::json!(peak));
        let job = s.export(output).map_err(|e| format!("export: {e}"))?;
        let started = std::time::Instant::now();
        let st = loop {
            let st = s.job(job).ok_or("export: the job disappeared")?;
            if st.finished {
                break st;
            }
            if started.elapsed().as_secs() > 900 {
                return Err(format!("export: still at {:.0}% after 15 minutes", st.progress * 100.0));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        if let Some(e) = st.error {
            return Err(format!("export: {e}"));
        }
        let bytes = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        steps.insert("export".into(), serde_json::json!({"bytes": bytes, "seconds": started.elapsed().as_secs_f64()}));
        let back = Studio::new().open_clip(output).map_err(|e| format!("reopen export: {e}"))?;
        steps.insert("exported".into(), serde_json::to_value(&back).unwrap_or_default());
        if bytes == 0 || lit == 0 {
            return Err("the export or the preview frame came out empty".into());
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

fn main() {
    register_engine_accelerators();
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if args.first().map(String::as_str) == Some("--self-test") {
        let get = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        std::process::exit(self_test(get(1), get(2), get(3)));
    }
    if args.first().map(String::as_str) == Some("--caption-test") {
        let get = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        std::process::exit(captioning::caption_test(get(1), get(2), get(3), get(4), get(5)));
    }
    let mut studio = Studio::new();
    // For checking the Captions tab without a speech model (tests, CI, agents): every clip is heard
    // saying this transcript (JSON, as a project file stores one).
    if let Some(j) = std::env::var_os("CLIPCUT_TEST_TRANSCRIPT").and_then(|p| std::fs::read_to_string(p).ok())
        && let Err(e) = studio.hear(&j)
    {
        eprintln!("CLIPCUT_TEST_TRANSCRIPT: {e}");
    }
    let app = App {
        studio: Arc::new(Mutex::new(studio)),
        player: Arc::new(Mutex::new(playback::Player::default())),
        peaks: Arc::default(),
        graphics: graphics::Jobs::default(),
        startup_files: Mutex::new(args.into_iter().filter(|a| !a.starts_with("--")).collect()),
    };
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                editing::save_on_close(tauri::Manager::app_handle(window));
            }
        })
        .invoke_handler(tauri::generate_handler![
            frame,
            thumb,
            play,
            pause,
            playback,
            seek,
            pick_export_path,
            export,
            export_status,
            cancel_export,
            reveal,
            editing::timeline,
            editing::set_colour,
            editing::set_look,
            editing::reset_colour,
            editing::colour_everywhere,
            editing::looks,
            editing::look_previews,
            editing::scope,
            editing::props_at,
            editing::toggle_keyframe,
            editing::set_easing,
            graphics::graphic_begin,
            graphics::graphic_frame,
            graphics::graphic_audio,
            graphics::graphic_finish,
            graphics::graphic_abort,
            graphics::graphic_info,
            editing::transition_presets,
            editing::transition_previews,
            editing::set_transition,
            editing::set_transition_length,
            editing::remove_transitions,
            editing::transition_everywhere,
            editing::pick_clips,
            editing::add_clips,
            editing::append_media,
            editing::insert_media,
            editing::split,
            editing::split_keep,
            editing::delete_clips,
            editing::trim,
            editing::reorder,
            editing::undo,
            editing::redo,
            editing::save_project,
            editing::new_project,
            editing::pick_project,
            editing::open_project,
            editing::restore,
            editing::import_media,
            editing::add_to_lane,
            editing::add_text,
            editing::move_clip,
            editing::set_props,
            editing::set_canvas,
            editing::fit_clips,
            editing::set_lane,
            editing::fonts,
            editing::peaks,
            captioning::speech_models,
            captioning::cancel_captions,
            captioning::auto_captions,
            captioning::words,
            captioning::cut_words,
            captioning::remove_fillers,
            captioning::remove_pauses,
            captioning::set_caption_text,
            captioning::delete_captions,
            captioning::retime_caption,
            captioning::set_caption_style,
            captioning::set_captions_on,
            captioning::caption_presets,
            captioning::pick_captions_path,
            captioning::export_captions
        ])
        .run(tauri::generate_context!());
    if let Err(e) = built {
        eprintln!("ClipCut could not start: {e}");
        std::process::exit(1);
    }
}
