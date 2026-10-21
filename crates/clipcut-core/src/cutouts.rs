//! Remove background on any picture clip: the cut-out (a PNG of the same size with the background
//! transparent, made in the page by Clip Studio's `cutout.js`) takes the place of the clip's
//! picture, and everything else stays: place, size, keyframes, effects, timing. A clip marker
//! named "ClipCut cut-out" keeps the original picture's media item, so Put background back swaps
//! it in again and a second cut-out (another quality) starts from the original.

use filmcraft_project::{ClipId, ItemId, Label, Marker, MarkerId, MarkerKind, TrackItem};
use filmcraft_time::Tick;

use crate::{Error, Result, Studio, engine};

const CUT: &str = "ClipCut cut-out";

/// The original picture (media item) of a clip whose background was removed.
pub(crate) fn cut_from(i: &TrackItem) -> Option<u64> {
    i.markers.iter().find(|m| m.name == CUT).and_then(|m| m.comment.trim().parse().ok())
}

impl Studio {
    /// The still picture clip `clip` plays (a photo, not a video or a graphic), and its original
    /// before any cut-out: (clip, original media item).
    fn picture_clip(&self, clip: u64) -> Result<(TrackItem, u64)> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let (track, i) = q.find_item(ClipId(clip)).ok_or_else(|| Error::Invalid("that clip is no longer on the timeline".into()))?;
        if !q.video_tracks.iter().any(|t| t.id == track) {
            return Err(Error::Invalid("only a picture can have its background removed".into()));
        }
        let still = self
            .session
            .project
            .resolve_media(i.item)
            .is_some_and(|(_, m, _)| matches!(m.info.kind, filmcraft_media::MediaKind::Still) && m.info.video.is_some());
        if !still {
            return Err(Error::Invalid("Remove background works on photos and pictures; for a video, use Remove a colour".into()));
        }
        let original = cut_from(i).filter(|o| self.session.project.item(ItemId(*o)).is_some()).unwrap_or(i.item.0);
        Ok((i.clone(), original))
    }

    /// The media item clip `clip`'s background was removed from (or its own, when it wasn't):
    /// the picture a new cut-out is made of.
    pub fn cutout_source(&self, clip: u64) -> Result<u64> {
        self.picture_clip(clip).map(|(_, o)| o)
    }

    /// Puts the cut-out `png` (the same size as the clip's picture) in place of clip `clip`'s
    /// picture. One undo step.
    pub fn cut_out(&mut self, clip: u64, png: &str) -> Result<()> {
        let (_, original) = self.picture_clip(clip)?;
        let size = self.picture_size(ItemId(original));
        self.group("Remove Background", |s| {
            let item = s.import(png)?;
            if s.picture_size(ItemId(item)) != size {
                return Err(Error::Invalid("the cut-out came back at another size than the picture: try again".into()));
            }
            s.swap_picture(clip, item, Some(original))
        })
    }

    /// Clip `clip` shows its picture as it was before Remove background. One undo step.
    pub fn put_background_back(&mut self, clip: u64) -> Result<()> {
        let (i, original) = self.picture_clip(clip)?;
        if original == i.item.0 {
            return Err(Error::Invalid("this picture's background wasn't removed".into()));
        }
        self.group("Put Background Back", |s| s.swap_picture(clip, original, None))
    }

    /// Clip `clip` plays media `item`, marked as a cut-out of `original` (or unmarked).
    fn swap_picture(&mut self, clip: u64, item: u64, original: Option<u64>) -> Result<()> {
        self.session
            .edit_sequence("Remove Background", |q, ctx, _| {
                let Some((_, i)) = q.find_item_mut(ClipId(clip)) else { return Ok(()) };
                i.item = ItemId(item);
                i.markers.retain(|m| m.name != CUT);
                if let Some(o) = original {
                    i.markers.push(Marker {
                        id: MarkerId(ctx.alloc()),
                        start: i.source_in,
                        duration: Tick::ZERO,
                        name: CUT.into(),
                        comment: o.to_string(),
                        kind: MarkerKind::Comment,
                        color: Label::Violet,
                    });
                }
                Ok(())
            })
            .map_err(engine)
    }
}
