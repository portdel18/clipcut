//! The commands the timeline and project buttons call: adding clips, editing on the magnetic main
//! track (split, Q/W, ripple delete, trim, reorder), undo/redo, and projects that save themselves.
//!
//! Every edit returns the whole [`TimelineView`] so the interface redraws from one source of truth.
//!
//! Projects live in `Documents\ClipCut\<name>\<name>.fcproj` (named after the first clip, made
//! unique), with their render previews beside them. The interface asks for a save shortly after
//! each change; the last project opened comes back at the next start.

use std::path::{Path, PathBuf};

use clipcut_core::{Keep, ProjectView, Studio, TimelineView};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::{App, OPEN_EXTENSIONS, blocking, lock};

/// Where ClipCut keeps projects: `Documents\ClipCut` (the home folder when there is no Documents).
pub(crate) fn projects_root(handle: &tauri::AppHandle) -> PathBuf {
    let base = handle.path().document_dir().or_else(|_| handle.path().home_dir()).unwrap_or_else(|_| PathBuf::from("."));
    base.join("ClipCut")
}

/// The file that remembers the last project, in the app's settings folder.
fn last_project_file(handle: &tauri::AppHandle) -> Option<PathBuf> {
    handle.path().app_config_dir().ok().map(|d| d.join("last-project.txt"))
}

/// The keys the user set in Keyboard shortcuts (only the actions they changed), in the settings
/// folder: `{"split": ["S"], ...}`.
fn shortcuts_file(handle: &tauri::AppHandle) -> Option<PathBuf> {
    handle.path().app_config_dir().ok().map(|d| d.join("shortcuts.json"))
}

/// The most a shortcuts file may hold: far more than every action with several keys.
const SHORTCUTS_MAX: usize = 64 * 1024;

#[tauri::command]
pub fn load_shortcuts(handle: tauri::AppHandle) -> serde_json::Value {
    shortcuts_file(&handle)
        .and_then(|f| std::fs::read(f).ok())
        .filter(|b| b.len() <= SHORTCUTS_MAX)
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}

#[tauri::command]
pub fn save_shortcuts(keys: serde_json::Value, handle: tauri::AppHandle) -> Result<(), String> {
    if !keys.is_object() {
        return Err("the shortcuts couldn't be saved: they were not a list of actions".into());
    }
    let f = shortcuts_file(&handle).ok_or("the shortcuts couldn't be saved: there is no settings folder")?;
    let text = serde_json::to_string_pretty(&keys).map_err(|e| e.to_string())?;
    if text.len() > SHORTCUTS_MAX {
        return Err("the shortcuts couldn't be saved: there are too many keys".into());
    }
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("the shortcuts couldn't be saved in {}: {e}", dir.display()))?;
    }
    std::fs::write(&f, text).map_err(|e| format!("the shortcuts couldn't be saved in {}: {e}", f.display()))
}

fn remember(handle: &tauri::AppHandle, path: Option<&str>) {
    let Some(f) = last_project_file(handle) else { return };
    match path {
        Some(p) => {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&f, p);
        }
        None => {
            let _ = std::fs::remove_file(&f);
        }
    }
}

/// A folder name from a clip or project name: no characters Windows refuses, never empty.
pub(crate) fn safe_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).collect();
    // at most 80 characters, and no trailing dot or space (Windows drops them, so the folder
    // made wouldn't be the one asked for)
    let n: String = n.trim().chars().take(80).collect();
    let n = n.trim_end_matches(['.', ' ']).trim_start().to_string();
    if n.is_empty() {
        return "Untitled".to_string();
    }
    // Windows' device names can't be file or folder names, with any extension
    let stem = n.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().nth(3).is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)));
    if device { format!("{n}_") } else { n }
}

/// `root/<name>/<name>.fcproj`, or `<name> 2`, `<name> 3`… when that folder is taken.
fn new_project_path(root: &Path, name: &str) -> PathBuf {
    let base = safe_name(name);
    for n in 1..10_000 {
        let folder = if n == 1 { base.clone() } else { format!("{base} {n}") };
        let dir = root.join(&folder);
        if !dir.exists() {
            return dir.join(format!("{folder}.fcproj"));
        }
    }
    root.join(format!("{base}.fcproj"))
}

fn view(studio: &std::sync::Arc<std::sync::Mutex<Studio>>) -> TimelineView {
    lock(studio).timeline()
}

/// Runs an edit with playback stopped (the player plays a snapshot of the sequence) and returns
/// the timeline afterwards.
pub(crate) async fn edit(
    app: &tauri::State<'_, App>,
    f: impl FnOnce(&mut Studio) -> clipcut_core::Result<()> + Send + 'static,
) -> Result<TimelineView, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        f(&mut lock(&studio)).map_err(|e| e.to_string())?;
        Ok(view(&studio))
    })
    .await
}

#[tauri::command]
pub fn timeline(app: tauri::State<'_, App>) -> TimelineView {
    view(&app.studio)
}

#[tauri::command]
pub async fn pick_clips(handle: tauri::AppHandle) -> Result<Vec<String>, String> {
    blocking(move || {
        let files = handle.dialog().file().set_title("Import media").add_filter("Media", OPEN_EXTENSIONS).blocking_pick_files().unwrap_or_default();
        Ok(files.into_iter().filter_map(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()).collect())
    })
    .await
}

/// Adds files in order: videos and pictures to the end of the main track, sound to a lane at
/// `seconds` (the playhead). A file that can't be added doesn't stop the rest: its reason comes
/// back in `errors`.
#[tauri::command]
pub async fn add_clips(paths: Vec<String>, seconds: Option<f64>, app: tauri::State<'_, App>) -> Result<AddResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut errors = Vec::new();
        {
            let mut s = lock(&studio);
            for p in &paths {
                let name = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
                match s.add_file(p, seconds.unwrap_or(0.0)) {
                    // added, but its picture shows the unreadable slate: say why now
                    Ok(item) => errors.extend(s.check_picture(item).map(|why| format!("{name}: {why}"))),
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
        }
        Ok(AddResult { timeline: view(&studio), errors })
    })
    .await
}

#[derive(serde::Serialize)]
pub struct AddResult {
    timeline: TimelineView,
    errors: Vec<String>,
}

/// A sound-only file into an empty project: the project starts with it on a sound lane.
#[tauri::command]
pub async fn start_with_sound(item: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.start_with_sound(item).map(|_| ())).await
}

#[tauri::command]
pub async fn append_media(item: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.append_item(item)).await
}

/// A Media card dropped on the main track, before the clip at `index`.
#[tauri::command]
pub async fn insert_media(item: u64, index: usize, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.insert_item(item, index)).await
}

#[tauri::command]
pub async fn split(seconds: f64, selection: Vec<u64>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.split(seconds, &selection).map(|_| ())).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitKeepResult {
    timeline: TimelineView,
    playhead: f64,
}

/// Q (`keep: "right"`) or W (`keep: "left"`).
#[tauri::command]
pub async fn split_keep(seconds: f64, selection: Vec<u64>, keep: String, app: tauri::State<'_, App>) -> Result<SplitKeepResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let side = if keep == "right" { Keep::Right } else { Keep::Left };
        let playhead = lock(&studio).split_keep(seconds, &selection, side).map_err(|e| e.to_string())?;
        lock(&player).set_position(playhead);
        Ok(SplitKeepResult { timeline: view(&studio), playhead })
    })
    .await
}

#[tauri::command]
pub async fn delete_clips(clips: Vec<u64>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.delete(&clips)).await
}

#[tauri::command]
pub async fn trim(clip: u64, edge: String, delta: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.trim(clip, edge == "in", delta).map(|_| ())).await
}

#[tauri::command]
pub async fn reorder(clip: u64, to: usize, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.reorder(clip, to)).await
}

#[tauri::command]
pub async fn undo(app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, |s| s.undo()).await
}

#[tauri::command]
pub async fn redo(app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, |s| s.redo()).await
}

/* ---------- lanes, text and properties ---------- */

/// Imports files into the Media panel without putting them on the timeline (the Audio tab's
/// Import). Returns the new media items in order, and why any file couldn't be read.
#[tauri::command]
pub async fn import_media(paths: Vec<String>, app: tauri::State<'_, App>) -> Result<ImportResult, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let mut s = lock(&studio);
        let (mut items, mut errors) = (Vec::new(), Vec::new());
        for p in &paths {
            match s.import(p) {
                Ok(i) => items.push(i),
                Err(e) => {
                    let name = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
                    errors.push(format!("{name}: {e}"));
                }
            }
        }
        Ok(ImportResult { timeline: s.timeline(), items, errors })
    })
    .await
}

#[derive(serde::Serialize)]
pub struct ImportResult {
    timeline: TimelineView,
    items: Vec<u64>,
    errors: Vec<String>,
}

/// Media on a lane at `seconds`: a picture above the main track, sound below it.
#[tauri::command]
pub async fn add_to_lane(item: u64, seconds: f64, app: tauri::State<'_, App>) -> Result<ClipResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let clips = s.add_to_lane(item, seconds).map_err(|e| e.to_string())?;
        Ok(ClipResult { timeline: s.timeline(), clips })
    })
    .await
}

#[derive(serde::Serialize)]
pub struct ClipResult {
    timeline: TimelineView,
    /// The clips the action made (to select them).
    clips: Vec<u64>,
}

#[tauri::command]
pub async fn add_text(seconds: f64, duration: f64, text: String, style: serde_json::Value, app: tauri::State<'_, App>) -> Result<ClipResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let clip = s.add_text(seconds, duration, &text, &style).map_err(|e| e.to_string())?;
        Ok(ClipResult { timeline: s.timeline(), clips: vec![clip] })
    })
    .await
}

#[tauri::command]
pub async fn extract_audio(clip: u64, app: tauri::State<'_, App>) -> Result<ClipResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let sound = s.extract_audio(clip).map_err(|e| e.to_string())?;
        Ok(ClipResult { timeline: s.timeline(), clips: vec![sound] })
    })
    .await
}

#[tauri::command]
pub async fn unlink_sound(clip: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.unlink_sound(clip)).await
}

#[tauri::command]
pub async fn link_sound(clip: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.link_sound(clip)).await
}

#[tauri::command]
pub async fn move_clip(clip: u64, lane: String, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.move_clip(clip, &lane, seconds)).await
}

/// Ctrl+C: keeps clips `clips` (and their linked sound) to paste. Returns how many.
#[tauri::command]
pub fn copy_clips(clips: Vec<u64>, app: tauri::State<'_, App>) -> Result<usize, String> {
    lock(&app.studio).copy_clips(&clips).map_err(|e| e.to_string())
}

/// Ctrl+V: copies of the copied clips at `seconds` (the same media: graphics are not rendered again).
#[tauri::command]
pub async fn paste_clips(seconds: f64, app: tauri::State<'_, App>) -> Result<ClipResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let clips = s.paste(seconds).map_err(|e| e.to_string())?;
        Ok(ClipResult { timeline: s.timeline(), clips })
    })
    .await
}

#[tauri::command]
pub async fn reorder_clips(clips: Vec<u64>, to: usize, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.reorder_clips(&clips, to)).await
}

#[tauri::command]
pub async fn move_clips(clips: Vec<u64>, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.move_clips(&clips, seconds)).await
}

/// Properties from the Properties panel or the preview; `merge` joins a slider's run of changes
/// into one undo step.
#[tauri::command]
pub async fn set_props(clips: Vec<u64>, props: serde_json::Value, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_props(&clips, &props, merge.as_deref())).await
}

#[tauri::command]
pub async fn set_canvas(width: u32, height: u32, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_canvas(width, height)).await
}

#[tauri::command]
pub async fn fit_clips(clips: Vec<u64>, fill: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.fit_clips(&clips, fill)).await
}

#[tauri::command]
pub async fn set_lane(lane: String, on: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_lane(&lane, on)).await
}

#[tauri::command]
pub async fn fonts(app: tauri::State<'_, App>) -> Result<Vec<String>, String> {
    let studio = app.studio.clone();
    blocking(move || Ok(lock(&studio).fonts())).await
}

/// A media item's waveform: one byte (0…255, the loudest sample) per 1/100 s. Worked out once per
/// item, without holding the session.
#[tauri::command]
pub async fn peaks(item: u64, app: tauri::State<'_, App>) -> Result<tauri::ipc::Response, String> {
    if let Some(p) = lock(&app.peaks).get(&item) {
        return Ok(tauri::ipc::Response::new(p.as_ref().clone()));
    }
    let reader = lock(&app.studio).peaks(item).map_err(|e| e.to_string())?;
    let cache = app.peaks.clone();
    blocking(move || {
        let p = std::sync::Arc::new(reader.compute());
        lock(&cache).insert(item, p.clone());
        Ok(tauri::ipc::Response::new(p.as_ref().clone()))
    })
    .await
}

/* ---------- projects ---------- */

/// Saves the project, giving it a file in `Documents\ClipCut` the first time. Nothing to save
/// (an empty project) is not an error.
#[tauri::command]
pub async fn save_project(handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<ProjectView, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let mut s = lock(&studio);
        let v = s.timeline();
        if !v.has_sequence {
            return Ok(v.project);
        }
        match v.project.path {
            Some(_) => s.save().map_err(|e| e.to_string())?,
            None => {
                let name = s.info().map(|i| i.name).unwrap_or_else(|_| "Untitled".into());
                let root = projects_root(&handle);
                let path = new_project_path(&root, &name);
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't make the project folder {}: {e}", dir.display()))?;
                }
                let p = path.to_string_lossy().into_owned();
                s.save_as(&p).map_err(|e| e.to_string())?;
                remember(&handle, Some(&p));
            }
        }
        Ok(s.timeline().project)
    })
    .await
}

/// The window is closing: an edit made in the last moment (before the interface's autosave) is
/// kept, for a project that already has its file.
pub fn save_on_close(handle: &tauri::AppHandle) {
    let Some(app) = handle.try_state::<App>() else { return };
    let mut s = lock(&app.studio);
    let v = s.timeline();
    if v.has_sequence && v.project.dirty && v.project.path.is_some() {
        let _ = s.save();
    }
}

#[tauri::command]
pub async fn new_project(handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    remember(&handle, None);
    // media ids start again in another project
    lock(&app.peaks).clear();
    edit(&app, |s| s.new_project("Untitled")).await
}

#[tauri::command]
pub async fn pick_project(handle: tauri::AppHandle) -> Result<Option<String>, String> {
    let root = projects_root(&handle);
    blocking(move || {
        let mut d = handle.dialog().file().set_title("Open a project").add_filter("ClipCut project", &["fcproj"]);
        if root.is_dir() {
            d = d.set_directory(&root);
        }
        Ok(d.blocking_pick_file().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub async fn open_project(path: String, handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    let p = path.clone();
    lock(&app.peaks).clear();
    let v = edit(&app, move |s| s.open_project(&p)).await?;
    remember(&handle, Some(&path));
    Ok(v)
}

/// At start-up: files named on the command line start a new project with them, in order; otherwise the last
/// project comes back (if its file still exists).
#[tauri::command]
pub async fn restore(handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<AddResult, String> {
    let startup = std::mem::take(&mut *lock(&app.startup_files));
    let last = last_project_file(&handle).and_then(|f| std::fs::read_to_string(f).ok()).map(|s| s.trim().to_string()).filter(|p| Path::new(p).is_file());
    let studio = app.studio.clone();
    // the user's own titles are kept in the app's data folder
    let data = handle.path().app_data_dir().ok();
    blocking(move || {
        let mut s = lock(&studio);
        if let Some(d) = &data {
            s.set_data_dir(d);
        }
        let mut errors = Vec::new();
        if !startup.is_empty() {
            for f in &startup {
                if let Err(e) = s.add_file(f, 0.0) {
                    let name = Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
                    errors.push(format!("{name}: {e}"));
                }
            }
        } else if let Some(p) = last
            && let Err(e) = s.open_project(&p)
        {
            errors.push(format!("the last project couldn't be opened ({e}); starting a new one"));
            let _ = s.new_project("Untitled");
        }
        Ok(AddResult { timeline: s.timeline(), errors })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_are_safe_and_unique() {
        assert_eq!(safe_name("Interview: take 2?"), "Interview  take 2");
        assert_eq!(safe_name("  ...  "), "Untitled");
        // Windows' device names, with or without an extension, get a mark so a folder can be made
        assert_eq!(safe_name("con."), "con_");
        assert_eq!(safe_name("NUL"), "NUL_");
        assert_eq!(safe_name("aux.final"), "aux.final_");
        assert_eq!(safe_name("COM1"), "COM1_");
        assert_eq!(safe_name("lpt9"), "lpt9_");
        assert_eq!(safe_name("COM"), "COM");
        assert_eq!(safe_name("Console"), "Console");
        assert_eq!(safe_name("Communities"), "Communities");
        // cut to 80 characters without leaving a trailing dot or space
        assert_eq!(safe_name(&format!("{}. end", "a".repeat(79))), "a".repeat(79));
        assert_eq!(safe_name(&"é".repeat(200)).chars().count(), 80);
        let root = std::env::temp_dir().join(format!("clipcut-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let first = new_project_path(&root, "Mayor interview");
        assert_eq!(first, root.join("Mayor interview").join("Mayor interview.fcproj"));
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        assert_eq!(new_project_path(&root, "Mayor interview"), root.join("Mayor interview 2").join("Mayor interview 2.fcproj"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/* ---------- transitions ---------- */

#[tauri::command]
pub fn transition_presets() -> Vec<clipcut_core::TransitionPresetView> {
    clipcut_core::transition_presets()
}

/// Every transition's preview, `frames` pictures each, as one buffer: `w`, `h`, `frames` (u32 LE)
/// then the RGBA pictures, transition by transition in [`transition_presets`]' order.
#[tauri::command]
pub async fn transition_previews(w: u32, h: u32, frames: u32) -> Result<tauri::ipc::Response, String> {
    blocking(move || {
        let f = clipcut_core::transition_previews(w, h, frames).map_err(|e| e.to_string())?;
        let first = f.first().ok_or("no transitions")?;
        let mut out = Vec::with_capacity(12 + f.iter().map(|x| x.rgba.len()).sum::<usize>());
        let n = u32::try_from(f.len() / clipcut_core::TRANSITIONS.len().max(1)).unwrap_or(0);
        for v in [first.w, first.h, n] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for x in &f {
            out.extend_from_slice(&x.rgba);
        }
        Ok(tauri::ipc::Response::new(out))
    })
    .await
}

#[tauri::command]
pub async fn set_transition(from: u64, preset: String, seconds: Option<f64>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_transition(from, &preset, seconds)).await
}

#[tauri::command]
pub async fn set_transition_length(from: u64, seconds: f64, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_transition_length(from, seconds, merge.as_deref())).await
}

#[tauri::command]
pub async fn remove_transitions(from: Vec<u64>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.remove_transitions(&from)).await
}

#[tauri::command]
pub async fn transition_everywhere(preset: String, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.transition_everywhere(&preset, seconds).map(|_| ())).await
}

/* ---------- keyframes ---------- */

/// Clip `clip`'s properties as they are at `seconds` (animated ones from their keyframes).
#[tauri::command]
pub fn props_at(clip: u64, seconds: f64, app: tauri::State<'_, App>) -> Result<clipcut_core::ClipProps, String> {
    lock(&app.studio).props_at(clip, seconds).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_keyframe(clip: u64, prop: String, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.toggle_keyframe(clip, &prop, seconds).map(|_| ())).await
}

#[tauri::command]
pub async fn delete_keyframes(clip: u64, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.delete_keyframes(clip, seconds)).await
}

#[tauri::command]
pub async fn move_keyframes(clip: u64, from: f64, to: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.move_keyframes(clip, from, to)).await
}

#[tauri::command]
pub async fn set_hold(clip: u64, hold: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_hold(clip, hold)).await
}

#[tauri::command]
pub async fn set_easing(clip: u64, smooth: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_easing(clip, smooth)).await
}

/* ---------- colour: adjustments, filters, scopes ---------- */

#[tauri::command]
pub async fn set_colour(clips: Vec<u64>, values: serde_json::Value, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_colour(&clips, &values, merge.as_deref())).await
}

#[tauri::command]
pub async fn set_look(
    clips: Vec<u64>,
    look: Option<String>,
    intensity: f64,
    merge: Option<String>,
    app: tauri::State<'_, App>,
) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_look(&clips, look.as_deref(), intensity, merge.as_deref())).await
}

#[tauri::command]
pub async fn reset_colour(clips: Vec<u64>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.reset_colour(&clips)).await
}

#[tauri::command]
pub async fn colour_everywhere(clip: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.colour_everywhere(clip).map(|_| ())).await
}

#[tauri::command]
pub fn looks() -> Vec<&'static str> {
    clipcut_core::looks()
}

/// Frames as one buffer: count, then each frame's width and height (u32 LE) and RGBA pixels.
fn frames_response(frames: Vec<clipcut_core::Frame>) -> tauri::ipc::Response {
    let mut out = Vec::with_capacity(4 + frames.iter().map(|f| 8 + f.rgba.len()).sum::<usize>());
    out.extend_from_slice(&u32::try_from(frames.len()).unwrap_or(0).to_le_bytes());
    for f in &frames {
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        out.extend_from_slice(&f.rgba);
    }
    tauri::ipc::Response::new(out)
}

/// The Filters tab's previews: clip `clip`'s picture at `seconds`, then with each look.
#[tauri::command]
pub async fn look_previews(clip: u64, seconds: f64, width: u32, app: tauri::State<'_, App>) -> Result<tauri::ipc::Response, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let s = lock(&studio);
        s.look_previews(clip, seconds, width).map(frames_response).map_err(|e| e.to_string())
    })
    .await
}

/// Scope `kind` of the program frame at `seconds` (8 bytes of size, then RGBA, as `frame`).
#[tauri::command]
pub async fn scope(seconds: f64, kind: String, app: tauri::State<'_, App>) -> Result<tauri::ipc::Response, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let f = lock(&studio).scope(seconds, &kind).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(8 + f.rgba.len());
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        out.extend_from_slice(&f.rgba);
        Ok(tauri::ipc::Response::new(out))
    })
    .await
}

/// Sets a sound lane's (or the Mix's) volume, pan, mute or solo. Playback goes on, with the new mix.
#[tauri::command]
pub async fn set_mixer(lane: String, values: serde_json::Value, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        let mut s = lock(&studio);
        s.set_mixer(&lane, &values, merge.as_deref()).map_err(|e| e.to_string())?;
        let mut p = lock(&player);
        if p.playing()
            && let Ok(feed) = s.audio_feed()
        {
            p.update(feed);
        }
        Ok(s.timeline())
    })
    .await
}

/// Each sound lane's and the Mix's peak level since the last call, while playing.
#[tauri::command]
pub fn meters(app: tauri::State<'_, App>) -> Vec<clipcut_core::Level> {
    lock(&app.player).meters()
}

#[tauri::command]
pub async fn trim_to(clip: u64, edge_in: bool, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.trim_to(clip, edge_in, seconds).map(|_| ())).await
}

#[tauri::command]
pub async fn nudge(clips: Vec<u64>, frames: i64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.nudge(&clips, frames)).await
}

#[tauri::command]
pub async fn slip(clip: u64, frames: i64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.slip(clip, frames).map(|_| ())).await
}

#[tauri::command]
pub async fn roll(from: u64, frames: i64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.roll(from, frames)).await
}

/// The text animations, in and out, for the Animation section.
#[tauri::command]
pub fn text_animations() -> serde_json::Value {
    clipcut_core::text_animations()
}

#[tauri::command]
pub async fn set_text_animation(
    clips: Vec<u64>,
    enter: bool,
    id: Option<String>,
    seconds: f64,
    merge: Option<String>,
    app: tauri::State<'_, App>,
) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_text_animation(&clips, enter, id.as_deref(), seconds, merge.as_deref())).await
}

/// Music that ducks under speech (or not): the timeline, and how many dips were made.
#[derive(serde::Serialize)]
pub struct Ducked {
    timeline: TimelineView,
    dips: usize,
}

#[tauri::command]
pub async fn set_ducking(clips: Vec<u64>, on: bool, reduce_db: f64, app: tauri::State<'_, App>) -> Result<Ducked, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let dips = s.set_ducking(&clips, on, reduce_db).map_err(|e| e.to_string())?;
        Ok(Ducked { timeline: s.timeline(), dips })
    })
    .await
}

/// Beats found in a sound clip: the timeline, how many and the tempo.
#[derive(serde::Serialize)]
pub struct Beats {
    timeline: TimelineView,
    count: usize,
    bpm: f64,
}

#[tauri::command]
pub async fn mark_beats(clip: u64, app: tauri::State<'_, App>) -> Result<Beats, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let (count, bpm) = s.mark_beats(clip).map_err(|e| e.to_string())?;
        Ok(Beats { timeline: s.timeline(), count, bpm })
    })
    .await
}

#[tauri::command]
pub async fn clear_beats(clip: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.clear_beats(clip)).await
}

#[tauri::command]
pub async fn toggle_beat(clip: u64, seconds: f64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.toggle_beat(clip, seconds).map(|_| ())).await
}

/* ---------- titles (the engine's graphics templates, and the user's own) ---------- */

#[tauri::command]
pub async fn titles(app: tauri::State<'_, App>) -> Result<Vec<clipcut_core::TitleView>, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).titles().map_err(|e| e.to_string())).await
}

/// A title's picture as base64 PNG.
#[tauri::command]
pub async fn title_thumbnail(id: String, width: u32, app: tauri::State<'_, App>) -> Result<String, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).title_thumbnail(&id, width).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn add_title(id: String, seconds: f64, app: tauri::State<'_, App>) -> Result<ClipResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let clip = s.add_title(&id, seconds).map_err(|e| e.to_string())?;
        Ok(ClipResult { timeline: s.timeline(), clips: vec![clip] })
    })
    .await
}

#[tauri::command]
pub async fn title_controls(clip: u64, app: tauri::State<'_, App>) -> Result<serde_json::Value, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).title_controls(clip).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn set_title(clip: u64, values: serde_json::Value, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_title(clip, &values, merge.as_deref())).await
}

#[tauri::command]
pub async fn save_title(clip: u64, name: String, app: tauri::State<'_, App>) -> Result<String, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).save_title(clip, &name).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn remove_title(id: String, app: tauri::State<'_, App>) -> Result<(), String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).remove_title(&id).map_err(|e| e.to_string())).await
}

/* ---------- proxies (small copies for smooth playback) ---------- */

/// Starts making proxies of `items` (every video when empty); returns the job to follow with
/// `export_status`.
#[tauri::command]
pub async fn make_proxies(items: Vec<u64>, app: tauri::State<'_, App>) -> Result<u64, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).make_proxies(&items).map_err(|e| e.to_string())).await
}

/// Attaches proxies that finished: the timeline when something changed.
#[tauri::command]
pub fn poll_jobs(app: tauri::State<'_, App>) -> Option<TimelineView> {
    let mut s = lock(&app.studio);
    s.poll().then(|| s.timeline())
}

#[tauri::command]
pub async fn set_proxies(on: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_proxies(on)).await
}

/* ---------- sound sides, freeze, reverse, pitch, colour key ---------- */

#[tauri::command]
pub async fn set_channels(
    clips: Vec<u64>,
    mode: String,
    left_db: f64,
    right_db: f64,
    merge: Option<String>,
    app: tauri::State<'_, App>,
) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_channels(&clips, &mode, left_db, right_db, merge.as_deref())).await
}

/// Measures a clip's two sides (it decodes the sound, so it runs off the interface's thread).
#[tauri::command]
pub async fn channel_levels(clip: u64, app: tauri::State<'_, App>) -> Result<clipcut_core::ChannelLevels, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).channel_levels(clip).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn freeze(seconds: f64, clip: Option<u64>, length: f64, app: tauri::State<'_, App>) -> Result<ClipResult, String> {
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        let mut s = lock(&studio);
        let still = s.freeze(seconds, clip, length).map_err(|e| e.to_string())?;
        Ok(ClipResult { timeline: s.timeline(), clips: vec![still] })
    })
    .await
}

#[tauri::command]
pub async fn set_reverse(clips: Vec<u64>, on: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_reverse(&clips, on)).await
}

#[tauri::command]
pub async fn set_keep_pitch(clips: Vec<u64>, on: bool, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_keep_pitch(&clips, on)).await
}

/// A colour key as the interface sends it.
#[derive(serde::Deserialize)]
pub struct KeyArgs {
    color: String,
    strength: String,
    soften: f64,
    spill: f64,
}

#[tauri::command]
pub async fn set_key(clips: Vec<u64>, key: Option<KeyArgs>, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    edit(&app, move |s| s.set_key(&clips, key.as_ref().map(|k| (k.color.as_str(), k.strength.as_str(), k.soften, k.spill)), merge.as_deref())).await
}

#[tauri::command]
pub async fn pick_colour(seconds: f64, x: f64, y: f64, app: tauri::State<'_, App>) -> Result<String, String> {
    let studio = app.studio.clone();
    blocking(move || lock(&studio).pick_colour(seconds, x, y).map_err(|e| e.to_string())).await
}
