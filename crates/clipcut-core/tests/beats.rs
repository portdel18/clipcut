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

/// Waits for export job `job` and returns its status.
fn finish(s: &Studio, job: u64) -> clipcut_core::JobStatus {
    let t0 = Instant::now();
    loop {
        let st = s.job(job).unwrap();
        if st.finished {
            return st;
        }
        assert!(t0.elapsed() < Duration::from_secs(300), "export stuck at {:.0}%", st.progress * 100.0);
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The integrated loudness (LUFS) of a WAV file (16-, 24- or 32-bit PCM, or 32-bit float).
fn lufs(path: &std::path::Path) -> f64 {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[0..4], b"RIFF");
    let (mut at, mut fmt, mut data) = (12usize, None, None);
    while at + 8 <= b.len() {
        let len = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        match &b[at..at + 4] {
            b"fmt " => fmt = Some(at + 8),
            b"data" => data = Some((at + 8, len.min(b.len() - at - 8))),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let (f, (d, n)) = (fmt.unwrap(), data.unwrap());
    let tag = u16::from_le_bytes([b[f], b[f + 1]]);
    let ch = u16::from_le_bytes([b[f + 2], b[f + 3]]) as usize;
    let sr = u32::from_le_bytes(b[f + 4..f + 8].try_into().unwrap());
    let bits = u16::from_le_bytes([b[f + 14], b[f + 15]]) as usize;
    let k = bits / 8;
    let samples: Vec<f32> = b[d..d + n]
        .chunks_exact(k)
        .map(|x| match (tag, bits) {
            (3, 32) => f32::from_le_bytes(x.try_into().unwrap()),
            (_, 16) => f32::from(i16::from_le_bytes([x[0], x[1]])) / 32768.0,
            (_, 24) => (i32::from_le_bytes([0, x[0], x[1], x[2]]) >> 8) as f32 / 8_388_608.0,
            (_, 32) => i32::from_le_bytes(x.try_into().unwrap()) as f32 / 2_147_483_648.0,
            other => panic!("WAV format {other:?}"),
        })
        .collect();
    let mut m = filmcraft_audio_dsp::LoudnessMeter::new(f64::from(sr), ch);
    m.process_interleaved(&samples);
    m.integrated()
}

#[test]
fn exports_for_platforms_even_out_the_loudness() {
    let Some(v) = picture() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let c = s.export_choices().unwrap();
    let by = |id: &str| c.iter().find(|x| x.id == id).unwrap().clone();
    // the platforms start with it on, the project as mixed, the master never offers it
    assert!(by("social").loudness && by("social").loudness_on && by("youtube").loudness_on && by("small").loudness_on);
    assert!(by("project").loudness && !by("project").loudness_on);
    assert!(by("sound").loudness && !by("sound").loudness_on);
    assert!(!by("master").loudness);

    // the tone as mixed, and evened out to −14 LUFS
    let dir = fixtures_dir("clipcut-core");
    let (plain, even) = (dir.join(format!("loud-plain-{}.wav", std::process::id())), dir.join(format!("loud-even-{}.wav", std::process::id())));
    let job = s.export_as_with(plain.to_str().unwrap(), "sound", true, Some(false)).unwrap();
    let done = finish(&s, job);
    assert_eq!((done.error.clone(), done.loudness), (None, None), "{done:?}");
    let job = s.export_as_with(even.to_str().unwrap(), "sound", true, Some(true)).unwrap();
    let done = finish(&s, job);
    assert_eq!(done.error, None, "{done:?}");
    let (measured, gain) = done.loudness.expect("the export says what it measured");
    let before = lufs(&plain);
    assert!((measured - before).abs() < 1.0, "it measured {measured:.1} LUFS, the file is {before:.1}");
    assert!((gain - (clipcut_core::LOUDNESS_LUFS - measured)).abs() < 1.5, "gain {gain:.1} dB from {measured:.1} LUFS");
    let after = lufs(&even);
    assert!((after - clipcut_core::LOUDNESS_LUFS).abs() < 1.0, "{after:.1} LUFS after evening out (was {before:.1})");
    let _ = (std::fs::remove_file(&plain), std::fs::remove_file(&even));
}
