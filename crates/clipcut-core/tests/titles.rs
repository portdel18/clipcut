//! Titles: the engine's templates added at the playhead and filled in through their controls, and
//! a text clip saved as a title of one's own (its animation kept), used again and removed.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{MY_TITLES, Studio};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

/// A 4-second 1280 × 720 black clip with a tone.
fn black() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core titles")?;
    let out = fixtures_dir("clipcut-core").join("titles-black-720.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=black:size=1280x720:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn light(s: &Studio, t: f64) -> f64 {
    let f = s.frame(t, 320).unwrap();
    f.rgba.chunks(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum()
}

fn control(s: &mut Studio, clip: u64, id: &str) -> serde_json::Value {
    let c = s.title_controls(clip).unwrap();
    c.as_array().unwrap().iter().find(|x| x["id"] == id).unwrap_or_else(|| panic!("{id} in {c}"))["value"].clone()
}

#[test]
fn templates_are_added_and_filled_in() {
    let Some(v) = black() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let all = s.titles().unwrap();
    assert!(all.len() >= 8 && all.iter().any(|t| t.category == "Lower Thirds") && all.iter().all(|t| !t.mine), "{all:?}");
    let slab = all.iter().find(|t| t.id.contains("lower-third-slab")).unwrap().clone();
    assert!(s.title_thumbnail(&slab.id, 160).unwrap().len() > 100);

    let dark = light(&s, 1.5);
    let clip = s.add_title(&slab.id, 1.0).unwrap();
    let c = s.timeline().clips.into_iter().find(|c| c.id == clip).unwrap();
    assert_eq!((c.track.as_str(), c.title.as_deref()), ("V2", Some(slab.name.as_str())));
    assert!((c.start - 1.0).abs() < 1e-6 && (c.end - c.start - slab.seconds).abs() < 0.05, "{c:?}");
    assert!(light(&s, 1.5) > dark + 1000.0, "the title shows");

    // its own controls
    assert_eq!(control(&mut s, clip, "name"), json!("Alex Morgan"));
    s.set_title(clip, &json!({"name": "Jo Smith"}), None).unwrap();
    assert_eq!(control(&mut s, clip, "name"), json!("Jo Smith"));
    s.undo().unwrap();
    assert_eq!(control(&mut s, clip, "name"), json!("Alex Morgan"));
    s.undo().unwrap();
    assert!(s.timeline().clips.iter().all(|c| c.id != clip), "adding it was one undo step");

    // what is refused
    let clip = s.add_title(&slab.id, 1.0).unwrap();
    assert!(s.set_text_animation(&[clip], true, Some("fade"), 0.5, None).is_err(), "a title has its own motion");
    assert!(s.set_title(clip, &json!({}), None).is_err());
    assert!(s.add_title("no-such-title", 0.0).is_err());
    assert!(s.remove_title(&slab.id).is_err(), "the engine's own stay");
    assert!(s.save_title(clip, "Mine").is_err(), "nowhere to keep it yet");
}

#[test]
fn a_text_saved_as_a_title_comes_back_with_its_animation() {
    let Some(v) = black() else { return };
    let dir = fixtures_dir("clipcut-core").join(format!("titles-data-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Studio::new();
    s.set_data_dir(&dir);
    s.add_clip(v.to_str().unwrap()).unwrap();
    let text = s.add_text(0.0, 2.0, "SALE TODAY", &json!({"size": 90, "color": "#ffffff"})).unwrap();
    s.set_text_animation(&[text], true, Some("fade"), 0.5, None).unwrap();
    let plain = light(&s, 1.0);
    assert!(light(&s, 0.0) < plain * 0.1);
    assert!(s.save_title(text, "   ").is_err());

    let id = s.save_title(text, "Sale fade").unwrap();
    let mine = s.titles().unwrap().into_iter().find(|t| t.id == id).unwrap();
    assert!(mine.mine && mine.name == "Sale fade" && mine.category == MY_TITLES, "{mine:?}");
    assert!((mine.seconds - 2.0).abs() < 0.05);

    // used again, later on: it fades in the same way
    s.set_props(&[text], &json!({"opacity": 0.0}), None).unwrap();
    let again = s.add_title(&id, 2.0).unwrap();
    assert!(light(&s, 2.0) < plain * 0.1, "{} vs {plain}", light(&s, 2.0));
    assert!((light(&s, 3.0) - plain).abs() < plain * 0.05, "{} vs {plain}", light(&s, 3.0));
    let first = s.title_controls(again).unwrap()[0]["id"].as_str().unwrap().to_string();
    assert_eq!(control(&mut s, again, &first), json!("SALE TODAY"), "its words are a control");

    s.remove_title(&id).unwrap();
    assert!(s.titles().unwrap().iter().all(|t| t.id != id));
    let _ = std::fs::remove_dir_all(&dir);
}
