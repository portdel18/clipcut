//! Ducking, CapCut-style: music on a lane goes quieter while people talk. The engine's Essential
//! Sound ducking does it: the music clip is typed as Music, every untyped sound under it (the main
//! track's own sound, voiceovers) is what it ducks under, and the dips become keyframes on the
//! music's volume, so they show and can be changed like any other.

use filmcraft_project::{ClipId, ParamValue, TrackKind};
use serde_json::{Value, json};

use crate::{Error, Result, Studio, engine};

/// How far the music goes down by default, and the limits (dB).
pub const DUCK_DB: f64 = -15.0;
const DUCK_RANGE: (f64, f64) = (-40.0, -3.0);
/// How long the music takes to go down and come back up.
const FADE_SECONDS: f64 = 0.5;

/// A sound clip's ducking: how far it goes down under speech (dB), when it ducks.
pub(crate) fn ducking_of(i: &filmcraft_project::TrackItem) -> Option<f64> {
    let e = i.essential.as_ref()?;
    (e.ducking.enabled && e.kind == filmcraft_project::essential::AudioType::Music).then_some(e.ducking.reduce_db)
}

impl Studio {
    /// The sound clips on lanes among `clips` (the main track's sound is what they duck under).
    fn lane_sounds(&self, clips: &[u64]) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mut v = Vec::new();
        for c in clips {
            let Some((t, i)) = q.find_item(ClipId(*c)) else { continue };
            let Some(track) = q.track(t) else { continue };
            // a picture clip stands for its own sound
            let id = if track.kind == TrackKind::Audio {
                Some(i.id)
            } else {
                i.link.and_then(|l| q.audio_tracks.iter().flat_map(|t| &t.items).find(|a| a.link == Some(l)).map(|a| a.id))
            };
            let Some(id) = id else { continue };
            if q.audio_tracks.first().is_some_and(|t| t.items.iter().any(|a| a.id == id)) {
                return Err(Error::Invalid("the main track's sound is what music ducks under: select the music on a lane below it".into()));
            }
            v.push(id.0);
        }
        if v.is_empty() {
            return Err(Error::Invalid("select music on a lane below the main track first".into()));
        }
        Ok(v)
    }

    /// Makes the music among `clips` duck `reduce_db` under the speech and other sound it plays
    /// with (on), or play steadily again (off). Returns how many dips were made. One undo step;
    /// turning it on again updates the dips after edits.
    pub fn set_ducking(&mut self, clips: &[u64], on: bool, reduce_db: f64) -> Result<usize> {
        let clips = self.lane_sounds(clips)?;
        if !reduce_db.is_finite() {
            return Err(Error::Invalid("how far the music goes down must be a number".into()));
        }
        let reduce = reduce_db.clamp(DUCK_RANGE.0, DUCK_RANGE.1);
        if !on {
            return self.group("Stop Ducking", |s| {
                s.session
                    .edit_sequence("Stop Ducking", |q, _, _| {
                        for t in q.audio_tracks.iter_mut() {
                            for i in t.items.iter_mut().filter(|i| clips.contains(&i.id.0)) {
                                let Some(reduce) = ducking_of(i) else { continue };
                                // back to the level between the dips: the deepest dip is that level plus the reduction
                                // (music that is talked over from end to end has no keyframe at it)
                                if let Some(p) = i.effect_mut("volume").and_then(|e| e.param_mut("level"))
                                    && p.is_animated()
                                {
                                    let low = p.keyframes.iter().filter_map(|k| k.value.as_f64()).fold(f64::INFINITY, f64::min);
                                    p.keyframes.clear();
                                    p.value = ParamValue::Float(if low.is_finite() { low - reduce } else { 0.0 });
                                }
                                i.essential = None;
                            }
                        }
                        Ok(())
                    })
                    .map_err(engine)?;
                Ok(0)
            });
        }
        self.group("Duck Music", |s| {
            let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
            let untyped: Vec<u64> = clips
                .iter()
                .copied()
                .filter(|c| {
                    q.find_item(ClipId(*c)).is_some_and(|(_, i)| i.essential.as_ref().is_none_or(|e| e.kind != filmcraft_project::essential::AudioType::Music))
                })
                .collect();
            if !untyped.is_empty() {
                s.session.execute("essentialSound.setType", json!({"clips": untyped, "type": "music"})).map_err(engine)?;
            }
            s.session
                .execute(
                    "essentialSound.set",
                    json!({"clips": clips, "values": {"ducking.againstUntyped": true, "ducking.reduceDb": reduce, "ducking.fadeS": FADE_SECONDS}}),
                )
                .map_err(engine)?;
            let r = s.session.execute("essentialSound.generateDucking", json!({"clips": clips})).map_err(engine)?;
            Ok(r.get("clips")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|c| c.get("regions").and_then(Value::as_array)).map(Vec::len).sum())
                .unwrap_or(0))
        })
    }
}
