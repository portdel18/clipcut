//! Freeze frames, reverse and the voice's pitch, CapCut-style. Freeze puts a still of the frame at
//! the playhead into the video for a few seconds (the engine's Insert Frame Hold Segment) and
//! pushes the rest of the track along; the sound under it pauses too, so speech stays with its
//! picture. Reverse plays a clip (and its sound) backwards at the speed it has.
//!
//! A clip played faster or slower keeps its voice's pitch: its sound clip's Maintain Audio Pitch
//! (`TrackItem::maintain_pitch`) has the engine time-stretch it, joining slices of the sound
//! instead of resampling it, so voices stay clean. Played backwards, the engine can only resample,
//! so a reversed clip gets the engine's Pitch Shifter to bring it back (a chain of them past 2×,
//! each moving at most an octave). Projects from before Maintain Audio Pitch had Pitch Shifters on
//! forward clips too, which gave voices a hiss: [`Studio::fit_pitch`] swaps them when a project
//! opens. A clip marker named "ClipCut pitch" turns all of it off for the clip
//! ([`Studio::set_keep_pitch`]).

use filmcraft_project::{ClipId, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{Error, Result, Studio, engine};

/// The engine effect that moves the pitch back, and the clip marker that says not to.
const PITCH: &str = "pitch_shifter";
const PITCH_OFF: &str = "ClipCut pitch";
/// How far one Pitch Shifter moves (semitones), and the most chained.
const OCTAVE: f64 = 12.0;
const MOST_SHIFTERS: usize = 4;

/// Whether sound clip `i` keeps its pitch at other speeds.
pub(crate) fn keeps_pitch(i: &filmcraft_project::TrackItem) -> bool {
    !i.markers.iter().any(|m| m.name == PITCH_OFF)
}

/// The Pitch Shifters (semitones each) that bring sound played at `speed` (1 = normal) back to
/// its own pitch.
pub(crate) fn shifters_for(speed: f64) -> Vec<f64> {
    let speed = speed.abs();
    if !speed.is_finite() || speed <= 0.0 || (speed - 1.0).abs() < 1e-3 {
        return Vec::new();
    }
    let st = (-OCTAVE * speed.log2()).clamp(-OCTAVE * MOST_SHIFTERS as f64, OCTAVE * MOST_SHIFTERS as f64);
    let n = ((st.abs() / OCTAVE).ceil() as usize).clamp(1, MOST_SHIFTERS);
    vec![st / n as f64; n]
}

/// A freeze's length by default, and the limits (seconds).
pub const FREEZE_SECONDS: f64 = 3.0;
const FREEZE_RANGE: (f64, f64) = (0.1, 60.0);

impl Studio {
    /// Freezes the frame at `seconds` for `length` seconds: of clip `clip`, or of the main
    /// track's clip there. Returns the still's clip. One undo step.
    pub fn freeze(&mut self, seconds: f64, clip: Option<u64>, length: f64) -> Result<u64> {
        if !length.is_finite() {
            return Err(Error::Invalid("the freeze's length must be a number".into()));
        }
        let length = length.clamp(FREEZE_RANGE.0, FREEZE_RANGE.1);
        let t = self.frame_tick(seconds)?;
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let under = |c: &filmcraft_project::TrackItem| c.start <= t && t < c.end();
        let target = match clip {
            Some(c) => {
                let (tr, i) = q.find_item(ClipId(c)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
                if !q.track(tr).is_some_and(|t| t.kind == TrackKind::Video) {
                    return Err(Error::Invalid("a freeze is a still of a picture: select a video clip".into()));
                }
                if !under(i) {
                    return Err(Error::Invalid("move the playhead over the clip to freeze its frame there".into()));
                }
                i.clone()
            }
            None => q
                .video_tracks
                .first()
                .and_then(|tr| tr.items.iter().find(|i| under(i)))
                .cloned()
                .ok_or_else(|| Error::Invalid("move the playhead over a clip on the main track to freeze its frame".into()))?,
        };
        if target.frame_hold.is_some() {
            return Err(Error::Invalid("this is a still already: change its length by trimming it".into()));
        }
        let main = self.on_main(target.id.0);
        // the freeze goes in at the frame's start, so the frame shown is the one frozen
        let partners: Vec<u64> = target
            .link
            .map(|l| q.all_tracks().flat_map(|tr| tr.items.iter()).filter(|i| i.link == Some(l) && i.id != target.id).map(|i| i.id.0).collect())
            .unwrap_or_default();
        self.group_follow("Freeze Frame", None, |s| {
            // the sound pauses for the freeze: it is cut there, and what follows moves along with
            // the picture
            if t > target.start {
                for c in std::iter::once(target.id.0).chain(partners.iter().copied()) {
                    let cut = s.session.active_sequence().and_then(|q| q.find_item(ClipId(c))).is_some_and(|(_, i)| i.start < t && i.end() > t);
                    if cut {
                        s.session.execute("timeline.razor", json!({"time": t.0, "clip": c})).map_err(engine)?;
                    }
                }
            }
            let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
            let piece = q
                .all_tracks()
                .filter(|tr| tr.kind == TrackKind::Video)
                .flat_map(|tr| tr.items.iter())
                .find(|i| i.item == target.item && i.start <= t && t < i.end() && (i.id == target.id || i.start == t))
                .map(|i| i.id.0)
                .ok_or_else(|| Error::Engine("the clip to freeze went missing".into()))?;
            let r = s.session.execute("clip.insertFrameHoldSegment", json!({"clip": piece, "time": t.0, "seconds": length})).map_err(engine)?;
            let still = r.get("clip").and_then(Value::as_u64).ok_or_else(|| Error::Engine("the freeze was not made".into()))?;
            if main {
                s.pack_main()?;
            }
            Ok(still)
        })
    }

    /// Fits the sound of `clips` to its speed: Maintain Audio Pitch on when it keeps its pitch
    /// and plays forward, Pitch Shifters for a reversed one that keeps it (none otherwise). Inside
    /// an edit.
    pub(crate) fn fit_pitch(&mut self, clips: &[u64]) -> Result<()> {
        for c in self.sounds_of(clips)? {
            let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
            let Some((_, i)) = q.find_item(ClipId(c)) else { continue };
            let stretch = keeps_pitch(i) && !i.reverse;
            if i.maintain_pitch != stretch {
                self.session
                    .edit_sequence("Keep Pitch", |q, _, _| {
                        if let Some((_, i)) = q.find_item_mut(ClipId(c)) {
                            i.maintain_pitch = stretch;
                        }
                        Ok(())
                    })
                    .map_err(engine)?;
            }
            let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
            let Some((_, i)) = q.find_item(ClipId(c)) else { continue };
            let want = if keeps_pitch(i) && i.reverse { shifters_for(i.speed) } else { Vec::new() };
            let have: Vec<usize> = i.effects.iter().enumerate().filter(|(_, e)| e.effect == PITCH).map(|(k, _)| k).collect();
            let same = have.len() == want.len()
                && have
                    .iter()
                    .zip(&want)
                    .all(|(k, st)| i.effects.get(*k).and_then(|e| e.param("semitones")).and_then(|p| p.value.as_f64()).is_some_and(|v| (v - st).abs() < 1e-6));
            if same {
                continue;
            }
            for k in have.iter().rev() {
                self.session.execute("effects.remove", json!({"clip": c, "index": k})).map_err(engine)?;
            }
            for st in want {
                self.session.execute("effects.apply", json!({"clips": [c], "effect": PITCH})).map_err(engine)?;
                let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
                let index = q
                    .find_item(ClipId(c))
                    .and_then(|(_, i)| i.effects.iter().rposition(|e| e.effect == PITCH))
                    .ok_or_else(|| Error::Engine("the pitch could not be kept".into()))?;
                self.session.execute("effects.setParam", json!({"clip": c, "effect": index, "param": "semitones", "value": st})).map_err(engine)?;
            }
        }
        Ok(())
    }

    /// Brings an opened project's sound clips up to date: the Pitch Shifters projects from before
    /// Maintain Audio Pitch put on forward clips become Maintain Audio Pitch. Not an undo step.
    pub(crate) fn migrate_pitch(&mut self) -> Result<()> {
        let Some(q) = self.session.active_sequence() else { return Ok(()) };
        let old: Vec<u64> = q
            .audio_tracks
            .iter()
            .flat_map(|t| t.items.iter())
            .filter(|i| keeps_pitch(i) && !i.reverse && (!i.maintain_pitch || i.effects.iter().any(|e| e.effect == PITCH)))
            .filter(|i| (i.speed.abs() - 1.0).abs() > 1e-3 || i.effects.iter().any(|e| e.effect == PITCH))
            .map(|i| i.id.0)
            .collect();
        if old.is_empty() {
            return Ok(());
        }
        let before = self.session.history.undo.len();
        self.fit_pitch(&old)?;
        // opening a project is not something to undo
        self.session.history.undo.truncate(before);
        Ok(())
    }

    /// Whether the sound of `clips` keeps its pitch when played faster or slower (`on`, the
    /// default), or goes higher and lower with the speed. One undo step.
    pub fn set_keep_pitch(&mut self, clips: &[u64], on: bool) -> Result<()> {
        let sound = self.sounds_of(clips)?;
        if sound.is_empty() {
            return Err(Error::Invalid("select a clip with sound first".into()));
        }
        self.group(if on { "Keep Pitch" } else { "Pitch Follows Speed" }, |s| {
            s.session
                .edit_sequence("Keep Pitch", |q, ctx, _| {
                    for c in &sound {
                        let Some((_, i)) = q.find_item_mut(ClipId(*c)) else { continue };
                        i.markers.retain(|m| m.name != PITCH_OFF);
                        if !on {
                            i.markers.push(filmcraft_project::Marker {
                                id: filmcraft_project::MarkerId(ctx.alloc()),
                                start: i.source_in,
                                duration: Tick::ZERO,
                                name: PITCH_OFF.into(),
                                comment: String::new(),
                                kind: filmcraft_project::MarkerKind::Comment,
                                color: filmcraft_project::Label::Violet,
                            });
                        }
                    }
                    Ok(())
                })
                .map_err(engine)?;
            s.fit_pitch(&sound)
        })
    }

    /// Plays `clips` (and their sound) backwards, or forwards again (`on` false), at the speed
    /// they have. One undo step.
    pub fn set_reverse(&mut self, clips: &[u64], on: bool) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mut targets = Vec::new();
        for c in clips {
            let (tr, i) = q.find_item(ClipId(*c)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
            if i.frame_hold.is_some() || q.track(tr).is_none() {
                continue;
            }
            if s_is_still(&self.session, i) {
                continue;
            }
            targets.push((*c, i.speed.abs() * 100.0));
        }
        if targets.is_empty() {
            return Err(Error::Invalid("pictures and stills have nothing to play backwards: select a video or sound clip".into()));
        }
        self.group_follow(if on { "Reverse" } else { "Play Forwards" }, None, |s| {
            for (c, speed) in &targets {
                let main = s.on_main(*c);
                s.session.execute("clip.speedDuration", json!({"clips": [c], "speed": speed, "reverse": on, "ripple": main})).map_err(engine)?;
            }
            let ids: Vec<u64> = targets.iter().map(|(c, _)| *c).collect();
            s.fit_pitch(&ids)?;
            s.pack_main()
        })
    }
}

/// A still picture or a graphic: nothing to play backwards.
fn s_is_still(s: &filmcraft_engine::Session, i: &filmcraft_project::TrackItem) -> bool {
    match s.project.item(i.item).map(|it| &it.kind) {
        Some(filmcraft_project::ItemKind::Media(m)) => m.info.kind == filmcraft_media::MediaKind::Still,
        Some(_) => true,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pitch_comes_back_an_octave_at_a_time() {
        assert!(shifters_for(1.0).is_empty() && shifters_for(0.0).is_empty() && shifters_for(f64::NAN).is_empty());
        assert_eq!(shifters_for(2.0), [-12.0]);
        assert_eq!(shifters_for(0.5), [12.0]);
        let three = shifters_for(3.0);
        assert_eq!(three.len(), 2);
        assert!((three.iter().sum::<f64>() + 12.0 * 3f64.log2()).abs() < 1e-9);
        assert_eq!(shifters_for(1000.0).len(), MOST_SHIFTERS);
        assert!(shifters_for(-2.0) == [-12.0], "reversed plays at its speed");
    }

    /// A project saved before Maintain Audio Pitch: a clip at 150% whose sound has a Pitch Shifter.
    /// Opening it swaps the shifter for Maintain Audio Pitch, without an undo step.
    #[test]
    fn an_old_project_s_pitch_shifters_become_maintain_audio_pitch() {
        let Some(ff) = filmcraft_testkit::oracle::ffmpeg_or_skip("clipcut-core pitch migration") else { return };
        use filmcraft_testkit::fixtures::{fixtures_dir, generate};
        let dir = fixtures_dir(&format!("clipcut-core/pitch-{}", std::process::id()));
        let media = dir.join("tone.mp4");
        let made = generate(&media, |tmp| {
            std::process::Command::new(&ff)
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2"])
                .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000"])
                .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
                .arg(tmp)
                .status()
                .is_ok_and(|s| s.success())
        });
        let Some(media) = made else { return };
        let mut s = Studio::new();
        s.add_clip(media.to_str().unwrap()).unwrap();
        let id = s.timeline().clips.iter().find(|c| c.track == "V1").unwrap().id;
        s.set_props(&[id], &json!({"speed": 150.0}), None).unwrap();
        let snd = s.sounds_of(&[id]).unwrap()[0];
        let item = |s: &Studio| s.session.active_sequence().unwrap().find_item(ClipId(snd)).unwrap().1.clone();
        assert!(item(&s).maintain_pitch && !item(&s).effects.iter().any(|e| e.effect == PITCH), "new clips stretch");
        // as 0.4 left it
        s.session
            .edit_sequence("old", |q, _, _| {
                q.find_item_mut(ClipId(snd)).unwrap().1.maintain_pitch = false;
                Ok(())
            })
            .unwrap();
        s.session.execute("effects.apply", json!({"clips": [snd], "effect": PITCH})).unwrap();
        assert!(!item(&s).maintain_pitch && item(&s).effects.iter().any(|e| e.effect == PITCH));
        let proj = dir.join("Old.fcproj");
        s.save_as(proj.to_str().unwrap()).unwrap();
        let mut back = Studio::new();
        back.open_project(proj.to_str().unwrap()).unwrap();
        let i = back.session.active_sequence().unwrap().find_item(ClipId(snd)).unwrap().1.clone();
        assert!(i.maintain_pitch, "Maintain Audio Pitch after opening");
        assert!(!i.effects.iter().any(|e| e.effect == PITCH), "the Pitch Shifter is gone");
        assert_eq!(back.timeline().undo_label, None, "opening is not an undo step");
    }
}
