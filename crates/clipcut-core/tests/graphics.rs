//! Graphics from the Graphics tab: a folder of numbered frames (with transparency), maybe a
//! sound, and graphic.json; added on an overlay lane, updated in place, read back to edit.

use std::path::{Path, PathBuf};
use std::process::Command;

use clipcut_core::{ClipView, GRAPHIC_FILE, GRAPHIC_SOUND, Studio};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 2-second blue 320 × 180 video with a tone.
fn video() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core graphics")?;
    let out = fixtures_dir("clipcut-core").join("gr-blue.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=blue:size=320x180:rate=30:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// A graphic folder: `frames` 320 × 180 PNG frames, transparent but for a `colour` box in the
/// middle, its graphic.json, and a second of sound when `sound`.
fn graphic(name: &str, colour: &str, frames: u32, sound: bool) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core graphics")?;
    let dir = fixtures_dir("clipcut-core").join(format!("graphic-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let ok = Command::new(&ff)
        // opaque in the box, transparent elsewhere (drawbox would keep the transparency it draws over)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!(
                "color=c={colour}:size=320x180:rate=30,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(between(X,110,209)*between(Y,60,119),255,0)'"
            ),
        ])
        .args(["-frames:v", &frames.to_string(), "-start_number", "0"])
        .arg(dir.join("frame_%05d.png"))
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return None;
    }
    if sound {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=880:duration=1:sample_rate=48000"])
            .arg(dir.join(GRAPHIC_SOUND))
            .status()
            .ok()?;
    }
    let info = serde_json::json!({"tool": "lt", "state": {"line1": format!("Jordan Ellis ({colour})")}, "fps": 30, "frames": frames, "ext": "png"});
    std::fs::write(dir.join(GRAPHIC_FILE), info.to_string()).ok()?;
    Some(dir)
}

fn first(dir: &Path) -> String {
    dir.join("frame_00000.png").to_string_lossy().to_string()
}

fn clip(s: &Studio, id: u64) -> ClipView {
    s.timeline().clips.into_iter().find(|c| c.id == id).unwrap()
}

/// The program's colour at pixel (x, y) of a 320-wide frame at `t`.
fn pixel(s: &Studio, t: f64, x: usize, y: usize) -> [u8; 3] {
    let f = s.frame(t, 320).unwrap();
    let k = (y * f.w as usize + x) * 4;
    [f.rgba[k], f.rgba[k + 1], f.rgba[k + 2]]
}

#[test]
fn a_graphic_goes_over_the_video_and_is_updated_in_place() {
    let (Some(v), Some(g1), Some(g2)) = (video(), graphic("red", "red", 30, false), graphic("green", "lime", 45, true)) else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let made = s.add_graphic(&first(&g1), 30.0, 0.5, None).unwrap();
    assert_eq!(made.len(), 1, "a picture, no sound: {made:?}");
    let c = clip(&s, made[0]);
    assert_eq!(c.track, "V2");
    assert_eq!(c.graphic.as_deref(), Some("lt"));
    assert!(c.name.starts_with("graphic-red-"), "named after its folder: {}", c.name);
    assert!((c.start - 0.5).abs() < 1e-6 && (c.end - c.start - 1.0).abs() < 1e-6, "30 frames at 30 fps from 0.5 s: {c:?}");
    // the video shows through where the graphic is transparent
    let out = pixel(&s, 1.0, 20, 20);
    assert!(out[2] > 150 && out[0] < 60, "{out:?}");
    let box_ = pixel(&s, 1.0, 160, 90);
    assert!(box_[0] > 150 && box_[2] < 60, "{box_:?}");
    // its settings come back to open it again
    assert_eq!(s.graphic_of(made[0]).unwrap()["state"]["line1"], "Jordan Ellis (red)");
    assert!(s.graphic_of(main_clip(&s)).is_err(), "a camera clip isn't a graphic");

    // moved and faded, then updated: the new version keeps its place and look, and brings its sound
    s.set_props(&made, &serde_json::json!({"x": 100.0, "opacity": 50.0}), None).unwrap();
    let before = clip(&s, made[0]);
    let updated = s.add_graphic(&first(&g2), 30.0, 0.0, Some(made[0])).unwrap();
    assert_eq!(updated.len(), 2, "picture and sound: {updated:?}");
    let v = s.timeline();
    assert!(v.clips.iter().all(|c| c.id != made[0]), "the old version is gone");
    let n = clip(&s, updated[0]);
    assert_eq!((n.track.as_str(), n.start), ("V2", before.start));
    assert!((n.end - n.start - 1.5).abs() < 1e-6, "{n:?}");
    assert_eq!((n.props.x, n.props.opacity), (before.props.x, before.props.opacity));
    let snd = clip(&s, updated[1]);
    assert!(snd.track.starts_with('A') && snd.track != "A1" && (snd.start - n.start).abs() < 1e-6, "{snd:?}");
    assert_eq!(s.graphic_of(updated[0]).unwrap()["state"]["line1"], "Jordan Ellis (lime)");

    // updating that one again takes its sound with it
    let again = s.add_graphic(&first(&g1), 30.0, 0.0, Some(updated[0])).unwrap();
    let v = s.timeline();
    assert!(v.clips.iter().all(|c| c.id != updated[0] && c.id != updated[1]), "{:#?}", v.clips);
    assert_eq!(again.len(), 1);

    // each is one undo step
    s.undo().unwrap();
    assert!(s.timeline().clips.iter().any(|c| c.id == updated[0]));
    s.undo().unwrap();
    assert!(s.timeline().clips.iter().any(|c| c.id == made[0]));
    for d in [g1, g2] {
        let _ = std::fs::remove_dir_all(d);
    }
}

fn main_clip(s: &Studio) -> u64 {
    s.timeline().clips.iter().find(|c| c.track == "V1").unwrap().id
}

#[test]
fn a_graphic_fills_the_video_whatever_size_it_was_rendered_at() {
    let (Some(v), Some(g)) = (video(), graphic("big", "red", 10, false)) else { return };
    // the same graphic at twice the video's size (Clip Studio renders 1080 on the short side)
    let Some(ff) = ffmpeg_or_skip("clipcut-core graphics") else { return };
    let big = g.with_file_name(format!("{}-2x", g.file_name().unwrap().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&big);
    std::fs::create_dir_all(&big).unwrap();
    assert!(
        Command::new(&ff)
            .args(["-v", "error", "-y", "-start_number", "0", "-i"])
            .arg(g.join("frame_%05d.png"))
            .args(["-vf", "scale=640:360", "-start_number", "0"])
            .arg(big.join("frame_%05d.png"))
            .status()
            .unwrap()
            .success()
    );
    std::fs::copy(g.join(GRAPHIC_FILE), big.join(GRAPHIC_FILE)).unwrap();
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let made = s.add_graphic(&first(&big), 30.0, 0.0, None).unwrap();
    let c = clip(&s, made[0]);
    assert!((c.props.scale.unwrap() - 50.0).abs() < 0.5, "fitted to the 320-wide video: {:?}", c.props.scale);
    // the box sits in the middle of the frame, as it did in the graphic
    let px = pixel(&s, 0.1, 160, 90);
    assert!(px[0] > 150 && px[2] < 60, "{px:?}");
    let px = pixel(&s, 0.1, 20, 20);
    assert!(px[2] > 150, "{px:?}");
    // updated with one of another size: fitted again, keeping its opacity, not its old scale
    s.set_props(&made, &serde_json::json!({"opacity": 60.0}), None).unwrap();
    let small = s.add_graphic(&first(&g), 30.0, 0.0, Some(made[0])).unwrap();
    let n = clip(&s, small[0]);
    assert!((n.props.scale.unwrap() - 100.0).abs() < 0.5, "{:?}", n.props.scale);
    assert_eq!(n.props.opacity, Some(60.0));
    for d in [g, big] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_graphic_can_start_a_video_and_says_what_is_wrong() {
    let Some(g) = graphic("first", "yellow", 12, false) else { return };
    let mut s = Studio::new();
    let made = s.add_graphic(&first(&g), 24.0, 0.0, None).unwrap();
    let v = s.timeline();
    assert!(v.has_sequence);
    assert_eq!(v.clips.iter().filter(|c| c.track == "V1").count(), 1);
    assert!((v.duration - 0.5).abs() < 1e-6, "12 frames at 24 fps: {}", v.duration);
    assert_eq!(clip(&s, made[0]).graphic.as_deref(), Some("lt"));
    // frame rates an image sequence can't have
    assert!(s.add_graphic(&first(&g), 17.0, 0.0, None).is_err());
    // a folder with no frames
    assert!(s.add_graphic(&g.join("frame_99999.png").to_string_lossy(), 30.0, 0.0, None).is_err());
    let _ = std::fs::remove_dir_all(g);
}
