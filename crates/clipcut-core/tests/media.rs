//! Media people actually bring, and the engine problems FilmCraft users report with it: phone and
//! camera footage that won't open or looks wrong, sound that stops early, projects mixing frame
//! and sample rates, proxies made for only one of several clips, and deleted clips that undo
//! can't bring back. Each picture is checked against ffmpeg's own decode of the same frame.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A fixture made by ffmpeg with `args` (inputs and encoding), or `None` (test skipped).
fn make(name: &str, args: &[&str]) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core media")?;
    let out = fixtures_dir("clipcut-core").join(format!("media-{name}"));
    let ok = generate(&out, |tmp| Command::new(&ff).args(["-v", "error", "-y"]).args(args).arg(tmp).status().is_ok_and(|s| s.success()));
    ok.or_else(|| panic!("ffmpeg couldn't make {name}"))
}

/// The mean colour of ffmpeg's decode of `path` at `seconds`, scaled to `w` × `h` (rotation
/// applied, as players do).
fn ffmpeg_mean(path: &PathBuf, seconds: f64, w: u32, h: u32) -> [f64; 3] {
    let ff = ffmpeg_or_skip("clipcut-core media").unwrap();
    let out = Command::new(ff)
        .args(["-v", "error", "-ss", &format!("{seconds}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &format!("scale={w}:{h}:flags=area"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert_eq!(out.stdout.len(), (w * h * 3) as usize, "ffmpeg's frame of {path:?}");
    let mut c = [0.0; 3];
    for p in out.stdout.chunks(3) {
        for k in 0..3 {
            c[k] += f64::from(p[k]) / f64::from(w * h);
        }
    }
    c
}

fn mean(rgba: &[u8]) -> [f64; 3] {
    let n = (rgba.len() / 4).max(1) as f64;
    let mut c = [0.0; 3];
    for p in rgba.chunks(4) {
        for k in 0..3 {
            c[k] += f64::from(p[k]) / n;
        }
    }
    c
}

fn peak(feed: &clipcut_core::AudioFeed, seconds: f64, span: f64) -> f32 {
    let at = (seconds * f64::from(feed.sample_rate)) as i64;
    feed.mix(at, (span * f64::from(feed.sample_rate)) as usize).iter().fold(0f32, |m, v| m.max(v.abs()))
}

const PICTURE: &str = "testsrc2=size=640x360:rate=30:duration=2";
const TONE: &str = "sine=frequency=440:duration=2:sample_rate=48000";

/// One kind of footage: how ffmpeg makes it, and what it should open as.
struct Footage {
    name: &'static str,
    args: Vec<&'static str>,
    size: (u32, u32),
    fps: f64,
    sound: bool,
}

/// A phone held upright: landscape pixels with a 90° display matrix (`-display_rotation`, as phones
/// write it; ffmpeg 6 ignores the old `rotate` tag).
fn rotated() -> Vec<&'static str> {
    let plain = make(
        "plain-for-rotation.mp4",
        &["-f", "lavfi", "-i", PICTURE, "-f", "lavfi", "-i", TONE, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"],
    )
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_default();
    vec!["-display_rotation", "90", "-i", Box::leak(plain.into_boxed_str()), "-c", "copy"]
}

fn footage() -> Vec<Footage> {
    let v = |name, extra: &[&'static str], size, fps, sound| {
        let mut args = vec!["-f", "lavfi", "-i", PICTURE, "-f", "lavfi", "-i", TONE];
        args.extend_from_slice(extra);
        args.push("-shortest");
        Footage { name, args, size, fps, sound }
    };
    vec![
        v("h264-aac.mp4", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"], (640, 360), 30.0, true),
        // cameras: H.264 in .mov with PCM sound; 10-bit 4:2:2
        v("h264-pcm.mov", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"], (640, 360), 30.0, true),
        v("h264-422-10bit.mov", &["-c:v", "libx264", "-pix_fmt", "yuv422p10le", "-c:a", "pcm_s24le"], (640, 360), 30.0, true),
        // phones: HEVC (tagged hvc1 as iPhones do), 10-bit HEVC
        v("hevc.mp4", &["-c:v", "libx265", "-tag:v", "hvc1", "-pix_fmt", "yuv420p", "-x265-params", "log-level=error", "-c:a", "aac"], (640, 360), 30.0, true),
        v(
            "hevc-10bit.mov",
            &["-c:v", "libx265", "-tag:v", "hvc1", "-pix_fmt", "yuv420p10le", "-x265-params", "log-level=error", "-c:a", "aac"],
            (640, 360),
            30.0,
            true,
        ),
        // a phone held upright: landscape pixels with a 90° rotation in the file
        Footage { name: "portrait-rotated.mp4", args: rotated(), size: (360, 640), fps: 30.0, sound: true },
        // editing and web formats
        v("prores.mov", &["-c:v", "prores_ks", "-profile:v", "2", "-c:a", "pcm_s24le"], (640, 360), 30.0, true),
        v("dnxhr.mov", &["-c:v", "dnxhd", "-profile:v", "dnxhr_lb", "-pix_fmt", "yuv422p", "-c:a", "pcm_s16le"], (640, 360), 30.0, true),
        v("vp9-opus.webm", &["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-c:a", "libopus"], (640, 360), 30.0, true),
        v("av1.mp4", &["-c:v", "libsvtav1", "-preset", "12", "-c:a", "aac"], (640, 360), 30.0, true),
        v("mjpeg.avi", &["-c:v", "mjpeg", "-q:v", "3", "-c:a", "pcm_s16le"], (640, 360), 30.0, true),
        v("xvid-mp3.avi", &["-c:v", "mpeg4", "-q:v", "3", "-c:a", "libmp3lame"], (640, 360), 30.0, true),
        // screen recordings and odd sizes: 4:4:4, an odd width and height, no sound
        v("odd-444.mp4", &["-vf", "scale=641:359", "-c:v", "libx264", "-pix_fmt", "yuv444p", "-an"], (641, 359), 30.0, false),
        // 25 and 60 fps, and 29.97
        v("pal-25.mp4", &["-vf", "fps=25", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"], (640, 360), 25.0, true),
        v("ntsc-2997.mp4", &["-vf", "fps=30000/1001", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"], (640, 360), 29.97, true),
    ]
}

/// What the engine's own decoders can't show yet (H.264 beyond 8-bit 4:2:0, MPEG-4 Part 2): these
/// must be named when they're added (`check_picture`), with how to convert them.
const UNDECODABLE: &[&str] = &["h264-422-10bit.mov", "odd-444.mp4", "xvid-mp3.avi"];

#[test]
fn footage_from_cameras_and_phones_opens_and_looks_right() {
    if ffmpeg_or_skip("clipcut-core media").is_none() {
        return;
    }
    let mut problems = Vec::new();
    for f in footage() {
        let Some(path) = make(f.name, &f.args) else { return };
        let mut s = Studio::new();
        let info = match s.open_clip(path.to_str().unwrap()) {
            Ok(i) => i,
            Err(e) => {
                problems.push(format!("{}: doesn't open: {e}", f.name));
                continue;
            }
        };
        if (info.width, info.height) != f.size {
            problems.push(format!("{}: opens as {}×{}, not {}×{}", f.name, info.width, info.height, f.size.0, f.size.1));
        }
        if (info.fps - f.fps).abs() > 0.02 {
            problems.push(format!("{}: {} fps, not {}", f.name, info.fps, f.fps));
        }
        if (info.duration_seconds - 2.0).abs() > 0.15 {
            problems.push(format!("{}: {:.2} s long, not 2", f.name, info.duration_seconds));
        }
        let item = s.timeline().media.first().map(|m| m.item).unwrap();
        if let Some(why) = s.check_picture(item) {
            if !UNDECODABLE.contains(&f.name) {
                problems.push(format!("{}: picture can't be shown: {why}", f.name));
            }
            assert!(s.timeline().media[0].unreadable.as_deref() == Some(why.as_str()), "{}: the media card says why", f.name);
            // its sound still plays
            if f.sound && s.audio_feed().map(|feed| peak(&feed, 1.0, 0.1)).unwrap_or(0.0) < 0.05 {
                problems.push(format!("{}: no sound either", f.name));
            }
            continue;
        } else if UNDECODABLE.contains(&f.name) {
            problems.push(format!("{}: now decodes: take it off UNDECODABLE", f.name));
        }
        // the picture matches ffmpeg's (colour, orientation)
        match s.frame(1.0, 160) {
            Ok(fr) => {
                let ours = mean(&fr.rgba);
                let theirs = ffmpeg_mean(&path, 1.0, fr.w, fr.h);
                let off = (0..3).map(|k| (ours[k] - theirs[k]).abs()).fold(0.0, f64::max);
                if off > 8.0 {
                    problems.push(format!("{}: picture differs from ffmpeg's by {off:.1} ({ours:.0?} vs {theirs:.0?})", f.name));
                }
                // the corner too: a sideways picture can have the same mean
                let corner = |rgba: &[u8], w: u32| -> [f64; 3] {
                    let mut c = [0.0; 3];
                    let n = 16.0 * 16.0;
                    for y in 0..16 {
                        for x in 0..16 {
                            let i = ((y * w + x) * 4) as usize;
                            for k in 0..3 {
                                c[k] += f64::from(rgba[i + k]) / n;
                            }
                        }
                    }
                    c
                };
                let ff = ffmpeg_or_skip("clipcut-core media").unwrap();
                let raw = Command::new(ff)
                    .args(["-v", "error", "-ss", "1", "-i"])
                    .arg(&path)
                    .args(["-frames:v", "1", "-vf", &format!("scale={}:{}:flags=area", fr.w, fr.h), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                    .output()
                    .unwrap()
                    .stdout;
                if raw.len() == fr.rgba.len() {
                    let (a, b) = (corner(&fr.rgba, fr.w), corner(&raw, fr.w));
                    let off = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max);
                    if off > 24.0 {
                        problems.push(format!("{}: top-left corner differs from ffmpeg's by {off:.0} (turned or flipped?)", f.name));
                    }
                }
            }
            Err(e) => problems.push(format!("{}: no frame at 1 s: {e}", f.name)),
        }
        // the sound plays, start to end
        if f.sound {
            match s.audio_feed() {
                Ok(feed) => {
                    for t in [0.2, 1.0, 1.7] {
                        if peak(&feed, t, 0.1) < 0.05 {
                            problems.push(format!("{}: silent at {t} s", f.name));
                        }
                    }
                }
                Err(e) => problems.push(format!("{}: no sound: {e}", f.name)),
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn sound_files_play_to_their_end() {
    if ffmpeg_or_skip("clipcut-core media").is_none() {
        return;
    }
    // a 90-second voice-over: FilmCraft users report sound stopping after a few seconds
    let long = |rate: &'static str| format!("sine=frequency=330:duration=90:sample_rate={rate}");
    let cases: Vec<(&str, Vec<String>, f64)> = vec![
        (
            "voice-44k-mono.mp3",
            vec!["-f".into(), "lavfi".into(), "-i".into(), long("44100"), "-ac".into(), "1".into(), "-c:a".into(), "libmp3lame".into()],
            90.0,
        ),
        ("voice-48k.m4a", vec!["-f".into(), "lavfi".into(), "-i".into(), long("48000"), "-c:a".into(), "aac".into()], 90.0),
        (
            "music-96k-24bit.flac",
            vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "sine=frequency=330:duration=8:sample_rate=96000".into(),
                "-c:a".into(),
                "flac".into(),
                "-sample_fmt".into(),
                "s32".into(),
            ],
            8.0,
        ),
        (
            "music-24bit.wav",
            vec!["-f".into(), "lavfi".into(), "-i".into(), "sine=frequency=330:duration=8:sample_rate=48000".into(), "-c:a".into(), "pcm_s24le".into()],
            8.0,
        ),
        (
            "voice.ogg",
            vec!["-f".into(), "lavfi".into(), "-i".into(), "sine=frequency=330:duration=8:sample_rate=48000".into(), "-c:a".into(), "libopus".into()],
            8.0,
        ),
        (
            "music-alac.m4a",
            vec!["-f".into(), "lavfi".into(), "-i".into(), "sine=frequency=330:duration=8:sample_rate=44100".into(), "-c:a".into(), "alac".into()],
            8.0,
        ),
    ];
    let mut problems = Vec::new();
    for (name, args, secs) in cases {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let Some(path) = make(name, &args) else { return };
        let mut s = Studio::new();
        if let Err(e) = s.add_file(path.to_str().unwrap(), 0.0) {
            problems.push(format!("{name}: doesn't start a project: {e}"));
            continue;
        }
        let tl = s.timeline();
        if (tl.duration - secs).abs() > 0.2 {
            problems.push(format!("{name}: {:.2} s on the timeline, not {secs}", tl.duration));
        }
        let feed = match s.audio_feed() {
            Ok(f) => f,
            Err(e) => {
                problems.push(format!("{name}: no sound: {e}"));
                continue;
            }
        };
        let mut t = 0.5;
        while t < secs - 0.5 {
            if peak(&feed, t, 0.05) < 0.05 {
                problems.push(format!("{name}: silent at {t:.1} s of {secs}"));
                break;
            }
            t += if secs > 10.0 { 7.3 } else { 1.1 };
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

fn wait(s: &Studio, job: u64) -> clipcut_core::JobStatus {
    let t0 = Instant::now();
    loop {
        let st = s.job(job).unwrap();
        if st.finished {
            return st;
        }
        assert!(t0.elapsed() < Duration::from_secs(600), "job stuck at {:.0}%", st.progress * 100.0);
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Loudness of each `window`-second stretch of `path`'s sound, by ffmpeg.
fn ffmpeg_loudness(path: &PathBuf, window: f64) -> Vec<f32> {
    let ff = ffmpeg_or_skip("clipcut-core media").unwrap();
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(path).args(["-ac", "1", "-ar", "8000", "-f", "f32le", "-"]).output().unwrap();
    let samples: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    samples.chunks((8000.0 * window) as usize).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs()))).collect()
}

#[test]
fn a_project_mixing_frame_and_sample_rates_exports_whole() {
    let Some(a) = make(
        "mix-30-48k.mp4",
        &["-f", "lavfi", "-i", PICTURE, "-f", "lavfi", "-i", TONE, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"],
    ) else {
        return;
    };
    let Some(b) = make(
        "mix-25-44k.mov",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=25:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=550:duration=2:sample_rate=44100",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "pcm_s16le",
            "-shortest",
        ],
    ) else {
        return;
    };
    let Some(c) = make(
        "mix-60-portrait.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=360x640:rate=60:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:duration=2:sample_rate=48000",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
    ) else {
        return;
    };
    let mut s = Studio::new();
    for p in [&a, &b, &c] {
        s.add_clip(p.to_str().unwrap()).unwrap();
    }
    let tl = s.timeline();
    assert!((tl.duration - 6.0).abs() < 0.1, "{}", tl.duration);
    for choice in ["project", "social"] {
        let out = fixtures_dir("clipcut-core").join(format!("mix-export-{choice}-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let job = s.export_as(out.to_str().unwrap(), choice).unwrap();
        let st = wait(&s, job);
        assert_eq!(st.error, None, "{choice}: {st:?}");
        let mut back = Studio::new();
        let info = back.open_clip(out.to_str().unwrap()).unwrap();
        assert!((info.duration_seconds - 6.0).abs() < 0.15, "{choice}: {info:?}");
        // sound all the way through: every half second of all three clips
        let loud = ffmpeg_loudness(&out, 0.5);
        assert!(loud.len() >= 11, "{choice}: {} windows", loud.len());
        for (k, l) in loud.iter().take(11).enumerate() {
            assert!(*l > 0.05, "{choice}: silent around {:.1} s ({loud:?})", k as f64 * 0.5);
        }
        let _ = std::fs::remove_file(&out);
    }
    // sound only, from the same mix
    let out = fixtures_dir("clipcut-core").join(format!("mix-export-{}.wav", std::process::id()));
    let job = s.export_as(out.to_str().unwrap(), "sound").unwrap();
    assert_eq!(wait(&s, job).error, None);
    let loud = ffmpeg_loudness(&out, 0.5);
    assert!(loud.len() >= 11 && loud.iter().take(11).all(|l| *l > 0.05), "{loud:?}");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn proxies_are_made_for_every_clip_at_once() {
    let Some(a) = make("proxy-a.mp4", &["-f", "lavfi", "-i", PICTURE, "-c:v", "libx264", "-pix_fmt", "yuv420p"]) else { return };
    let Some(b) = make("proxy-b.mov", &["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=25:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p"]) else {
        return;
    };
    let dir = fixtures_dir("clipcut-core").join(format!("proxies-many-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Studio::new();
    s.set_data_dir(&dir);
    s.add_clip(a.to_str().unwrap()).unwrap();
    s.add_clip(b.to_str().unwrap()).unwrap();
    s.save_as(dir.join("p.fcproj").to_str().unwrap()).unwrap();
    let job = s.make_proxies(&[]).unwrap();
    assert_eq!(wait(&s, job).error, None);
    s.poll();
    let media = s.timeline().media;
    assert_eq!(media.len(), 2);
    assert!(media.iter().all(|m| m.proxy), "every clip gets its proxy, not just one: {media:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleted_clips_come_back_with_undo() {
    let Some(a) =
        make("undo-a.mp4", &["-f", "lavfi", "-i", PICTURE, "-f", "lavfi", "-i", TONE, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
    else {
        return;
    };
    let mut s = Studio::new();
    for _ in 0..3 {
        s.add_clip(a.to_str().unwrap()).unwrap();
    }
    let ids: Vec<u64> = s.timeline().clips.iter().filter(|c| c.track == "V1").map(|c| c.id).collect();
    assert_eq!(ids.len(), 3);
    // the middle one changed, then two deleted at once
    s.set_props(&[ids[1]], &serde_json::json!({"opacity": 50.0}), None).unwrap();
    let before = s.timeline();
    s.delete(&[ids[1], ids[2]]).unwrap();
    assert_eq!(s.timeline().clips.iter().filter(|c| c.track == "V1").count(), 1);
    s.undo().unwrap();
    let back = s.timeline();
    assert_eq!(back.clips.len(), before.clips.len());
    assert!((back.duration - before.duration).abs() < 1e-6);
    let start = back.clips.iter().find(|c| c.id == ids[1]).map(|c| c.start).unwrap();
    let props = s.props_at(ids[1], start + 0.5).unwrap();
    assert!(props.opacity.is_some_and(|o| (o - 50.0).abs() < 0.01), "the clip comes back as it was: {props:?}");
    // and redo deletes them again, undo brings them back again
    s.redo().unwrap();
    assert_eq!(s.timeline().clips.iter().filter(|c| c.track == "V1").count(), 1);
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.len(), before.clips.len());
    // a saved project keeps the restored clips
    let dir = fixtures_dir("clipcut-core").join(format!("undo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("u.fcproj");
    s.save_as(p.to_str().unwrap()).unwrap();
    let mut again = Studio::new();
    again.open_project(p.to_str().unwrap()).unwrap();
    assert_eq!(again.timeline().clips.len(), before.clips.len());
    let _ = std::fs::remove_dir_all(&dir);
}
