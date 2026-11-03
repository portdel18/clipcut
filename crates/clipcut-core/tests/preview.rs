//! The player's frames: the frame feed (a snapshot that renders on any thread, at the snapped
//! preview scale) and proxies (made in the background, attached, used for preview only).

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 3-second 640 × 360 test picture with a tone.
fn picture() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core preview")?;
    let out = fixtures_dir("clipcut-core").join("preview-640.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=3"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn mean(f: &clipcut_core::Frame) -> [f64; 3] {
    let n = (f.rgba.len() / 4).max(1) as f64;
    let mut c = [0.0; 3];
    for p in f.rgba.chunks(4) {
        for k in 0..3 {
            c[k] += f64::from(p[k]) / n;
        }
    }
    c
}

#[test]
fn the_feed_renders_a_snapshot_on_other_threads() {
    let Some(v) = picture() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    // a 300-pixel player gets the canvas at half size (the smallest whole fraction that fills
    // it), a 400-pixel one the whole canvas, and quarter when asked
    assert_eq!(s.frame_feed(400, "auto").unwrap().render(0, false).unwrap().w, 640);
    let feed = s.frame_feed(300, "auto").unwrap();
    assert_eq!(feed.frames, 90);
    assert_eq!(feed.frame_at(1.0), 30);
    let f = feed.render(30, false).unwrap();
    assert_eq!((f.w, f.h), (320, 180));
    let q = s.frame_feed(300, "quarter").unwrap().render(30, false).unwrap();
    assert_eq!((q.w, q.h), (160, 90));
    // the same picture as the paused player's
    let paused = s.frame(1.0, 320).unwrap();
    let (a, b) = (mean(&f), mean(&paused));
    assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1.0), "{a:?} {b:?}");

    // frames render on other threads, from the project as it was
    s.set_props(&[s.timeline().clips[0].id], &serde_json::json!({"opacity": 0.0}), None).unwrap();
    let frames: Vec<_> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..3)
            .map(|k| {
                sc.spawn({
                    let feed = feed.clone();
                    move || feed.render(30 + k, true).unwrap()
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(frames.iter().all(|f| (f.w, f.h) == (320, 180) && mean(f)[0] > 40.0), "the snapshot is before the opacity change");
    assert!(mean(&s.frame_feed(300, "auto").unwrap().render(30, false).unwrap())[0] < 2.0, "a new feed sees it");
    // a picture to design a graphic over leaves out the clips asked
    let id = s.timeline().clips.iter().find(|c| c.track == "V1").unwrap().id;
    s.undo().unwrap();
    let all = mean(&s.frame_without(1.0, 320, &[]).unwrap());
    let none = mean(&s.frame_without(1.0, 320, &[id]).unwrap());
    assert!(all[0] > 40.0 && none[0] < 2.0, "{all:?} {none:?}");
    assert!(s.timeline().clips.iter().any(|c| c.id == id), "the project itself is unchanged");
    // past the end is the last frame, not a failure
    assert!(feed.render(10_000, false).is_ok() && feed.render(-5, false).is_ok());
}

#[test]
fn proxies_are_made_attached_and_used_for_the_player_only() {
    let Some(v) = picture() else { return };
    let dir = fixtures_dir("clipcut-core").join(format!("proxies-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Studio::new();
    s.set_data_dir(&dir);
    s.add_clip(v.to_str().unwrap()).unwrap();
    s.save_as(dir.join("p.fcproj").to_str().unwrap()).unwrap();
    assert!(!s.timeline().media[0].proxy && !s.timeline().proxies);
    let before = mean(&s.frame(1.0, 640).unwrap());

    let job = s.make_proxies(&[]).unwrap();
    let t0 = Instant::now();
    loop {
        let st = s.job(job).unwrap();
        if st.finished {
            assert_eq!(st.error, None, "{st:?}");
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(120), "proxies stuck at {:.0}%", st.progress * 100.0);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(s.poll(), "the finished proxy is attached");
    assert!(s.timeline().media[0].proxy);
    assert!(std::fs::read_dir(dir.join("Proxies")).unwrap().count() == 1, "kept in the project's folder");

    // the player uses it: the same picture, from the small copy
    s.set_proxies(true).unwrap();
    assert!(s.timeline().proxies);
    let with = mean(&s.frame(1.0, 640).unwrap());
    assert!((0..3).all(|k| (with[k] - before[k]).abs() < 6.0), "{before:?} {with:?}");
    s.set_proxies(false).unwrap();
    assert!(!s.timeline().proxies);
    assert!(s.make_proxies(&[999_999]).is_err(), "nothing to make");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn big_videos_are_flagged_and_proxies_of_unsaved_projects_stay_on_this_computer() {
    // 4K and up, or more than 1080p at 30 fps a second (phones' 4K 60, 1080p 60)
    use clipcut_core::heavy;
    assert!(heavy(3840, 2160, 60.0) && heavy(3840, 2160, 30.0) && heavy(3840, 2160, 24.0) && heavy(4096, 2160, 25.0));
    assert!(heavy(2160, 3840, 30.0), "upright phone video too");
    assert!(heavy(1920, 1080, 60.0) && heavy(1920, 1080, 59.94) && heavy(2560, 1440, 30.0));
    assert!(!heavy(1920, 1080, 30.0) && !heavy(1920, 1080, 29.97) && !heavy(1280, 720, 60.0) && !heavy(640, 360, 30.0));
    assert!(!heavy(1920, 1080, f64::NAN) && !heavy(0, 0, 60.0) && heavy(u32::MAX, u32::MAX, f64::INFINITY));

    let Some(v) = picture() else { return };
    let dir = fixtures_dir("clipcut-core").join(format!("proxies-data-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Studio::new();
    s.set_data_dir(&dir);
    s.add_clip(v.to_str().unwrap()).unwrap();
    let m = s.timeline().media[0].clone();
    assert_eq!((m.kind, m.fps, m.codec.as_str(), m.heavy), ("video", 30.0, "h264", false));
    // never saved: in ClipCut's own folder, not next to the media (a slow or removable drive)
    let job = s.make_proxies(&[]).unwrap();
    let t0 = Instant::now();
    while !s.job(job).unwrap().finished {
        assert!(t0.elapsed() < Duration::from_secs(120), "proxies stuck");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(s.job(job).unwrap().error, None);
    assert!(s.poll() && s.timeline().media[0].proxy);
    assert_eq!(std::fs::read_dir(dir.join("Proxies")).unwrap().count(), 1, "in the data folder");
    assert!(!v.parent().unwrap().join("Proxies").join(format!("{}_Proxy.mov", v.file_stem().unwrap().to_string_lossy())).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hevc_video_is_named_hevc() {
    let Some(ff) = ffmpeg_or_skip("clipcut-core hevc") else { return };
    let out = fixtures_dir("clipcut-core").join("hevc-small.mp4");
    let ok = generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=60:duration=0.5"])
            .args(["-c:v", "libx265", "-x265-params", "log-level=error", "-tag:v", "hvc1", "-pix_fmt", "yuv420p"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    });
    let Some(path) = ok else { return };
    let mut s = Studio::new();
    s.import(path.to_str().unwrap()).unwrap();
    let m = s.timeline().media[0].clone();
    assert_eq!((m.codec.as_str(), m.fps, m.heavy), ("hevc", 60.0, false));
}
