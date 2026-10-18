//! Transitions on the main track's cuts: adding, changing, lengths, everywhere, removing, how
//! they follow edits, their sound crossfade, saving, and that each one draws something between
//! its two clips.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{Studio, TRANSITIONS, TimelineView, TransitionView, transition_previews};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 2-second 320 × 180, 30 fps clip of one flat colour with a tone.
fn clip(name: &str, colour: &str, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core transitions")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("color=c={colour}:size=320x180:rate=30:duration=2")])
            .args(["-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration=2:sample_rate=48000")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Red, green and blue clips back to back on the main track.
fn three() -> Option<Studio> {
    let mut s = Studio::new();
    for (n, c, hz) in [("tr-red", "red", 440), ("tr-green", "green", 550), ("tr-blue", "blue", 660)] {
        s.add_clip(clip(n, c, hz)?.to_str()?).unwrap();
    }
    Some(s)
}

fn main_ids(v: &TimelineView) -> Vec<u64> {
    let mut m: Vec<_> = v.clips.iter().filter(|c| c.track == "V1").collect();
    m.sort_by(|a, b| a.start.total_cmp(&b.start));
    m.iter().map(|c| c.id).collect()
}

fn trs(s: &Studio) -> Vec<TransitionView> {
    s.timeline().transitions
}

const EPS: f64 = 1e-3;

/// Every transition is centred on a cut between the two clips it names.
fn assert_on_cuts(s: &Studio) {
    let v = s.timeline();
    let ids = main_ids(&v);
    for t in &v.transitions {
        let k = ids.iter().position(|c| *c == t.from).expect("its clip is on the main track");
        assert_eq!(ids.get(k + 1), Some(&t.to), "{t:#?} joins clips that don't meet: {ids:?}");
        let b = v.clips.iter().find(|c| c.id == t.to).unwrap();
        assert!((t.at - b.start).abs() < EPS, "{t:#?} isn't at its cut {}", b.start);
        // within half a frame: an odd number of frames has its extra one after the cut
        assert!(((t.start + t.end) / 2.0 - t.at).abs() < 1.0 / 60.0 + EPS, "{t:#?} isn't centred on its cut");
        assert!(t.end - t.start <= t.max + EPS, "{t:#?} is longer than its clips allow");
    }
}

/// The average colour of the program frame at `t`.
fn colour_at(s: &Studio, t: f64) -> [f64; 3] {
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

#[test]
fn transitions_go_on_cuts_and_change_and_go() {
    let Some(mut s) = three() else { return };
    let ids = main_ids(&s.timeline());
    let length = s.timeline().duration;
    s.set_transition(ids[0], "mix", None).unwrap();
    let t = trs(&s);
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].from, t[0].to, t[0].preset.as_str(), t[0].name.as_str()), (ids[0], ids[1], "mix", "Mix"));
    assert!((t[0].end - t[0].start - 0.5).abs() < EPS, "half a second by default: {t:#?}");
    assert_on_cuts(&s);
    // the timeline keeps its length: the transition plays over the cut
    assert!((s.timeline().duration - length).abs() < EPS);
    // the last clip has nothing after it
    assert!(s.set_transition(ids[2], "mix", None).is_err());
    assert!(s.set_transition(ids[0], "no-such", None).is_err());

    // another kind keeps the length; a new length is fitted to the clips (2 s each)
    s.set_transition(ids[0], "push-left", None).unwrap();
    let t = trs(&s);
    assert_eq!((t.len(), t[0].preset.as_str()), (1, "push-left"));
    assert!((t[0].end - t[0].start - 0.5).abs() < 0.04);
    s.set_transition_length(ids[0], 9.0, None).unwrap();
    let t = trs(&s);
    assert!((t[0].end - t[0].start - 2.0).abs() < 0.04, "fitted to its clips: {t:#?}");
    assert!(s.set_transition_length(ids[0], -1.0, None).is_err());
    // a slider dragged through lengths is one undo step
    s.set_transition_length(ids[0], 1.0, Some("drag")).unwrap();
    s.set_transition_length(ids[0], 1.2, Some("drag")).unwrap();
    assert!((trs(&s)[0].end - trs(&s)[0].start - 1.2).abs() < 0.04, "{:#?}", trs(&s));
    s.undo().unwrap();
    assert!((trs(&s)[0].end - trs(&s)[0].start - 2.0).abs() < 0.04, "{:#?}", trs(&s));
    s.undo().unwrap();
    assert_eq!(trs(&s)[0].preset, "push-left");
    assert!((trs(&s)[0].end - trs(&s)[0].start - 0.5).abs() < 0.04, "{:#?}", trs(&s));
    s.undo().unwrap();
    assert_eq!(trs(&s)[0].preset, "mix");

    // everywhere, then removed
    assert_eq!(s.transition_everywhere("black", 0.8).unwrap(), 2);
    let t = trs(&s);
    assert!(t.iter().all(|t| t.preset == "black" && (t.end - t.start - 0.8).abs() < 0.04), "{t:#?}");
    assert_on_cuts(&s);
    s.remove_transitions(&[ids[0]]).unwrap();
    assert_eq!(trs(&s).iter().map(|t| t.from).collect::<Vec<_>>(), vec![ids[1]]);
    s.undo().unwrap();
    assert_eq!(trs(&s).len(), 2);
}

#[test]
fn transitions_follow_the_main_track() {
    let Some(mut s) = three() else { return };
    let ids = main_ids(&s.timeline());
    s.transition_everywhere("mix", 0.5).unwrap();

    // trimming the middle clip: both stay on their cuts
    s.trim(ids[1], false, -0.5).unwrap();
    assert_eq!(trs(&s).len(), 2);
    assert_on_cuts(&s);
    // a clip shorter than the transition shortens it
    s.trim(ids[1], true, 1.2).unwrap();
    assert_eq!(trs(&s).len(), 2, "{:#?}", trs(&s));
    assert!(trs(&s).iter().all(|t| t.end - t.start <= 0.3 + EPS), "{:#?}", trs(&s));
    assert_on_cuts(&s);
    s.undo().unwrap();
    s.undo().unwrap();

    // a split inside a clip keeps the transitions at its ends
    s.split(3.0, &[]).unwrap();
    assert_eq!(trs(&s).len(), 2, "{:#?}", trs(&s));
    assert_on_cuts(&s);
    s.undo().unwrap();

    // reordering: the cuts that no longer exist lose their transitions; the rest stay
    s.reorder(ids[2], 0).unwrap();
    let after = main_ids(&s.timeline());
    assert_eq!(after, vec![ids[2], ids[0], ids[1]]);
    assert_eq!(trs(&s).iter().map(|t| (t.from, t.to)).collect::<Vec<_>>(), vec![(ids[0], ids[1])]);
    assert_on_cuts(&s);
    s.undo().unwrap();
    assert_eq!(trs(&s).len(), 2);

    // deleting the middle clip: its cuts go with it
    s.delete(&[ids[1]]).unwrap();
    assert!(trs(&s).is_empty(), "{:#?}", trs(&s));
    s.undo().unwrap();
    assert_eq!(trs(&s).len(), 2);
    assert_on_cuts(&s);
}

#[test]
fn transitions_mix_the_picture_and_the_sound_and_are_saved() {
    let Some(mut s) = three() else { return };
    let ids = main_ids(&s.timeline());
    // a plain cut: red right up to 2 s
    let red = colour_at(&s, 1.9);
    assert!(red[0] > 150.0 && red[1] < 60.0, "{red:?}");
    s.set_transition(ids[0], "mix", Some(1.0)).unwrap();
    // inside the mix, red and green at once
    let mid = colour_at(&s, 2.0);
    assert!(mid[0] > 40.0 && mid[1] > 40.0, "a mix shows both clips: {mid:?}");
    // before and after it, each clip alone
    let before = colour_at(&s, 1.3);
    assert!(before[0] > 150.0 && before[1] < 60.0, "{before:?}");

    // the green clip's 550 Hz tone just before the cut at `cut`
    let hz550 = |s: &Studio, cut: f64| {
        let feed = s.audio_feed().unwrap();
        let rate = f64::from(feed.sample_rate);
        let (from, n) = (((cut - 0.15) * rate) as i64, (0.1 * rate) as usize);
        let buf = feed.mix(from, n);
        let (mut re, mut im) = (0.0, 0.0);
        for (i, f) in buf.chunks(2).enumerate() {
            let w = 2.0 * std::f64::consts::PI * 550.0 * (from as f64 + i as f64) / rate;
            re += f64::from(f[0]) * w.cos();
            im += f64::from(f[0]) * w.sin();
        }
        re.hypot(im) / n as f64
    };
    // these clips use all their media: a crossfade would only dip the sound, so it cuts
    assert!(hz550(&s, 2.0) < 0.002, "{}", hz550(&s, 2.0));
    // with half a second to spare on both sides, the sound crossfades under the picture
    s.remove_transitions(&[ids[0]]).unwrap();
    s.trim(ids[0], false, -0.5).unwrap();
    s.trim(ids[1], true, 0.5).unwrap();
    let cut = s.timeline().clips.iter().find(|c| c.id == ids[1]).unwrap().start;
    assert!((cut - 1.5).abs() < EPS);
    let plain = hz550(&s, cut);
    s.set_transition(ids[0], "mix", Some(1.0)).unwrap();
    let mixed = hz550(&s, cut);
    assert!(mixed > 0.02 && mixed > 10.0 * plain, "no crossfade under the transition: {mixed} vs {plain} at 550 Hz");

    // saved and opened again
    let path = fixtures_dir("clipcut-core").join(format!("transitions-{}.fcproj", std::process::id()));
    s.save_as(path.to_str().unwrap()).unwrap();
    let mut back = Studio::new();
    back.open_project(path.to_str().unwrap()).unwrap();
    let t = trs(&back);
    assert_eq!((t.len(), t[0].preset.as_str()), (1, "mix"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn every_transition_draws_between_its_clips() {
    let Some(mut s) = three() else { return };
    let ids = main_ids(&s.timeline());
    let red = colour_at(&s, 1.0);
    let green = colour_at(&s, 3.0);
    let dist = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>();
    for p in TRANSITIONS {
        s.set_transition(ids[0], p.id, Some(1.0)).unwrap();
        // half way, the frame is neither clip alone
        let c = colour_at(&s, 2.0);
        assert!(dist(c, red) > 4.0 && dist(c, green) > 4.0, "{} draws nothing between its clips: {c:?}", p.name);
    }
    // the Transitions tab's previews: one run of frames for each, starting on the first picture
    // and ending on the second
    let f = transition_previews(64, 36, 6).unwrap();
    assert_eq!(f.len(), TRANSITIONS.len() * 6);
    for (k, p) in TRANSITIONS.iter().enumerate() {
        let run = &f[k * 6..k * 6 + 6];
        assert_eq!(run[0].rgba, f[0].rgba, "{} doesn't start on the first picture", p.name);
        assert_eq!(run[5].rgba, f[5].rgba, "{} doesn't end on the second picture", p.name);
        assert!(run[2].rgba != run[0].rgba && run[2].rgba != run[5].rgba, "{} shows nothing in between", p.name);
    }
}

#[test]
fn transitions_survive_the_edits_that_need_room() {
    let Some(mut s) = three() else { return };
    let ids = main_ids(&s.timeline());
    s.transition_everywhere("wipe", 1.5).unwrap();
    // speeding the middle clip up to 4× makes it half a second: its transitions shrink to fit
    s.set_props(&[ids[1]], &serde_json::json!({"speed": 400.0}), None).unwrap();
    let t = trs(&s);
    assert_eq!(t.len(), 2, "{t:#?}");
    assert!(t.iter().all(|t| t.end - t.start <= 0.5 + EPS), "{t:#?}");
    assert_on_cuts(&s);
}
