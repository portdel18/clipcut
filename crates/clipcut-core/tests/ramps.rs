//! Speed ramps: a clip speeds up and slows down along its footage, its sound with it, and the main
//! track moves along; splits carry on, one undo puts it back, a plain speed replaces it.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{ClipView, Studio};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// 4 s at 30 fps of a moving test pattern with a steady 440 Hz tone.
fn clip() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core ramps")?;
    let out = fixtures_dir("clipcut-core").join("ramp-tone.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le", "-shortest"])
            .arg(tmp.with_extension("mov"))
            .status()
            .is_ok_and(|s| s.success())
            && std::fs::rename(tmp.with_extension("mov"), tmp).is_ok()
    })
}

fn on(s: &Studio, track: &str) -> Vec<ClipView> {
    let mut v: Vec<ClipView> = s.timeline().clips.into_iter().filter(|c| c.track == track).collect();
    v.sort_by(|a, b| a.start.total_cmp(&b.start));
    v
}

/// The tone's pitch (Hz, from zero crossings) over `[t, t + 0.06)` of the mix.
fn pitch(s: &Studio, t: f64) -> f64 {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let n = (0.06 * rate) as usize;
    let left: Vec<f32> = feed.mix((t * rate) as i64, n).chunks(2).map(|p| p[0]).collect();
    let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    crossings as f64 / 2.0 / 0.06
}

/// Fast, slow motion on the middle of the footage, fast again.
const MOMENT: [[f64; 2]; 5] = [[0.0, 2.0], [0.35, 2.0], [0.5, 0.25], [0.65, 2.0], [1.0, 2.0]];

#[test]
fn a_speed_curve_plays_the_footage_fast_and_slow_and_the_main_track_follows() {
    let Some(v) = clip() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let a = on(&s, "V1")[0].id;
    assert!((pitch(&s, 1.0) - 440.0).abs() < 25.0, "the tone as recorded: {}", pitch(&s, 1.0));

    s.set_ramp(a, &MOMENT).unwrap();
    let v1 = on(&s, "V1");
    let (first, second) = (&v1[0], &v1[1]);
    // the same footage, played in another length: the second clip starts where it ends now
    assert!((first.source_in - 0.0).abs() < 0.01 && (first.source_out - 4.0).abs() < 0.02, "{first:?}");
    assert!((first.end - first.start - 4.0).abs() > 0.2, "its length changed: {first:?}");
    assert!((second.start - first.end).abs() < 0.04, "the main track moved along: {v1:?}");
    // its sound has the same curve and length
    let a1 = on(&s, "A1");
    assert!((a1[0].end - first.end).abs() < 0.04, "{a1:?}");
    for c in [first, &a1[0]] {
        assert_eq!(c.props.ramp.len(), MOMENT.len(), "{:?}", c.props.ramp);
        for (got, want) in c.props.ramp.iter().zip(MOMENT) {
            assert!((got[0] - want[0]).abs() < 0.01 && (got[1] - want[1]).abs() < 0.01, "{:?}", c.props.ramp);
        }
    }
    // the sound speeds up and slows down with the picture: 2× an octave up, ¼× two octaves down
    let pitches: Vec<f64> = (0..((first.end - 0.1) / 0.06) as usize).map(|k| pitch(&s, k as f64 * 0.06)).collect();
    let (lo, hi) = pitches.iter().fold((f64::MAX, 0.0f64), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    assert!((hi - 880.0).abs() < 45.0, "fast part {hi} Hz");
    assert!((lo - 110.0).abs() < 25.0, "slow part {lo} Hz");
    assert!((pitch(&s, 0.2) - 880.0).abs() < 45.0, "it starts fast: {}", pitch(&s, 0.2));
    // and the picture is there all along
    for t in [0.1, first.end / 2.0, first.end - 0.05] {
        assert!(s.frame(t, 160).is_ok(), "frame at {t}");
    }

    // a split carries on where the whole clip was: same sound either side, same length overall
    let mid = first.start + (first.end - first.start) * 0.45;
    let (before, end_before) = (pitch(&s, mid - 0.07), first.end);
    s.split(mid, &[a]).unwrap();
    let v1 = on(&s, "V1");
    assert_eq!(v1.len(), 3);
    assert!(!v1[1].props.ramp.is_empty(), "the second piece keeps its curve");
    assert!((v1[1].end - end_before).abs() < 0.04, "{v1:?}");
    assert!((pitch(&s, mid - 0.07) - before).abs() < 15.0);
    s.undo().unwrap();
    assert_eq!(on(&s, "V1").len(), 2);

    // one speed all along replaces the curve, keeping the footage
    s.set_props(&[a], &json!({"speed": 200}), None).unwrap();
    let c = &on(&s, "V1")[0];
    assert!(c.props.ramp.is_empty() && (c.end - c.start - 2.0).abs() < 0.04, "{c:?}");
    assert!(on(&s, "A1")[0].props.ramp.is_empty());
    s.undo().unwrap();
    assert_eq!(on(&s, "V1")[0].props.ramp.len(), MOMENT.len(), "one undo brings the curve back");

    // none: back to normal speed, the footage at its own length
    s.set_ramp(a, &[]).unwrap();
    let v1 = on(&s, "V1");
    assert!(v1[0].props.ramp.is_empty() && (v1[0].end - 4.0).abs() < 0.04 && (v1[1].start - 4.0).abs() < 0.04, "{v1:?}");
    assert!((pitch(&s, 1.0) - 440.0).abs() < 25.0);
    s.undo().unwrap();
    s.undo().unwrap();
    let v1 = on(&s, "V1");
    assert!(v1[0].props.ramp.is_empty() && (v1[0].end - 4.0).abs() < 0.04, "undone back to the start: {v1:?}");

    // refused in plain words
    assert!(s.set_ramp(a, &[[0.5, 1.0]]).is_err());
    assert!(s.set_ramp(a, &[[0.0, f64::INFINITY], [1.0, 1.0]]).is_err());
    assert!(s.set_ramp(987_654, &MOMENT).is_err());
    s.set_reverse(&[a], true).unwrap();
    let e = s.set_ramp(a, &MOMENT).unwrap_err().to_string();
    assert!(e.contains("Reverse"), "{e}");
}
