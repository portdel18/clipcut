//! The mixer: each sound lane's volume, pan, mute and solo, and the Mix's volume, on the engine's
//! mixer strips (`A1` is the main track's sound, `A2`… the lanes below it, `Mix` everything); and
//! the level of each while the video plays.

use std::sync::Arc;

use filmcraft_project::TrackId;
use filmcraft_project::mixer::MASTER_STRIP;
use filmcraft_render::mixer::LiveMix;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Error, Result, Studio, engine};

/// The Mix's settings, as the Mixer tab shows them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MixView {
    pub volume_db: f64,
}

/// One strip's peak level since the last reading, left and right (linear, 0…1 and over).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    /// `A1`, `A2`… or `Mix`.
    pub lane: String,
    pub left: f32,
    pub right: f32,
}

/// The levels of a playing [`crate::AudioFeed`], read from any thread.
#[derive(Clone)]
pub struct Meters {
    pub(crate) live: Arc<LiveMix>,
    pub(crate) lanes: Vec<(TrackId, String)>,
}

impl Meters {
    /// Each strip's peak since the last call (strips that played nothing are left out).
    pub fn take(&self) -> Vec<Level> {
        let mut m = self.live.take_meters();
        self.lanes
            .iter()
            .filter_map(|(id, lane)| {
                let peaks = m.remove(id)?;
                let left = peaks.first().copied().unwrap_or(0.0);
                Some(Level { lane: lane.clone(), left, right: peaks.get(1).copied().unwrap_or(left) })
            })
            .collect()
    }
}

/// The strips of `seq` by lane name.
pub(crate) fn strip_lanes(seq: &filmcraft_project::Sequence) -> Vec<(TrackId, String)> {
    let mut v: Vec<(TrackId, String)> = seq.audio_tracks.iter().enumerate().map(|(n, t)| (t.id, format!("A{}", n + 1))).collect();
    v.push((MASTER_STRIP, "Mix".into()));
    v
}

impl Studio {
    pub(crate) fn mix_view(&self) -> MixView {
        let vol = self.session.active_sequence().and_then(|q| q.strip(MASTER_STRIP).map(|t| t.volume_db)).unwrap_or(0.0);
        MixView { volume_db: vol }
    }

    /// Sets sound lane `lane`'s (or the `Mix`'s) `volumeDb`, `pan` (−100 left … 100 right),
    /// `muted` and `solo`. Calls with the same `merge` key (a dragged fader) are one undo step.
    pub fn set_mixer(&mut self, lane: &str, p: &Value, merge: Option<&str>) -> Result<()> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let mix = lane.eq_ignore_ascii_case("mix");
        if !mix && !strip_lanes(q).iter().any(|(_, l)| l.eq_ignore_ascii_case(lane)) {
            return Err(Error::Invalid(format!("{lane} is not a sound lane")));
        }
        let obj = p.as_object().ok_or_else(|| Error::Invalid("nothing to change".into()))?;
        let mut set = serde_json::Map::new();
        for (k, v) in obj {
            match k.as_str() {
                "volumeDb" | "pan" => {
                    let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| Error::Invalid(format!("{k} must be a number")))?;
                    set.insert(k.clone(), json!(x));
                }
                "muted" | "solo" if !mix => {
                    set.insert(k.clone(), json!(v.as_bool().ok_or_else(|| Error::Invalid(format!("{k} is on or off")))?));
                }
                _ => return Err(Error::Invalid(format!("the {} has no “{k}”", if mix { "Mix" } else { "lane" }))),
            }
        }
        set.insert("strip".into(), json!(if mix { "Mix" } else { lane }));
        let label = if set.contains_key("solo") {
            "Solo"
        } else if set.contains_key("muted") {
            "Mute"
        } else {
            "Mixer"
        };
        let key = merge.map(|m| format!("mixer:{lane}:{m}"));
        self.group_merge(label, key.as_deref(), |s| s.session.execute("mixer.setStrip", Value::Object(set)).map(|_| ()).map_err(engine))
    }
}
