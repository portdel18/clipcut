//! Never crash: every Studio edit and query, fed the numbers a broken interface, a hand-edited
//! project or a damaged file could send (NaN, infinities, huge, negative, zero, unknown ids),
//! returns an error or a sane result — never a panic. Damaged project files don't open, and the
//! project that was open stays usable.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::process::Command;

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use serde_json::json;

fn clip() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core hostile")?;
    let out = fixtures_dir("clipcut-core").join("hostile-2s.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2:sample_rate=48000",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

const NUMS: [f64; 10] = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0, 1e-12, 1e12, -1e12, f64::MAX, f64::MIN_POSITIVE];
const INTS: [i64; 6] = [i64::MIN, -1_000_000_000, -1, 0, 1_000_000_000, i64::MAX];

fn ids(s: &Studio) -> Vec<u64> {
    s.timeline().clips.iter().map(|c| c.id).collect()
}

/// Runs `f`, noting `name` when it panics.
fn guard(crashed: &mut Vec<String>, name: String, s: &mut Studio, f: impl FnOnce(&mut Studio)) {
    if catch_unwind(AssertUnwindSafe(|| f(s))).is_err() {
        crashed.push(name);
    }
}

#[test]
fn hostile_numbers_never_crash_an_edit() {
    let Some(path) = clip() else { return };
    let mut s = Studio::new();
    for _ in 0..3 {
        s.add_clip(path.to_str().unwrap()).unwrap();
    }
    s.add_text(0.5, 1.0, "Hello", &json!({})).unwrap();
    let mut crashed = Vec::new();
    let c = &mut crashed;
    for x in NUMS {
        let v = if x.is_finite() { json!(x) } else { json!(null) };
        guard(c, format!("split {x}"), &mut s, |s| {
            let _ = s.split(x, &[]);
        });
        guard(c, format!("frames {x}"), &mut s, |s| {
            let _ = s.frame(x, 64);
            let _ = s.preview_frame(x, 64, "auto");
            let _ = s.frame_without(x, 64, &[]);
            let _ = s.scope(x, "waveform");
            let _ = s.scope(x, "histogram");
        });
        guard(c, format!("add_text {x}"), &mut s, |s| {
            let _ = s.add_text(x, x, "x", &json!({}));
        });
        guard(c, format!("trim {x}"), &mut s, |s| {
            let k = ids(s)[0];
            let _ = s.trim(k, true, x);
            let _ = s.trim_to(k, false, x);
        });
        guard(c, format!("keyframe {x}"), &mut s, |s| {
            let k = ids(s)[0];
            let _ = s.toggle_keyframe(k, "opacity", x);
            let _ = s.props_at(k, x);
        });
        guard(c, format!("transition {x}"), &mut s, |s| {
            let k = ids(s)[0];
            let _ = s.set_transition(k, "cross-dissolve", Some(x));
            let _ = s.set_transition_length(k, x, None);
            let _ = s.transition_everywhere("cross-dissolve", x);
        });
        guard(c, format!("move_clip {x}"), &mut s, |s| {
            let k = *ids(s).last().unwrap();
            let _ = s.move_clip(k, "V2", x);
        });
        guard(c, format!("pauses {x}"), &mut s, |s| {
            let _ = s.remove_pauses(x, x);
        });
        guard(c, format!("set_props {x}"), &mut s, |s| {
            let k = ids(s);
            for key in ["speed", "opacity", "scale", "rotation", "x", "y", "volumeDb", "fadeIn", "fadeOut", "at"] {
                let _ = s.set_props(&k, &json!({ key: v }), None);
            }
            let _ = s.set_props(&k, &json!({"speed": v, "opacity": v, "scale": v, "x": v, "y": v, "fadeIn": v, "volumeDb": v}), Some("drag"));
        });
        guard(c, format!("colour {x}"), &mut s, |s| {
            let k = ids(s);
            let _ = s.set_colour(&k, &json!({"exposure": v, "contrast": v, "saturation": v, "temperature": v}), None);
            let _ = s.set_look(&k, Some("Night"), x, None);
        });
        guard(c, format!("mixer {x}"), &mut s, |s| {
            let _ = s.set_mixer("A1", &json!({"volume": v, "pan": v}), None);
            let k = ids(s);
            let _ = s.set_ducking(&k, true, x);
        });
        guard(c, format!("animation {x}"), &mut s, |s| {
            let k = ids(s);
            let _ = s.set_text_animation(&k, true, Some("pop"), x, None);
        });
        guard(c, format!("caption retime {x}"), &mut s, |s| {
            let _ = s.retime_caption(1, x, -x);
        });
        let _ = s.undo();
    }
    for n in INTS {
        guard(c, format!("nudge {n}"), &mut s, |s| {
            let k = ids(s);
            let _ = s.nudge(&k, n);
        });
        guard(c, format!("slip {n}"), &mut s, |s| {
            let k = ids(s)[0];
            let _ = s.slip(k, n);
        });
        guard(c, format!("roll {n}"), &mut s, |s| {
            let k = ids(s)[0];
            let _ = s.roll(k, n);
        });
        guard(c, format!("audio mix {n}"), &mut s, |s| {
            if let Ok(f) = s.audio_feed() {
                let _ = f.mix(n, 64);
            }
        });
        guard(c, format!("feed render {n}"), &mut s, |s| {
            if let Ok(f) = s.frame_feed(64, "auto") {
                let _ = f.render(n, true);
            }
        });
    }
    guard(c, "unknown ids".into(), &mut s, |s| {
        let bad = [0, u64::MAX, 987_654_321];
        let _ = s.delete(&bad);
        let _ = s.set_props(&bad, &json!({"opacity": 50}), None);
        let _ = s.reorder(u64::MAX, usize::MAX);
        let _ = s.insert_item(u64::MAX, usize::MAX);
        let _ = s.add_to_lane(u64::MAX, 0.0);
        let _ = s.start_with_sound(u64::MAX);
        let _ = s.peaks(u64::MAX);
        let _ = s.mark_beats(u64::MAX);
        let _ = s.toggle_beat(u64::MAX, 0.0);
        let _ = s.graphic_of(u64::MAX);
        let _ = s.title_controls(u64::MAX);
        let _ = s.check_picture(u64::MAX);
        let _ = s.colour_everywhere(u64::MAX);
        let _ = s.look_previews(u64::MAX, 0.0, 64);
        let _ = s.cut_words(usize::MAX, 0);
        let _ = s.cut_words(5, usize::MAX);
        let _ = s.frame_without(0.0, 64, &bad);
        let _ = s.make_proxies(&bad);
    });
    guard(c, "sizes".into(), &mut s, |s| {
        for (w, h) in [(0, 0), (1, 1), (u32::MAX, u32::MAX), (7, 100_000), (1_000_000, 2)] {
            let _ = s.set_canvas(w, h);
        }
        let _ = s.frame(0.5, 0);
        let _ = s.frame(0.5, u32::MAX);
        let _ = s.frame_feed(0, "nonsense");
        let _ = s.scope(0.5, "nonsense");
        let k = ids(s)[0];
        let _ = s.look_previews(k, 0.5, u32::MAX);
    });
    guard(c, "strings".into(), &mut s, |s| {
        let k = ids(s);
        let _ = s.set_props(&k, &json!({"text": {"text": "\u{0}".repeat(10_000), "size": 1e300, "font": "\u{202e}"}}), None);
        let _ = s.set_props(&k, &json!("not an object"), None);
        let _ = s.set_props(&k, &json!({"speed": "fast", "opacity": [1, 2], "scale": {"x": 1}}), None);
        let _ = s.set_caption_style(&json!({"size": -5, "colour": "#zzz"}));
        let _ = s.set_text_animation(&k, false, Some("no-such-animation"), 1.0, None);
        let _ = s.set_look(&k, Some("no such look"), 1.0, None);
        let _ = s.set_transition(k[0], "no-such-transition", None);
        let _ = s.export_as("/nonexistent/dir/out.mp4", "youtube");
        let _ = s.export_as("", "nonsense");
        let _ = s.add_title("../../etc/passwd", 0.0);
        let _ = s.hear("{not json");
        let _ = s.hear(&json!({"words": [{"text": "a", "start": -1e308, "end": f64::MAX}]}).to_string());
    });
    assert!(crashed.is_empty(), "these crashed: {crashed:?}");
    // and the project still works afterwards
    let _ = s.undo();
    assert!(s.frame(0.5, 64).is_ok());
}

#[test]
fn damaged_project_files_are_refused_and_the_open_project_survives() {
    let Some(path) = clip() else { return };
    let dir = fixtures_dir("clipcut-core").join(format!("hostile-projects-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let good = dir.join("good.fcproj");
    s.save_as(good.to_str().unwrap()).unwrap();
    let bytes = std::fs::read(&good).unwrap();
    let mut flipped = bytes.clone();
    for k in (bytes.len() / 3..bytes.len() * 2 / 3).step_by(97) {
        flipped[k] ^= 0x5a;
    }
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("zeros", vec![0; 4096]),
        ("noise", (0..20_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect()),
        ("half", bytes[..bytes.len() / 2].to_vec()),
        ("almost", bytes[..bytes.len().saturating_sub(1)].to_vec()),
        ("text", b"this is not a project".to_vec()),
        ("json", br#"{"version": 99999, "items": "lots", "sequences": [{"fps": -1, "width": 0}]}"#.to_vec()),
        ("flipped", flipped),
    ];
    let mut crashed = Vec::new();
    for (name, data) in cases {
        let p = dir.join(format!("{name}.fcproj"));
        std::fs::write(&p, &data).unwrap();
        let mut t = Studio::new();
        t.add_clip(path.to_str().unwrap()).unwrap();
        let r = catch_unwind(AssertUnwindSafe(|| {
            let opened = t.open_project(p.to_str().unwrap()).is_ok();
            // whether it opened or not, the studio still answers
            let _ = t.timeline();
            let _ = t.frame(0.5, 64);
            opened
        }));
        match r {
            Err(_) => crashed.push(name),
            Ok(true) => {}
            Ok(false) => assert!(t.timeline().has_sequence, "{name}: a project that won't open leaves the open one as it was"),
        }
    }
    assert!(crashed.is_empty(), "damaged projects crashed: {crashed:?}");
    // a project whose media has gone opens, with the offline slate in its place
    let moved = dir.join("moved.mp4");
    std::fs::copy(&path, &moved).unwrap();
    let mut u = Studio::new();
    u.add_clip(moved.to_str().unwrap()).unwrap();
    let p = dir.join("missing-media.fcproj");
    u.save_as(p.to_str().unwrap()).unwrap();
    std::fs::remove_file(&moved).unwrap();
    let mut v = Studio::new();
    v.open_project(p.to_str().unwrap()).unwrap();
    assert!(v.frame(0.5, 64).is_ok(), "missing media shows the offline slate, not an error");
    let _ = std::fs::remove_dir_all(&dir);
}
