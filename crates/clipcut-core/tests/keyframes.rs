//! Keyframes: opacity, position and volume changing over a clip; toggling, the value between
//! keyframes, easing, still properties, undo, moving clips, and what is refused.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{KeyTrack, Studio, TimelineView};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// A 2-second 320 × 180, 30 fps white clip with a tone.
fn clip(name: &str, colour: &str) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core keyframes")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("color=c={colour}:size=320x180:rate=30:duration=2")])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn studio() -> Option<(Studio, u64)> {
    let mut s = Studio::new();
    s.add_clip(clip("kf-white", "white")?.to_str()?).unwrap();
    let id = main(&s.timeline())[0];
    Some((s, id))
}

fn main(v: &TimelineView) -> Vec<u64> {
    let mut m: Vec<_> = v.clips.iter().filter(|c| c.track == "V1").collect();
    m.sort_by(|a, b| a.start.total_cmp(&b.start));
    m.iter().map(|c| c.id).collect()
}

fn keys(s: &Studio, id: u64, prop: &str) -> Option<KeyTrack> {
    s.timeline().clips.into_iter().find(|c| c.id == id)?.keys.into_iter().find(|k| k.prop == prop)
}

/// The program's average brightness at `t` (0–255).
fn bright(s: &Studio, t: f64) -> f64 {
    let f = s.frame(t, 160).unwrap();
    f.rgba.chunks(4).map(|p| f64::from(p[0])).sum::<f64>() / (f.rgba.len() / 4).max(1) as f64
}

/// The sound's loudness (RMS) around `t`.
fn loud(s: &Studio, t: f64) -> f64 {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let buf = feed.mix(((t - 0.05) * rate) as i64, (0.1 * rate) as usize);
    (buf.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / buf.len().max(1) as f64).sqrt()
}

#[test]
fn opacity_fades_between_keyframes() {
    let Some((mut s, id)) = studio() else { return };
    assert!(keys(&s, id, "opacity").is_none(), "nothing is animated at first");
    // a keyframe at 0.5 s with the opacity it has (100), then 0 at 1.5 s
    assert!(s.toggle_keyframe(id, "opacity", 0.5).unwrap());
    s.set_props(&[id], &json!({"opacity": 0.0, "at": 1.5}), None).unwrap();
    let k = keys(&s, id, "opacity").unwrap();
    assert_eq!(k.times.len(), 2);
    assert!((k.times[0] - 0.5).abs() < 1e-6 && (k.times[1] - 1.5).abs() < 1e-6, "{k:?}");
    assert!(!k.smooth);
    // the value between them, and the picture
    let half = s.props_at(id, 1.0).unwrap().opacity.unwrap();
    assert!((half - 50.0).abs() < 2.0, "{half}");
    assert!((s.props_at(id, 0.2).unwrap().opacity.unwrap() - 100.0).abs() < 1e-6);
    assert!(s.props_at(id, 1.8).unwrap().opacity.unwrap() < 1e-6);
    let (b0, b1, b2) = (bright(&s, 0.2), bright(&s, 1.0), bright(&s, 1.8));
    // half opacity over black blends in linear light: about 188 in sRGB, not 128
    assert!(b0 > 230.0 && (b1 - 188.0).abs() < 20.0 && b2 < 15.0, "{b0} {b1} {b2}");

    // changing it at a keyframe moves that keyframe's value, not the other's
    s.set_props(&[id], &json!({"opacity": 40.0, "at": 0.5}), None).unwrap();
    assert!((s.props_at(id, 0.5).unwrap().opacity.unwrap() - 40.0).abs() < 1e-6);
    assert_eq!(keys(&s, id, "opacity").unwrap().times.len(), 2);

    // smooth: eases out of the first keyframe, so a quarter of the way it hasn't moved a quarter
    s.set_easing(id, true).unwrap();
    assert!(keys(&s, id, "opacity").unwrap().smooth);
    let linear_quarter = 40.0 - 0.25 * 40.0;
    let eased = s.props_at(id, 0.75).unwrap().opacity.unwrap();
    assert!((eased - linear_quarter).abs() > 2.0 && eased > linear_quarter, "{eased}");
    s.set_easing(id, false).unwrap();
    assert!(!keys(&s, id, "opacity").unwrap().smooth);

    // toggling at a keyframe removes it; the last one leaves the opacity still, at its value
    assert!(!s.toggle_keyframe(id, "opacity", 1.5).unwrap());
    assert_eq!(keys(&s, id, "opacity").unwrap().times.len(), 1);
    assert!(!s.toggle_keyframe(id, "opacity", 0.5).unwrap());
    assert!(keys(&s, id, "opacity").is_none());
    assert!((s.props_at(id, 1.9).unwrap().opacity.unwrap() - 40.0).abs() < 1e-6);

    // each change was one undo step
    s.undo().unwrap();
    assert_eq!(keys(&s, id, "opacity").unwrap().times.len(), 1);
    s.undo().unwrap();
    assert_eq!(keys(&s, id, "opacity").unwrap().times.len(), 2);
}

#[test]
fn position_and_volume_take_keyframes_and_still_ones_just_change() {
    let Some((mut s, id)) = studio() else { return };
    // position: x from 60 to 260 across the clip
    s.set_props(&[id], &json!({"x": 60.0, "at": 0.0}), None).unwrap();
    assert!(keys(&s, id, "position").is_none(), "a still property just changes");
    assert!((s.props_at(id, 1.5).unwrap().x.unwrap() - 60.0).abs() < 1e-6);
    assert!(s.toggle_keyframe(id, "position", 0.0).unwrap());
    s.set_props(&[id], &json!({"x": 260.0, "at": 1.0}), None).unwrap();
    let p = s.props_at(id, 0.5).unwrap();
    assert!((p.x.unwrap() - 160.0).abs() < 2.0, "{p:?}");
    assert!((p.y.unwrap() - 90.0).abs() < 1e-6, "y stays where it was: {p:?}");

    // volume, on the clip's own sound: loud, then silent
    assert!(s.toggle_keyframe(id, "volume", 0.2).unwrap());
    s.set_props(&[id], &json!({"volumeDb": -60.0, "at": 1.6}), None).unwrap();
    assert_eq!(keys(&s, id, "volume").unwrap().times.len(), 2, "the picture clip shows its sound's keyframes");
    assert!(s.props_at(id, 1.8).unwrap().volume_db.unwrap() <= -59.0);
    let (a, b) = (loud(&s, 0.3), loud(&s, 1.8));
    assert!(a > 0.03 && b < a / 100.0, "{a} {b}");

    // what is refused
    assert!(s.toggle_keyframe(id, "speed", 0.5).is_err());
    assert!(s.toggle_keyframe(id, "opacity", 5.0).is_err(), "off the clip");
}

#[test]
fn keyframes_move_with_their_clip() {
    let Some((mut s, a)) = studio() else { return };
    s.add_clip(clip("kf-black", "black").unwrap().to_str().unwrap()).unwrap();
    let b = main(&s.timeline())[1];
    s.toggle_keyframe(b, "scale", 2.5).unwrap();
    s.set_props(&[b], &json!({"scale": 50.0, "at": 3.5}), None).unwrap();
    assert_eq!(keys(&s, b, "scale").unwrap().times, vec![2.5, 3.5]);
    // b goes first: its keyframes go with it, at the same place in the clip
    s.reorder(b, 0).unwrap();
    let k = keys(&s, b, "scale").unwrap();
    assert!((k.times[0] - 0.5).abs() < 1e-6 && (k.times[1] - 1.5).abs() < 1e-6, "{k:?}");
    assert!(keys(&s, a, "scale").is_none());
    // trimming its start keeps them on the same frames of the picture
    s.trim(b, true, 0.25).unwrap();
    let k = keys(&s, b, "scale").unwrap();
    assert!((k.times[0] - 0.25).abs() < 0.02, "{k:?}");
}
