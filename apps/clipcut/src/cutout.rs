//! The background-removal models Graphics' Thumbnails uses (Clip Studio's `js/cutout.js` asks for one
//! through `HOST.model`, ClipCut's `ui/app.js`): Standard is ISNet general-use (DIS, Apache-2.0),
//! Best and Best for people BiRefNet general and portrait (MIT). Each is downloaded once from
//! rembg's release into the models folder and checked by size and SHA-256 every time it is read.
//! The page's own download would need the release to allow it (CORS); here it doesn't matter.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tauri::Emitter;

/// A model Clip Studio pins (`MODELS` in its `js/cutout.js`: the same names, files and checksums).
pub(crate) struct Model {
    pub name: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

const RELEASE: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/";

pub(crate) const MODELS: [Model; 3] = [
    Model {
        name: "isnet-general-use",
        file: "isnet-general-use.onnx",
        sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a",
        bytes: 178_648_008,
    },
    Model {
        name: "birefnet-general",
        file: "BiRefNet-general-epoch_244.onnx",
        sha256: "58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c",
        bytes: 972_666_916,
    },
    Model {
        name: "birefnet-portrait",
        file: "BiRefNet-portrait-epoch_150.onnx",
        sha256: "1ba1c8ff5a7bbfadc8d8d13fb11d7be793f91f23d9d466549e37a854f6668f99",
        bytes: 972_666_916,
    },
];

impl Model {
    fn url(&self) -> String {
        format!("{RELEASE}{}", self.file)
    }
}

pub(crate) fn find(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The kept model, when it is there and whole.
fn kept(m: &Model, path: &Path) -> Option<Vec<u8>> {
    let len = std::fs::metadata(path).ok()?.len();
    if len != m.bytes {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    (sha(&data) == m.sha256).then_some(data)
}

/// Model `m`'s bytes: kept in `dir`, else downloaded there (`progress(done, total)` as it comes).
pub(crate) async fn fetch(m: &'static Model, dir: &Path, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<Vec<u8>, String> {
    let path = dir.join(m.file);
    let p = path.clone();
    if let Some(d) = tauri::async_runtime::spawn_blocking(move || kept(m, &p)).await.map_err(|e| e.to_string())? {
        return Ok(d);
    }
    let (url, bytes) = (m.url(), m.bytes);
    let fail = |e: reqwest::Error| format!("Couldn't download the background model ({e}). Check the internet connection and try again.");
    let mut r = crate::updates::client()?.get(&url).send().await.map_err(fail)?;
    if !r.status().is_success() {
        return Err(format!("Couldn't download the background model (the server answered {}). Try again later.", r.status().as_u16()));
    }
    let mut data = Vec::with_capacity(usize::try_from(bytes).unwrap_or(0));
    let mut last = 0u64;
    while let Some(chunk) = r.chunk().await.map_err(fail)? {
        if (data.len() + chunk.len()) as u64 > bytes {
            return Err("The background model download is bigger than it should be, so ClipCut won't use it. Try again later.".into());
        }
        data.extend_from_slice(&chunk);
        let done = data.len() as u64;
        if done - last >= 1 << 20 || done == bytes {
            last = done;
            progress(done, bytes);
        }
    }
    if data.len() as u64 != bytes {
        return Err(format!("The background model download was cut short ({} of {bytes} bytes). Try again.", data.len()));
    }
    let (dir, keep) = (dir.to_path_buf(), path);
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        if sha(&data) != m.sha256 {
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
    let m = find(&name).ok_or_else(|| format!("ClipCut has no model called {name}."))?;
    let dir = models_dir(&handle);
    let h = handle.clone();
    let data = fetch(m, &dir, &move |done, total| {
        let _ = h.emit("cutout-model", serde_json::json!({ "done": done, "total": total }));
    })
    .await?;
    Ok(tauri::ipc::Response::new(data))
}

/// The size the release gives for model `m`'s file, without downloading it.
async fn listed_size(m: &Model) -> Result<u64, String> {
    let r = crate::updates::client()?.head(m.url()).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("the server answered {}", r.status().as_u16()));
    }
    // the header itself: `content_length()` is the body's size, nothing for a HEAD
    r.headers().get(reqwest::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).ok_or_else(|| "the server gave no size".into())
}

/// `--model-test <dir> <report.json>`, without a window (Windows CI): downloads the Standard model
/// into `dir` (or finds it there) and checks it is the file the checksum pins; each Best model's
/// release file must be there at the size pinned (their checksums were taken from those files).
pub(crate) fn model_test(dir: &str, report: &str) -> i32 {
    let started = std::time::Instant::now();
    let standard = &MODELS[0];
    let r = tauri::async_runtime::block_on(fetch(standard, Path::new(dir), &|_, _| {}));
    let others: Vec<serde_json::Value> = MODELS[1..]
        .iter()
        .map(|m| {
            let size = tauri::async_runtime::block_on(listed_size(m));
            serde_json::json!({"name": m.name, "listed": size.as_ref().ok(), "error": size.as_ref().err(), "ok": size.as_ref().is_ok_and(|n| *n == m.bytes)})
        })
        .collect();
    let all = r.is_ok() && others.iter().all(|o| o["ok"].as_bool() == Some(true));
    let out = serde_json::json!({
        "ok": all,
        "error": r.as_ref().err(),
        "bytes": r.as_ref().map(|d| d.len()).unwrap_or(0),
        "kept": Path::new(dir).join(standard.file).exists(),
        "others": others,
        "seconds": started.elapsed().as_secs_f64(),
    });
    let _ = std::fs::write(report, serde_json::to_string_pretty(&out).unwrap_or_default());
    if all { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_model_is_used_only_when_it_is_whole_and_the_right_file() {
        let dir = std::env::temp_dir().join(format!("clipcut-cutout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = &MODELS[0];
        let path = dir.join(m.file);
        assert!(kept(m, &path).is_none(), "nothing there");
        std::fs::write(&path, b"short").unwrap();
        assert!(kept(m, &path).is_none(), "the wrong size");
        // the right size with other bytes: the checksum refuses it
        std::fs::write(&path, vec![0u8; m.bytes as usize]).unwrap();
        assert!(kept(m, &path).is_none(), "the wrong checksum");
        // every model has its own name and file
        assert_eq!(find("birefnet-general").map(|m| m.file), Some("BiRefNet-general-epoch_244.onnx"));
        assert_eq!(find("birefnet-portrait").map(|m| m.bytes), Some(972_666_916));
        assert!(find("u2net").is_none());
        let files: std::collections::HashSet<_> = MODELS.iter().map(|m| m.file).collect();
        assert_eq!(files.len(), MODELS.len());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(sha(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
