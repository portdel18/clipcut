//! The background-removal model Graphics' Thumbnails uses (Clip Studio's `js/cutout.js` asks for it
//! through `HOST.model`, ClipCut's `ui/app.js`): ISNet general-use (DIS, Apache-2.0), downloaded once
//! from rembg's release into the models folder and checked by size and SHA-256 every time it is read.
//! The page's own download would need the release to allow it (CORS); here it doesn't matter.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tauri::Emitter;

/// The model Clip Studio pins (`MODEL` in its `js/cutout.js`).
pub(crate) const NAME: &str = "isnet-general-use";
const FILE: &str = "isnet-general-use.onnx";
pub(crate) const URL: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx";
pub(crate) const SHA256: &str = "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a";
pub(crate) const BYTES: u64 = 178_648_008;

fn sha(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The kept model, when it is there and whole.
fn kept(path: &Path) -> Option<Vec<u8>> {
    let len = std::fs::metadata(path).ok()?.len();
    if len != BYTES {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    (sha(&data) == SHA256).then_some(data)
}

/// The model's bytes: kept in `dir`, else downloaded there (`progress(done, total)` as it comes).
pub(crate) async fn fetch(dir: &Path, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<Vec<u8>, String> {
    let path = dir.join(FILE);
    let p = path.clone();
    if let Some(d) = tauri::async_runtime::spawn_blocking(move || kept(&p)).await.map_err(|e| e.to_string())? {
        return Ok(d);
    }
    let fail = |e: reqwest::Error| format!("Couldn't download the background model ({e}). Check the internet connection and try again.");
    let mut r = crate::updates::client()?.get(URL).send().await.map_err(fail)?;
    if !r.status().is_success() {
        return Err(format!("Couldn't download the background model (the server answered {}). Try again later.", r.status().as_u16()));
    }
    let mut data = Vec::with_capacity(usize::try_from(BYTES).unwrap_or(0));
    let mut last = 0u64;
    while let Some(chunk) = r.chunk().await.map_err(fail)? {
        if (data.len() + chunk.len()) as u64 > BYTES {
            return Err("The background model download is bigger than it should be, so ClipCut won't use it. Try again later.".into());
        }
        data.extend_from_slice(&chunk);
        let done = data.len() as u64;
        if done - last >= 1 << 20 || done == BYTES {
            last = done;
            progress(done, BYTES);
        }
    }
    if data.len() as u64 != BYTES {
        return Err(format!("The background model download was cut short ({} of {BYTES} bytes). Try again.", data.len()));
    }
    let (dir, keep) = (dir.to_path_buf(), path);
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        if sha(&data) != SHA256 {
            return Err("The background model that came down isn't the right file (its checksum is wrong), so ClipCut won't use it. Try again later.".into());
        }
        // kept for next time; a folder that can't be written only means downloading again
        if std::fs::create_dir_all(&dir).is_ok() {
            let part = keep.with_extension("part");
            if std::fs::write(&part, &data).is_ok() {
                let _ = std::fs::rename(&part, &keep);
            }
        }
        Ok(data)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where the model is kept: beside the speech models.
fn models_dir(handle: &tauri::AppHandle) -> PathBuf {
    crate::captioning::models_dir(handle)
}

/// The model `name` for Clip Studio, downloading it the first time (with `cutout-model` events:
/// `{done, total}` bytes).
#[tauri::command]
pub(crate) async fn cutout_model(name: String, handle: tauri::AppHandle) -> Result<tauri::ipc::Response, String> {
    if name != NAME {
        return Err(format!("ClipCut has no model called {name}."));
    }
    let dir = models_dir(&handle);
    let h = handle.clone();
    let data = fetch(&dir, &move |done, total| {
        let _ = h.emit("cutout-model", serde_json::json!({ "done": done, "total": total }));
    })
    .await?;
    Ok(tauri::ipc::Response::new(data))
}

/// `--model-test <dir> <report.json>`: downloads the model into `dir` (or finds it there) and checks
/// it, without a window (Windows CI: the release is where `URL` says, and is the file `SHA256` pins).
pub(crate) fn model_test(dir: &str, report: &str) -> i32 {
    let started = std::time::Instant::now();
    let r = tauri::async_runtime::block_on(fetch(Path::new(dir), &|_, _| {}));
    let out = serde_json::json!({
        "ok": r.is_ok(),
        "error": r.as_ref().err(),
        "bytes": r.as_ref().map(|d| d.len()).unwrap_or(0),
        "kept": Path::new(dir).join(FILE).exists(),
        "seconds": started.elapsed().as_secs_f64(),
    });
    let _ = std::fs::write(report, serde_json::to_string_pretty(&out).unwrap_or_default());
    if r.is_ok() { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_model_is_used_only_when_it_is_whole_and_the_right_file() {
        let dir = std::env::temp_dir().join(format!("clipcut-cutout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        assert!(kept(&path).is_none(), "nothing there");
        std::fs::write(&path, b"short").unwrap();
        assert!(kept(&path).is_none(), "the wrong size");
        // the right size with other bytes: the checksum refuses it
        std::fs::write(&path, vec![0u8; BYTES as usize]).unwrap();
        assert!(kept(&path).is_none(), "the wrong checksum");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(sha(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
