//! How fast a TikTok-size export runs, and how much of it is the H.264 encoder: 20 s of 1080 × 1920
//! camera-like footage with burnt-in captions, exported as `social` and again as MJPEG (a quick
//! encoder), so the difference is roughly the H.264 encoder's share. A measurement, not a check:
//! `cargo test --release -p clipcut-core --test export_speed -- --ignored --nocapture`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clipcut_core::Studio;
use filmcraft_project::transcript::{Transcript, Word};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use filmcraft_time::Tick;
use serde_json::json;

const SECONDS: u32 = 20;

fn footage() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core export_speed")?;
    let out = fixtures_dir("clipcut-core").join("speed-1080x1920.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=size=1080x1920:rate=30:duration={SECONDS}")])
            .args(["-f", "lavfi", "-i", &format!("sine=frequency=440:duration={SECONDS}:sample_rate=48000")])
            // grain, so it compresses like a phone video rather than a test card
            .args(["-vf", "noise=alls=12:allf=t", "-c:v", "libx264", "-preset", "veryfast", "-b:v", "10M", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Waits for the export and prints where its time went (the engine's stage clocks; render and
/// encode overlap, on different threads).
fn wait(s: &Studio, job: u64) -> Duration {
    let before = filmcraft_export::stage_times();
    let t0 = Instant::now();
    loop {
        let st = s.job(job).unwrap();
        if st.finished {
            assert!(st.error.is_none(), "{:?}", st.error);
            let stages: Vec<String> = filmcraft_export::stage_times()
                .iter()
                .zip(&before)
                .map(|((stage, after), (_, b))| (stage.name(), after.saturating_sub(*b) as f64 / 1e9))
                .filter(|(_, sec)| *sec > 0.05)
                .map(|(n, sec)| format!("{n} {sec:.1} s"))
                .collect();
            println!("  stages: {}", stages.join(", "));
            return t0.elapsed();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn tiktok_export_speed() {
    let Some(v) = footage() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    s.set_canvas(1080, 1920).unwrap();
    let words = (0..SECONDS * 5)
        .map(|i| {
            let a = f64::from(i) * 0.2;
            Word::new(["the", "city", "is", "ready", "for", "this", "change"][i as usize % 7], Tick::from_seconds_f64(a), Tick::from_seconds_f64(a + 0.18))
        })
        .collect();
    let speech = Transcript { language: "en".into(), words, ..Default::default() };
    s.session().transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: speech, id: "fixed".into() }));
    let job = s.transcribe_job(&std::env::temp_dir(), "whisper-base", None, false).unwrap();
    let results = job.run(&mut |_, _| true).unwrap();
    assert!(s.finish_captions(results).unwrap() > 10);

    let dir = fixtures_dir("clipcut-core");
    // CLIPCUT_SPEED_PROFILE=1: a third of a second only, MJPEG, for a profiler (valgrind --tool=callgrind)
    if std::env::var_os("CLIPCUT_SPEED_PROFILE").is_some() {
        let out = dir.join("speed-profile.mov");
        let p = json!({"path": out.to_str().unwrap(), "format": "mjpeg", "width": 1080, "height": 1920, "quality": 30, "range": "custom", "startSeconds": 1.0, "endSeconds": 1.3});
        let r = s.session().execute("file.exportMedia", p).unwrap();
        println!("profile run: {:.1} s", wait(&s, r["job"].as_u64().unwrap()).as_secs_f64());
        return;
    }
    let frames = f64::from(SECONDS * 30);
    let h264 = dir.join("speed-out.mp4");
    let job = s.export_as(h264.to_str().unwrap(), "social").unwrap();
    let t = wait(&s, job);
    println!("social H.264: {:.1} s, {:.1} fps ({:.2}× real time)", t.as_secs_f64(), frames / t.as_secs_f64(), f64::from(SECONDS) / t.as_secs_f64());

    let mjpeg = dir.join("speed-out.mov");
    let r = s
        .session()
        .execute(
            "file.exportMedia",
            json!({"path": mjpeg.to_str().unwrap(), "format": "mjpeg", "width": 1080, "height": 1920, "burnCaptions": true, "quality": 30}),
        )
        .unwrap();
    let t2 = wait(&s, r["job"].as_u64().unwrap());
    println!("same, MJPEG: {:.1} s, {:.1} fps", t2.as_secs_f64(), frames / t2.as_secs_f64());
    println!("H.264 encoder ≈ {:.0} % of the export", 100.0 * (1.0 - t2.as_secs_f64() / t.as_secs_f64()).max(0.0));
    for (what, w, h, burn) in [("MJPEG, no captions", 1080, 1920, false), ("MJPEG, no captions, 270 × 480 (mostly decoding)", 270, 480, false)] {
        let r = s
            .session()
            .execute(
                "file.exportMedia",
                json!({"path": mjpeg.to_str().unwrap(), "format": "mjpeg", "width": w, "height": h, "burnCaptions": burn, "quality": 30}),
            )
            .unwrap();
        let t3 = wait(&s, r["job"].as_u64().unwrap());
        println!("{what}: {:.1} s, {:.1} fps", t3.as_secs_f64(), frames / t3.as_secs_f64());
    }
    println!("threads: {}", std::thread::available_parallelism().map_or(0, |n| n.get()));
    let _ = std::fs::remove_file(h264);
    let _ = std::fs::remove_file(mjpeg);
}
