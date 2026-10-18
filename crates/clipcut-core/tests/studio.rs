//! Open → preview → audio → export through the real engine, on a clip ffmpeg generates.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 2-second 320 × 240, 30 fps H.264 clip with a 440 Hz tone, or `None` (test skipped).
fn clip() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core studio")?;
    let out = fixtures_dir("clipcut-core").join("testsrc-2s.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=30:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2:sample_rate=48000",
            ])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn open_preview_mix_and_export() {
    let Some(path) = clip() else { return };
    let mut s = Studio::new();
    let info = s.open_clip(path.to_str().unwrap()).unwrap();
    assert_eq!((info.width, info.height), (320, 240));
    assert!((info.fps - 30.0).abs() < 0.01, "{info:?}");
    assert!((info.duration_seconds - 2.0).abs() < 0.1, "{info:?}");
    assert_eq!(info.name, "testsrc-2s");

    // A preview frame at half width, not black (testsrc2 is colour bars and a moving pattern)
    let f = s.frame(1.0, 160).unwrap();
    assert_eq!((f.w, f.h), (160, 120));
    assert_eq!(f.rgba.len(), 160 * 120 * 4);
    let lit = f.rgba.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count();
    assert!(lit > 160 * 120 / 4, "frame mostly black ({lit} lit pixels)");
    // Never upscaled; times past the end clamp to the last frame
    assert_eq!(s.frame(0.0, 4000).unwrap().w, 320);
    assert!(s.frame(99.0, 160).is_ok());
    assert!(s.frame(f64::NAN, 160).is_err());

    // The tone comes through the mixer; past the end is silence
    let feed = s.audio_feed().unwrap();
    assert_eq!(feed.sample_rate, 48_000);
    let block = feed.mix(24_000, 2048);
    assert_eq!(block.len(), 4096);
    let peak = block.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.05, "silent mix (peak {peak})");
    assert!(feed.mix(feed.length + 10, 512).iter().all(|v| *v == 0.0));
    assert!(feed.mix(-1024, 512).iter().all(|v| *v == 0.0));

    // Export to H.264 MP4 as a background job, then open the result
    let out = fixtures_dir("clipcut-core").join(format!("export-{}.mp4", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let job = s.export(out.to_str().unwrap()).unwrap();
    let t0 = Instant::now();
    let done = loop {
        let st = s.job(job).unwrap();
        if st.finished {
            break st;
        }
        assert!(t0.elapsed() < Duration::from_secs(300), "export stuck at {:.0}%", st.progress * 100.0);
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(done.error, None, "{done:?}");
    assert!(std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) > 1000);
    let mut back = Studio::new();
    let again = back.open_clip(out.to_str().unwrap()).unwrap();
    assert_eq!((again.width, again.height), (320, 240));
    assert!((again.duration_seconds - 2.0).abs() < 0.1, "{again:?}");
    let _ = std::fs::remove_file(&out);

    // Unknown formats are refused before anything starts
    assert!(s.export("out.mkv").is_err());
}

#[test]
fn a_cancelled_export_reports_it() {
    let Some(path) = clip() else { return };
    let mut s = Studio::new();
    s.open_clip(path.to_str().unwrap()).unwrap();
    let out = fixtures_dir("clipcut-core").join(format!("cancel-{}.mov", std::process::id()));
    let job = s.export(out.to_str().unwrap()).unwrap();
    s.cancel(job).unwrap();
    let t0 = Instant::now();
    let st = loop {
        let st = s.job(job).unwrap();
        if st.finished {
            break st;
        }
        assert!(t0.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(20));
    };
    // Either it was cancelled, or it was so short it finished first
    if let Some(e) = &st.error {
        assert!(e.to_lowercase().contains("cancel"), "{e}");
    }
    let _ = std::fs::remove_file(&out);
}
