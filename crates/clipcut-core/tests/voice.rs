//! Voice cleanup: Reduce noise takes steady hiss out of the gaps between words and leaves the voice;
//! Enhance voice changes it; both are one undo step and follow the clip's sound.

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::{Studio, Voice};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// 4 s: a "voice" (300 Hz, on for half of every second) over steady white hiss.
fn noisy() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core voice")?;
    let out = fixtures_dir("clipcut-core").join("voice-noisy.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=300:duration=4:sample_rate=48000"])
            .args(["-f", "lavfi", "-i", "anoisesrc=duration=4:color=white:amplitude=0.05:sample_rate=48000"])
            .args([
                "-filter_complex",
                "[1:a]volume='if(lt(mod(t,1),0.5),0.4,0)':eval=frame[v];[v][2:a]amix=inputs=2:normalize=0[a]",
                "-map",
                "0:v",
                "-map",
                "[a]",
            ])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "256k", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// RMS of the mix (left) over `[t, t + d)`.
fn rms(s: &Studio, t: f64, d: f64) -> f64 {
    let feed = s.audio_feed().unwrap();
    let rate = f64::from(feed.sample_rate);
    let buf = feed.mix((t * rate) as i64, (d * rate) as usize);
    (buf.iter().step_by(2).map(|v| f64::from(*v).powi(2)).sum::<f64>() / (buf.len() / 2).max(1) as f64).sqrt()
}

fn db(v: f64) -> f64 {
    20.0 * v.max(1e-9).log10()
}

#[test]
fn reduce_noise_quiets_the_gaps_and_keeps_the_voice() {
    let Some(path) = noisy() else { return };
    let mut s = Studio::new();
    s.add_clip(path.to_str().unwrap()).unwrap();
    let v = s.timeline();
    let pic = v.clips.iter().find(|c| c.track == "V1").unwrap().id;
    let snd = v.clips.iter().find(|c| c.track == "A1").unwrap().id;
    assert_eq!(v.clips.iter().find(|c| c.id == snd).unwrap().props.voice.as_ref().map(|x| (x.noise, x.enhance)), Some((None, None)));
    // a gap between words late in the clip (the noise floor is followed from the start), and a word
    let (gap0, word0) = (rms(&s, 3.6, 0.3), rms(&s, 3.1, 0.3));

    // the video clip's own sound gets it
    s.set_voice(&[pic], Voice { noise: Some(80.0), enhance: None, high: false }, None).unwrap();
    let v = s.timeline();
    assert_eq!(v.undo_label.as_deref(), Some("Voice Cleanup"));
    assert_eq!(v.clips.iter().find(|c| c.id == snd).unwrap().props.voice.as_ref().and_then(|x| x.noise), Some(80.0));
    let (gap1, word1) = (rms(&s, 3.6, 0.3), rms(&s, 3.1, 0.3));
    assert!(db(gap1) < db(gap0) - 10.0, "the hiss between words: {:.1} dB → {:.1} dB", db(gap0), db(gap1));
    assert!(db(word1) > db(word0) - 3.0, "the voice: {:.1} dB → {:.1} dB", db(word0), db(word1));

    // Enhance voice on as well: the sound changes, Reduce noise stays
    s.set_voice(&[snd], Voice { noise: Some(80.0), enhance: Some(100.0), high: true }, None).unwrap();
    let x = s.timeline().clips.iter().find(|c| c.id == snd).unwrap().props.voice.clone().unwrap();
    assert_eq!((x.noise, x.enhance, x.tone), (Some(80.0), Some(100.0), "high"));
    assert!((db(rms(&s, 3.1, 0.3)) - db(word1)).abs() > 0.1, "Enhance voice changes the voice");

    // a dragged slider is one undo step; both off; undo brings them back
    s.set_voice(&[snd], Voice { noise: Some(40.0), enhance: Some(100.0), high: true }, Some("noise")).unwrap();
    s.set_voice(&[snd], Voice { noise: Some(30.0), enhance: Some(100.0), high: true }, Some("noise")).unwrap();
    s.set_voice(&[snd], Voice::default(), None).unwrap();
    assert_eq!(s.timeline().clips.iter().find(|c| c.id == snd).unwrap().props.voice.as_ref().map(|x| (x.noise, x.enhance)), Some((None, None)));
    assert!((db(rms(&s, 3.6, 0.3)) - db(gap0)).abs() < 0.5, "off: as it was");
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.iter().find(|c| c.id == snd).unwrap().props.voice.as_ref().and_then(|x| x.noise), Some(30.0));
    s.undo().unwrap();
    assert_eq!(s.timeline().clips.iter().find(|c| c.id == snd).unwrap().props.voice.as_ref().and_then(|x| x.noise), Some(80.0));
    // hostile numbers are refused
    assert!(s.set_voice(&[snd], Voice { noise: Some(f64::NAN), enhance: None, high: false }, None).is_err());
}
