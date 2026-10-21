//! The magnetic main track: split, Q/W, ripple delete, trims, reorder, undo/redo and projects,
//! through the real engine, on clips ffmpeg generates.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{ClipView, Keep, Studio, TimelineView};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 2-second 320 × 180, 30 fps H.264 clip with a tone, from a lavfi `source`.
fn clip(name: &str, source: &str, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core timeline")?;
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

fn tone(name: &str) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core timeline")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.wav"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=330:duration=1:sample_rate=48000"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn two_clips() -> Option<(Studio, PathBuf, PathBuf)> {
    let a = clip("tl-a", "testsrc2", 440)?;
    let b = clip("tl-b", "smptebars", 660)?;
    let mut s = Studio::new();
    s.add_clip(a.to_str()?).unwrap();
    s.add_clip(b.to_str()?).unwrap();
    Some((s, a, b))
}

fn main_track(v: &TimelineView) -> Vec<ClipView> {
    let mut m: Vec<_> = v.clips.iter().filter(|c| c.track == "V1").cloned().collect();
    m.sort_by(|a, b| a.start.total_cmp(&b.start));
    m
}

const EPS: f64 = 1e-6;

/// The main track starts at 0 with no gaps, and each clip's own sound lines up with it.
fn assert_magnetic(v: &TimelineView) {
    let main = main_track(v);
    let mut at = 0.0;
    for c in &main {
        assert!((c.start - at).abs() < EPS, "gap or overlap before clip {} at {} (expected {at}): {:#?}", c.id, c.start, main);
        at = c.end;
        if let Some(link) = c.link {
            for a in v.clips.iter().filter(|a| a.link == Some(link) && a.track.starts_with('A')) {
                assert!((a.start - c.start).abs() < EPS && (a.end - c.end).abs() < EPS, "sound {} drifted from clip {}: {a:?} vs {c:?}", a.id, c.id);
            }
        }
    }
    assert!((v.duration - at).abs() < 0.05, "sequence {} vs main track end {at}", v.duration);
}

#[test]
fn clips_append_back_to_back_with_their_sound() {
    let Some((s, _, _)) = two_clips() else { return };
    let v = s.timeline();
    assert!(v.has_sequence);
    let main = main_track(&v);
    assert_eq!(main.len(), 2);
    assert!((main[1].start - 2.0).abs() < EPS && (main[1].end - 4.0).abs() < EPS, "{main:#?}");
    assert_eq!(v.clips.iter().filter(|c| c.track == "A1").count(), 2, "each clip brings its sound");
    assert_eq!(v.media.len(), 2);
    assert_eq!(v.media[0].kind, "video");
    assert!((v.media[0].duration - 2.0).abs() < 0.05);
    assert_magnetic(&v);
}

#[test]
fn split_undo_and_redo() {
    let Some((mut s, _, _)) = two_clips() else { return };
    // no selection: the main track is cut at the playhead (with its sound)
    assert_eq!(s.split(1.0, &[]).unwrap(), 1);
    let v = s.timeline();
    assert_eq!(main_track(&v).len(), 3);
    assert_eq!(v.clips.iter().filter(|c| c.track == "A1").count(), 3, "the sound is cut too");
    assert!((v.duration - 4.0).abs() < 0.05, "a split changes no length");
    assert_eq!(v.undo_label.as_deref(), Some("Split"));
    assert_magnetic(&v);
    // the second half keeps playing the media from where the cut was
    let second = &main_track(&v)[1];
    assert!((second.source_in - 1.0).abs() < 0.05, "{second:?}");
    s.undo().unwrap();
    assert_eq!(main_track(&s.timeline()).len(), 2);
    s.redo().unwrap();
    assert_eq!(main_track(&s.timeline()).len(), 3);
    // on a cut or outside every clip there is nothing to split
    assert!(s.split(1.0, &[]).is_err());
    assert!(s.split(99.0, &[]).is_err());
    assert!(s.split(f64::NAN, &[]).is_err());
}

#[test]
fn split_only_cuts_the_selected_clip() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let main = main_track(&s.timeline());
    // the second clip is selected; a cut at 1 s lies in the first, so nothing happens
    assert!(s.split(1.0, &[main[1].id]).is_err());
    assert_eq!(s.split(3.0, &[main[1].id]).unwrap(), 1);
    assert_eq!(main_track(&s.timeline()).len(), 3);
}

#[test]
fn q_removes_left_of_the_playhead_and_w_removes_right() {
    let Some((mut s, _, _)) = two_clips() else { return };
    // Q at 3 s: clip B loses [2, 3); everything after closes up; the playhead goes to 2 s
    let at = s.split_keep(3.0, &[], Keep::Right).unwrap();
    assert!((at - 2.0).abs() < EPS, "{at}");
    let v = s.timeline();
    assert!((v.duration - 3.0).abs() < 0.05, "{}", v.duration);
    let main = main_track(&v);
    assert_eq!(main.len(), 2);
    assert!((main[1].source_in - 1.0).abs() < 0.05, "B now starts 1 s into its media: {:?}", main[1]);
    assert_magnetic(&v);
    assert_eq!(v.undo_label.as_deref(), Some("Delete to the Left"));
    // W at 0.5 s: clip A loses [0.5, 2); the playhead stays
    let at = s.split_keep(0.5, &[], Keep::Left).unwrap();
    assert!((at - 0.5).abs() < EPS);
    let v = s.timeline();
    assert!((v.duration - 1.5).abs() < 0.05, "{}", v.duration);
    assert_magnetic(&v);
    // one undo brings back exactly one step
    s.undo().unwrap();
    assert!((s.timeline().duration - 3.0).abs() < 0.05);
}

#[test]
fn delete_closes_the_gap() {
    let Some((mut s, _, _)) = two_clips() else { return };
    s.split(1.0, &[]).unwrap();
    let main = main_track(&s.timeline());
    s.delete(&[main[1].id]).unwrap();
    let v = s.timeline();
    assert_eq!(main_track(&v).len(), 2);
    assert!((v.duration - 3.0).abs() < 0.05);
    assert_eq!(v.clips.iter().filter(|c| c.track == "A1").count(), 2, "its sound went with it");
    assert_magnetic(&v);
    assert!(s.delete(&[]).is_err());
}

#[test]
fn trims_ripple_the_main_track() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let a = main_track(&s.timeline())[0].id;
    // shorten A's end by half a second: B follows
    let d = s.trim(a, false, -0.5).unwrap();
    assert!((d + 0.5).abs() < 0.05, "{d}");
    let v = s.timeline();
    assert!((v.duration - 3.5).abs() < 0.05);
    assert_magnetic(&v);
    // trim A's start in by 0.25 s: A still starts at 0, 0.25 s into its media
    s.trim(a, true, 0.25).unwrap();
    let v = s.timeline();
    let first = &main_track(&v)[0];
    assert!(first.start.abs() < EPS && (first.source_in - 0.25).abs() < 0.05, "{first:?}");
    assert_magnetic(&v);
    // extending past the media's start stops at it
    let d = s.trim(a, true, -10.0).unwrap();
    assert!((d + 0.25).abs() < 0.05, "stopped at the media start, not {d}");
    assert_magnetic(&s.timeline());
    // a zero trim changes nothing
    assert_eq!(s.trim(a, false, 0.0).unwrap(), 0.0);
}

#[test]
fn reorder_moves_a_clip_and_its_sound() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let main = main_track(&s.timeline());
    let (a, b) = (main[0].id, main[1].id);
    s.reorder(b, 0).unwrap();
    let v = s.timeline();
    let now = main_track(&v);
    assert_eq!(now.iter().map(|c| c.id).collect::<Vec<_>>(), vec![b, a]);
    assert_magnetic(&v);
    assert_eq!(v.undo_label.as_deref(), Some("Reorder"));
    s.undo().unwrap();
    assert_eq!(main_track(&s.timeline()).iter().map(|c| c.id).collect::<Vec<_>>(), vec![a, b]);
    assert!(s.reorder(999_999, 0).is_err());
}

#[test]
fn projects_save_and_reopen() {
    let Some((mut s, _, _)) = two_clips() else { return };
    s.split(1.0, &[]).unwrap();
    let dir = fixtures_dir(&format!("clipcut-core/project-{}", std::process::id()));
    let path = dir.join("Interview.fcproj");
    s.save_as(path.to_str().unwrap()).unwrap();
    assert!(path.exists());
    // previews go in a ClipCut-named folder beside the project, never FilmCraft's
    assert!(!dir.join("FilmCraft Previews").exists(), "the engine's default folder name leaked out");
    let v = s.timeline();
    assert!(!v.project.dirty);
    assert_eq!(v.project.path.as_deref(), path.to_str());
    s.split(3.0, &[]).unwrap();
    assert!(s.timeline().project.dirty);
    s.save().unwrap();
    let mut back = Studio::new();
    back.open_project(path.to_str().unwrap()).unwrap();
    let w = back.timeline();
    assert_eq!(main_track(&w).len(), 4);
    assert!((w.duration - 4.0).abs() < 0.05);
    assert_magnetic(&w);
    assert!(back.frame(2.5, 160).is_ok(), "the reopened project still plays its media");
    // a new project is empty
    back.new_project("Fresh").unwrap();
    assert!(!back.timeline().has_sequence);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sound_only_files_wait_for_audio_tracks() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let Some(wav) = tone("tl-tone") else { return };
    let e = s.add_clip(wav.to_str().unwrap()).unwrap_err().to_string();
    assert!(e.contains("no picture"), "{e}");
    // it is still in the project's media, for later
    assert!(s.timeline().media.iter().any(|m| m.kind == "audio"));
    assert_magnetic(&s.timeline());
}

#[test]
fn thumbnails_show_the_media_not_the_program() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let v = s.timeline();
    let (a, b) = (v.media[0].item, v.media[1].item);
    // Q at 3 s takes B's first second off the timeline; its media still has it
    s.split_keep(3.0, &[], Keep::Right).unwrap();
    let th = s.thumbs();
    let f = th.frame(b, 0.5, 160).unwrap();
    assert_eq!((f.w, f.h), (160, 90));
    let g = th.frame(a, 0.5, 160).unwrap();
    assert_ne!(f.rgba, g.rgba, "two different clips give two different pictures");
    // never upscaled, clamped past the end, and bad input is an error
    assert_eq!(th.frame(a, 99.0, 4000).unwrap().w, 320);
    assert!(th.frame(a, f64::NAN, 160).is_err());
    assert!(th.frame(999_999, 0.0, 160).is_err());
    let Some(wav) = tone("tl-tone") else { return };
    let _ = s.add_clip(wav.to_str().unwrap());
    let audio = s.timeline().media.iter().find(|m| m.kind == "audio").map(|m| m.item).unwrap();
    assert!(s.thumbs().frame(audio, 0.0, 160).is_err(), "sound has no picture");
}

#[test]
fn media_drops_in_between_clips_as_one_step() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let v = s.timeline();
    let before = main_track(&v).iter().map(|c| c.id).collect::<Vec<_>>();
    let b = v.media[1].item;
    s.insert_item(b, 1).unwrap();
    let v = s.timeline();
    let now = main_track(&v);
    assert_eq!(now.len(), 3);
    assert_eq!((now[0].id, now[2].id), (before[0], before[1]), "the new clip sits between the two");
    assert_eq!(now[1].item, b);
    assert_magnetic(&v);
    assert_eq!(v.undo_label.as_deref(), Some("Add Clip"));
    s.undo().unwrap();
    assert_eq!(main_track(&s.timeline()).iter().map(|c| c.id).collect::<Vec<_>>(), before, "one undo takes it back out");
    // past the end appends
    s.insert_item(b, 99).unwrap();
    assert_eq!(main_track(&s.timeline())[2].item, b);
    assert!(s.insert_item(999_999, 0).is_err());
    assert_eq!(main_track(&s.timeline()).len(), 3, "a failed drop changes nothing");
}

#[test]
fn every_edit_is_one_undo_step() {
    let Some((mut s, _, _)) = two_clips() else { return };
    let a = main_track(&s.timeline())[0].id;
    s.split(1.0, &[]).unwrap();
    let mid = main_track(&s.timeline())[1].id;
    s.delete(&[mid]).unwrap();
    assert_eq!(s.timeline().undo_label.as_deref(), Some("Delete"));
    s.trim(a, false, -0.5).unwrap();
    assert_eq!(s.timeline().undo_label.as_deref(), Some("Trim"));
    s.undo().unwrap();
    assert!((s.timeline().duration - 3.0).abs() < 0.05, "one undo reverses the whole trim");
    s.undo().unwrap();
    assert!((s.timeline().duration - 4.0).abs() < 0.05, "one undo brings the deleted piece back");
    assert_magnetic(&s.timeline());
}

#[test]
fn several_main_clips_move_as_one_block_and_change_together() {
    let Some((mut s, a, b)) = two_clips() else { return };
    s.add_clip(a.to_str().unwrap()).unwrap();
    s.add_clip(b.to_str().unwrap()).unwrap();
    let m = main_track(&s.timeline());
    assert_eq!(m.len(), 4);
    let ids: Vec<u64> = m.iter().map(|c| c.id).collect();
    // the first two, dragged to the end: they keep their order and the rest close up
    s.reorder_clips(&[ids[1], ids[0]], 2).unwrap();
    let v = s.timeline();
    let order: Vec<u64> = main_track(&v).iter().map(|c| c.id).collect();
    assert_eq!(order, vec![ids[2], ids[3], ids[0], ids[1]]);
    assert_magnetic(&v);
    assert_eq!(v.undo_label.as_deref(), Some("Reorder"));
    // their sound came along
    for c in main_track(&v) {
        let snd = v.clips.iter().find(|x| x.track == "A1" && x.link == c.link).unwrap();
        assert!((snd.start - c.start).abs() < 1e-6, "{c:?} {snd:?}");
    }
    s.undo().unwrap();
    assert_eq!(main_track(&s.timeline()).iter().map(|c| c.id).collect::<Vec<_>>(), ids);
    assert!(s.reorder_clips(&[], 0).is_err());
    // the volume of several at once, one undo step
    s.set_props(&[ids[0], ids[2]], &serde_json::json!({"volumeDb": -12.0}), None).unwrap();
    let v = s.timeline();
    let vol = |id: u64| {
        let pic = v.clips.iter().find(|c| c.id == id).unwrap();
        v.clips.iter().find(|x| x.track == "A1" && x.link == pic.link).unwrap().props.volume_db
    };
    assert_eq!((vol(ids[0]), vol(ids[2]), vol(ids[1])), (Some(-12.0), Some(-12.0), Some(0.0)));
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.iter().filter(|c| c.props.volume_db == Some(-12.0)).count(), 0);
}
