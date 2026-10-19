//! Two-mic recordings: a sound clip recorded with one microphone on each side (one person on the
//! left, one on the right), or one microphone on one side, plays as recorded (`stereo`), one side in
//! both ears (`left`, `right`), or both sides mixed into both ears (`mono`, each side at its own
//! level so two voices can be evened out). It is the engine's Channel Mixer on the clip
//! (`channel_mixer_a`), so it is heard in the player and in exports alike.
//! [`Studio::channel_levels`] measures the two sides to suggest which one a clip needs.

use filmcraft_project::{ClipId, ParamValue, TrackItem, TrackKind};
use serde::Serialize;
use serde_json::json;

use crate::{Error, Result, Studio, engine};

/// The engine effect that routes the two sides.
const MIXER: &str = "channel_mixer_a";
/// How far one side's level goes down (dB) when two voices are evened out.
pub const SIDE_DB: (f64, f64) = (-24.0, 0.0);
/// The longest stretch measured (seconds): enough to hear both voices.
const MEASURE_SECONDS: f64 = 600.0;

/// How a sound clip's two sides play.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsView {
    /// `stereo`, `left`, `right`, `mono`, or `custom` (a Channel Mixer set some other way).
    pub mode: String,
    /// With `mono`: each side's level in the mix, dB (0 = as recorded).
    pub left_db: f64,
    pub right_db: f64,
}

/// What the two sides of a clip sound like.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLevels {
    /// Each side's loudness while anyone talks (dBFS, RMS), and the voices' own levels: the
    /// left side's while it is the louder one, the right side's likewise.
    pub left_db: f64,
    pub right_db: f64,
    pub left_voice_db: Option<f64>,
    pub right_voice_db: Option<f64>,
    /// How alike the two sides are (1: the same sound, 0: unrelated).
    pub likeness: f64,
    /// What the clip seems to need: `left` or `right` (the other side is about silent), `mono`
    /// (a different voice on each side), or nothing (stereo as it is).
    pub suggest: Option<&'static str>,
    /// For `mono`: the levels that even the two voices out.
    pub even_left_db: f64,
    pub even_right_db: f64,
}

fn pct(i: &TrackItem, id: &str) -> f64 {
    match i.effects.iter().find(|e| e.effect == MIXER).and_then(|e| e.param(id)).map(|p| &p.value) {
        Some(ParamValue::Float(v)) if v.is_finite() => *v,
        _ => 0.0,
    }
}

fn to_db(p: f64) -> f64 {
    if p <= 0.0 { SIDE_DB.0 } else { (20.0 * (p / 100.0).log10()).clamp(SIDE_DB.0, SIDE_DB.1) }
}

fn to_pct(db: f64) -> f64 {
    (100.0 * 10f64.powf(db.clamp(SIDE_DB.0, SIDE_DB.1) / 20.0) * 100.0).round() / 100.0
}

/// How sound clip `i`'s two sides play now.
pub(crate) fn channels_of(i: &TrackItem) -> ChannelsView {
    let Some(e) = i.effects.iter().find(|e| e.effect == MIXER) else { return ChannelsView { mode: "stereo".into(), ..Default::default() } };
    let inverted = ["invert_l", "invert_r"].iter().any(|k| e.param(k).and_then(|p| p.value.as_bool()).unwrap_or(false));
    let (ll, lr, rl, rr) = (pct(i, "l_from_l"), pct(i, "l_from_r"), pct(i, "r_from_l"), pct(i, "r_from_r"));
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    let mode = if !e.enabled || inverted {
        "custom"
    } else if near(ll, 100.0) && near(lr, 0.0) && near(rl, 0.0) && near(rr, 100.0) {
        "stereo"
    } else if ll > 0.0 && near(lr, 0.0) && near(rl, ll) && near(rr, 0.0) {
        "left"
    } else if near(ll, 0.0) && lr > 0.0 && near(rl, 0.0) && near(rr, lr) {
        "right"
    } else if ll > 0.0 && lr > 0.0 && near(rl, ll) && near(rr, lr) {
        "mono"
    } else {
        "custom"
    };
    let (left_db, right_db) = if mode == "mono" { (to_db(ll), to_db(lr)) } else { (0.0, 0.0) };
    ChannelsView { mode: mode.into(), left_db, right_db }
}

/// Loudness (dBFS) of an RMS sum of squares over `n` samples.
fn db_of(sum: f64, n: usize) -> f64 {
    if n == 0 || sum <= 0.0 { -120.0 } else { (10.0 * (sum / n as f64).log10()).max(-120.0) }
}

/// Measures two sides: their loudness where anyone talks, each voice's own level, and how alike
/// they are. 50 ms blocks; a block counts as talking when either side is above -50 dBFS.
pub(crate) fn measure(l: &[f32], r: &[f32], sr: u32) -> ChannelLevels {
    let block = (sr as usize / 20).max(1);
    let (mut sl, mut sr2, mut slr, mut n) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    let (mut vl, mut nl, mut vr, mut nr) = (0.0f64, 0usize, 0.0f64, 0usize);
    for (a, b) in l.chunks(block).zip(r.chunks(block)) {
        let ea: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum();
        let eb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum();
        let len = a.len().min(b.len());
        if len == 0 || db_of(ea.max(eb), len) < -50.0 {
            continue;
        }
        sl += ea;
        sr2 += eb;
        slr += a.iter().zip(b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum::<f64>();
        n += len;
        // a side twice as loud (+3 dB) as the other is its own voice talking
        if ea > 2.0 * eb {
            vl += ea;
            nl += len;
        } else if eb > 2.0 * ea {
            vr += eb;
            nr += len;
        }
    }
    let (left_db, right_db) = (db_of(sl, n), db_of(sr2, n));
    let likeness = if sl > 0.0 && sr2 > 0.0 { (slr / (sl * sr2).sqrt()).abs().min(1.0) } else { 0.0 };
    let (left_voice_db, right_voice_db) = ((nl > 0).then(|| db_of(vl, nl)), (nr > 0).then(|| db_of(vr, nr)));
    let suggest = if n == 0 {
        None
    } else if right_db < left_db - 20.0 {
        Some("left")
    } else if left_db < right_db - 20.0 {
        Some("right")
    } else if likeness < 0.6 {
        Some("mono")
    } else {
        None
    };
    // the louder voice comes down to the quieter one
    let (even_left_db, even_right_db) = match (left_voice_db, right_voice_db) {
        (Some(a), Some(b)) if a > b => ((b - a).clamp(SIDE_DB.0, 0.0), 0.0),
        (Some(a), Some(b)) => (0.0, (a - b).clamp(SIDE_DB.0, 0.0)),
        _ => (0.0, 0.0),
    };
    ChannelLevels { left_db, right_db, left_voice_db, right_voice_db, likeness, suggest, even_left_db, even_right_db }
}

impl Studio {
    /// The sound clips `clips` stand for: sound clips themselves, a picture's own sound.
    pub(crate) fn sounds_of(&self, clips: &[u64]) -> Result<Vec<u64>> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mut out = Vec::new();
        for c in clips {
            let Some((t, i)) = q.find_item(ClipId(*c)) else { continue };
            if q.track(t).is_some_and(|t| t.kind == TrackKind::Audio) {
                out.push(*c);
            } else if let Some(l) = i.link {
                out.extend(q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|a| a.link == Some(l)).map(|a| a.id.0));
            }
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// How many channels sound clip `clip`'s media has.
    fn source_channels(&self, clip: u64) -> u32 {
        let Some(q) = self.session.active_sequence() else { return 0 };
        let Some((_, i)) = q.find_item(ClipId(clip)) else { return 0 };
        self.session.project.item(i.item).and_then(|it| it.as_media()).and_then(|m| m.info.audio_streams.get(i.audio_stream)).map_or(0, |a| a.channels)
    }

    /// Plays the sound of `clips` as recorded (`stereo`), one side in both ears (`left`,
    /// `right`), or both sides in both ears (`mono`, each at `left_db` / `right_db`, -24…0 dB).
    /// Clips whose sound has one channel are left as they are. Calls with the same `merge` key (a
    /// dragged slider) are one undo step.
    pub fn set_channels(&mut self, clips: &[u64], mode: &str, left_db: f64, right_db: f64, merge: Option<&str>) -> Result<()> {
        let gains = match mode {
            "stereo" => None,
            "left" => Some([100.0, 0.0, 100.0, 0.0]),
            "right" => Some([0.0, 100.0, 0.0, 100.0]),
            "mono" => {
                if !left_db.is_finite() || !right_db.is_finite() {
                    return Err(Error::Invalid("each side's level must be a number".into()));
                }
                let (a, b) = (to_pct(left_db), to_pct(right_db));
                Some([a, b, a, b])
            }
            _ => return Err(Error::Invalid(format!("{mode} is not a way to play the sides: stereo, left, right or mono"))),
        };
        let sound = self.sounds_of(clips)?;
        let two: Vec<u64> = sound.iter().copied().filter(|c| self.source_channels(*c) >= 2).collect();
        if two.is_empty() {
            return Err(Error::Invalid(if sound.is_empty() {
                "select a clip with sound first".into()
            } else {
                "this sound has one channel: it already plays in both ears".into()
            }));
        }
        let label = match mode {
            "left" => "Left Side Only",
            "right" => "Right Side Only",
            "mono" => "Mix Sides",
            _ => "Stereo",
        };
        self.group_merge(label, merge, |s| {
            for c in &two {
                let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
                let at = q.find_item(ClipId(*c)).and_then(|(_, i)| i.effects.iter().position(|e| e.effect == MIXER));
                match (gains, at) {
                    (None, Some(index)) => {
                        s.session.execute("effects.remove", json!({"clip": c, "index": index})).map_err(engine)?;
                    }
                    (None, None) => {}
                    (Some(g), at) => {
                        if at.is_none() {
                            s.session.execute("effects.apply", json!({"clips": [c], "effect": MIXER})).map_err(engine)?;
                        }
                        for (param, v) in ["l_from_l", "l_from_r", "r_from_l", "r_from_r"].iter().zip(g) {
                            s.session.execute("effects.setParam", json!({"clip": c, "effect": MIXER, "param": param, "value": v})).map_err(engine)?;
                        }
                        for param in ["invert_l", "invert_r"] {
                            s.session.execute("effects.setParam", json!({"clip": c, "effect": MIXER, "param": param, "value": false})).map_err(engine)?;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Measures the two sides of clip `clip`'s sound (as recorded, whatever its channels are set
    /// to), to suggest how it should play.
    pub fn channel_levels(&self, clip: u64) -> Result<ChannelLevels> {
        let sound = self.sounds_of(&[clip])?;
        let c = *sound.first().ok_or_else(|| Error::Invalid("this clip has no sound".into()))?;
        if self.source_channels(c) < 2 {
            return Err(Error::Invalid("this sound has one channel: it already plays in both ears".into()));
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let sr = q.settings.sample_rate.max(1);
        let (_, item) = q.find_item(ClipId(c)).ok_or(Error::NothingOpen)?;
        // as recorded: without the clip's own routing
        let mut item = item.clone();
        item.effects.retain(|e| e.effect != MIXER);
        let a0 = item.start.to_units_floor(i64::from(sr));
        let a1 = item.end().to_units_floor(i64::from(sr));
        let most = (MEASURE_SECONDS * f64::from(sr)) as i64;
        let frames = usize::try_from((a1 - a0).clamp(0, most)).unwrap_or(0);
        let provider = self.session.media.full_res_provider(self.session.project.clone(), self.session.services.clone());
        let [l, r] = filmcraft_render::audio::clip_signal(&item, a0, frames, sr, &provider).ok_or_else(|| Error::Invalid("this clip has no sound".into()))?;
        Ok(measure(&l, &r, sr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, amp: f32, sr: u32, secs: f64, on: impl Fn(f64) -> bool) -> Vec<f32> {
        (0..(f64::from(sr) * secs) as usize)
            .map(|k| {
                let t = k as f64 / f64::from(sr);
                if on(t) { amp * (std::f64::consts::TAU * hz * t).sin() as f32 } else { 0.0 }
            })
            .collect()
    }

    #[test]
    fn the_sides_are_measured_and_the_fix_suggested() {
        let sr = 8000;
        // two people taking turns, each on a side, the right one 6 dB quieter
        let l = tone(220.0, 0.5, sr, 4.0, |t| (t as i64) % 2 == 0);
        let r = tone(330.0, 0.25, sr, 4.0, |t| (t as i64) % 2 == 1);
        let m = measure(&l, &r, sr);
        assert_eq!(m.suggest, Some("mono"), "{m:?}");
        assert!(m.likeness < 0.1);
        assert!((m.even_left_db + 6.02).abs() < 0.2 && m.even_right_db == 0.0, "{m:?}");
        // one microphone, on the left
        let m = measure(&l, &vec![0.0; l.len()], sr);
        assert_eq!(m.suggest, Some("left"));
        let m = measure(&vec![0.0; r.len()], &r, sr);
        assert_eq!(m.suggest, Some("right"));
        // the same sound on both sides is fine as it is; silence suggests nothing
        assert_eq!(measure(&l, &l, sr).suggest, None);
        assert_eq!(measure(&[0.0; 800], &[0.0; 800], sr).suggest, None);
        assert_eq!(measure(&[], &[], sr).suggest, None);
    }

    #[test]
    fn levels_and_percents_round_trip() {
        for db in [0.0, -3.0, -6.0, -24.0] {
            assert!((to_db(to_pct(db)) - db).abs() < 0.01, "{db}");
        }
        assert_eq!(to_pct(5.0), 100.0);
        assert_eq!(to_db(0.0), SIDE_DB.0);
    }
}
