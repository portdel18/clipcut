//! The mixer (lane volume, pan, mute, solo, the Mix, meters) and precise trimming (to the
//! playhead, nudges, slips, rolls).

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{Studio, TimelineView};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// A `secs`-second 320 × 180 test picture with a `hz` tone.
fn clip(name: &str, secs: u32, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core mixer")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=size=320x180:rate=30:duration={secs}")])
            .args(["-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration={secs}:sample_rate=48000")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn tone(name: &str, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core mixer")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.wav"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration=3:sample_rate=48000")])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn main(v: &TimelineView) -> Vec<u64> {
    let mut m: Vec<_> = v.clips.iter().filter(|c| c.track == "V1").collect();
    m.sort_by(|a, b| a.start.total_cmp(&b.start));
    m.iter().map(|c| c.id).collect()
}

/// RMS of the left and right channels over 0.5–1.0 s.
fn rms(s: &Studio) -> (f64, f64) {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let buf = feed.mix((0.5 * rate) as i64, (0.5 * rate) as usize);
    let ch = |k: usize| (buf.iter().skip(k).step_by(2).map(|v| f64::from(*v).powi(2)).sum::<f64>() / (buf.len() / 2).max(1) as f64).sqrt();
    (ch(0), ch(1))
}
const DB: fn(f64, f64) -> f64 = |a, b| 20.0 * (a / b).log10();

#[test]
fn lanes_have_volume_pan_mute_and_solo() {
    let (Some(v), Some(m)) = (clip("mx-a", 3, 440), tone("mx-music", 660)) else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let base = rms(&s);
    // volume: 12 dB down
    s.set_mixer("A1", &json!({"volumeDb": -12.0}), None).unwrap();
    let a1 = s.timeline().lanes.into_iter().find(|l| l.id == "A1").unwrap();
    assert_eq!(a1.volume_db, -12.0);
    let quiet = rms(&s);
    assert!((DB(quiet.0, base.0) + 12.0).abs() < 1.0, "{base:?} → {quiet:?}");
    // a dragged fader is one undo step
    s.set_mixer("A1", &json!({"volumeDb": -6.0}), Some("drag")).unwrap();
    s.set_mixer("A1", &json!({"volumeDb": -3.0}), Some("drag")).unwrap();
    s.undo().unwrap();
    assert_eq!(s.timeline().lanes.iter().find(|l| l.id == "A1").unwrap().volume_db, -12.0);
    s.set_mixer("A1", &json!({"volumeDb": 0.0}), None).unwrap();
    // pan hard left: the right side goes quiet
    s.set_mixer("A1", &json!({"pan": -100.0}), None).unwrap();
    let (l, r) = rms(&s);
    assert!(l > base.0 * 0.5 && r < l / 30.0, "{l} {r}");
    s.set_mixer("A1", &json!({"pan": 0.0}), None).unwrap();
    // the Mix
    s.set_mixer("Mix", &json!({"volumeDb": -6.0}), None).unwrap();
    assert_eq!(s.timeline().mix.volume_db, -6.0);
    assert!((DB(rms(&s).0, base.0) + 6.0).abs() < 1.0);
    s.set_mixer("Mix", &json!({"volumeDb": 0.0}), None).unwrap();

    // music on a lane; solo it and the main track's sound drops out
    let item = s.import(m.to_str().unwrap()).unwrap();
    s.add_to_lane(item, 0.0).unwrap();
    let both = rms(&s).0;
    s.set_mixer("A2", &json!({"solo": true}), None).unwrap();
    assert!(s.timeline().lanes.iter().find(|l| l.id == "A2").unwrap().solo);
    let solo = rms(&s).0;
    assert!(solo < both * 0.9 && solo > 0.01, "{both} {solo}");
    // muted, a lane is silent
    s.set_mixer("A2", &json!({"solo": false, "muted": true}), None).unwrap();
    assert!((rms(&s).0 - base.0).abs() < base.0 * 0.1);

    // meters: each strip's level as the feed plays
    let feed = s.audio_feed().unwrap();
    let meters = feed.meters();
    let _ = feed.mix(0, 4800);
    let lv = meters.take();
    let a1 = lv.iter().find(|l| l.lane == "A1").expect("A1 played");
    assert!(a1.left > 0.05, "{lv:?}");
    assert!(lv.iter().any(|l| l.lane == "Mix" && l.left > 0.05), "{lv:?}");
    assert!(meters.take().is_empty() || meters.take().iter().all(|l| l.left == 0.0), "read once");

    // what is refused
    assert!(s.set_mixer("A9", &json!({"volumeDb": 0.0}), None).is_err());
    assert!(s.set_mixer("Mix", &json!({"solo": true}), None).is_err());
    assert!(s.set_mixer("A1", &json!({"volumeDb": "loud"}), None).is_err());
}

#[test]
fn trims_to_the_playhead_nudges_slips_and_rolls() {
    let (Some(a), Some(b), Some(m)) = (clip("mx-long-a", 4, 440), clip("mx-long-b", 4, 550), tone("mx-music2", 660)) else { return };
    let mut s = Studio::new();
    s.add_clip(a.to_str().unwrap()).unwrap();
    s.add_clip(b.to_str().unwrap()).unwrap();
    let ids = main(&s.timeline());
    let c = |s: &Studio, id: u64| s.timeline().clips.into_iter().find(|c| c.id == id).unwrap();

    // to the playhead: the start of the first clip, then the end of the second
    s.trim_to(ids[0], true, 1.0).unwrap();
    assert!((c(&s, ids[0]).source_in - 1.0).abs() < 1e-6 && c(&s, ids[0]).start == 0.0, "the main track closes up");
    s.trim_to(ids[1], false, 5.0).unwrap();
    assert!((c(&s, ids[1]).end - 5.0).abs() < 1e-6);
    assert!(s.trim_to(ids[1], false, 9.0).is_err(), "outside the clip");

    // slip: other frames in the same place
    let (start, len) = (c(&s, ids[1]).start, c(&s, ids[1]).end - c(&s, ids[1]).start);
    assert_eq!(s.slip(ids[1], 15).unwrap(), 15);
    let n = c(&s, ids[1]);
    assert!((n.source_in - 0.5).abs() < 1e-6 && n.start == start && (n.end - n.start - len).abs() < 1e-6, "{n:?}");
    assert!(s.slip(ids[0], -60).unwrap() >= -30, "stops at the media's start");

    // roll: the cut moves, the video keeps its length
    let total = s.timeline().duration;
    let cut = c(&s, ids[1]).start;
    s.roll(ids[0], 10).unwrap();
    assert!((c(&s, ids[1]).start - (cut + 10.0 / 30.0)).abs() < 1e-6);
    assert!((s.timeline().duration - total).abs() < 1e-6);
    // linked sound follows
    let snd = s.timeline().clips.into_iter().filter(|x| x.track == "A1").map(|x| x.start).fold(f64::NAN, f64::max);
    assert!((snd - (cut + 10.0 / 30.0)).abs() < 1e-6, "{snd}");
    assert!(s.roll(ids[1], 5).is_err(), "the last clip has no cut after it");
    // a cut can't move past the media: the first clip shows its media's end at most a second later
    assert!(s.roll(ids[0], 100).is_ok_and(|_| c(&s, ids[1]).start > cut));
    let at = c(&s, ids[1]).start;
    assert!(s.roll(ids[0], 1).is_err(), "nothing left to roll into");
    assert_eq!(c(&s, ids[1]).start, at);
    s.undo().unwrap();
    assert!((c(&s, ids[1]).start - (cut + 10.0 / 30.0)).abs() < 1e-6, "the refused roll left no undo step");
    assert!(s.slip(ids[1], -1000).is_ok() && s.slip(ids[1], -1).is_err(), "the media's first frames");
    s.undo().unwrap();

    // nudge: lane clips by a frame; main clips stay put
    let item = s.import(m.to_str().unwrap()).unwrap();
    let lane = s.add_to_lane(item, 1.0).unwrap()[0];
    s.nudge(&[lane], 3).unwrap();
    assert!((c(&s, lane).start - (1.0 + 3.0 / 30.0)).abs() < 1e-6);
    s.nudge(&[lane], -3).unwrap();
    assert!((c(&s, lane).start - 1.0).abs() < 1e-6);
    assert!(s.nudge(&[ids[0]], 1).is_err());
    assert!(s.nudge(&[lane], -100).is_err(), "not before the start");
}
