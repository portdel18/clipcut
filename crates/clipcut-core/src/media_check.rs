//! Footage whose picture the engine can't decode (H.264 in 10-bit, 4:2:2 or 4:4:4 colour,
//! interlaced video, codecs without a decoder): the engine shows its "unreadable" slate and keeps
//! the sound. ClipCut says so when the file is added, in the user's words, with how to fix it,
//! instead of leaving a red slate to be found on the timeline.

use filmcraft_project::ItemId;
use filmcraft_render::offline::OfflineReason;

use crate::Studio;

/// What the engine's reason means for someone holding the file.
pub fn plain_reason(engine: &str) -> String {
    let e = engine.to_ascii_lowercase();
    let what = if e.contains("chroma_format_idc 2") {
        "4:2:2 colour (a camera's high-quality mode)"
    } else if e.contains("chroma_format_idc 3") {
        "4:4:4 colour (a screen recorder's or camera's top-quality mode)"
    } else if e.contains("bit depth") {
        "10-bit colour (a camera's or phone's HDR or log mode)"
    } else if e.contains("interlaced") || e.contains("field") || e.contains("mbaff") {
        "interlaced video (1080i, from a camcorder or TV)"
    } else if e.contains("lossless") {
        "lossless H.264"
    } else {
        "in a format ClipCut can't decode yet"
    };
    format!(
        "ClipCut can't show its picture yet: it's {what}. Its sound still plays. To use the picture, convert it to H.264 MP4 \
         (8-bit, for example HandBrake's “Fast 1080p30” preset) and add the copy."
    )
}

impl Studio {
    /// Decodes media item `item`'s first frame, as the player will, and returns why its picture
    /// can't be shown (see [`plain_reason`]), or None when it can (or it has no picture).
    pub fn check_picture(&self, item: u64) -> Option<String> {
        let has_video = self.session.project.resolve_media(ItemId(item)).is_some_and(|(_, m, _)| m.info.video.is_some());
        if !has_video {
            return None;
        }
        // the engine notes the reason the first time a frame fails; the frame itself is the slate
        let _ = self.thumbs().frame(item, 0.0, 64);
        self.session.media.offline_status(ItemId(item)).filter(|st| st.reason == OfflineReason::Unreadable).map(|st| plain_reason(&st.error))
    }
}

#[cfg(test)]
mod tests {
    use super::plain_reason;

    #[test]
    fn reasons_are_in_plain_words() {
        assert!(plain_reason("H.264: chroma_format_idc 2 (only 4:2:0 is implemented)").contains("4:2:2 colour"));
        assert!(plain_reason("H.264: bit depth luma 10 chroma 10 (only 8-bit is implemented)").contains("10-bit"));
        assert!(plain_reason("H.264: interlaced (field / MBAFF) coding").contains("interlaced"));
        assert!(plain_reason("no decoder for mp4v").contains("can't decode yet"));
        assert!(plain_reason("").contains("convert it to H.264"));
    }
}
