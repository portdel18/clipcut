//! Auto-captions, captions that follow the speech through main-track edits, transcript cutting
//! and subtitle files, through the real engine. The speech model is a fixed transcript (the
//! engine's `FixedTranscriber`), so these run without downloading Whisper.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use clipcut_core::{CaptionView, ClipView, Studio};
use filmcraft_project::transcript::{Transcript, Word};
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;
use filmcraft_time::Tick;
use serde_json::json;

fn clip(name: &str, source: &str, hz: u32) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core captions")?;
    let out = fixtures_dir("clipcut-core").join(format!("{name}.mp4"));
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("{source}=size=320x180:rate=30:duration=2")])
            .args(["-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration=2:sample_rate=48000")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// What the fixed "model" hears in every clip: a filler, then a pause before the last word.
fn speech() -> Transcript {
    let w = |text: &str, a: f64, b: f64| Word::new(text, Tick::from_seconds_f64(a), Tick::from_seconds_f64(b));
    Transcript {
        language: "en".into(),
        words: vec![w("Hello", 0.0, 0.3), w("there", 0.35, 0.6), w("um", 0.65, 0.8), w("this", 0.85, 1.0), w("is", 1.05, 1.15), w("Doe.", 1.7, 1.95)],
        ..Default::default()
    }
}

/// Two 2-second clips on the main track, transcribed and captioned.
fn captioned() -> Option<Studio> {
    let a = clip("tl-a", "testsrc2", 440)?;
    let b = clip("tl-b", "smptebars", 660)?;
    let mut s = Studio::new();
    s.add_clip(a.to_str()?).unwrap();
    s.add_clip(b.to_str()?).unwrap();
    s.session().transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: speech(), id: "fixed".into() }));
    let job = s.transcribe_job(&std::env::temp_dir(), "whisper-base", None, false).unwrap();
    assert_eq!(job.len(), 2, "each clip's media once");
    let mut last = 0.0;
    let results = job
        .run(&mut |f, _| {
            assert!(f + 1e-6 >= last, "progress went back: {f} after {last}");
            last = f;
            true
        })
        .unwrap();
    assert!((last - 1.0).abs() < 1e-6);
    let n = s.finish_captions(results).unwrap();
    assert!(n >= 2, "{n} captions");
    Some(s)
}

fn caps(s: &Studio) -> Vec<CaptionView> {
    s.timeline().captions.items
}
fn main_track(s: &Studio) -> Vec<ClipView> {
    let mut m: Vec<_> = s.timeline().clips.into_iter().filter(|c| c.track == "V1").collect();
    m.sort_by(|a, b| a.start.total_cmp(&b.start));
    m
}
/// Every caption sits on the main track and none overlap.
fn assert_tidy(s: &Studio) {
    let c = caps(s);
    let end = s.timeline().duration;
    for (k, x) in c.iter().enumerate() {
        assert!(x.end > x.start && x.end <= end + 0.05, "{x:?} (timeline {end})");
        if let Some(n) = c.get(k + 1) {
            assert!(n.start >= x.end - 1e-6, "{x:?} overlaps {n:?}");
        }
    }
}

#[test]
fn auto_captions_come_from_the_speech_in_one_step() {
    let Some(mut s) = captioned() else { return };
    let v = s.timeline();
    assert!(v.captions.exists && v.captions.on);
    assert_eq!((v.captions.transcribed, v.captions.untranscribed), (2, 0));
    assert_eq!(v.undo_label.as_deref(), Some("Auto Captions"));
    let c = caps(&s);
    assert!(c.iter().any(|x| x.text.contains("Hello") && x.start < 0.1), "{c:#?}");
    assert!(c.iter().any(|x| x.text.contains("Hello") && (x.start - 2.0).abs() < 0.1), "the second clip's speech: {c:#?}");
    assert_tidy(&s);
    // no caption runs across the cut between the two clips
    assert!(c.iter().all(|x| x.end <= 2.0 + 1e-6 || x.start >= 2.0 - 1e-6), "{c:#?}");
    // the words, in timeline time, for the transcript panel
    let w = s.words();
    assert_eq!(w.len(), 12);
    assert!((w[6].start - 2.0).abs() < 0.05 && w[6].text == "Hello");
    // a second run only transcribes what's new
    assert!(s.transcribe_job(&std::env::temp_dir(), "whisper-base", None, false).unwrap().is_empty());
    s.undo().unwrap();
    assert!(caps(&s).is_empty(), "one undo takes the captions back off");
}

#[test]
fn captions_follow_the_speech_through_edits() {
    let Some(mut s) = captioned() else { return };
    let before = caps(&s);
    let main = main_track(&s);
    // reorder: the second clip's captions move to the front with it
    s.reorder(main[1].id, 0).unwrap();
    let c = caps(&s);
    assert_eq!(c.len(), before.len());
    assert!(c.iter().all(|x| x.start < 2.0 - 1e-6 || x.start >= 2.0 - 1e-6));
    let first = c.iter().min_by(|a, b| a.start.total_cmp(&b.start)).unwrap();
    assert!(first.start.abs() < 0.05, "{c:#?}");
    assert_tidy(&s);
    s.undo().unwrap();
    assert_eq!(caps(&s), before, "undo puts them back");
    // deleting the first clip takes its captions with it and the rest move up
    s.delete(&[main[0].id]).unwrap();
    let c = caps(&s);
    assert_eq!(c.len(), before.len() / 2, "{c:#?}");
    assert!(c[0].start.abs() < 0.05);
    assert_tidy(&s);
    s.undo().unwrap();
    // trimming a clip's start cuts the words that went
    s.trim(main[0].id, true, 0.5).unwrap();
    let c = caps(&s);
    assert!(!c.iter().filter(|x| x.start < 1.5).any(|x| x.text.contains("Hello")), "Hello was trimmed away: {c:#?}");
    assert_tidy(&s);
    s.undo().unwrap();
    // double speed halves the times
    s.set_props(&[main[0].id], &json!({"speed": 200.0}), None).unwrap();
    let c = caps(&s);
    assert!(c.iter().filter(|x| x.start < 1.0).all(|x| x.end <= 1.0 + 0.05), "{c:#?}");
    assert_tidy(&s);
    // lane edits leave them alone
    let fresh = caps(&s);
    s.add_text(0.5, 1.0, "Title", &json!({})).unwrap();
    assert_eq!(caps(&s), fresh);
}

#[test]
fn transcript_cutting_removes_words_fillers_and_pauses() {
    let Some(mut s) = captioned() else { return };
    let total = s.timeline().duration;
    // "um" goes from both clips, and from the captions' words, which stay one caption each
    let n0 = caps(&s).len();
    assert_eq!(s.remove_fillers().unwrap(), 2);
    assert_eq!(caps(&s).len(), n0, "a caption with a filler cut out of it stays whole: {:#?}", caps(&s));
    let v = s.timeline();
    assert!(v.duration < total - 0.2, "{} vs {total}", v.duration);
    assert_eq!(v.undo_label.as_deref(), Some("Remove Filler Words"));
    assert!(caps(&s).iter().all(|c| !c.text.split_whitespace().any(|w| w == "um")), "{:#?}", caps(&s));
    assert!(s.words().iter().all(|w| w.text != "um"));
    assert_eq!(s.remove_fillers().unwrap(), 0, "nothing left to remove");
    // the half-second pause before "Doe." shrinks
    let d = s.timeline().duration;
    let (n, saved) = s.remove_pauses(0.4, 0.1).unwrap();
    assert_eq!(n, 2);
    assert!(saved > 0.3 && (s.timeline().duration - (d - saved)).abs() < 0.05, "{saved}");
    // cut two words by hand: "this is" in the first clip
    let w = s.words();
    let i = w.iter().position(|w| w.text == "this").unwrap();
    let d = s.timeline().duration;
    s.cut_words(i, i + 1).unwrap();
    assert!(s.timeline().duration < d - 0.2);
    assert_eq!(s.timeline().undo_label.as_deref(), Some("Cut Words"));
    assert_eq!(s.words().iter().filter(|w| w.text == "this").count(), 1, "only the second clip's is left");
    assert!(caps(&s)[0].text.starts_with("Hello there"), "{:#?}", caps(&s));
    assert_tidy(&s);
    // the main track is still magnetic
    let m = main_track(&s);
    for p in m.windows(2) {
        assert!((p[1].start - p[0].end).abs() < 1e-6, "{m:#?}");
    }
    assert!(s.cut_words(999, 999).is_err());
}

#[test]
fn captions_are_edited_styled_and_saved() {
    let Some(mut s) = captioned() else { return };
    let c = caps(&s);
    // edited words are kept through edits: a hand-made caption isn't rewritten
    s.set_caption_text(c[0].id, "Hello, everyone").unwrap();
    assert_eq!(caps(&s)[0].text, "Hello, everyone");
    s.remove_fillers().unwrap();
    assert_eq!(caps(&s)[0].text, "Hello, everyone");
    // moving one; it may not cover another
    let c = caps(&s);
    s.retime_caption(c[0].id, 0.1, 0.4).unwrap();
    let now = caps(&s);
    assert!((now[0].start - 0.1).abs() < 0.04 && (now[0].end - 0.4).abs() < 0.04, "{now:#?}");
    assert!(s.retime_caption(c[0].id, 0.0, 3.0).is_err());
    // an emptied caption goes
    let n = caps(&s).len();
    s.set_caption_text(caps(&s)[1].id, "  ").unwrap();
    assert_eq!(caps(&s).len(), n - 1);
    // the look
    s.set_caption_style(&json!({"color": "#ffd400", "position": "top", "size": 80, "background": true})).unwrap();
    let st = s.timeline().captions.style;
    assert_eq!((st.color.as_str(), st.position.as_str(), st.size, st.background), ("#ffd400", "top", 80.0, true));
    // they show in the player: the frame changes when they're hidden
    let t = (caps(&s)[0].start + caps(&s)[0].end) / 2.0;
    let with = s.frame(t, 320).unwrap().rgba;
    s.set_captions_on(false).unwrap();
    assert!(!s.timeline().captions.on);
    assert_ne!(with, s.frame(t, 320).unwrap().rgba, "the captions were drawn");
    s.set_captions_on(true).unwrap();
    // a subtitle file
    let srt = fixtures_dir(&format!("clipcut-core/captions-{}", std::process::id())).join("interview.srt");
    std::fs::create_dir_all(srt.parent().unwrap()).unwrap();
    assert_eq!(s.export_captions(srt.to_str().unwrap()).unwrap(), caps(&s).len());
    let text = std::fs::read_to_string(&srt).unwrap();
    assert!(text.contains("-->") && text.contains("Hello, everyone"), "{text}");
    // a saved project keeps where every caption's words are: it still follows after reopening
    let proj = srt.with_file_name("Interview.fcproj");
    s.save_as(proj.to_str().unwrap()).unwrap();
    let mut back = Studio::new();
    back.open_project(proj.to_str().unwrap()).unwrap();
    assert_eq!(caps(&back).len(), caps(&s).len());
    // (the filler cut split each clip in two: the first piece is 0.63 s)
    let first = main_track(&back)[0].clone();
    let gone = first.end - first.start;
    let was = caps(&back);
    back.delete(&[first.id]).unwrap();
    let now = caps(&back);
    assert_eq!(now.len(), was.len() - 1, "the hand-moved caption over the deleted piece went: {now:#?}");
    for (n, w) in now.iter().zip(was.iter().skip(1)) {
        assert!((n.start - (w.start - gone)).abs() < 0.04, "{n:?} should be {gone} s earlier than {w:?}");
    }
    let _ = std::fs::remove_dir_all(srt.parent().unwrap());
}

#[test]
fn no_model_no_speech_and_cancel_say_so() {
    let Some(a) = clip("tl-a", "testsrc2", 440) else { return };
    let mut s = Studio::new();
    // nothing on the timeline
    assert!(s.transcribe_job(&std::env::temp_dir(), "whisper-base", None, false).is_err());
    s.add_clip(a.to_str().unwrap()).unwrap();
    // no model downloaded (and no stand-in)
    let empty = std::env::temp_dir().join(format!("clipcut-no-models-{}", std::process::id()));
    let e = s.transcribe_job(&empty, "whisper-base", None, false).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("isn't downloaded"), "{e}");
    let models = clipcut_core::speech_models(&empty);
    assert!(models.len() >= 3 && models.iter().all(|m| !m.installed && m.size > 1_000_000));
    // cancelling stops the run
    s.session().transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: speech(), id: "fixed".into() }));
    let job = s.transcribe_job(&empty, "whisper-base", Some("en".into()), false).unwrap();
    let e = job.run(&mut |_, _| false).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("cancelled"), "{e}");
    // nothing heard: no captions, a clear reason
    s.session().transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: Transcript::default(), id: "fixed".into() }));
    let job = s.transcribe_job(&empty, "whisper-base", None, false).unwrap();
    let r = job.run(&mut |_, _| true).unwrap();
    let e = s.finish_captions(r).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("no speech"), "{e}");
    assert!(!s.timeline().captions.exists, "a failed run leaves nothing behind");
}

#[test]
fn exports_burn_the_captions_in() {
    let Some(mut s) = captioned() else { return };
    let c = caps(&s)[0].clone();
    let t = (c.start + c.end) / 2.0;
    let out = fixtures_dir("clipcut-core").join(format!("captions-burn-{}.mp4", std::process::id()));
    let job = s.export(out.to_str().unwrap()).unwrap();
    let t0 = std::time::Instant::now();
    loop {
        let st = s.job(job).unwrap();
        if st.finished {
            assert_eq!(st.error, None);
            break;
        }
        assert!(t0.elapsed().as_secs() < 300, "export stuck");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mut back = Studio::new();
    back.open_clip(out.to_str().unwrap()).unwrap();
    let burnt = back.frame(t, 320).unwrap().rgba;
    let with = s.frame(t, 320).unwrap().rgba;
    s.set_captions_on(false).unwrap();
    let without = s.frame(t, 320).unwrap().rgba;
    let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y))).sum::<u64>();
    assert!(diff(&burnt, &with) < diff(&burnt, &without), "the export doesn't look like the player with its captions");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn extracted_sound_stays_put_with_its_captions() {
    let Some(mut s) = captioned() else { return };
    let before = caps(&s);
    let main = main_track(&s);
    let second = main[1].clone();
    let sound = s.extract_audio(second.id).unwrap();
    let v = s.timeline();
    assert_eq!(v.undo_label.as_deref(), Some("Extract Audio"));
    let snd = v.clips.iter().find(|c| c.id == sound).unwrap();
    assert_eq!(snd.track, "A2", "{snd:?}");
    assert!((snd.start - second.start).abs() < 1e-6 && (snd.end - second.end).abs() < 1e-6, "same place: {snd:?}");
    assert!(snd.link.is_none() && v.clips.iter().find(|c| c.id == second.id).unwrap().link.is_none(), "no longer linked");
    assert!(!v.clips.iter().any(|c| c.track == "A1" && c.start >= 2.0 - 1e-6), "the main track's sound there is gone");
    assert_eq!(caps(&s), before, "the captions stay over the words");
    // it is a clip of its own now; the first clip's sound can be extracted from the sound itself
    assert!(s.extract_audio(sound).is_err());
    let first_sound = v.clips.iter().find(|c| c.track == "A1" && c.start < 0.1).unwrap().id;

    // deleting the first clip moves the picture up, but the sound and its captions stay
    s.delete(&[main[0].id]).unwrap();
    let v = s.timeline();
    assert!(v.clips.iter().find(|c| c.id == second.id).unwrap().start.abs() < 1e-6, "the picture moved up");
    assert!((v.clips.iter().find(|c| c.id == sound).unwrap().start - 2.0).abs() < 1e-6, "the sound stayed");
    let c = caps(&s);
    assert_eq!(c.len(), before.len() / 2, "{c:#?}");
    assert!(c.iter().all(|x| x.start >= 2.0 - 1e-6) && c.iter().any(|x| x.text.contains("Hello")), "{c:#?}");
    assert_tidy(&s);
    s.undo().unwrap();
    s.undo().unwrap();
    let v = s.timeline();
    assert!(v.clips.iter().find(|c| c.id == sound).is_some_and(|c| c.track == "A1" && c.link.is_some()), "one undo step puts it back");
    assert_eq!(caps(&s), before);

    let picked = s.extract_audio(first_sound).unwrap();
    assert_eq!(picked, first_sound);
    assert_eq!(s.timeline().clips.into_iter().find(|c| c.id == first_sound).unwrap().track, "A2");
    // a picture without sound has none to extract
    assert!(s.extract_audio(main[0].id).is_err());
    assert!(s.extract_audio(987_654).is_err());
}
