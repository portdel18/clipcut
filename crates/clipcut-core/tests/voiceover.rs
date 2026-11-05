//! Voiceover: a script spoken by a built-in voice goes on a sound lane at the playhead.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// A 4-second picture with a tone.
fn picture() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core voiceover")?;
    let out = fixtures_dir("clipcut-core").join("voiceover-pic.mp4");
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

#[test]
fn a_script_is_spoken_onto_a_sound_lane_at_the_playhead() {
    let models = fixtures_dir("clipcut-core").join(format!("voice-models-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&models);
    // built-in voices are ready; the natural ones wait for their download
    let voices = clipcut_core::voiceover_voices(&models);
    let basic = voices.iter().find(|v| !v.natural).expect("a built-in voice");
    assert!(basic.installed);
    let natural: Vec<_> = voices.iter().filter(|v| v.natural).collect();
    assert!(!natural.is_empty() && natural.iter().all(|v| !v.installed), "natural voices listed, not yet downloaded");
    let (size, missing) = clipcut_core::natural_voices_size(&models);
    assert!(size > 300_000_000 && missing == size, "{size} {missing}");

    // spoken: a WAV of about the script's length; slower is longer
    let (wav, secs) = clipcut_core::speak(&models, &basic.id, "The council approved the new transit plan.", 1.0).unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert!((1.0..8.0).contains(&secs), "{secs} s");
    let (_, slow) = clipcut_core::speak(&models, &basic.id, "The council approved the new transit plan.", 0.6).unwrap();
    assert!(slow > secs * 1.3, "{slow} vs {secs}");
    // what is refused, in plain words
    let e = clipcut_core::speak(&models, &basic.id, "   ", 1.0).unwrap_err().to_string();
    assert!(e.contains("type what"), "{e}");
    let e = clipcut_core::speak(&models, &natural[0].id, "Hello", 1.0).unwrap_err().to_string();
    assert!(e.contains("download"), "{e}");
    assert!(clipcut_core::speak(&models, "no-such-voice", "Hello", 1.0).is_err());
    assert!(clipcut_core::speak(&models, &basic.id, "Hello", f64::NAN).is_err());

    // onto a sound lane at the playhead, beside the main track's own sound
    let Some(v) = picture() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let dir = fixtures_dir("clipcut-core").join(format!("voiceovers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = clipcut_core::next_voiceover_path(&dir);
    assert!(path.ends_with("Voiceover 1.wav"));
    std::fs::write(&path, &wav).unwrap();
    assert!(clipcut_core::next_voiceover_path(&dir).ends_with("Voiceover 2.wav"));
    let ids = s.add_voiceover(path.to_str().unwrap(), 1.0).unwrap();
    let tl = s.timeline();
    let clip = tl.clips.iter().find(|c| ids.contains(&c.id)).unwrap();
    assert!(clip.track.starts_with('A') && clip.track != "A1", "on a sound lane: {}", clip.track);
    assert!((clip.start - 1.0).abs() < 0.04 && (clip.end - clip.start - secs).abs() < 0.1, "{clip:?}");
    assert_eq!(tl.clips.iter().filter(|c| c.track == "A1").count(), 1, "the main track's sound is untouched");
    let _ = (std::fs::remove_dir_all(&dir), std::fs::remove_dir_all(&models));
}
