//! Background fill (Canvas › Blur): a vertical clip in a horizontal video shows its own picture,
//! zoomed in, blurred and darkened, in the space beside it.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

fn clip(name: &str, colour: &str, size: &str) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core fill")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("color=c={colour}:size={size}:rate=30:duration=2")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn pixel(s: &Studio, t: f64, x: usize, y: usize) -> [u8; 3] {
    let f = s.frame(t, 320).unwrap();
    let k = (y * f.w as usize + x) * 4;
    [f.rgba[k], f.rgba[k + 1], f.rgba[k + 2]]
}

#[test]
fn a_vertical_clip_fills_the_sides_with_itself_blurred() {
    let (Some(wide), Some(tall)) = (clip("fill-wide-blue", "blue", "320x180"), clip("fill-tall-red", "red", "180x320")) else { return };
    let mut s = Studio::new();
    s.add_clip(wide.to_str().unwrap()).unwrap();
    s.add_clip(tall.to_str().unwrap()).unwrap();
    let v = s.timeline();
    let red = v.clips.iter().find(|c| c.track == "V1" && c.start > 1.0).unwrap().clone();
    let at = red.start + 0.5;
    // fitted: black beside it
    assert!(pixel(&s, at, 10, 90).iter().all(|c| *c < 20), "{:?}", pixel(&s, at, 10, 90));
    s.set_fill(&[red.id], Some((60.0, 25.0, 100.0, 10.0)), None).unwrap();
    let v = s.timeline();
    assert_eq!(v.undo_label.as_deref(), Some("Background Fill"));
    let f = v.clips.iter().find(|c| c.id == red.id).unwrap().props.fill.clone().unwrap();
    assert_eq!((f.blur, f.darken, f.zoom, f.rounding), (60.0, 25.0, 100.0, 10.0));
    // the sides: the red, a little darker; the clip itself unchanged
    let side = pixel(&s, at, 10, 90);
    assert!(side[0] > 120 && side[1] < 40 && side[2] < 40, "{side:?}");
    let mid = pixel(&s, at, 160, 90);
    assert!(mid[0] > 230, "{mid:?}");
    // the clip before it, which fills the frame, has none
    assert!(v.clips.iter().find(|c| c.track == "V1" && c.start < 1.0).unwrap().props.fill.is_none());
    // a dragged slider is one undo step; off again; undo brings it back
    s.set_fill(&[red.id], Some((80.0, 25.0, 100.0, 10.0)), Some("blur")).unwrap();
    s.set_fill(&[red.id], Some((90.0, 25.0, 100.0, 10.0)), Some("blur")).unwrap();
    s.set_fill(&[red.id], None, None).unwrap();
    assert!(pixel(&s, at, 10, 90).iter().all(|c| *c < 20));
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.iter().find(|c| c.id == red.id).unwrap().props.fill.as_ref().map(|f| f.blur), Some(90.0));
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.iter().find(|c| c.id == red.id).unwrap().props.fill.as_ref().map(|f| f.blur), Some(60.0));
    // hostile numbers are refused, sound or text isn't a picture
    assert!(s.set_fill(&[red.id], Some((f64::NAN, 0.0, 100.0, 0.0)), None).is_err());
    assert!(s.set_fill(&[], Some((60.0, 25.0, 100.0, 0.0)), None).is_err());
}
