//! Split at scene changes: a clip of three shots is cut where each new shot starts, with its sound,
//! in one undo step.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// 6 s at 30 fps: red for 2 s, a moving test pattern for 2 s, blue for 2 s, with a tone.
fn three_shots() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core scenes")?;
    let out = fixtures_dir("clipcut-core").join("three-shots.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y"])
            .args(["-f", "lavfi", "-i", "color=c=red:size=320x180:rate=30:duration=2"])
            .args(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2"])
            .args(["-f", "lavfi", "-i", "color=c=blue:size=320x180:rate=30:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000"])
            .args(["-filter_complex", "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]", "-map", "[v]", "-map", "3:a"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn a_clip_is_cut_where_each_new_shot_starts() {
    let Some(v) = three_shots() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let tl = s.timeline();
    let id = tl.clips.iter().find(|c| c.track == "V1").unwrap().id;
    let scan = s.scene_scan(id).unwrap();
    assert_eq!(scan.frames(), 180);
    let mut seen = 0.0;
    let cuts = scan
        .run(&mut |f| {
            assert!(f >= seen, "progress only goes forward");
            seen = f;
            true
        })
        .unwrap();
    assert_eq!(seen, 1.0);
    // at 2 s and 4 s (within a frame)
    let tick = |secs: f64| (secs * 254_016_000_000.0) as i64;
    let frame = tick(1.0 / 30.0);
    assert_eq!(cuts.len(), 2, "{cuts:?}");
    assert!((cuts[0] - tick(2.0)).abs() <= frame && (cuts[1] - tick(4.0)).abs() <= frame, "{cuts:?}");

    // cut: three pieces of picture and three of sound, back to back, one undo step
    let pieces = |s: &Studio, track: &str| {
        let mut v: Vec<(f64, f64)> = s.timeline().clips.iter().filter(|c| c.track == track).map(|c| (c.start, c.end)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    };
    assert_eq!(s.split_at_scenes(id, &cuts).unwrap(), 2);
    let v1 = pieces(&s, "V1");
    assert_eq!(v1.len(), 3, "{v1:?}");
    assert!((v1[1].0 - 2.0).abs() < 0.04 && (v1[2].0 - 4.0).abs() < 0.04 && (v1[2].1 - 6.0).abs() < 0.04, "{v1:?}");
    assert_eq!(pieces(&s, "A1").len(), 3, "the sound is cut with it");
    s.undo().unwrap();
    assert_eq!(pieces(&s, "V1").len(), 1, "one undo puts it back");

    // cuts outside the clip, or off it after a trim, are left out; a stop changes nothing
    assert_eq!(s.split_at_scenes(id, &[-5, tick(99.0)]).unwrap(), 0);
    assert!(scan.run(&mut |_| false).is_err());
    assert_eq!(pieces(&s, "V1").len(), 1);
    // not a video clip
    let snd = s.timeline().clips.iter().find(|c| c.track == "A1").unwrap().id;
    assert!(s.scene_scan(snd).is_err());
    assert!(s.scene_scan(987_654).is_err());
}
