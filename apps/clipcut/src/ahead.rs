//! Rendering ahead while the video plays: worker threads render the next frames from a
//! [`FrameFeed`] (a snapshot of the sequence, so they never wait for the project) into a small
//! cache, a few frames past the playhead. The player shows the frame for the sound's clock when
//! it is ready, else the latest one before it; frames the playhead has passed are skipped, so a
//! slow frame drops frames instead of slowing the picture down.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use clipcut_core::{Frame, FrameFeed};

/// Frames rendered ahead of the playhead.
const LOOKAHEAD: i64 = 8;

pub struct Ahead {
    feed: FrameFeed,
    frames: Mutex<BTreeMap<i64, Arc<Frame>>>,
    /// The next frame a worker takes.
    next: AtomicI64,
    /// Every how many frames one is rendered (a shuttle speed skips the rest).
    step: i64,
    /// The frame showing now.
    head: AtomicI64,
    stop: AtomicBool,
    threads: Mutex<Vec<JoinHandle<()>>>,
    /// The player size and quality the feed was made for.
    pub max_width: u32,
    pub quality: String,
}

impl Ahead {
    /// Starts `workers` threads rendering from `seconds`, one frame in every `step`.
    pub fn start(feed: FrameFeed, seconds: f64, workers: usize, step: i64, max_width: u32, quality: &str) -> Arc<Self> {
        let first = feed.frame_at(seconds);
        let me = Arc::new(Ahead {
            feed,
            frames: Mutex::default(),
            next: AtomicI64::new(first),
            step: step.clamp(1, 8),
            head: AtomicI64::new(first),
            stop: AtomicBool::new(false),
            threads: Mutex::default(),
            max_width,
            quality: quality.to_string(),
        });
        let threads = (0..workers.max(1))
            .filter_map(|k| {
                let a = me.clone();
                std::thread::Builder::new()
                    .name(format!("clipcut-ahead-{k}"))
                    .spawn(move || {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.work()));
                    })
                    .ok()
            })
            .collect();
        *me.threads.lock().unwrap_or_else(PoisonError::into_inner) = threads;
        me
    }

    fn work(&self) {
        while !self.stop.load(Ordering::Acquire) {
            let head = self.head.load(Ordering::Acquire);
            let cur = self.next.load(Ordering::Acquire);
            // never a frame the playhead has passed
            let k = cur.max(head);
            if k >= head + LOOKAHEAD * self.step || k >= self.feed.frames {
                std::thread::sleep(Duration::from_millis(3));
                continue;
            }
            if self.next.compare_exchange(cur, k + self.step, Ordering::AcqRel, Ordering::Acquire).is_err() {
                continue;
            }
            let Ok(f) = self.feed.render(k, true) else { continue };
            let mut m = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
            m.insert(k, Arc::new(f));
            // frames the playhead passed go, but the newest of them stays: it is what shows when
            // the next frame is late
            let head = self.head.load(Ordering::Acquire);
            if let Some(&keep) = m.range(..=head).next_back().map(|(k, _)| k) {
                m.retain(|&n, _| n >= keep);
            }
        }
    }

    /// The frame to show at `seconds`: that frame if it is ready, else the latest before it, else
    /// (when the workers have nothing after 0.1 s) that frame rendered here.
    pub fn frame(&self, seconds: f64) -> Option<Arc<Frame>> {
        let k = self.feed.frame_at(seconds);
        self.head.fetch_max(k, Ordering::AcqRel);
        // just after starting there is nothing yet: wait a little for the workers' first frame
        // rather than decoding the same frame alongside them
        for _ in 0..25 {
            if let Some(f) = self.frames.lock().unwrap_or_else(PoisonError::into_inner).range(..=k).next_back().map(|(_, f)| f.clone()) {
                return Some(f);
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        self.feed.render(k, true).ok().map(Arc::new)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let threads = std::mem::take(&mut *self.threads.lock().unwrap_or_else(PoisonError::into_inner));
        for t in threads {
            let _ = t.join();
        }
    }
}

impl Drop for Ahead {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Plays three seconds of a 1080p clip on a 30 fps clock and counts the frames shown on time.
    #[test]
    #[ignore = "a measurement: needs target/fixtures/bench/hd.mp4"]
    fn frames_are_ready_on_time() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/fixtures/bench/hd.mp4");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let mut s = clipcut_core::Studio::new();
        s.add_clip(path).unwrap();
        for workers in [1, 2, 3] {
            let a = Ahead::start(s.frame_feed(960, "auto").unwrap(), 1.0, workers, 1, 960, "auto");
            let t0 = Instant::now();
            let (mut on_time, mut shown) = (0, 0);
            while t0.elapsed().as_secs_f64() < 3.0 {
                let t = 1.0 + t0.elapsed().as_secs_f64();
                let want = a.feed.frame_at(t);
                let f = a.frame(t).unwrap();
                shown += 1;
                let m = a.frames.lock().unwrap();
                if m.get(&want).is_some_and(|x| Arc::ptr_eq(x, &f)) {
                    on_time += 1;
                }
                drop(m);
                std::thread::sleep(Duration::from_millis(16));
            }
            a.stop();
            println!("{workers} workers: {on_time} of {shown} requests had their exact frame ready");
        }
        // the old way: render each frame when it is asked for
        let t0 = Instant::now();
        let mut n = 0;
        while t0.elapsed().as_secs_f64() < 3.0 {
            s.preview_frame(1.0 + t0.elapsed().as_secs_f64(), 960, "auto").unwrap();
            n += 1;
        }
        println!("rendering on demand: {:.1} frames a second", f64::from(n) / 3.0);
    }
}
