//! Lanes above and below the main track (overlays, text, music), clip properties, the canvas and
//! waveforms, through the real engine, on media ffmpeg generates.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{ClipView, Studio, TimelineView};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

fn clip(name: &str, source: &str, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core lanes")?;
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

/// 3 seconds of a tone: the "music".
fn music() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core lanes")?;
    let out = fixtures_dir("clipcut-core").join("ln-music.wav");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=220:duration=3:sample_rate=48000"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Two 2-second clips on the main track, and the music in the project's media.
fn setup() -> Option<(Studio, u64, u64)> {
    let a = clip("tl-a", "testsrc2", 440)?;
    let b = clip("tl-b", "smptebars", 660)?;
    let m = music()?;
    let mut s = Studio::new();
    s.add_clip(a.to_str()?).unwrap();
    s.add_clip(b.to_str()?).unwrap();
    let _ = s.add_clip(m.to_str()?);
    let v = s.timeline();
    let music = v.media.iter().find(|m| m.kind == "audio")?.item;
    let video = v.media.iter().find(|m| m.kind == "video")?.item;
    Some((s, music, video))
}

fn on(v: &TimelineView, track: &str) -> Vec<ClipView> {
    let mut c: Vec<_> = v.clips.iter().filter(|c| c.track == track).cloned().collect();
    c.sort_by(|a, b| a.start.total_cmp(&b.start));
    c
}
fn by_id(v: &TimelineView, id: u64) -> ClipView {
    v.clips.iter().find(|c| c.id == id).cloned().unwrap()
}
const EPS: f64 = 1e-6;

#[test]
fn music_goes_below_and_stays_put_when_the_main_track_changes() {
    let Some((mut s, music, _)) = setup() else { return };
    let ids = s.add_to_lane(music, 1.0).unwrap();
    assert_eq!(ids.len(), 1);
    let v = s.timeline();
    let m = by_id(&v, ids[0]);
    assert_eq!((m.track.as_str(), m.kind), ("A2", "audio"));
    assert!((m.start - 1.0).abs() < EPS && (m.end - 4.0).abs() < 0.05, "{m:?}");
    assert_eq!(v.undo_label.as_deref(), Some("Add Sound"));
    // a second one where the first is goes on the next lane down
    let second = s.add_to_lane(music, 2.0).unwrap()[0];
    assert_eq!(by_id(&s.timeline(), second).track, "A3");
    // deleting, trimming and Q on the main track move nothing on the lanes
    let main = on(&s.timeline(), "V1");
    s.delete(&[main[0].id]).unwrap();
    s.trim(main[1].id, false, -0.5).unwrap();
    s.split_keep(0.5, &[], clipcut_core::Keep::Right).unwrap();
    let v = s.timeline();
    assert!((by_id(&v, ids[0]).start - 1.0).abs() < EPS, "the music moved: {:?}", by_id(&v, ids[0]));
    assert!((by_id(&v, second).start - 2.0).abs() < EPS);
    // deleting a lane clip leaves its place empty and closes nothing
    s.delete(&[ids[0]]).unwrap();
    let v = s.timeline();
    assert!(v.clips.iter().all(|c| c.id != ids[0]));
    assert!((by_id(&v, second).start - 2.0).abs() < EPS);
    // a lane clip trims without moving anything else
    s.trim(second, true, 0.5).unwrap();
    let c = by_id(&s.timeline(), second);
    assert!((c.start - 2.5).abs() < 0.05 && (c.source_in - 0.5).abs() < 0.05, "{c:?}");
}

#[test]
fn overlays_carry_their_sound_and_move_as_one() {
    let Some((mut s, _, video)) = setup() else { return };
    let ids = s.add_to_lane(video, 0.5).unwrap();
    let v = s.timeline();
    let pic = ids.iter().map(|i| by_id(&v, *i)).find(|c| c.track == "V2").expect("the picture on V2");
    let snd = ids.iter().map(|i| by_id(&v, *i)).find(|c| c.track.starts_with('A')).expect("its sound");
    assert_eq!(snd.track, "A2");
    assert_eq!(pic.kind, "video");
    assert!(pic.link.is_some() && pic.link == snd.link);
    // the main track is untouched
    assert_eq!(on(&v, "V1").len(), 2);
    assert!((v.duration - 4.0).abs() < 0.05);
    // moving the picture takes its sound along
    s.move_clip(pic.id, "V2", 1.5).unwrap();
    let v = s.timeline();
    assert!((by_id(&v, pic.id).start - 1.5).abs() < EPS && (by_id(&v, snd.id).start - 1.5).abs() < EPS);
    assert_eq!(v.undo_label.as_deref(), Some("Move"));
    // a second overlay where the first is goes on V3, and moving it onto the first bumps a lane
    let other = s.add_to_lane(video, 0.0).unwrap();
    let v = s.timeline();
    let opic = other.iter().map(|i| by_id(&v, *i)).find(|c| c.track.starts_with('V')).unwrap();
    assert_eq!(opic.track, "V3");
    s.move_clip(opic.id, "V2", 1.0).unwrap();
    assert_eq!(by_id(&s.timeline(), opic.id).track, "V3", "it can't land on the first overlay");
    // pictures never go below the main track
    assert!(s.move_clip(pic.id, "A2", 0.0).is_err());
    // nothing goes before 0
    s.move_clip(pic.id, "V2", -5.0).unwrap();
    assert!(by_id(&s.timeline(), pic.id).start.abs() < EPS);
}

#[test]
fn a_box_selection_moves_together_in_one_step() {
    let Some((mut s, music, video)) = setup() else { return };
    let ids = s.add_to_lane(video, 0.5).unwrap();
    let tune = s.add_to_lane(music, 2.0).unwrap()[0];
    let v = s.timeline();
    let pic = ids.iter().map(|i| by_id(&v, *i)).find(|c| c.track == "V2").unwrap();
    let snd = ids.iter().map(|i| by_id(&v, *i)).find(|c| c.track.starts_with('A')).unwrap();
    let tune_lane = by_id(&v, tune).track;
    let main = on(&v, "V1");
    // the picture, the music and a main clip are selected: the lane clips move, the main clip stays
    s.move_clips(&[pic.id, tune, main[0].id], 1.0).unwrap();
    let v = s.timeline();
    assert!((by_id(&v, pic.id).start - 1.5).abs() < EPS, "{:?}", by_id(&v, pic.id));
    assert!((by_id(&v, snd.id).start - 1.5).abs() < EPS, "its sound goes with it");
    assert!((by_id(&v, tune).start - 3.0).abs() < EPS);
    assert_eq!((by_id(&v, pic.id).track, by_id(&v, tune).track), ("V2".to_string(), tune_lane.clone()));
    assert!(by_id(&v, main[0].id).start.abs() < EPS);
    assert_eq!(v.undo_label.as_deref(), Some("Move"));
    // one undo puts all of them back
    s.undo().unwrap();
    let v = s.timeline();
    assert!((by_id(&v, pic.id).start - 0.5).abs() < EPS && (by_id(&v, tune).start - 2.0).abs() < EPS);
    // the set stops at 0, keeping its spacing
    s.move_clips(&[pic.id, tune], -10.0).unwrap();
    let v = s.timeline();
    assert!(by_id(&v, pic.id).start.abs() < EPS && (by_id(&v, tune).start - 1.5).abs() < EPS);
    // the main track alone is not a lane selection
    assert!(s.move_clips(&[main[0].id], 1.0).is_err());
    assert!(s.move_clips(&[pic.id], f64::NAN).is_err());
}

#[test]
fn text_is_added_styled_and_edited() {
    let Some((mut s, _, _)) = setup() else { return };
    let id = s.add_text(1.0, 2.0, "Mayor Jane Doe", &json!({"color": "#ffcc00", "stroke": true})).unwrap();
    let v = s.timeline();
    let c = by_id(&v, id);
    assert_eq!((c.track.as_str(), c.kind), ("V2", "text"));
    assert!((c.start - 1.0).abs() < EPS && (c.end - 3.0).abs() < 0.05, "{c:?}");
    let t = c.props.text.clone().unwrap();
    assert_eq!(t.text, "Mayor Jane Doe");
    assert_eq!(t.color, "#ffcc00");
    assert!(t.stroke);
    assert_eq!(t.align, "center");
    assert_eq!(v.undo_label.as_deref(), Some("Add Text"));
    // the words are on the picture: yellow pixels where there were none
    let yellow = |s: &Studio| s.frame(2.0, 320).unwrap().rgba.chunks(4).filter(|p| p[0] > 200 && p[1] > 160 && p[2] < 80).count();
    let with = yellow(&s);
    s.set_props(&[id], &json!({"text": "Hi", "bold": true, "size": 40.0}), None).unwrap();
    let t = by_id(&s.timeline(), id).props.text.unwrap();
    assert_eq!((t.text.as_str(), t.bold, t.size), ("Hi", true, 40.0));
    s.set_props(&[id], &json!({"opacity": 0.0}), None).unwrap();
    assert!(yellow(&s) < with, "opacity 0 hides the text ({} vs {with})", yellow(&s));
    // text and trims: it can be longer than when it was made
    s.trim(id, false, 3.0).unwrap();
    assert!((by_id(&s.timeline(), id).end - 6.0).abs() < 0.05);
}

#[test]
fn properties_set_read_back_and_merge_into_one_step() {
    let Some((mut s, music, _)) = setup() else { return };
    let main = on(&s.timeline(), "V1")[0].clone();
    // a main clip's volume is its own sound's
    for db in [-2.0, -4.0, -6.0] {
        s.set_props(&[main.id], &json!({"volumeDb": db}), Some("volume")).unwrap();
    }
    let v = s.timeline();
    let snd = v.clips.iter().find(|c| c.track == "A1" && c.link == main.link).unwrap();
    assert_eq!(snd.props.volume_db, Some(-6.0));
    assert_eq!(v.undo_label.as_deref(), Some("Volume"));
    s.undo().unwrap();
    let v = s.timeline();
    let snd = v.clips.iter().find(|c| c.track == "A1" && c.link == main.link).unwrap();
    assert_eq!(snd.props.volume_db, Some(0.0), "one undo takes back the whole drag");
    // position, scale, rotation and opacity on the picture
    s.set_props(&[main.id], &json!({"x": 100.0, "scale": 50.0, "rotation": 10.0, "opacity": 80.0}), None).unwrap();
    let p = by_id(&s.timeline(), main.id).props;
    assert_eq!((p.x, p.y, p.scale, p.rotation, p.opacity), (Some(100.0), Some(90.0), Some(50.0), Some(10.0), Some(80.0)));
    // fades on music, kept within half the clip
    let m = s.add_to_lane(music, 0.0).unwrap()[0];
    s.set_props(&[m], &json!({"fadeIn": 0.5, "fadeOut": 9.0}), None).unwrap();
    let p = by_id(&s.timeline(), m).props;
    assert!((p.fade_in - 0.5).abs() < 0.05 && (p.fade_out - 1.5).abs() < 0.05, "{p:?}");
    s.set_props(&[m], &json!({"fadeIn": 0.0}), None).unwrap();
    assert!(by_id(&s.timeline(), m).props.fade_in.abs() < EPS);
    // the music really fades out: quieter near its end than in the middle (the main track's own
    // tone muted, so only the music is measured)
    s.set_lane("A1", false).unwrap();
    let feed = s.audio_feed().unwrap();
    let rms = |at: f64| {
        let b = feed.mix((at * 48_000.0) as i64, 2400);
        (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt()
    };
    assert!(rms(2.9) < rms(1.4) * 0.5, "no fade: {} vs {}", rms(2.9), rms(1.4));
    // bad input is refused, not stored
    assert!(s.set_props(&[main.id], &json!({"scale": "big"}), None).is_err());
    assert!(s.set_props(&[], &json!({"scale": 50.0}), None).is_err());
}

#[test]
fn speed_keeps_the_main_track_closed() {
    let Some((mut s, _, _)) = setup() else { return };
    let main = on(&s.timeline(), "V1");
    s.set_props(&[main[0].id], &json!({"speed": 200.0}), None).unwrap();
    let v = s.timeline();
    let now = on(&v, "V1");
    assert!((now[0].end - 1.0).abs() < 0.05 && (now[1].start - now[0].end).abs() < EPS, "{now:#?}");
    assert_eq!(now[0].props.speed, 200.0);
    assert!((v.duration - 3.0).abs() < 0.05);
}

#[test]
fn the_canvas_turns_vertical() {
    let Some((mut s, _, _)) = setup() else { return };
    s.set_canvas(1080, 1920).unwrap();
    let v = s.timeline();
    assert_eq!((v.width, v.height), (1080, 1920));
    assert_eq!(v.undo_label.as_deref(), Some("Canvas"));
    let f = s.frame(0.5, 270).unwrap();
    assert_eq!((f.w, f.h), (270, 480));
    let main = on(&v, "V1");
    s.fit_clips(&[main[0].id], true).unwrap();
    assert!(s.set_canvas(0, 1920).is_err());
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!((s.timeline().width, s.timeline().height), (320, 180));
}

#[test]
fn lanes_mute_and_hide() {
    let Some((mut s, music, _)) = setup() else { return };
    s.add_to_lane(music, 0.0).unwrap();
    let lane = |s: &Studio, id: &str| s.timeline().lanes.iter().find(|l| l.id == id).cloned().unwrap();
    assert!(lane(&s, "A2").on);
    assert_eq!(lane(&s, "A2").clips, 1);
    s.set_lane("A2", false).unwrap();
    assert!(!lane(&s, "A2").on);
    s.set_lane("V1", false).unwrap();
    assert!(!lane(&s, "V1").on);
    assert!(s.set_lane("A9", false).is_err());
    assert!(s.set_lane("nope", false).is_err());
}

#[test]
fn waveforms_follow_the_sound() {
    let Some((s, music, video)) = setup() else { return };
    let p = s.peaks(music).unwrap().compute();
    assert!((p.len() as i64 - 300).abs() <= 2, "{} peaks for 3 s", p.len());
    // ffmpeg's sine is an eighth of full scale: 32 of 255
    assert!(p.iter().all(|v| (28..=36).contains(v)), "a steady tone is as loud throughout: {:?}", &p[..10]);
    assert!(!s.peaks(video).unwrap().compute().is_empty());
    assert!(s.peaks(999_999).is_err());
}

#[test]
fn dropped_files_go_where_they_belong() {
    let Some((mut s, _, _)) = setup() else { return };
    let (Some(m), Some(a)) = (music(), clip("tl-a", "testsrc2", 440)) else { return };
    s.add_file(m.to_str().unwrap(), 1.0).unwrap();
    s.add_file(a.to_str().unwrap(), 1.0).unwrap();
    let v = s.timeline();
    assert_eq!(on(&v, "V1").len(), 3, "the video joins the main track");
    let lane = on(&v, "A2");
    assert_eq!(lane.len(), 1, "the sound goes on a lane");
    assert!((lane[0].start - 1.0).abs() < EPS);
}

#[test]
fn clips_move_freely_between_lanes_and_the_main_track() {
    let Some((mut s, _, video)) = setup() else { return };
    let first = s.add_to_lane(video, 0.0).unwrap();
    let second = s.add_to_lane(video, 3.0).unwrap();
    let v = s.timeline();
    let pic = |ids: &[u64], v: &TimelineView| ids.iter().map(|i| by_id(v, *i)).find(|c| c.track.starts_with('V')).unwrap();
    let (a, b) = (pic(&first, &v), pic(&second, &v));
    assert_eq!((a.track.as_str(), b.track.as_str()), ("V2", "V2"));
    // up to a lane of its own, and back down to one that is free there
    s.move_clip(b.id, "V3", 3.0).unwrap();
    assert_eq!(by_id(&s.timeline(), b.id).track, "V3");
    s.move_clip(b.id, "V2", 3.0).unwrap();
    assert_eq!(by_id(&s.timeline(), b.id).track, "V2", "a free lane takes it");
    // onto a lane that is taken there: a new lane right above it, never back where it came from
    s.move_clip(b.id, "V3", 3.0).unwrap();
    s.move_clip(b.id, "V2", 0.5).unwrap();
    let v = s.timeline();
    assert_eq!(by_id(&v, b.id).track, "V3", "just above the lane it was dropped on: {:#?}", on(&v, "V3"));
    assert_eq!(by_id(&v, a.id).track, "V2", "the clip that was there stays");
    // into the main track, at the cut nearest where it is dropped (between the two clips)
    let mains = on(&v, "V1");
    assert_eq!(mains.len(), 2);
    s.move_clip(a.id, "V1", 1.9).unwrap();
    let v = s.timeline();
    let m = on(&v, "V1");
    assert_eq!(m.len(), 3, "{m:#?}");
    assert_eq!(m[1].id, a.id);
    assert!((m[1].start - 2.0).abs() < EPS && (m[2].start - 4.0).abs() < EPS, "{m:#?}");
    // its sound came with it, onto the main track's sound, in step
    let snd = on(&v, "A1");
    assert!(snd.iter().any(|c| c.link == by_id(&v, a.id).link && (c.start - 2.0).abs() < EPS), "{snd:#?}");
    assert_eq!(v.undo_label.as_deref(), Some("Move"));
    // and off the main track again: the main track closes up, its sound goes to a lane
    s.move_clip(a.id, "V2", 1.0).unwrap();
    let v = s.timeline();
    assert_eq!(on(&v, "V1").len(), 2);
    assert!((on(&v, "V1")[1].start - 2.0).abs() < EPS, "the main track closed up");
    let moved = by_id(&v, a.id);
    assert!(moved.track.starts_with('V') && moved.track != "V1" && (moved.start - 1.0).abs() < EPS, "{moved:?}");
    let msnd = v.clips.iter().find(|c| c.link == moved.link && c.id != a.id).unwrap();
    assert!(msnd.track.starts_with('A') && msnd.track != "A1" && (msnd.start - 1.0).abs() < EPS, "{msnd:?}");
    // one undo step each
    s.undo().unwrap();
    assert_eq!(on(&s.timeline(), "V1").len(), 3);
    // text stays on the lanes; the main track's only clip stays put
    let t = s.add_text(0.0, 1.0, "Hello", &json!({})).unwrap();
    assert!(s.move_clip(t, "V1", 0.0).is_err());
    let mut one = Studio::new();
    one.add_clip(clip("tl-a", "testsrc2", 440).unwrap().to_str().unwrap()).unwrap();
    let only = on(&one.timeline(), "V1")[0].id;
    assert!(one.move_clip(only, "V2", 0.0).is_err());
}
