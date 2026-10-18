//! Titles, CapCut's text templates: the engine's graphics templates (lower thirds, titles, end
//! credits, callouts, a ticker) placed at the playhead on a free lane and filled in through their
//! own controls; and any text clip saved as a title of the user's own, kept in the app's data
//! folder and listed with the rest.

use std::path::Path;

use filmcraft_project::{ClipId, ItemKind};
use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::{Value, json};

use crate::lanes::Lane;
use crate::{Error, Result, Studio, engine};

/// The category the user's own titles go in.
pub const MY_TITLES: &str = "My Titles";

/// A title in the Text tab.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleView {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub seconds: f64,
    /// One the user saved (it can be removed).
    pub mine: bool,
}

/// The template a clip was made from, by name, when it was.
pub(crate) fn title_of(i: &filmcraft_project::TrackItem) -> Option<String> {
    i.graphic.as_ref()?.template.as_ref().map(|t| t.name.clone())
}

impl Studio {
    /// Where the user's own titles (and the engine's other per-user files) are kept: `dir`.
    pub fn set_data_dir(&mut self, dir: &Path) {
        self.session.prefs_path = Some(dir.join("preferences.json"));
    }

    /// Every title: the engine's, then the user's own.
    pub fn titles(&mut self) -> Result<Vec<TitleView>> {
        let r = self.session.execute("graphics.template.list", json!({})).map_err(engine)?;
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut v: Vec<TitleView> = r
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| TitleView {
                        id: s(t, "id"),
                        name: s(t, "name"),
                        category: s(t, "category"),
                        description: s(t, "description"),
                        seconds: t.get("seconds").and_then(Value::as_f64).unwrap_or(5.0),
                        mine: t.get("source").and_then(Value::as_str) == Some("user"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        // the user's own last
        v.sort_by_key(|t| t.mine);
        Ok(v)
    }

    /// Title `id`'s picture, `width` pixels wide, as PNG in base64 (transparent where the title is).
    pub fn title_thumbnail(&mut self, id: &str, width: u32) -> Result<String> {
        let r = self.session.execute("graphics.template.thumbnail", json!({"template": id, "width": width.clamp(32, 640)})).map_err(engine)?;
        r.get("pngBase64").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::Engine("the title has no picture".into()))
    }

    /// Adds title `id` at `seconds` on the first free overlay lane, fitted to the canvas. One undo
    /// step. Returns the new clip.
    pub fn add_title(&mut self, id: &str, seconds: f64) -> Result<u64> {
        let t = self.frame_tick(seconds)?;
        let length =
            self.titles()?.into_iter().find(|x| x.id == id).map(|x| x.seconds).ok_or_else(|| Error::Invalid(format!("there is no title called “{id}”")))?;
        self.group("Add Title", |s| {
            s.unlock_lanes()?;
            let lane = s.free_lane(Lane::Video, t, t + Tick::from_seconds_f64(length.max(0.1)), &[])?;
            let r = s.session.execute("graphics.template.apply", json!({"template": id, "time": t.0, "track": lane})).map_err(engine)?;
            r.get("clip").and_then(Value::as_u64).ok_or_else(|| Error::Engine("the title was not made".into()))
        })
    }

    /// The things title clip `clip` lets you change (`[{id, name, kind, value, min, max}]`), with
    /// their values at the playhead time `seconds`.
    pub fn title_controls(&mut self, clip: u64) -> Result<Value> {
        let r = self.session.execute("graphics.template.controls", json!({"clip": clip})).map_err(engine)?;
        Ok(r.get("controls").cloned().unwrap_or(json!([])))
    }

    /// Changes title clip `clip`'s controls (`values`: control id → value). Calls with the same
    /// `merge` key (a dragged slider) fold into one undo step.
    pub fn set_title(&mut self, clip: u64, values: &Value, merge: Option<&str>) -> Result<()> {
        if values.as_object().is_none_or(|o| o.is_empty()) {
            return Err(Error::Invalid("nothing to change".into()));
        }
        let key = merge.map(|m| format!("title:{clip}:{m}"));
        self.group_merge("Change Title", key.as_deref(), |s| {
            s.session.execute("graphics.template.set", json!({"clip": clip, "values": values})).map(|_| ()).map_err(engine)
        })
    }

    /// Saves text clip `clip` (its words, look and animations) as a title of the user's own called
    /// `name`, to use again from the Text tab. Returns its id.
    pub fn save_title(&mut self, clip: u64, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("give the title a name".into()));
        }
        if self.session.prefs_path.is_none() {
            return Err(Error::Invalid("there is nowhere to keep titles yet".into()));
        }
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let is_text =
            q.find_item(ClipId(clip)).and_then(|(_, i)| self.session.project.item(i.item)).is_some_and(|it| matches!(it.kind, ItemKind::Graphic { .. }));
        if !is_text {
            return Err(Error::Invalid("select a text clip to save as a title".into()));
        }
        let r = self.session.execute("graphics.template.export", json!({"clip": clip, "name": name, "category": MY_TITLES})).map_err(engine)?;
        r.get("id").or_else(|| r.get("template")).and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::Engine("the title was not saved".into()))
    }

    /// Removes title `id`, one of the user's own.
    pub fn remove_title(&mut self, id: &str) -> Result<()> {
        if !self.titles()?.iter().any(|t| t.id == id && t.mine) {
            return Err(Error::Invalid("only titles you saved can be removed".into()));
        }
        self.session.execute("graphics.template.remove", json!({"template": id})).map(|_| ()).map_err(engine)
    }
}
