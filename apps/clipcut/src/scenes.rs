//! Split at scene changes: the clip's frames are read off the studio (progress as `scene-progress`
//! events, `0…1`; `cancel_scenes` stops it without changing anything), then the clip is cut at
//! each change in one undo step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clipcut_core::TimelineView;
use tauri::Emitter;

use crate::{App, blocking, lock};

/// Asks a running scene search to stop.
static CANCEL: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn cancel_scenes() {
    CANCEL.store(true, Ordering::Relaxed);
}

#[derive(serde::Serialize)]
pub struct SceneSplit {
    timeline: TimelineView,
    /// Cuts made.
    cuts: usize,
}

#[tauri::command]
pub async fn split_scenes(clip: u64, handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<SceneSplit, String> {
    CANCEL.store(false, Ordering::Relaxed);
    let (studio, player) = (app.studio.clone(), app.player.clone());
    blocking(move || {
        lock(&player).stop();
        // the studio is held only to gather the frames' times and to make the cuts
        let scan = lock(&studio).scene_scan(clip).map_err(|e| e.to_string())?;
        let mut shown = Instant::now().checked_sub(Duration::from_secs(1)).unwrap_or_else(Instant::now);
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan.run(&mut |f| {
                if shown.elapsed() >= Duration::from_millis(100) || f >= 1.0 {
                    shown = Instant::now();
                    let _ = handle.emit("scene-progress", f.clamp(0.0, 1.0));
                }
                !CANCEL.load(Ordering::Relaxed)
            })
        }))
        .map_err(|_| "looking for scene changes failed; please try again".to_string())?
        .map_err(|e| e.to_string())?;
        let mut s = lock(&studio);
        let cuts = s.split_at_scenes(clip, &found).map_err(|e| e.to_string())?;
        Ok(SceneSplit { timeline: s.timeline(), cuts })
    })
    .await
}
