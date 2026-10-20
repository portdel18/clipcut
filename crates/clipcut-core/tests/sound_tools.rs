//! Sound and time tools through the real engine: two-mic recordings (one side, or both sides in
//! both ears), freeze frames, reverse, and speed that keeps the voice's pitch.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{Studio, TimelineView};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 2-second 320 × 180 clip, 30 fps, with `left` and `right` as its two sides (lavfi sources).
fn clip(name: &str, left: &str, right: &str) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core sound tools")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2"])
            .args(["-f", "lavfi", "-i", &format!("{left}:duration=2:sample_rate=48000")])
            .args(["-f", "lavfi", "-i", &format!("{right}:duration=2:sample_rate=48000")])
            .args(["-filter_complex", "[1:a][2:a]join=inputs=2:channel_layout=stereo[a]", "-map", "0:v", "-map", "[a]"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest"])
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

/// Each side's loudness (RMS) around `t`.
fn sides(s: &Studio, t: f64) -> (f64, f64) {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let buf = feed.mix(((t - 0.1) * rate) as i64, (0.2 * rate) as usize);
    let rms = |k: usize| (buf.iter().skip(k).step_by(2).map(|v| f64::from(*v).powi(2)).sum::<f64>() / (buf.len() / 2).max(1) as f64).sqrt();
    (rms(0), rms(1))
}

#[test]
fn a_microphone_on_one_side_plays_in_both_ears() {
    let Some(path) = clip("ch-left-only", "sine=frequency=440", "anullsrc=channel_layout=mono") else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let id = main(&s.timeline())[0];
    let (l, r) = sides(&s, 1.0);
    assert!(l > 0.05 && r < 0.001, "as recorded: {l} {r}");
    let m = s.channel_levels(id).unwrap();
    assert_eq!(m.suggest, Some("left"), "{m:?}");
    let snd = |s: &Studio| s.timeline().clips.into_iter().find(|c| c.track == "A1").unwrap();
    assert_eq!(snd(&s).props.channels.unwrap().mode, "stereo");

    s.set_channels(&[id], "left", 0.0, 0.0, None).unwrap();
    let (l2, r2) = sides(&s, 1.0);
    assert!((l2 - l).abs() < l * 0.05 && (r2 - l).abs() < l * 0.05, "both ears: {l2} {r2} (was {l})");
    assert_eq!(snd(&s).props.channels.unwrap().mode, "left");
    assert_eq!(s.timeline().undo_label.as_deref(), Some("Left Side Only"));
    // measuring hears the clip as recorded, whatever it is set to
    assert_eq!(s.channel_levels(id).unwrap().suggest, Some("left"));
    // the other side only: silence, as asked
    s.set_channels(&[id], "right", 0.0, 0.0, None).unwrap();
    let (l3, r3) = sides(&s, 1.0);
    assert!(l3 < 0.001 && r3 < 0.001);
    // back as recorded, and undo
    s.set_channels(&[id], "stereo", 0.0, 0.0, None).unwrap();
    assert_eq!(snd(&s).props.channels.unwrap().mode, "stereo");
    assert!(sides(&s, 1.0).1 < 0.001);
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(snd(&s).props.channels.unwrap().mode, "left");
    assert!(s.set_channels(&[id], "sideways", 0.0, 0.0, None).is_err());
    assert!(s.set_channels(&[987_654], "left", 0.0, 0.0, None).is_err());
}

#[test]
fn two_voices_one_on_each_side_mix_into_both_ears() {
    let Some(path) = clip("ch-two-voices", "sine=frequency=440", "aevalsrc=0.03125*sin(2*PI*660*t)") else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let id = main(&s.timeline())[0];
    let m = s.channel_levels(id).unwrap();
    assert_eq!(m.suggest, Some("mono"), "{m:?}");
    let (l, r) = sides(&s, 1.0);
    s.set_channels(&[id], "mono", 0.0, 0.0, None).unwrap();
    let (a, b) = sides(&s, 1.0);
    // both voices on both sides: the two unrelated tones add as powers
    let both = (l * l + r * r).sqrt();
    assert!((a - both).abs() < both * 0.05 && (b - both).abs() < both * 0.05, "{a} {b} vs {both}");
    // the left voice down 12 dB, dragged as one undo step
    s.set_channels(&[id], "mono", -6.0, 0.0, Some("drag")).unwrap();
    s.set_channels(&[id], "mono", -12.0, 0.0, Some("drag")).unwrap();
    let c = s.timeline().clips.into_iter().find(|c| c.track == "A1").unwrap().props.channels.unwrap();
    assert_eq!(c.mode, "mono");
    assert!((c.left_db + 12.0).abs() < 0.05 && c.right_db.abs() < 0.05, "{c:?}");
    let (a2, _) = sides(&s, 1.0);
    let want = ((l / 4.0).powi(2) + r * r).sqrt();
    assert!((a2 - want).abs() < want * 0.06, "{a2} vs {want}");
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.into_iter().find(|c| c.track == "A1").unwrap().props.channels.unwrap().left_db, 0.0, "the drag was one step");
    assert!(s.set_channels(&[id], "mono", f64::NAN, 0.0, None).is_err());
}

/// The program's average colour at `t`.
fn colour(s: &Studio, t: f64) -> [f64; 3] {
    let f = s.frame(t, 160).unwrap();
    let n = (f.rgba.len() / 4).max(1) as f64;
    let mut c = [0.0; 3];
    for p in f.rgba.chunks(4) {
        for k in 0..3 {
            c[k] += f64::from(p[k]) / n;
        }
    }
    c
}
fn same(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.5)
}

#[test]
fn a_freeze_holds_the_frame_and_pushes_the_rest_along() {
    let Some(path) = clip("ch-left-only", "sine=frequency=440", "anullsrc=channel_layout=mono") else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let length = s.timeline().duration;
    let at1 = colour(&s, 1.0);
    let at15 = colour(&s, 1.5);
    let (l15, _) = sides(&s, 1.5);
    let still = s.freeze(1.0, None, 3.0).unwrap();
    let v = s.timeline();
    for c in v.clips.iter() {
        eprintln!("{} {} {:.3}-{:.3} link {:?} freeze {}", c.id, c.track, c.start, c.end, c.link, c.props.freeze);
    }
    assert!((v.duration - length - 3.0).abs() < 0.04, "{} vs {length}", v.duration);
    assert_eq!(v.undo_label.as_deref(), Some("Freeze Frame"));
    let st = v.clips.iter().find(|c| c.id == still).unwrap();
    assert!(st.props.freeze && st.track == "V1" && (st.start - 1.0).abs() < 1e-6 && (st.end - 4.0).abs() < 1e-6, "{st:?}");
    // the frame holds, then the clip carries on where it was
    for t in [1.0, 2.5, 3.9] {
        assert!(same(colour(&s, t), at1), "frozen at {t}");
    }
    assert!(same(colour(&s, 4.5), at15), "carries on after the freeze");
    // the sound pauses, then carries on with its picture
    assert!(sides(&s, 2.5).0 < 0.001, "silent during the freeze");
    assert!((sides(&s, 4.5).0 - l15).abs() < l15 * 0.1, "the sound carries on with the picture");
    s.undo().unwrap();
    assert!((s.timeline().duration - length).abs() < 1e-6, "one undo step");
    assert!(s.freeze(99.0, None, 3.0).is_err());
}

#[test]
fn reverse_plays_a_clip_backwards() {
    let Some(path) = clip("ch-left-only", "sine=frequency=440", "anullsrc=channel_layout=mono") else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let id = main(&s.timeline())[0];
    let first = colour(&s, 0.1);
    let last = colour(&s, 1.9);
    s.set_reverse(&[id], true).unwrap();
    let c = s.timeline().clips.into_iter().find(|c| c.id == id).unwrap();
    assert!(c.props.reverse && (c.end - 2.0).abs() < 0.04, "{c:?}");
    assert!(same(colour(&s, 0.1), last) && same(colour(&s, 1.9), first), "backwards");
    assert!(sides(&s, 1.0).0 > 0.05, "its sound plays backwards too");
    s.set_reverse(&[id], false).unwrap();
    assert!(same(colour(&s, 0.1), first));
    s.undo().unwrap();
    assert!(s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().props.reverse);
}

/// The left side's pitch around `t` (Hz), from its zero crossings.
fn pitch(s: &Studio, t: f64) -> f64 {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let buf = feed.mix(((t - 0.2) * rate) as i64, (0.4 * rate) as usize);
    let left: Vec<f32> = buf.iter().step_by(2).copied().collect();
    let ups = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    ups as f64 / 0.4
}

#[test]
fn faster_or_slower_keeps_the_voice_s_pitch() {
    let Some(path) = clip("ch-left-only", "sine=frequency=440", "anullsrc=channel_layout=mono") else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let id = main(&s.timeline())[0];
    assert!((pitch(&s, 1.0) - 440.0).abs() < 10.0, "{}", pitch(&s, 1.0));
    let snd = |s: &Studio| s.timeline().clips.into_iter().find(|c| c.track == "A1").unwrap();
    assert!(snd(&s).props.keep_pitch, "on by default");
    for (speed, at) in [(200.0, 0.5), (300.0, 0.35), (50.0, 2.0)] {
        s.set_props(&[id], &serde_json::json!({"speed": speed}), None).unwrap();
        let p = pitch(&s, at);
        assert!((p - 440.0).abs() < 15.0, "at {speed}%: {p} Hz");
        assert!(sides(&s, at).0 > 0.05, "still loud at {speed}%");
    }
    // off: the pitch goes up with the speed, as a tape would
    s.set_props(&[id], &serde_json::json!({"speed": 200.0}), None).unwrap();
    s.set_keep_pitch(&[id], false).unwrap();
    assert!(!snd(&s).props.keep_pitch);
    assert!((pitch(&s, 0.5) - 880.0).abs() < 20.0, "{}", pitch(&s, 0.5));
    // and a speed change while off leaves it off
    s.set_props(&[id], &serde_json::json!({"speed": 150.0}), None).unwrap();
    assert!((pitch(&s, 0.6) - 660.0).abs() < 20.0, "{}", pitch(&s, 0.6));
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(snd(&s).props.keep_pitch && (pitch(&s, 0.5) - 440.0).abs() < 15.0, "one undo step each");
    // back at normal speed nothing is left on the sound
    s.set_props(&[id], &serde_json::json!({"speed": 100.0}), None).unwrap();
    assert!((pitch(&s, 1.0) - 440.0).abs() < 10.0);
}

/// A 2-second green screen with a red box in the middle.
fn green_screen() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core colour key")?;
    let out = fixtures_dir("clipcut-core").join("key-green.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=0x00b140:size=320x180:rate=30:duration=2"])
            .args(["-vf", "drawbox=x=120:y=50:w=80:h=80:color=red:t=fill", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// The program's colour at canvas pixel `x`, `y`.
fn at(s: &Studio, t: f64, x: usize, y: usize) -> [u8; 3] {
    let f = s.frame(t, 320).unwrap();
    let k = (y * f.w as usize + x) * 4;
    [f.rgba[k], f.rgba[k + 1], f.rgba[k + 2]]
}

#[test]
fn a_colour_is_picked_and_removed() {
    let Some(path) = green_screen() else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let id = main(&s.timeline())[0];
    let g = at(&s, 1.0, 20, 20);
    assert!(g[1] > 140 && g[0] < 40, "green as recorded: {g:?}");
    let picked = s.pick_colour(1.0, 20.0, 20.0).unwrap();
    let ch = |k: usize| u8::from_str_radix(&picked[1 + 2 * k..3 + 2 * k], 16).unwrap();
    assert!((i32::from(ch(1)) - i32::from(g[1])).abs() < 6 && ch(0) < 40, "{picked} vs {g:?}");

    s.set_key(&[id], Some((&picked, "normal", 0.0, 50.0)), None).unwrap();
    let k = s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().props.key.unwrap();
    assert_eq!((k.color.as_str(), k.strength.as_str()), (picked.as_str(), "normal"));
    let gone = at(&s, 1.0, 20, 20);
    assert!(gone.iter().all(|v| *v < 30), "the green is gone (black under it): {gone:?}");
    let red = at(&s, 1.0, 160, 90);
    assert!(red[0] > 180 && red[1] < 60, "the red box stays: {red:?}");
    // picking again sees the video as it is, not the keyed picture
    assert_eq!(s.pick_colour(1.0, 20.0, 20.0).unwrap(), picked);
    // softer, then back as recorded
    s.set_key(&[id], Some((&picked, "strong", 30.0, 50.0)), Some("drag")).unwrap();
    assert_eq!(s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().props.key.unwrap().strength, "strong");
    s.set_key(&[id], None, None).unwrap();
    assert!(s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().props.key.is_none());
    assert!(at(&s, 1.0, 20, 20)[1] > 140);
    s.undo().unwrap();
    assert!(at(&s, 1.0, 20, 20)[1] < 30, "undo keys it again");
    assert!(s.set_key(&[id], Some(("green", "normal", 0.0, 50.0)), None).is_err());
    assert!(s.set_key(&[id], Some(("#00ff00", "loud", 0.0, 50.0)), None).is_err());
    assert!(s.pick_colour(1.0, -5.0, 20.0).is_err());
}

#[test]
fn a_still_comes_from_the_clip_s_own_media_at_full_size() {
    let Some(path) = clip("ch-left-only", "sine=frequency=440", "anullsrc=channel_layout=mono") else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let ids = main(&s.timeline());
    let (item, at, name) = s.still_source(2.5, None).unwrap();
    assert!((at - 0.5).abs() < 0.02, "half a second into the second clip: {at}");
    assert_eq!(name, "ch-left-only 00m00.50s.png");
    let f = s.stills().frame(item, at, 8192).unwrap();
    assert_eq!((f.w, f.h), (320, 180), "the media's own size");
    // a sped-up clip: the media time under the playhead
    s.set_props(&[ids[0]], &serde_json::json!({"speed": 200.0}), None).unwrap();
    assert!((s.still_source(0.5, Some(ids[0])).unwrap().1 - 1.0).abs() < 0.04);
    assert!(s.still_source(0.5, Some(ids[1])).is_err(), "not under the playhead");
    assert!(s.still_source(99.0, None).is_err());
}

/// How periodic the left side is around `t` at period `p` samples: the normalised autocorrelation
/// at one period (1 for a steady tone; noise and smearing take it down). Read in 1024-sample
/// blocks, as the player reads.
fn periodicity(s: &Studio, t: f64, p: usize) -> f64 {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let at = (t * rate) as i64;
    let mut left = Vec::new();
    for b in 0..24 {
        left.extend(feed.mix(at + b * 1024, 1024).iter().step_by(2).map(|v| f64::from(*v)));
    }
    let (mut xy, mut xx, mut yy) = (0.0, 1e-12, 1e-12);
    for i in 0..left.len() - p {
        xy += left[i] * left[i + p];
        xx += left[i] * left[i];
        yy += left[i + p] * left[i + p];
    }
    xy / (xx * yy).sqrt()
}

#[test]
fn a_faster_voice_stays_clean() {
    // a voice-like sound: a 150 Hz voice with its harmonics, swelling three times a second
    let voice = "aevalsrc=(0.3*sin(2*PI*150*t)+0.2*sin(2*PI*300*t)+0.12*sin(2*PI*450*t)+0.08*sin(2*PI*750*t))*(0.6+0.4*sin(2*PI*3*t))";
    let Some(path) = clip("ch-voice", voice, voice) else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let id = main(&s.timeline())[0];
    let own = periodicity(&s, 0.3, 320);
    for speed in [150.0, 200.0, 75.0] {
        s.set_props(&[id], &serde_json::json!({"speed": speed}), None).unwrap();
        let at = 0.3 * 100.0 / speed;
        let p = periodicity(&s, at, 320);
        eprintln!("at {speed}%: periodicity {p:.4} (the clip's own {own:.4})");
        assert!(p > 0.97 && p > own - 0.02, "at {speed}%: periodicity {p:.4} (the clip's own {own:.4})");
        assert!((pitch(&s, at + 0.2) - 150.0).abs() < 6.0, "at {speed}%: {} Hz", pitch(&s, at + 0.2));
    }
}
