//! Thumbnails never move the player's decoder: a filmstrip picture is a jump to another moment of
//! the file, and on the player's source the next picture the player wanted then meant going back
//! to a keyframe (the 0.4.10 report: 17 such jumps in two seconds of playback).

use std::path::PathBuf;
use std::process::Command;

use clipcut_core::Studio;
use filmcraft_testkit::fixtures::{fixtures_dir, generate};
use filmcraft_testkit::oracle::ffmpeg_or_skip;

/// 3 s of 4K at 30 fps, one keyframe at the start: the decoder's cache holds only a few dozen 4K
/// pictures, so a thumbnail away from the player's position has to be decoded, as a phone
/// video's does.
fn clip() -> Option<PathBuf> {
    let ff = ffmpeg_or_skip("clipcut-core thumbs")?;
    let out = fixtures_dir("clipcut-core").join("long-gop-4k.mp4");
    generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=3840x2160:rate=30:duration=3"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "300", "-bf", "2", "-preset", "ultrafast"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn a_thumbnail_leaves_the_players_decoder_where_it_was() {
    let Some(v) = clip() else { return };
    let mut s = Studio::new();
    s.add_clip(v.to_str().unwrap()).unwrap();
    let item = s.timeline().clips[0].item;
    let feed = s.frame_feed(640, "auto").unwrap();
    // playing from 1.5 s
    for k in [45, 47, 49] {
        feed.render(k, true).unwrap();
    }
    // the filmstrip wants pictures of the start (the clip before it on the timeline, the Media
    // panel's card)
    let thumbs = s.thumbs();
    for t in [0.1, 0.3] {
        let f = thumbs.frame(item, t, 160).unwrap();
        assert!(f.w > 0 && f.w < 3840, "{}", f.w);
    }
    // the player carries on from where it was, without going back to the keyframe
    let before = filmcraft_codecs::gop_stats();
    for k in [51, 53, 55] {
        feed.render(k, true).unwrap();
    }
    let after = filmcraft_codecs::gop_stats();
    // shared, its decoder would now be at the start: 47 pictures to decode to get back (measured)
    assert!(after.decoded - before.decoded < 20, "{} pictures decoded for three", after.decoded - before.decoded);
    assert_eq!(after.seeks - before.seeks, 0, "the player had to go back to a keyframe");
    // and a picture just behind it is still in its cache
    let before = filmcraft_codecs::gop_stats();
    feed.render(54, true).unwrap();
    let after = filmcraft_codecs::gop_stats();
    assert_eq!((after.hits - before.hits, after.seeks - before.seeks), (1, 0));
}
