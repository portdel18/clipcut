//! Check playback (the Media panel): measures, on this computer, why a video plays slowly: which
//! decoder takes it (the graphics card or the processor, and why the graphics card declined), how
//! many frames a second it decodes, and how fast its drive reads it. The report is shown in plain
//! words and copied as text, so it can be sent along.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use clipcut_core::{FrameFeed, MediaProbe};
use filmcraft_time::Tick;
use serde::Serialize;

/// Log lines kept for the report (the engine logs why a hardware decoder declined a video).
const KEPT: usize = 200;
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

struct Recent;

impl log::Log for Recent {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info && (m.target().starts_with("filmcraft") || m.target().starts_with("clipcut"))
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let mut q = LOG.lock().unwrap_or_else(PoisonError::into_inner);
        if q.len() >= KEPT {
            q.pop_front();
        }
        q.push_back(format!("{}", r.args()));
    }
    fn flush(&self) {}
}

static RECENT: Recent = Recent;

/// Keeps the engine's recent log lines for the report (call once at startup).
pub fn keep_log() {
    if log::set_logger(&RECENT).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// What the engine said about hardware decoding since ClipCut started.
fn decoder_notes() -> Vec<String> {
    let q = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let mut out: Vec<String> = Vec::new();
    for l in q.iter().filter(|l| l.contains("hardware") || l.contains("NVDEC") || l.contains("VA-API") || l.contains("decod")) {
        if !out.contains(l) {
            out.push(l.clone());
        }
    }
    let n = out.len();
    out.split_off(n.saturating_sub(8))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCheck {
    pub name: String,
    /// Plain-words findings, the most important first.
    pub findings: Vec<String>,
    /// Plays smoothly from the file itself.
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub videos: Vec<VideoCheck>,
    /// Everything measured, to copy.
    pub text: String,
}

/// Disk: sequential MB/s over up to 32 MB from a third into the file, and the average time of
/// 12 reads of 256 KB spread over it (a hard drive's seeks), in ms.
fn drive(path: &str, size: u64) -> Option<(f64, f64)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    f.seek(SeekFrom::Start(size / 3)).ok()?;
    let t0 = Instant::now();
    let mut read = 0u64;
    while read < 32 << 20 && t0.elapsed() < Duration::from_secs(4) {
        match f.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n as u64,
        }
    }
    let secs = t0.elapsed().as_secs_f64().max(1e-6);
    let seq = read as f64 / 1e6 / secs;
    let mut small = vec![0u8; 256 << 10];
    let t1 = Instant::now();
    let mut jumps = 0u32;
    for k in 1..=12u64 {
        let at = (size / 13).saturating_mul(k);
        if f.seek(SeekFrom::Start(at)).is_ok() && f.read(&mut small).is_ok() {
            jumps += 1;
        }
    }
    let jump_ms = t1.elapsed().as_secs_f64() * 1000.0 / f64::from(jumps.max(1));
    (read > 0).then_some((seq, jump_ms))
}

struct Decode {
    first_ms: f64,
    fps: f64,
    frames: u32,
    by_gpu: u64,
    error: Option<String>,
}

/// Decodes up to 2 seconds of `p` from 40 % into it, as the player does (draft, ¼ size).
fn decode(p: &MediaProbe) -> Decode {
    let rate = p.info.video.as_ref().map_or(filmcraft_time::FrameRate::new(30, 1), |v| v.frame_rate);
    let step = rate.frame_duration();
    let start = Tick((p.info.duration.0 / 5) * 2);
    let want = (rate.as_f64() * 2.0).clamp(10.0, 240.0) as u32;
    let before = filmcraft_codecs::hw::hw_stats();
    let frame = |t: Tick| p.source.video_frame(filmcraft_media::FrameRequest { time: t, scale: 0.25 });
    filmcraft_media::cancel::with_draft(true, || {
        let t0 = Instant::now();
        if let Err(e) = frame(start) {
            return Decode { first_ms: 0.0, fps: 0.0, frames: 0, by_gpu: 0, error: Some(e.to_string()) };
        }
        let first_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        let mut frames = 0u32;
        let mut t = start;
        while frames < want && t1.elapsed() < Duration::from_secs(10) {
            t = Tick(t.0.saturating_add(step.0));
            if t >= p.info.duration || frame(t).is_err() {
                break;
            }
            frames += 1;
        }
        let fps = f64::from(frames) / t1.elapsed().as_secs_f64().max(1e-6);
        let by_gpu = filmcraft_codecs::hw::hw_stats().frames.saturating_sub(before.frames);
        Decode { first_ms, fps, frames, by_gpu, error: None }
    })
}

/// The player at work on `p`'s clip on the timeline (`span`, seconds): up to 2 seconds of its
/// middle drawn as playback draws it (draft, every `render_step`-th frame, 3 workers). Returns the
/// pictures drawn a second and the pictures a second the player shows there.
fn player(feed: &FrameFeed, span: (f64, f64)) -> (f64, f64) {
    let (start, end) = span;
    let t0 = start.max((start + end) / 2.0 - 1.0);
    let step = crate::ahead::render_step(1.0, feed.fps());
    let (first, last) = (feed.frame_at(t0), feed.frame_at(end.min(t0 + 2.0)));
    let next = std::sync::atomic::AtomicI64::new(first);
    let drawn = std::sync::atomic::AtomicU32::new(0);
    let t = Instant::now();
    std::thread::scope(|sc| {
        for _ in 0..3 {
            sc.spawn(|| {
                while t.elapsed() < Duration::from_secs(10) {
                    let k = next.fetch_add(step, std::sync::atomic::Ordering::Relaxed);
                    if k >= last {
                        break;
                    }
                    if feed.render(k, true).is_ok() {
                        drawn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
    });
    let shown = feed.fps() / step as f64;
    (f64::from(drawn.into_inner()) / t.elapsed().as_secs_f64().max(1e-6), shown)
}

fn colour_words(p: &MediaProbe) -> String {
    let Some(v) = p.info.video.as_ref() else { return String::new() };
    let tf = format!("{:?}", v.color.transfer).to_lowercase();
    let hdr = if tf.contains("pq") || tf.contains("2084") {
        ", HDR (PQ: HDR10 / HDR10+)"
    } else if tf.contains("hlg") {
        ", HDR (HLG)"
    } else {
        ""
    };
    let bits = if v.pixel_format.contains("10") || v.pixel_format.contains("16") { ", 10-bit" } else { "" };
    format!("{}{bits}{hdr}", v.codec)
}

/// Checks each of `probes` (blocking: a few seconds each). `caps` is the system's decoder report
/// (Windows: Media Foundation's capabilities); `version` is ClipCut's, as installed.
pub fn run(probes: &[MediaProbe], caps: Option<&str>, version: &str, feed: Option<&FrameFeed>) -> Report {
    let mut videos = Vec::new();
    let mut text = format!(
        "ClipCut {version} playback check\nSystem: {} {}\nHardware decoding: {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        filmcraft_codecs::hw::hw_backend().unwrap_or("none registered"),
    );
    if let Some(c) = caps {
        text += c.trim_end();
        text.push('\n');
    }
    for p in probes {
        let v = p.info.video.as_ref();
        let (w, h, fps) = v.map_or((0, 0, 0.0), |v| (v.width, v.height, v.frame_rate.as_f64()));
        let secs = p.info.duration.seconds().max(0.001);
        let size = std::fs::metadata(&p.path).map(|m| m.len()).unwrap_or(0);
        let need = size as f64 / 1e6 / secs;
        let disk = drive(&p.path, size);
        let d = decode(p);
        // the whole player (decoding, colour, drawing), where the video is on the timeline
        let drawn = feed.zip(p.on_timeline).map(|(f, span)| player(f, span));
        let notes = decoder_notes();
        let mut findings = Vec::new();
        let mut ok = true;
        let what = colour_words(p);
        // the decoder
        if let Some(e) = &d.error {
            ok = false;
            findings.push(format!("ClipCut can't decode this video: {e}"));
        } else if d.by_gpu > 0 {
            findings.push("Your graphics card decodes it.".into());
        } else {
            ok = false;
            let why = match filmcraft_codecs::hw::hw_backend() {
                None => " ClipCut found no hardware decoder on this computer.".to_string(),
                Some(_) if notes.is_empty() => " The graphics card gave no reason.".into(),
                Some(_) => format!(" Why: {}", notes.join(" / ")),
            };
            findings.push(format!("Your processor decodes it, not the graphics card.{why}"));
        }
        // decoding speed: long-GOP video needs every frame decoded, so all `fps` of them a second
        if d.error.is_none() {
            let keeps_up = d.fps >= fps * 0.9;
            ok &= keeps_up;
            findings.push(format!(
                "Decoding: {:.0} frames a second; the video has {:.0}{}",
                d.fps,
                fps,
                if keeps_up { ", so it keeps up." } else { ", so playback can't keep up." }
            ));
        }
        // the drive
        match disk {
            Some((seq, jump)) => {
                let slow = seq < need * 2.0 || jump > 30.0;
                ok &= !slow;
                findings.push(format!(
                    "Drive: reads {seq:.0} MB a second (the video needs {need:.1}); jumping within the file takes {jump:.1} ms{}",
                    if slow { ", too slow for smooth playback." } else { ", fast enough." }
                ));
            }
            None => findings.push("Drive: ClipCut couldn't read the file to measure it.".into()),
        }
        // the player as a whole: decoding, the colour conversion (HDR to normal) and drawing
        match drawn {
            Some((pps, shown)) => {
                let keeps_up = pps >= shown * 0.95;
                if ok && !keeps_up {
                    findings.push(format!(
                        "Player: draws {pps:.0} pictures a second where it shows {shown:.0}: decoding and the drive keep up, so drawing the picture (and its HDR colour) is what slows it."
                    ));
                } else {
                    findings.push(format!(
                        "Player: draws {pps:.0} pictures a second where it shows {shown:.0}{}",
                        if keeps_up { ", so it plays smoothly." } else { "." }
                    ));
                }
                ok &= keeps_up;
            }
            None if feed.is_some() => findings.push("Player: not timed (the video isn't on the timeline).".into()),
            None => {}
        }
        text += &format!(
            "\n{}\n  {what}, {w}x{h}, {fps:.2} fps, {:.0} MB, {secs:.1} s, {need:.1} MB/s\n  file: {}\n  decode: first frame {:.0} ms, then {:.1} fps over {} frames, {} by the graphics card{}\n  drive: {}\n  player: {}\n",
            p.name,
            size as f64 / 1e6,
            p.path,
            d.first_ms,
            d.fps,
            d.frames,
            d.by_gpu,
            d.error.as_ref().map(|e| format!(", error: {e}")).unwrap_or_default(),
            disk.map(|(s, j)| format!("{s:.0} MB/s, {j:.1} ms a jump")).unwrap_or_else(|| "not measured".into()),
            drawn.map(|(d, s)| format!("{d:.1} pictures a second drawn, {s:.0} shown")).unwrap_or_else(|| "not timed".into()),
        );
        if !notes.is_empty() {
            text += &format!("  decoder notes: {}\n", notes.join(" | "));
        }
        videos.push(VideoCheck { name: p.name.clone(), findings, ok });
    }
    if probes.is_empty() {
        text += "\nNo videos in the project.\n";
    }
    Report { videos, text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_lines_about_decoding_are_kept_once() {
        keep_log();
        log::info!(target: "filmcraft_platform", "hardware decoding declined for HEVC video: Main 10 not listed");
        log::info!(target: "filmcraft_platform", "hardware decoding declined for HEVC video: Main 10 not listed");
        log::info!(target: "someone_else", "hardware decoding is great");
        log::debug!(target: "filmcraft_platform", "hardware detail");
        let notes = decoder_notes();
        assert_eq!(notes, vec!["hardware decoding declined for HEVC video: Main 10 not listed".to_string()]);
    }

    #[test]
    fn a_video_is_checked_in_plain_words() {
        let out = std::env::temp_dir().join(format!("clipcut-check-{}.mp4", std::process::id()));
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=3", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&out)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        let mut s = clipcut_core::Studio::new();
        s.add_clip(out.to_str().unwrap()).unwrap();
        // the player has shown these frames: the check must not count its cached ones
        for k in 0..90 {
            let _ = s.preview_frame(f64::from(k) / 30.0, 640, "auto");
        }
        let feed = s.frame_feed(320, "auto").ok();
        let r = run(&s.media_probes(8), Some("adapter: Test GPU"), "0.4.6", feed.as_ref());
        let _ = std::fs::remove_file(&out);
        assert_eq!(r.videos.len(), 1);
        let v = &r.videos[0];
        assert!(v.findings.iter().any(|f| f.starts_with("Your processor decodes it") || f.starts_with("Your graphics card decodes it")), "{v:?}");
        // decoded for real, not served from the player's cache (which once read 1.4 million fps)
        let fps: f64 = v.findings.iter().find_map(|f| f.strip_prefix("Decoding: ")?.split(' ').next()?.parse().ok()).unwrap();
        assert!(fps > 0.0 && fps < 20_000.0, "{fps} frames a second");
        assert!(v.findings.iter().any(|f| f.starts_with("Drive: reads")), "{v:?}");
        assert!(
            r.text.contains("ClipCut 0.4.6 playback check")
                && r.text.contains("adapter: Test GPU")
                && r.text.contains("640x360")
                && r.text.contains("30.00 fps"),
            "{}",
            r.text
        );
        assert!(v.findings.iter().any(|f| f.starts_with("Player: draws")), "{v:?}");
        assert!(r.text.contains("pictures a second drawn, 30 shown"), "{}", r.text);
        assert!(run(&[], None, "0.4.6", None).text.contains("No videos"));
    }

    #[test]
    fn the_drive_is_measured_and_a_missing_file_is_not() {
        let dir = std::env::temp_dir().join(format!("clipcut-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("big.bin");
        std::fs::write(&f, vec![7u8; 8 << 20]).unwrap();
        let (seq, jump) = drive(f.to_str().unwrap(), 8 << 20).unwrap();
        assert!(seq > 0.0 && jump >= 0.0);
        assert!(drive(dir.join("nope.mp4").to_str().unwrap(), 100).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
