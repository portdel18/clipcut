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
    assert!(s.props_at(id, 1.5).unwrap().opacity.unwrap() < 1e-6);
    let (b0, b1, b2) = (bright(&s, 0.2), bright(&s, 1.0), bright(&s, 1.5));
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
    // held, so it stays silent after the last keyframe
    s.set_hold(id, true).unwrap();
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

#[test]
fn a_keyframe_singled_out_is_deleted_or_moved_on_its_own() {
    let Some((mut s, id)) = studio() else { return };
    // scale at 0.5 and 1.5 (100 → 300), opacity and volume keyed at 1.5 too
    assert!(s.toggle_keyframe(id, "scale", 0.5).unwrap());
    s.set_props(&[id], &json!({"scale": 300.0, "at": 1.5}), None).unwrap();
    assert!(s.toggle_keyframe(id, "opacity", 1.5).unwrap());
    assert!(s.toggle_keyframe(id, "volume", 1.5).unwrap());
    let n = s.timeline().clips.len();

    // moving the diamond at 1.5 moves every property keyed there, values kept
    s.move_keyframes(id, 1.5, 1.0).unwrap();
    for prop in ["scale", "opacity", "volume"] {
        let k = keys(&s, id, prop).unwrap();
        assert!(k.times.iter().any(|t| (t - 1.0).abs() < 1e-6) && !k.times.iter().any(|t| (t - 1.5).abs() < 1e-6), "{prop}: {k:?}");
    }
    assert!((s.props_at(id, 1.0).unwrap().scale.unwrap() - 300.0).abs() < 1e-6);
    let mid = s.props_at(id, 0.8).unwrap().scale.unwrap();
    assert!((mid - 220.0).abs() < 1.0, "three fifths of the way at 0.8: {mid}, keys {:?}", keys(&s, id, "scale"));
    s.undo().unwrap();
    assert!(keys(&s, id, "volume").unwrap().times.iter().any(|t| (t - 1.5).abs() < 1e-6), "one undo step");
    // kept on the clip
    s.move_keyframes(id, 1.5, 99.0).unwrap();
    assert!(keys(&s, id, "scale").unwrap().times.iter().all(|t| *t < 2.0));
    s.undo().unwrap();

    // deleting it takes those keyframes only: the clip and the other keyframe stay
    s.delete_keyframes(id, 1.5).unwrap();
    assert_eq!(s.timeline().clips.len(), n, "the clip stays");
    assert_eq!(keys(&s, id, "scale").map(|k| k.times.len()).unwrap_or(0), 1);
    assert!(keys(&s, id, "opacity").is_none_or(|k| k.times.is_empty()));
    assert!(keys(&s, id, "volume").is_none_or(|k| k.times.is_empty()));
    s.undo().unwrap();
    assert_eq!(keys(&s, id, "scale").unwrap().times.len(), 2, "one undo step");

    // nothing there, or no such clip
    assert!(s.delete_keyframes(id, 1.2).is_err());
    assert!(s.move_keyframes(id, 1.2, 0.3).is_err());
    assert!(s.delete_keyframes(987_654, 1.5).is_err());
}

#[test]
fn a_main_track_clip_fades_its_own_sound() {
    let Some((mut s, id)) = studio() else { return };
    s.add_clip(clip("kf-white", "white").unwrap().to_str().unwrap()).unwrap();
    let before = loud(&s, 0.1);
    s.set_props(&[id], &json!({"fadeIn": 0.8, "fadeOut": 0.5}), None).unwrap();
    let snd = s.timeline().clips.into_iter().find(|c| c.track == "A1" && c.start < 0.1).unwrap();
    assert!((snd.props.fade_in - 0.8).abs() < 0.05 && (snd.props.fade_out - 0.5).abs() < 0.05, "{:?}", snd.props);
    assert!(loud(&s, 0.1) < before * 0.5, "quiet at the start: {} vs {before}", loud(&s, 0.1));
    assert!(loud(&s, 1.0) > before * 0.8, "full in the middle");
    assert!(loud(&s, 1.95) < before * 0.5, "quiet at the end, before the next clip");
    assert!(loud(&s, 2.5) > before * 0.8, "the next clip is untouched");
    // the fades stay on the clip's edges when the main track changes around it
    let second = main(&s.timeline())[1];
    s.reorder(second, 0).unwrap();
    let snd = s.timeline().clips.into_iter().find(|c| c.track == "A1" && c.start > 1.0).unwrap();
    assert!((snd.props.fade_in - 0.8).abs() < 0.05 && (snd.props.fade_out - 0.5).abs() < 0.05, "after moving: {:?}", snd.props);
    assert!(loud(&s, 2.1) < before * 0.5 && loud(&s, 0.1) > before * 0.8, "the faded clip is second now");
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(s.timeline().clips.iter().all(|c| c.props.fade_in == 0.0), "the fades were one undo step");
}

#[test]
fn after_the_last_keyframe_a_clip_goes_straight_back_unless_it_holds() {
    let Some((mut s, id)) = studio() else { return };
    let full = bright(&s, 1.5);
    // opacity 100 % at 0.5 → 20 % at 1.0; scale 100 % at 0.5 → 300 % at 1.0
    assert!(s.toggle_keyframe(id, "opacity", 0.5).unwrap());
    s.set_props(&[id], &json!({"opacity": 20.0, "at": 1.0}), None).unwrap();
    assert!(s.toggle_keyframe(id, "scale", 0.5).unwrap());
    s.set_props(&[id], &json!({"scale": 300.0, "at": 1.0}), None).unwrap();
    let op = |s: &Studio, t: f64| s.props_at(id, t).unwrap().opacity.unwrap();
    // 20 % over black is about 124 in sRGB (the blend is in linear light)
    assert!(bright(&s, 1.0) < full * 0.6, "dim at the last keyframe: {} vs {full}", bright(&s, 1.0));
    // a frame later, and to the end, it is back at the first keyframe's values
    let next = 1.0 + 1.0 / 30.0;
    assert!(bright(&s, next) > full * 0.95, "back a frame later: {}", bright(&s, next));
    assert!(bright(&s, 1.8) > full * 0.95);
    let p = s.props_at(id, 1.5).unwrap();
    assert!((p.scale.unwrap() - 100.0).abs() < 1e-6 && (p.opacity.unwrap() - 100.0).abs() < 1e-6, "{p:?}");
    // the way back is ClipCut's own: never shown, never selectable
    for prop in ["opacity", "scale"] {
        assert_eq!(keys(&s, id, prop).unwrap().times.len(), 2, "{prop}: {:?}", keys(&s, id, prop));
    }
    assert!(s.delete_keyframes(id, next).is_err());
    assert!(!s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().hold);

    // a later keyframe moves the way back after it
    s.set_props(&[id], &json!({"opacity": 50.0, "at": 1.5}), None).unwrap();
    assert!((op(&s, 1.25) - 35.0).abs() < 2.0, "still animating at 1.25: {}", op(&s, 1.25));
    assert!(bright(&s, 1.6) > full * 0.95, "back after the new last keyframe");
    assert_eq!(keys(&s, id, "opacity").unwrap().times.len(), 3);
    // deleting it puts the way back after the one before
    s.delete_keyframes(id, 1.5).unwrap();
    assert!((op(&s, 1.2) - 100.0).abs() < 1e-6);
    // moving the last keyframe takes the way back with it
    s.move_keyframes(id, 1.0, 1.4).unwrap();
    assert!((op(&s, 1.4) - 20.0).abs() < 1e-6 && (op(&s, 1.6) - 100.0).abs() < 1e-6);
    s.undo().unwrap();

    // holding keeps the last values to the end, in one undo step
    s.set_hold(id, true).unwrap();
    assert!(s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().hold);
    assert!(bright(&s, 1.8) < full * 0.6, "held: {}", bright(&s, 1.8));
    assert!((s.props_at(id, 1.8).unwrap().scale.unwrap() - 300.0).abs() < 1e-6);
    // and keeps holding through keyframe edits
    s.set_props(&[id], &json!({"opacity": 40.0, "at": 1.0}), None).unwrap();
    assert!((op(&s, 1.8) - 40.0).abs() < 1e-6);
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(!s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().hold);
    assert!(bright(&s, 1.8) > full * 0.95, "back again after undoing the hold");
    s.set_hold(id, false).unwrap();
    assert!(s.set_hold(987_654, true).is_err());

    // one keyframe, or keyframes that end where they began, need no way back
    let Some((mut s, id)) = studio() else { return };
    assert!(s.toggle_keyframe(id, "volume", 0.5).unwrap());
    assert!(s.timeline().clips.iter().all(|c| !c.hold));
    let loud_before = loud(&s, 1.5);
    s.set_props(&[id], &json!({"volumeDb": -30.0, "at": 1.0}), None).unwrap();
    assert!(loud(&s, 0.9) < loud_before * 0.2, "quiet near the last keyframe");
    assert!(loud(&s, 1.5) > loud_before * 0.8, "the sound comes back too: {} vs {loud_before}", loud(&s, 1.5));
}

#[test]
fn a_fade_in_works_on_any_main_clip_and_next_to_a_transition() {
    let Some((mut s, _)) = studio() else { return };
    s.add_clip(clip("kf-white", "white").unwrap().to_str().unwrap()).unwrap();
    s.add_clip(clip("kf-white", "white").unwrap().to_str().unwrap()).unwrap();
    let m = main(&s.timeline());
    let full = loud(&s, 3.0);
    // the second clip, with a clip before it: its sound starts quiet
    s.set_props(&[m[1]], &json!({"fadeIn": 1.0}), None).unwrap();
    assert!(loud(&s, 2.1) < full * 0.4, "quiet just after the cut: {} vs {full}", loud(&s, 2.1));
    assert!(loud(&s, 3.5) > full * 0.8);
    assert!(loud(&s, 1.9) > full * 0.8, "the clip before is untouched: {}", loud(&s, 1.9));
    // the last clip too
    s.set_props(&[m[2]], &json!({"fadeIn": 0.5}), None).unwrap();
    assert!(loud(&s, 4.1) < full * 0.5, "{}", loud(&s, 4.1));
    assert!(loud(&s, 2.1) < full * 0.4, "the first fade stays: {}", loud(&s, 2.1));

    // with a transition on the cut before it, the fade still starts quiet
    s.undo().unwrap();
    s.undo().unwrap();
    s.set_transition(m[0], "mix", Some(0.5)).unwrap();
    s.set_props(&[m[1]], &json!({"fadeIn": 1.0}), None).unwrap();
    assert!(loud(&s, 2.1) < full * 0.4, "next to a transition: {}", loud(&s, 2.1));
    assert!(loud(&s, 3.5) > full * 0.8);
}
