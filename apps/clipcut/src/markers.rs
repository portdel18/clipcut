//! Markers with notes: add, change, move, delete, and YouTube chapters from them. Markers don't
//! change the picture or the sound, so these leave playback running (a marker can be dropped
//! while the video plays).

use clipcut_core::{Chapters, TimelineView};
use serde_json::Value;

use crate::{App, blocking, lock};

#[derive(serde::Serialize)]
pub struct Marked {
    timeline: TimelineView,
    marker: u64,
}

#[tauri::command]
pub async fn add_marker(seconds: f64, name: String, notes: String, colour: Option<String>, app: tauri::State<'_, App>) -> Result<Marked, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let mut s = lock(&studio);
        let marker = s.add_marker(seconds, &name, &notes, colour.as_deref()).map_err(|e| e.to_string())?;
        Ok(Marked { timeline: s.timeline(), marker })
    })
    .await
}

/// `changes`: any of `name`, `notes`, `colour`, `seconds`. Calls with the same `merge` key are
/// one undo step (typing).
#[tauri::command]
pub async fn edit_marker(id: u64, changes: Value, merge: Option<String>, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let mut s = lock(&studio);
        s.edit_marker(id, &changes, merge.as_deref()).map_err(|e| e.to_string())?;
        Ok(s.timeline())
    })
    .await
}

#[tauri::command]
pub async fn delete_marker(id: u64, app: tauri::State<'_, App>) -> Result<TimelineView, String> {
    let studio = app.studio.clone();
    blocking(move || {
        let mut s = lock(&studio);
        s.delete_marker(id).map_err(|e| e.to_string())?;
        Ok(s.timeline())
    })
    .await
}

#[tauri::command]
pub fn chapters(app: tauri::State<'_, App>) -> Result<Chapters, String> {
    lock(&app.studio).chapters().map_err(|e| e.to_string())
}
