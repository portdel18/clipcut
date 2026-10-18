//! Text animations: fades, slides, pops and typewriters in and out, measured on the rendered
//! frames; trimming keeps them on the clip's ends, editing the text rebuilds them, and what is
//! refused.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{Studio, TextAnimations};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// A 4-second 640 × 360 black clip with a tone.
fn black() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core animations")?;
    let out = fixtures_dir("clipcut-core").join("anim-black-420.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=black:size=640x360:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// A studio with white text "HELLO WORLD" over black from 0 to 3 s.
fn studio() -> Option<(Studio, u64)> {
    let mut s = Studio::new();
    s.add_clip(black()?.to_str()?).unwrap();
    let t = s.add_text(0.0, 3.0, "HELLO WORLD", &json!({"size": 60, "color": "#ffffff"})).unwrap();
    Some((s, t))
}

/// The text as rendered at `t`: how much light, its bounding box (x0, y0, x1, y1) and its centre.
#[derive(Debug)]
struct Seen {
    light: f64,
    bbox: Option<[usize; 4]>,
}
impl Seen {
    fn centre(&self) -> (f64, f64) {
        let b = self.bbox.unwrap();
        ((b[0] + b[2]) as f64 / 2.0, (b[1] + b[3]) as f64 / 2.0)
    }
    fn width(&self) -> usize {
        self.bbox.map_or(0, |b| b[2] - b[0])
    }
}
fn seen(s: &Studio, t: f64) -> Seen {
    let f = s.frame(t, 640).unwrap();
    let (w, mut light, mut bbox) = (f.w as usize, 0.0, None::<[usize; 4]>);
    for (k, p) in f.rgba.chunks(4).enumerate() {
        let v = f64::from(p[0]);
        light += v;
        if v > 100.0 {
            let (x, y) = (k % w, k / w);
            bbox = Some(match bbox {
                None => [x, y, x, y],
                Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
            });
        }
    }
    Seen { light, bbox }
}

fn anims(s: &Studio, id: u64) -> TextAnimations {
    s.timeline().clips.into_iter().find(|c| c.id == id).unwrap().props.text.unwrap().animations
}

#[test]
fn fades_rises_and_pops_in_and_out() {
    let Some((mut s, t)) = studio() else { return };
    let plain = seen(&s, 1.5);
    assert!(plain.light > 0.0 && seen(&s, 0.0).light > plain.light * 0.9, "no animation: the text is there from the start");

    // fade in over half a second
    s.set_text_animation(&[t], true, Some("fade"), 0.5, None).unwrap();
    let a = anims(&s, t);
    assert_eq!(a.enter.as_ref().map(|v| (v.id.as_str(), v.seconds)), Some(("fade", 0.5)));
    assert!(a.leave.is_none());
    let (l0, l1, l2) = (seen(&s, 0.0).light, seen(&s, 0.25).light, seen(&s, 1.0).light);
    assert!(l0 < plain.light * 0.05 && l1 > l0 && l1 < l2 && (l2 - plain.light).abs() < plain.light * 0.02, "{l0} {l1} {l2} {}", plain.light);

    // rise: comes up from below
    s.set_text_animation(&[t], true, Some("rise"), 0.5, None).unwrap();
    let (y_early, y_plain) = (seen(&s, 0.2).centre().1, seen(&s, 1.0).centre().1);
    assert!(y_early > y_plain + 3.0, "{y_early} {y_plain}");
    assert!((seen(&s, 1.0).centre().0 - plain.centre().0).abs() < 1.0, "it ends where it was");

    // pop: grows from its middle, overshooting a little
    s.set_text_animation(&[t], true, Some("pop"), 0.5, None).unwrap();
    let small = seen(&s, 0.1);
    let over = seen(&s, 0.35);
    let after = seen(&s, 1.0);
    assert!(small.width() < plain.width() / 2, "{small:?} {plain:?}");
    assert!(over.width() > plain.width(), "{over:?} {plain:?}");
    let (a, b) = (small.centre(), plain.centre());
    assert!((a.0 - b.0).abs() < 4.0 && (a.1 - b.1).abs() < 4.0, "from the middle: {a:?} {b:?}");
    assert!((after.centre().0 - b.0).abs() < 1.0 && (after.centre().1 - b.1).abs() < 1.0 && after.width().abs_diff(plain.width()) <= 1);

    // fade out: gone on the last frame, there just before the outro
    s.set_text_animation(&[t], false, Some("fade"), 0.5, None).unwrap();
    let a = anims(&s, t);
    assert_eq!((a.enter.unwrap().id, a.leave.unwrap().id), ("pop".to_string(), "fade".to_string()));
    let last = 3.0 - 1.0 / 30.0;
    assert!(seen(&s, last).light < plain.light * 0.05, "{}", seen(&s, last).light);
    assert!((seen(&s, 2.4).light - plain.light).abs() < plain.light * 0.02);

    // each change was one undo step
    s.undo().unwrap();
    assert!(anims(&s, t).leave.is_none());
    s.undo().unwrap();
    assert_eq!(anims(&s, t).enter.unwrap().id, "rise");

    // none: the text is plain again
    s.set_text_animation(&[t], true, None, 0.5, None).unwrap();
    assert_eq!(anims(&s, t), TextAnimations::default());
    let back = seen(&s, 0.0);
    assert!((back.light - plain.light).abs() < plain.light * 0.02 && back.bbox == plain.bbox, "{back:?} {plain:?}");
}

#[test]
fn typewriters_follow_the_text_and_trims_keep_the_ends() {
    let Some((mut s, t)) = studio() else { return };
    let plain = seen(&s, 1.5);
    s.set_text_animation(&[t], true, Some("type"), 1.0, None).unwrap();
    assert!(seen(&s, 0.0).bbox.is_none(), "nothing typed yet");
    let half = seen(&s, 0.5).width();
    assert!(half > plain.width() / 4 && half < plain.width() * 3 / 4, "{half} of {}", plain.width());
    assert_eq!(seen(&s, 1.2).bbox, plain.bbox);
    // word by word: the first word, then both
    s.set_text_animation(&[t], true, Some("words"), 1.0, None).unwrap();
    let one = seen(&s, 0.6).width();
    assert!(one > plain.width() / 3 && one < plain.width() * 2 / 3, "{one} of {}", plain.width());

    // new words: the animation types them instead
    s.set_props(&[t], &json!({"text": "HI"}), None).unwrap();
    let short = seen(&s, 1.5);
    assert!(short.width() < plain.width() / 2);
    assert_eq!(seen(&s, 1.2).bbox, short.bbox, "typed out by the end of the animation");
    assert_eq!(s.timeline().clips.into_iter().find(|c| c.id == t).unwrap().props.text.unwrap().text, "HI");
    s.set_props(&[t], &json!({"text": "HELLO WORLD"}), None).unwrap();

    // a fade out, then the clip ends a second earlier: it still fades out at its new end
    s.set_text_animation(&[t], false, Some("fade"), 0.5, None).unwrap();
    s.trim(t, false, -1.0).unwrap();
    let last = 2.0 - 1.0 / 30.0;
    assert!(seen(&s, last).light < plain.light * 0.05, "{}", seen(&s, last).light);
    assert!((seen(&s, 1.4).light - plain.light).abs() < plain.light * 0.02, "{}", seen(&s, 1.4).light);
    assert!(seen(&s, 0.0).bbox.is_none(), "the way in is still there");

    // what is refused
    assert!(s.set_text_animation(&[t], true, Some("spin"), 0.5, None).is_err());
    assert!(s.set_text_animation(&[t], false, Some("pop"), 0.5, None).is_err(), "pop is a way in");
    assert!(s.set_text_animation(&[t], true, Some("fade"), f64::NAN, None).is_err());
    let video = s.timeline().clips.into_iter().find(|c| c.track == "V1").unwrap().id;
    assert!(s.set_text_animation(&[video], true, Some("fade"), 0.5, None).is_err());
}
