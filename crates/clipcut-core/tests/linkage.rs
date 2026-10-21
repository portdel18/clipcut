//! Linkage: clips on the lanes follow the main-track clip they start on when the main track is
//! reordered, trimmed or cut, and go when their footage is deleted (CapCut's Linkage).

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{ClipView, Studio, TimelineView};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

fn clip(name: &str, source: &str, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core linkage")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("{source}=size=320x180:rate=30:duration=2")])
            .args(["-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration=2:sample_rate=48000")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// 3 seconds of a tone: the music.
fn music() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core linkage")?;
    let out = fixtures_dir("clipcut-core").join("ln-music.wav");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=220:duration=3:sample_rate=48000"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn by_id(v: &TimelineView, id: u64) -> Option<ClipView> {
    v.clips.iter().find(|c| c.id == id).cloned()
}
fn start(s: &Studio, id: u64) -> Option<f64> {
    by_id(&s.timeline(), id).map(|c| (c.start * 30.0).round() / 30.0)
}
fn main_clips(s: &Studio) -> Vec<ClipView> {
    let mut c: Vec<_> = s.timeline().clips.into_iter().filter(|c| c.track == "V1").collect();
    c.sort_by(|a, b| a.start.total_cmp(&b.start));
    c
}

/// Main track: A (0–2 s), B (2–4 s). On the lanes: B-roll at B's cut (2 s), text over B's middle
/// (2.5 s), music from the start (0–3 s, past A), and a sticker after the end (5 s).
struct Set {
    s: Studio,
    a: u64,
    b: u64,
    broll: u64,
    text: u64,
    music: u64,
    after: u64,
}

fn setup() -> Option<Set> {
    let (fa, fb, fm) = (clip("lk-a", "testsrc2", 440)?, clip("lk-b", "smptebars", 660)?, music()?);
    let mut s = Studio::new();
    s.add_clip(fa.to_str()?).unwrap();
    s.add_clip(fb.to_str()?).unwrap();
    let _ = s.add_clip(fm.to_str()?);
    let m = s.timeline().media.iter().find(|m| m.kind == "audio")?.item;
    // the music as it may have been added: off the lanes, to start from a known state
    let extra: Vec<u64> = s.timeline().clips.iter().filter(|c| c.track != "A1" && c.item == m).map(|c| c.id).collect();
    if !extra.is_empty() {
        s.delete(&extra).unwrap();
    }
    let mains = main_clips(&s);
    let (a, b) = (mains[0].id, mains[1].id);
    let item_a = mains[0].item;
    let broll = *s.add_to_lane(item_a, 2.0).unwrap().iter().find(|c| by_id(&s.timeline(), **c).unwrap().track.starts_with('V')).unwrap();
    let text = s.add_text(2.5, 1.0, "Over B", &json!({})).unwrap();
    let music = s.add_to_lane(m, 0.0).unwrap()[0];
    let after = s.add_text(5.0, 1.0, "After the end", &json!({})).unwrap();
    Some(Set { s, a, b, broll, text, music, after })
}

#[test]
fn lane_clips_follow_a_reorder_and_come_back_with_undo() {
    let Some(mut k) = setup() else { return };
    assert_eq!(start(&k.s, k.broll), Some(2.0));
    k.s.reorder(k.b, 0).unwrap();
    // B now plays 0–2, A 2–4: what started on each goes with it
    assert_eq!(start(&k.s, k.broll), Some(0.0), "B-roll at B's cut");
    assert_eq!(start(&k.s, k.text), Some(0.5), "text over the same moment of B");
    assert_eq!(start(&k.s, k.music), Some(2.0), "music at A's start");
    assert_eq!(start(&k.s, k.after), Some(5.0), "past the main track's end: stays");
    // one undo step
    k.s.undo().unwrap();
    assert_eq!((start(&k.s, k.broll), start(&k.s, k.text), start(&k.s, k.music)), (Some(2.0), Some(2.5), Some(0.0)));
}

#[test]
fn a_trim_moves_what_follows_and_cuts_away_what_was_on_the_trimmed_part() {
    let Some(mut k) = setup() else { return };
    // A loses its first half second: everything after closes up by that much
    k.s.trim(k.a, true, 0.5).unwrap();
    assert_eq!(start(&k.s, k.broll), Some(1.5));
    assert_eq!(start(&k.s, k.text), Some(2.0));
    assert_eq!(start(&k.s, k.music), Some(0.0), "music at A's (new) start");
    // B loses its first 0.7 s: the text's moment (0.5 s into B) is gone, and the text with it;
    // the B-roll at B's cut stays at the cut
    k.s.trim(k.b, true, 0.7).unwrap();
    assert_eq!(start(&k.s, k.broll), Some(1.5));
    assert_eq!(start(&k.s, k.text), None, "deleted with its footage");
}

#[test]
fn deleting_a_main_clip_deletes_what_started_on_it_but_not_a_music_bed() {
    let Some(mut k) = setup() else { return };
    k.s.delete(&[k.b]).unwrap();
    assert_eq!(start(&k.s, k.broll), None);
    assert_eq!(start(&k.s, k.text), None);
    assert_eq!(start(&k.s, k.music), Some(0.0));
    k.s.undo().unwrap();
    assert_eq!((start(&k.s, k.broll), start(&k.s, k.text)), (Some(2.0), Some(2.5)));
    // the music starts on A but runs on past it: deleting A leaves it where it is
    k.s.delete(&[k.a]).unwrap();
    assert_eq!(start(&k.s, k.music), Some(0.0));
    assert_eq!((start(&k.s, k.broll), start(&k.s, k.text)), (Some(0.0), Some(0.5)), "B's clips follow B to the start");
}

#[test]
fn a_split_keeps_everything_in_place() {
    let Some(mut k) = setup() else { return };
    k.s.split(3.0, &[]).unwrap();
    assert_eq!(main_clips(&k.s).len(), 3);
    assert_eq!((start(&k.s, k.broll), start(&k.s, k.text), start(&k.s, k.music)), (Some(2.0), Some(2.5), Some(0.0)));
    // the second piece of B (from 3 s) moved to the front: nothing started on it
    let second = main_clips(&k.s)[2].id;
    k.s.reorder(second, 0).unwrap();
    assert_eq!(start(&k.s, k.text), Some(3.5), "text on B's first piece, now 3–4");
}

#[test]
fn with_linkage_off_the_lanes_stay_put() {
    let Some(mut k) = setup() else { return };
    k.s.set_linkage(false);
    k.s.reorder(k.b, 0).unwrap();
    assert_eq!((start(&k.s, k.broll), start(&k.s, k.text), start(&k.s, k.music)), (Some(2.0), Some(2.5), Some(0.0)));
    k.s.delete(&[k.a]).unwrap();
    assert!(start(&k.s, k.broll).is_some() && start(&k.s, k.text).is_some());
}

#[test]
fn lane_clips_that_meet_on_one_lane_take_another() {
    let Some(mut k) = setup() else { return };
    // a second B-roll on the same lane at A's cut, as long as A: after swapping A and B, both
    // B-rolls would want 0–2 s on their lane at some point; nothing may overlap
    let item_b = main_clips(&k.s)[1].item;
    let lane = by_id(&k.s.timeline(), k.broll).unwrap().track;
    let other = *k.s.add_to_lane(item_b, 0.0).unwrap().iter().find(|c| by_id(&k.s.timeline(), **c).unwrap().track.starts_with('V')).unwrap();
    k.s.move_clip(other, &lane, 0.0).unwrap();
    k.s.reorder(k.b, 0).unwrap();
    let v = k.s.timeline();
    let (x, y) = (by_id(&v, k.broll).unwrap(), by_id(&v, other).unwrap());
    assert_eq!(((x.start * 30.0).round(), (y.start * 30.0).round()), (0.0, 60.0));
    let lanes: Vec<_> = v.clips.iter().filter(|c| c.track.starts_with('V') && c.track != "V1").collect();
    for p in &lanes {
        for q in &lanes {
            assert!(p.id == q.id || p.track != q.track || p.end <= q.start + 1e-6 || q.end <= p.start + 1e-6, "{p:?} overlaps {q:?}");
        }
    }
}
