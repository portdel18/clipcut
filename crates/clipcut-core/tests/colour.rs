//! Colour: adjustments and filters on the engine's Lumetri Color, reset, copying a clip's colour to
//! the main track, the Filters tab's previews, and the scopes of the program frame.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{SCOPES, Studio, TimelineView, looks};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// A 1-second 320 × 180 clip of one colour (`0xRRGGBB`), with a tone.
fn clip(name: &str, colour: &str) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core colour")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}-420.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("color=c={colour}:size=320x180:rate=30:duration=1")])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
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

/// The program's average colour at `t`.
fn rgb(s: &Studio, t: f64) -> [f64; 3] {
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

fn one(colour: &str) -> Option<(Studio, u64)> {
    let mut s = Studio::new();
    s.add_clip(clip(&format!("col-{}", colour.trim_start_matches("0x")), colour)?.to_str()?).unwrap();
    let id = main(&s.timeline())[0];
    Some((s, id))
}

fn colour(s: &Studio, id: u64) -> Option<clipcut_core::ColourView> {
    s.timeline().clips.into_iter().find(|c| c.id == id)?.props.colour
}

#[test]
fn adjustments_change_the_picture_and_reset_takes_them_off() {
    let Some((mut s, id)) = one("0x808080") else { return };
    assert!(colour(&s, id).is_none(), "untouched clips have no colour yet");
    let grey = rgb(&s, 0.5);
    // brighter
    s.set_colour(&[id], &json!({"exposure": 1.0}), None).unwrap();
    let c = colour(&s, id).unwrap();
    assert_eq!((c.values["exposure"], c.values["saturation"], c.look.as_deref()), (1.0, 100.0, None));
    let bright = rgb(&s, 0.5);
    assert!(bright[0] > grey[0] + 25.0, "{grey:?} → {bright:?}");
    // warmer
    s.set_colour(&[id], &json!({"exposure": 0.0, "temperature": 60.0}), None).unwrap();
    let warm = rgb(&s, 0.5);
    assert!(warm[0] > warm[2] + 10.0, "{warm:?}");
    // a dragged slider is one undo step
    s.set_colour(&[id], &json!({"contrast": 10.0}), Some("drag")).unwrap();
    s.set_colour(&[id], &json!({"contrast": 40.0}), Some("drag")).unwrap();
    s.undo().unwrap();
    assert_eq!(colour(&s, id).unwrap().values["contrast"], 0.0);
    // reset: no colour, the picture as it was
    s.reset_colour(&[id]).unwrap();
    assert!(colour(&s, id).is_none());
    let back = rgb(&s, 0.5);
    assert!((0..3).all(|k| (back[k] - grey[k]).abs() < 2.0), "{grey:?} vs {back:?}");
    // what is refused
    assert!(s.set_colour(&[id], &json!({"glow": 3.0}), None).is_err());
    assert!(s.set_colour(&[id], &json!({"exposure": "lots"}), None).is_err());
    let sound = s.timeline().clips.iter().find(|c| c.track == "A1").unwrap().id;
    assert!(s.set_colour(&[sound], &json!({"exposure": 1.0}), None).is_err());
}

#[test]
fn saturation_and_filters_and_copying_to_every_clip() {
    let Some((mut s, id)) = one("0xd03020") else { return };
    s.set_colour(&[id], &json!({"saturation": 0.0}), None).unwrap();
    let g = rgb(&s, 0.5);
    assert!((g[0] - g[2]).abs() < 6.0 && (g[0] - g[1]).abs() < 6.0, "no colour left: {g:?}");
    s.set_colour(&[id], &json!({"saturation": 100.0}), None).unwrap();
    let red = rgb(&s, 0.5);

    // filters: the engine's own looks
    let all = looks();
    assert!(all.len() >= 5 && all.contains(&"Monochrome"), "{all:?}");
    s.set_look(&[id], Some("Monochrome"), 100.0, None).unwrap();
    assert_eq!(colour(&s, id).unwrap().look.as_deref(), Some("Monochrome"));
    let mono = rgb(&s, 0.5);
    assert!((mono[0] - mono[2]).abs() < 12.0, "{mono:?}");
    // at no strength, the picture is as without
    s.set_look(&[id], Some("Monochrome"), 0.0, None).unwrap();
    let none = rgb(&s, 0.5);
    assert!((none[0] - red[0]).abs() < 4.0, "{red:?} vs {none:?}");
    s.set_look(&[id], None, 100.0, None).unwrap();
    assert_eq!(colour(&s, id).unwrap().look, None);
    assert!(s.set_look(&[id], Some("Not A Look"), 100.0, None).is_err());

    // previews: the clip's own picture, then one per look
    let p = s.look_previews(id, 0.5, 96).unwrap();
    assert_eq!(p.len(), all.len() + 1);
    let differ = p.iter().skip(1).filter(|f| f.rgba != p[0].rgba).count();
    assert!(differ >= all.len() - 1, "{differ} of {} looks change the picture", all.len());

    // copied to every clip on the main track
    s.add_clip(clip("col-2060d0", "0x2060d0").unwrap().to_str().unwrap()).unwrap();
    s.set_colour(&[id], &json!({"exposure": 0.5, "vignette": -1.0}), None).unwrap();
    assert_eq!(s.colour_everywhere(id).unwrap(), 1);
    let other = main(&s.timeline())[1];
    let c = colour(&s, other).unwrap();
    assert_eq!((c.values["exposure"], c.values["vignette"]), (0.5, -1.0));
    s.undo().unwrap();
    assert!(colour(&s, other).is_none(), "one undo step");
}

#[test]
fn scopes_read_the_program_frame() {
    let (Some((white, _)), Some((black, _)), Some((red, _))) = (one("0xffffff"), one("0x000000"), one("0xff0000")) else { return };
    for s in [&white, &black, &red] {
        for k in SCOPES {
            let f = s.scope(0.5, k).unwrap();
            assert!(f.w > 32 && f.h > 32 && f.rgba.len() == (f.w * f.h * 4) as usize, "{k}");
        }
    }
    // the waveform's trace sits high for white, low for black (a bright, greenish trace row)
    let row_of = |s: &Studio| {
        let f = s.scope(0.5, "waveform").unwrap();
        let (w, h) = (f.w as usize, f.h as usize);
        (0..h).max_by_key(|y| (0..w).map(|x| u32::from(f.rgba[(y * w + x) * 4 + 1])).sum::<u32>()).unwrap() as f64 / h as f64
    };
    assert!(row_of(&white) < 0.1, "{}", row_of(&white));
    assert!(row_of(&black) > 0.9, "{}", row_of(&black));
    // the histogram of red: red at the top end, no green there
    let f = red.scope(0.5, "histogram").unwrap();
    let (w, h) = (f.w as usize, f.h as usize);
    let at = |x: usize| &f.rgba[((h - 2) * w + x) * 4..((h - 2) * w + x) * 4 + 3];
    // (red decodes as 254 or 255: one of the top bins)
    let top = (w - 4..w).max_by_key(|x| at(*x)[0]).unwrap();
    assert!(at(top)[0] > at(top)[1] + 50, "{:?}", at(top));
    assert!(at(0)[1] > at(0)[0] + 50, "green and blue sit at the bottom end: {:?}", at(0));
    assert!(red.scope(0.5, "spectrum").is_err());
}
