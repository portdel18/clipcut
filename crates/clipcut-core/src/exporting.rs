//! Quick exports, CapCut-style: a few choices by where the video goes (as the project is,
//! TikTok / Reels / Shorts, YouTube, YouTube 4K, a small file to send, the sound alone), each
//! a size, frame rate and bitrate for the engine's export; and a ProRes master.

use serde::Serialize;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

/// One way to export, as the Export dialog shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportChoice {
    pub id: &'static str,
    pub name: &'static str,
    /// What it makes, in a line (`1080 × 1920, 30 fps, about 18 MB`).
    pub detail: String,
    /// A word of warning, when the video's shape doesn't suit it.
    pub note: Option<String>,
    pub width: u32,
    pub height: u32,
    /// `mp4`, `mov` or `wav`.
    pub ext: &'static str,
    /// Roughly how big the file will be, bytes.
    pub bytes: u64,
}

/// Bits a second of sound in an MP4 (AAC), and of 48 kHz 16-bit stereo WAV.
const AAC_KBPS: f64 = 320.0;
const WAV_KBPS: f64 = 48_000.0 * 16.0 * 2.0 / 1000.0;

/// `w × h` scaled so its long side is `long` (even numbers, as H.264 needs).
fn long_side(w: u32, h: u32, long: u32) -> (u32, u32) {
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let k = f64::from(long) / w.max(h);
    let even = |v: f64| ((v * k / 2.0).round() as u32).max(1) * 2;
    (even(w), even(h))
}

/// `w × h` scaled so its short side is `short`.
fn short_side(w: u32, h: u32, short: u32) -> (u32, u32) {
    let (fw, fh) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let long = (f64::from(short) * fw.max(fh) / fw.min(fh)).round() as u32;
    long_side(w, h, long)
}

/// `16:9`, `9:16`, `1:1`, `4:5`, or the size.
fn shape_name(w: u32, h: u32) -> String {
    let r = f64::from(w) / f64::from(h.max(1));
    for (a, b) in [(16, 9), (9, 16), (1, 1), (4, 5)] {
        if (r - f64::from(a) / f64::from(b)).abs() < 0.01 {
            return format!("{a}:{b}");
        }
    }
    format!("{w} × {h}")
}

fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 1000.0 {
        format!("about {:.1} GB", mb / 1000.0)
    } else if mb >= 10.0 {
        format!("about {mb:.0} MB")
    } else {
        format!("about {mb:.1} MB")
    }
}

impl Studio {
    /// The ways to export the video as it is now.
    pub fn export_choices(&self) -> Result<Vec<ExportChoice>> {
        let info = self.info()?;
        let (w, h, fps, secs) = (info.width, info.height, info.fps, info.duration_seconds.max(0.0));
        let vertical = h > w && (f64::from(w) / f64::from(h) - 9.0 / 16.0).abs() < 0.01;
        let shape = shape_name(w, h);
        let rate = if (fps - fps.round()).abs() < 0.01 { format!("{fps:.0}") } else { format!("{fps:.2}") };
        let bytes = |kbps: f64| ((kbps + AAC_KBPS) * 1000.0 / 8.0 * secs) as u64;
        let video = |id, name, (cw, ch): (u32, u32), kbps: f64, note: Option<String>| {
            let b = bytes(kbps);
            ExportChoice { id, name, detail: format!("{cw} × {ch}, {rate} fps, {}", size_text(b)), note, width: cw, height: ch, ext: "mp4", bytes: b }
        };
        // the engine's own default: 0.2 bits a pixel
        let own = f64::from(w) * f64::from(h) * fps * 0.2 / 1000.0;
        let social_note =
            (!vertical).then(|| format!("Your video is {shape}: it will have bars above and below. Change Ratio to 9:16 first to fill the phone's screen."));
        // ProRes 422 HQ: about 4.4 bits a pixel
        let prores = f64::from(w) * f64::from(h) * fps * 4.4 / 1000.0;
        let wav = ((WAV_KBPS * 1000.0 / 8.0) * secs) as u64;
        Ok(vec![
            video("project", "As the project", (w, h), own, None),
            video("social", "TikTok, Reels, Shorts", (1080, 1920), 12_000.0, social_note),
            video("youtube", "YouTube", long_side(w, h, 1920), 16_000.0, None),
            video("youtube4k", "YouTube 4K", long_side(w, h, 3840), 45_000.0, None),
            video("small", "Small file", short_side(w, h, 720), 4_000.0, Some("For sending in a message: smaller, a little softer.".into())),
            ExportChoice {
                id: "master",
                name: "Master (ProRes)",
                detail: format!("{w} × {h}, {rate} fps, ProRes 422 HQ, {}", size_text(bytes(prores))),
                note: Some("Best quality, for editing again or archiving: a large file.".into()),
                width: w,
                height: h,
                ext: "mov",
                bytes: bytes(prores),
            },
            ExportChoice {
                id: "sound",
                name: "Sound only",
                detail: format!("WAV, 48 kHz stereo, {}", size_text(wav)),
                note: None,
                width: 0,
                height: 0,
                ext: "wav",
                bytes: wav,
            },
        ])
    }

    /// Exports the whole video to `path` the way export choice `choice` says (one of
    /// [`Studio::export_choices`]). Returns the job id.
    pub fn export_as(&mut self, path: &str, choice: &str) -> Result<u64> {
        self.export_as_with(path, choice, true)
    }

    /// [`Studio::export_as`], with the graphics card's encoder allowed (`hardware`) or not (the
    /// software encoder only: the retry after a hardware encoder failed partway).
    pub fn export_as_with(&mut self, path: &str, choice: &str, hardware: bool) -> Result<u64> {
        let c = self.export_choices()?.into_iter().find(|c| c.id == choice).ok_or_else(|| Error::Invalid(format!("there is no export called “{choice}”")))?;
        let fps = self.info()?.fps;
        // the GPU composites and encodes where the machine has one that can (a real adapter;
        // NVENC for H.264); everything else falls back to the CPU, frame by frame
        let hw = if hardware { "auto" } else { "off" };
        let mut p = json!({"path": path, "burnCaptions": self.burn_captions(), "gpuRendering": "auto", "hardwareEncoding": hw});
        match c.id {
            "sound" => p["format"] = json!("wav"),
            "master" => p["format"] = json!("prores"),
            "project" => p["format"] = json!("h264"),
            id => {
                p["format"] = json!("h264");
                p["width"] = json!(c.width);
                p["height"] = json!(c.height);
                p["fps"] = json!(fps);
                let kbps = match id {
                    "social" => 12_000,
                    "youtube" => 16_000,
                    "youtube4k" => 45_000,
                    _ => 4_000,
                };
                p["bitrateKbps"] = json!(kbps);
            }
        }
        if !path.to_ascii_lowercase().ends_with(&format!(".{}", c.ext)) {
            return Err(Error::Invalid(format!("save it as a .{} file", c.ext)));
        }
        let r = self.session.execute("file.exportMedia", p).map_err(engine)?;
        r.get("job").and_then(serde_json::Value::as_u64).ok_or_else(|| Error::Engine("the export did not start".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_keep_the_shape_in_even_numbers() {
        assert_eq!(long_side(1280, 720, 1920), (1920, 1080));
        assert_eq!(long_side(720, 1280, 1920), (1080, 1920));
        assert_eq!(long_side(1080, 1350, 3840), (3072, 3840));
        assert_eq!(short_side(1920, 1080, 720), (1280, 720));
        assert_eq!(short_side(1080, 1920, 720), (720, 1280));
        assert_eq!(short_side(1000, 1000, 720), (720, 720));
        assert_eq!(size_text(18_400_000), "about 18 MB");
        assert_eq!((shape_name(1280, 720), shape_name(1080, 1350), shape_name(1000, 300)), ("16:9".into(), "4:5".into(), "1000 × 300".into()));
        assert_eq!(size_text(2_340_000), "about 2.3 MB");
    }
}
