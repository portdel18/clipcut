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
    eprintln!("player picture {}x{}", first.width, first.height);
    // one worker, in order
    let t = Instant::now();
    let mut n = 0;
    for k in (60..120).step_by(2) {
        feed.render(k, true).unwrap();
        n += 1;
    }
    eprintln!("in order: {:.1} pictures a second", n as f64 / t.elapsed().as_secs_f64());
    // as the check does it
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
}
