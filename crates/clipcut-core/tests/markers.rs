//! Markers with notes: added at a moment with a name, notes and a colour, changed, moved and
//! taken away in one undo step each; over the main track they follow its footage through edits;
//! named markers make YouTube chapters.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{MarkerView, Studio};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// 4 s at 30 fps with a tone.
fn clip() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core markers")?;
    let out = fixtures_dir("clipcut-core").join("markers-clip.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn markers(s: &Studio) -> Vec<MarkerView> {
    s.timeline().markers
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.04
}

#[test]
fn markers_keep_their_notes_and_follow_their_footage() {
    let Some(v) = clip() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let v1 = |s: &Studio| {
        let mut c: Vec<_> = s.timeline().clips.into_iter().filter(|c| c.track == "V1").collect();
        c.sort_by(|a, b| a.start.total_cmp(&b.start));
        c
    };
    let (a, b) = (v1(&s)[0].id, v1(&s)[1].id);

    // added on the frame grid, with its words and colour (blue unless chosen)
    let m = s.add_marker(5.01, "  Wide shot  ", "cut to the wide shot here", None).unwrap();
    let start = s.add_marker(0.0, "Intro", "", Some("#ff6b5e")).unwrap();
    let list = markers(&s);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, start, "in time order");
    let w = list.iter().find(|x| x.id == m).unwrap();
    assert!(near(w.time, 5.0) && w.name == "Wide shot" && w.notes == "cut to the wide shot here" && w.colour == clipcut_core::MARKER_COLOUR, "{w:?}");
    assert_eq!(s.marker_clip(m), Some(b));

    // changed in one step; typing in its notes is one undo step
    s.edit_marker(m, &json!({"notes": "cut"}), Some("notes:1")).unwrap();
    s.edit_marker(m, &json!({"notes": "cut to the close-up"}), Some("notes:1")).unwrap();
    s.edit_marker(m, &json!({"name": "Close-up", "colour": "#46c99a"}), None).unwrap();
    let w = markers(&s).into_iter().find(|x| x.id == m).unwrap();
    assert!(w.name == "Close-up" && w.notes == "cut to the close-up" && w.colour == "#46c99a", "{w:?}");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(markers(&s).into_iter().find(|x| x.id == m).unwrap().notes, "cut to the wide shot here", "the typing was one step");

    // the first clip deleted: the main track closes up and the marker stays on its footage
    s.delete(&[a]).unwrap();
    let w = markers(&s).into_iter().find(|x| x.id == m).unwrap();
    assert!(near(w.time, 1.0), "it moved with its footage: {w:?}");
    assert_eq!(s.marker_clip(m), Some(b));
    // the start marker's footage is gone: it stays where it was, notes and all
    assert!(markers(&s).iter().any(|x| x.id == start && near(x.time, 0.0)));
    s.undo().unwrap();
    assert!(near(markers(&s).into_iter().find(|x| x.id == m).unwrap().time, 5.0), "undone with the edit");

    // a split before it changes nothing for it; moving it puts it elsewhere
    s.split(4.5, &[]).unwrap();
    assert!(near(markers(&s).into_iter().find(|x| x.id == m).unwrap().time, 5.0));
    s.edit_marker(m, &json!({"seconds": 2.0}), None).unwrap();
    assert!(near(markers(&s).into_iter().find(|x| x.id == m).unwrap().time, 2.0));

    // chapters: named markers, with what YouTube would refuse
    let c = s.chapters().unwrap();
    assert_eq!(c.text, "0:00 Intro\n0:02 Wide shot");
    assert!(c.problems.iter().any(|p| p.contains("three")), "{:?}", c.problems);
    assert!(c.problems.iter().any(|p| p.contains("10 seconds")), "{:?}", c.problems);

    // taken away in one step
    s.delete_marker(m).unwrap();
    assert_eq!(markers(&s).len(), 1);
    s.undo().unwrap();
    assert_eq!(markers(&s).len(), 2);

    // refused in plain words
    assert!(s.add_marker(f64::NAN, "x", "", None).is_err());
    assert!(s.add_marker(1.0, "x", "", Some("teal")).is_err());
    assert!(s.edit_marker(987_654, &json!({"name": "x"}), None).is_err());
    assert!(s.edit_marker(m, &json!({"name": 5}), None).is_err());
    assert!(s.delete_marker(987_654).is_err());
    // long words are cut short, not refused
    let long = s.add_marker(1.0, &"n".repeat(1000), &"x".repeat(50_000), None).unwrap();
    let l = markers(&s).into_iter().find(|x| x.id == long).unwrap();
    assert!(l.name.len() == 200 && l.notes.len() == 10_000);
}
