//! Voice cleanup, CapCut-style: Reduce noise and Enhance voice on a clip's sound. Reduce noise is
//! AI noise reduction (DeepFilterNet, `ai_denoise.rs`: speech kept, any other sound taken out) or
//! the engine's own DeNoise (spectral gating, for steady hiss only); Enhance voice is the engine's
//! Enhance Speech (a voice EQ and a gentle leveller). They go straight on the sound clip, after its
//! other effects, noise first. Not through Essential Sound: its audio types would
//! clash with ducked music. The engine compensates their delay, so the voice stays in sync.

use filmcraft_project::{ClipId, ParamValue, TrackItem, find_effect};
use serde::Serialize;

use crate::{Error, Result, Studio, engine};

const NOISE: &str = "denoise";
const AI: &str = crate::ai_denoise::ID;
const ENHANCE: &str = "speech_enhance";

/// A sound clip's voice cleanup.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    /// Reduce noise: how strongly, 0…100, when it is on.
    pub noise: Option<f64>,
    /// Reduce noise uses AI (else the engine's DeNoise).
    pub ai: bool,
    /// Enhance voice: how much, 0…100, when it is on.
    pub enhance: Option<f64>,
    /// Enhance voice for a `low` or a `high` voice.
    pub tone: &'static str,
}

/// What a voice edit sets: each `None` is off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Voice {
    pub noise: Option<f64>,
    /// Reduce noise with AI (else the engine's DeNoise).
    pub ai: bool,
    pub enhance: Option<f64>,
    /// A higher voice (else a lower one).
    pub high: bool,
}

fn ours(i: &TrackItem, id: &str) -> Option<usize> {
    i.effects.iter().position(|e| e.effect == id && !e.essential)
}

fn amount(i: &TrackItem, id: &str, param: &str) -> Option<f64> {
    let e = i.effects.get(ours(i, id)?)?;
    e.enabled.then(|| e.param(param).and_then(|p| p.value.as_f64()).filter(|v| v.is_finite()).unwrap_or(0.0))
}

/// Sound clip `i`'s voice cleanup.
pub(crate) fn voice_of(i: &TrackItem) -> VoiceView {
    let high = ours(i, ENHANCE).and_then(|k| i.effects.get(k)).and_then(|e| e.param("tone")).is_some_and(|p| matches!(p.value, ParamValue::Choice(1)));
    let ai = amount(i, AI, "amount");
    VoiceView {
        noise: ai.or_else(|| amount(i, NOISE, "amount")),
        ai: ai.is_some(),
        enhance: amount(i, ENHANCE, "mix"),
        tone: if high { "high" } else { "low" },
    }
}

impl Studio {
    /// Sets the voice cleanup of `clips` (their sound: a video clip's own, or a sound clip).
    /// Calls with the same `merge` key (a dragged slider) are one undo step.
    pub fn set_voice(&mut self, clips: &[u64], voice: Voice, merge: Option<&str>) -> Result<()> {
        let sounds = self.sounds_of(clips)?;
        if sounds.is_empty() {
            return Err(Error::Invalid("select a clip with sound first".into()));
        }
        let clamp = |v: Option<f64>| -> Result<Option<f64>> {
            match v {
                Some(v) if !v.is_finite() => Err(Error::Invalid("the voice settings must be numbers".into())),
                Some(v) => Ok(Some(v.clamp(0.0, 100.0))),
                None => Ok(None),
            }
        };
        let (noise, enhance) = (clamp(voice.noise)?, clamp(voice.enhance)?);
        let label = if noise.is_some() || enhance.is_some() { "Voice Cleanup" } else { "No Voice Cleanup" };
        self.group_merge(label, merge, |s| {
            s.session
                .edit_sequence(label, |q, _, _| {
                    for c in &sounds {
                        let Some((_, i)) = q.find_item_mut(ClipId(*c)) else { continue };
                        // kept where they are when they stay on (a slider moving changes nothing else);
                        // added after the clip's other effects, noise before the voice
                        // one way of reducing noise at a time
                        let (classic, ai) = (noise.is_some() && !voice.ai, noise.is_some() && voice.ai);
                        for (id, on, param, v) in [(NOISE, classic, "amount", noise), (AI, ai, "amount", noise), (ENHANCE, enhance.is_some(), "mix", enhance)] {
                            let at = ours(i, id);
                            match (on, at) {
                                (false, Some(k)) => {
                                    i.effects.remove(k);
                                }
                                (false, None) => {}
                                (true, at) => {
                                    let k = match at {
                                        Some(k) => k,
                                        None => {
                                            let Some(def) = find_effect(id) else { continue };
                                            // the voice goes after the noise, so it isn't made louder first
                                            let before = if id == ENHANCE { None } else { ours(i, ENHANCE) };
                                            let k = before.unwrap_or(i.effects.len());
                                            i.effects.insert(k, def.instance());
                                            k
                                        }
                                    };
                                    let Some(e) = i.effects.get_mut(k) else { continue };
                                    e.enabled = true;
                                    if let (Some(p), Some(v)) = (e.param_mut(param), v) {
                                        p.keyframes.clear();
                                        p.value = ParamValue::Float(v);
                                    }
                                    if id == ENHANCE
                                        && let Some(p) = e.param_mut("tone")
                                    {
                                        p.value = ParamValue::Choice(u32::from(voice.high));
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(engine)
        })
    }
}
