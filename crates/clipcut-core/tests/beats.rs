//! Beat markers on music (found, moved with the clip, added and removed by hand) and the quick
//! export choices (sizes for each, the vertical warning, an export at TikTok size).

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// 8 seconds of clicks at 120 beats a minute, with a low thump on each.
fn clicks() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core beats")?;
    let out = fixtures_dir("clipcut-core").join("beats-120.wav");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("aevalsrc='0.8*exp(-40*mod(t,0.5))*sin(2*PI*90*t)+0.4*exp(-200*mod(t,0.5))*sin(2*PI*2000*t)':s=48000:d=8")
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// A 4-second 1280 × 720 clip with a tone.
fn picture() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core beats")?;
    let out = fixtures_dir("clipcut-core").join("beats-pic-720.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn beats(s: &Studio, id: u64) -> Vec<f64> {
    s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().beats
}

#[test]
fn beats_are_found_kept_on_the_clip_and_edited_by_hand() {
    let (Some(v), Some(m)) = (picture(), clicks()) else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let item = s.import(m.to_str().unwrap()).unwrap();
    let music = s.add_to_lane(item, 1.0).unwrap()[0];
    assert!(beats(&s, music).is_empty());

    let (n, bpm) = s.mark_beats(music).unwrap();
    assert!((bpm - 120.0).abs() < 3.0, "{bpm}");
    let b = beats(&s, music);
    assert_eq!(b.len(), n);
    assert!(n >= 13, "{n}: {b:?}");
    // half a second apart, on the clicks (which start at 1 s on the timeline)
    for w in b.windows(2) {
        assert!((w[1] - w[0] - 0.5).abs() < 0.03, "{b:?}");
    }
    assert!(b.iter().all(|t| ((t - 1.0) / 0.5 - ((t - 1.0) / 0.5).round()).abs() < 0.1), "{b:?}");

    // the beats move with the clip
    s.move_clip(music, "A2", 2.0).unwrap();
    let moved = beats(&s, music);
    assert!(moved.iter().zip(&b).all(|(x, y)| (x - y - 1.0).abs() < 1e-6), "{moved:?}");

    // by hand: one taken away, one put back
    let at = moved[3];
    assert!(!s.toggle_beat(music, at).unwrap());
    assert_eq!(beats(&s, music).len(), n - 1);
    assert!(s.toggle_beat(music, at).unwrap());
    assert_eq!(beats(&s, music).len(), n);
    assert!(s.toggle_beat(music, 0.5).is_err(), "not over the clip");

    s.clear_beats(music).unwrap();
    assert!(beats(&s, music).is_empty());
    s.undo().unwrap();
    assert_eq!(beats(&s, music).len(), n, "one undo step");

    // a steady tone has no beat
    let main = s.timeline().clips.into_iter().find(|c| c.track == "V1").unwrap().id;
    assert!(s.mark_beats(main).is_err());
}

#[test]
fn export_choices_size_the_video_for_where_it_goes() {
    let Some(v) = picture() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let c = s.export_choices().unwrap();
    let by = |id: &str| c.iter().find(|x| x.id == id).unwrap().clone();
    assert_eq!((by("project").width, by("project").height), (1280, 720));
    assert_eq!((by("social").width, by("social").height), (1080, 1920));
    assert!(by("social").note.as_deref().is_some_and(|n| n.contains("16:9")), "{:?}", by("social"));
    assert_eq!((by("youtube").width, by("youtube").height), (1920, 1080));
    assert_eq!((by("youtube4k").width, by("youtube4k").height), (3840, 2160));
    assert_eq!((by("small").width, by("small").height), (1280, 720));
    assert_eq!(by("sound").ext, "wav");
    assert_eq!((by("master").ext, by("master").width), ("mov", 1280));
    // 4 s at 12 Mbps plus sound: about 6 MB
    assert!((5_000_000..8_000_000).contains(&by("social").bytes), "{}", by("social").bytes);

    // vertical: no warning for TikTok, and YouTube keeps the shape
    s.set_canvas(1080, 1920).unwrap();
    let c = s.export_choices().unwrap();
    let social = c.iter().find(|x| x.id == "social").unwrap();
    assert!(social.note.is_none());
    let yt = c.iter().find(|x| x.id == "youtube").unwrap();
    assert_eq!((yt.width, yt.height), (1080, 1920));

    // an export at TikTok size
    let out = fixtures_dir("clipcut-core").join(format!("social-{}.mp4", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let job = s.export_as(out.to_str().unwrap(), "social").unwrap();
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
    let mut back = Studio::new();
    let again = back.open_clip(out.to_str().unwrap()).unwrap();
    assert_eq!((again.width, again.height), (1080, 1920));
    let _ = std::fs::remove_file(&out);

    // what is refused
    assert!(s.export_as("x.mp4", "vhs").is_err());
    assert!(s.export_as("x.mp4", "sound").is_err(), "sound goes in a .wav");
}
