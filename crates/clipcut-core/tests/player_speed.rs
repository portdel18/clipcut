//! How fast the player draws a 4K HLG HEVC 10-bit 60 fps clip, as Check playback times it (draft,
//! every other frame, 3 workers), and one worker in order. Run by hand:
//! `PLAYER_CLIP=path.mp4 cargo test -p clipcut-core --test player_speed -- --ignored --nocapture`

use std::time::{Duration, Instant};

use clipcut_core::Studio;

#[test]
#[ignore]
fn player_speed() {
    let Ok(path) = std::env::var("PLAYER_CLIP") else { return };
    let mut s = Studio::new();
    s.add_clip(&path).unwrap();
    let feed = s.frame_feed(960, "auto").unwrap();
    let first = feed.render(0, true).unwrap();
    eprintln!("player picture {}x{}", first.w, first.h);
    // one worker, in order
    let t = Instant::now();
    let mut n = 0;
    for k in (60..120).step_by(2) {
        feed.render(k, true).unwrap();
        n += 1;
    }
    eprintln!("in order: {:.1} pictures a second", n as f64 / t.elapsed().as_secs_f64());
    // drawing alone: a frame the decoder has just given (cached)
    feed.render(100, true).unwrap();
    let t = Instant::now();
    for _ in 0..10 {
        feed.render(100, true).unwrap();
    }
    eprintln!("drawing a cached frame: {:.0} ms", t.elapsed().as_secs_f64() * 100.0);
    if std::env::var("PLAYER_DRAW_LOOP").is_ok() {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(40) {
            feed.render(100, true).unwrap();
        }
        return;
    }
    // the decoding alone, in order, every other frame, at the player's size
    let probe = s.media_probes(1).into_iter().next().unwrap();
    let rate = 60.0;
    let t = Instant::now();
    let mut n = 0;
    filmcraft_media::cancel::with_draft(true, || {
        for k in (240..300).step_by(2) {
            let f =
                probe.source.video_frame(filmcraft_media::FrameRequest { time: filmcraft_time::Tick::from_seconds_f64(k as f64 / rate), scale: 0.5 }).unwrap();
            if n == 0 {
                eprintln!("decoded {}x{} {:?}", f.width, f.height, std::mem::discriminant(&f.data));
            }
            n += 1;
        }
    });
    eprintln!("decode only: {:.1} pictures a second", n as f64 / t.elapsed().as_secs_f64());
    // as the check does it
    let g0 = filmcraft_codecs::gop::gop_stats();
    let next = std::sync::atomic::AtomicI64::new(120);
    let drawn = std::sync::atomic::AtomicU32::new(0);
    let t = Instant::now();
    std::thread::scope(|sc| {
        for _ in 0..3 {
            sc.spawn(|| {
                while t.elapsed() < Duration::from_secs(20) {
                    let k = next.fetch_add(2, std::sync::atomic::Ordering::Relaxed);
                    if k >= 230 {
                        break;
                    }
                    let a = Instant::now();
                    feed.render(k, true).unwrap();
                    drawn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if a.elapsed() > Duration::from_millis(300) {
                        eprintln!("  frame {k}: {} ms", a.elapsed().as_millis());
                    }
                }
            });
        }
    });
    eprintln!("3 workers: {:.1} pictures a second", f64::from(drawn.into_inner()) / t.elapsed().as_secs_f64());
    let g = filmcraft_codecs::gop::gop_stats();
    eprintln!(
        "decoder: {} hits, {} misses, {} seeks, {} decoded, {} evicted, {:.0} ms decoding",
        g.hits - g0.hits,
        g.misses - g0.misses,
        g.seeks - g0.seeks,
        g.decoded - g0.decoded,
        g.evicted - g0.evicted,
        (g.decode_ns - g0.decode_ns) as f64 / 1e6
    );
}
