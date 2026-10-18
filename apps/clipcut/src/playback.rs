//! Playback: the sequence's sound on the default output device, which is also the clock the picture
//! follows (the interface asks for [`Player::position`] and shows that frame). Without an output
//! device, playback is silent and runs on the wall clock.
//!
//! Two threads per play: one owns the `cpal` stream (not `Send` on every platform, so it never
//! leaves its thread), the other mixes the sequence ahead into a small queue at the device's sample
//! rate (linear resampling when the device and the sequence rates differ). The device callback
//! only copies from that queue, so a slow mix shows up as a short gap, never a stalled callback.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clipcut_core::{AudioFeed, Level, Meters};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// How far ahead the mixer runs, in seconds.
const AHEAD: f64 = 0.3;
/// Device frames mixed per step.
const CHUNK: usize = 1024;

struct Clock {
    /// Device frames actually played (queue underruns are not counted, so the picture waits too).
    played: AtomicU64,
    /// The device's sample rate once the stream is open; 0 until then.
    rate: AtomicU32,
    /// Set when no device could be opened: the wall clock runs instead.
    silent: AtomicBool,
    started: Instant,
}

struct Running {
    from: f64,
    end: f64,
    clock: Arc<Clock>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// The feed the mixer thread plays; swapped by [`Player::update`].
    feed: Arc<Mutex<Arc<AudioFeed>>>,
    meters: Meters,
}

impl Running {
    fn position(&self) -> f64 {
        let c = &self.clock;
        let rate = c.rate.load(Ordering::Acquire);
        let t = if rate > 0 {
            self.from + c.played.load(Ordering::Acquire) as f64 / rate as f64
        } else if c.silent.load(Ordering::Acquire) {
            self.from + c.started.elapsed().as_secs_f64()
        } else {
            self.from
        };
        t.min(self.end)
    }
}

/// The playback state the interface polls.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct State {
    pub seconds: f64,
    pub playing: bool,
    /// False when playing without sound (no output device).
    pub sound: bool,
}

#[derive(Default)]
pub struct Player {
    run: Option<Running>,
    last: f64,
}

impl Player {
    /// Starts playing `feed` from `from` seconds; stops by itself at `end`.
    pub fn play(&mut self, feed: AudioFeed, from: f64, end: f64) {
        self.stop();
        let from = if from.is_finite() { from.clamp(0.0, end.max(0.0)) } else { 0.0 };
        let clock = Arc::new(Clock { played: AtomicU64::new(0), rate: AtomicU32::new(0), silent: AtomicBool::new(false), started: Instant::now() });
        let stop = Arc::new(AtomicBool::new(false));
        let queue: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let meters = feed.meters();
        let feed = Arc::new(Mutex::new(Arc::new(feed)));
        let mut threads = Vec::new();
        {
            let (clock, stop, queue) = (clock.clone(), stop.clone(), queue.clone());
            threads.push(std::thread::spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| output(&clock, &stop, &queue))).is_err() {
                    clock.silent.store(true, Ordering::Release);
                }
            }));
        }
        {
            let (feed, clock, stop, queue) = (feed.clone(), clock.clone(), stop.clone(), queue.clone());
            threads.push(std::thread::spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mix(&feed, from, &clock, &stop, &queue)));
            }));
        }
        self.run = Some(Running { from, end, clock, stop, threads, feed, meters });
    }

    /// Goes on playing with `feed` (the sequence after a mixer change) from where it is: the
    /// change is heard once the sound already mixed ahead has played.
    pub fn update(&mut self, feed: AudioFeed) {
        if let Some(r) = &mut self.run {
            r.meters = feed.meters();
            *r.feed.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(feed);
        }
    }

    pub fn playing(&self) -> bool {
        self.run.is_some()
    }

    /// Each sound lane's and the Mix's peak level since the last reading (nothing when stopped).
    pub fn meters(&self) -> Vec<Level> {
        self.run.as_ref().map(|r| r.meters.take()).unwrap_or_default()
    }

    /// Stops playing and keeps the position.
    pub fn stop(&mut self) {
        if let Some(r) = self.run.take() {
            self.last = r.position();
            r.stop.store(true, Ordering::Release);
            for t in r.threads {
                let _ = t.join();
            }
        }
    }

    pub fn state(&mut self) -> State {
        let Some(r) = &self.run else { return State { seconds: self.last, playing: false, sound: true } };
        let seconds = r.position();
        let sound = !r.clock.silent.load(Ordering::Acquire);
        if seconds >= r.end {
            self.stop();
            return State { seconds: self.last, playing: false, sound };
        }
        State { seconds, playing: true, sound }
    }

    /// Where a stopped player resumes.
    pub fn set_position(&mut self, seconds: f64) {
        if self.run.is_none() && seconds.is_finite() {
            self.last = seconds.max(0.0);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Owns the output stream until `stop`. Marks the clock silent when no device can be opened.
fn output(clock: &Arc<Clock>, stop: &Arc<AtomicBool>, queue: &Arc<Mutex<VecDeque<f32>>>) {
    match open_stream(clock, queue) {
        Ok(stream) => {
            while !stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(15));
            }
            drop(stream);
        }
        Err(e) => {
            eprintln!("clipcut: playing without sound: {e}");
            clock.silent.store(true, Ordering::Release);
        }
    }
}

fn open_stream(clock: &Arc<Clock>, queue: &Arc<Mutex<VecDeque<f32>>>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or("no output device")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let channels = usize::from(cfg.channels());
    let rate = cfg.sample_rate().0;
    if channels == 0 || rate == 0 {
        return Err("the output device reports no channels or sample rate".into());
    }
    let config: cpal::StreamConfig = cfg.clone().into();
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&dev, &config, channels, clock, queue),
        cpal::SampleFormat::I16 => build::<i16>(&dev, &config, channels, clock, queue),
        cpal::SampleFormat::I32 => build::<i32>(&dev, &config, channels, clock, queue),
        cpal::SampleFormat::U16 => build::<u16>(&dev, &config, channels, clock, queue),
        cpal::SampleFormat::U8 => build::<u8>(&dev, &config, channels, clock, queue),
        cpal::SampleFormat::F64 => build::<f64>(&dev, &config, channels, clock, queue),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    clock.rate.store(rate, Ordering::Release);
    Ok(stream)
}

fn build<T>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    clock: &Arc<Clock>,
    queue: &Arc<Mutex<VecDeque<f32>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let (clock, queue) = (clock.clone(), queue.clone());
    dev.build_output_stream(
        config,
        move |buf: &mut [T], _| {
            buf.fill(T::EQUILIBRIUM);
            // Never block the device thread: skip this buffer if the mixer holds the queue
            let Ok(mut q) = queue.try_lock() else { return };
            let mut played = 0u64;
            for frame in buf.chunks_mut(channels) {
                if q.len() < 2 {
                    break;
                }
                let (l, r) = (q.pop_front().unwrap_or(0.0), q.pop_front().unwrap_or(0.0));
                match frame {
                    [m] => *m = T::from_sample((l + r) * 0.5),
                    [a, b, ..] => {
                        *a = T::from_sample(l);
                        *b = T::from_sample(r);
                    }
                    [] => {}
                }
                played += 1;
            }
            clock.played.fetch_add(played, Ordering::Release);
        },
        |e| eprintln!("clipcut: audio output: {e}"),
        None,
    )
}

/// Keeps the queue about [`AHEAD`] seconds full, resampling the sequence mix to the device rate.
fn mix(slot: &Mutex<Arc<AudioFeed>>, from: f64, clock: &Arc<Clock>, stop: &Arc<AtomicBool>, queue: &Arc<Mutex<VecDeque<f32>>>) {
    // Wait for the device rate (or for the news that there is no device)
    let rate = loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        if clock.silent.load(Ordering::Acquire) {
            return mix_silent(slot, from, clock, stop);
        }
        let r = clock.rate.load(Ordering::Acquire);
        if r > 0 {
            break r;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let src_rate = slot.lock().unwrap_or_else(PoisonError::into_inner).sample_rate.max(1);
    let step = f64::from(src_rate) / f64::from(rate);
    let target = ((AHEAD * f64::from(rate)) as usize).max(CHUNK) * 2;
    let mut pos = from * f64::from(src_rate);
    let mut out = Vec::with_capacity(CHUNK * 2);
    while !stop.load(Ordering::Acquire) {
        let len = queue.lock().unwrap_or_else(PoisonError::into_inner).len();
        if len >= target {
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        // Source samples covering CHUNK device frames, plus one for interpolation
        let first = pos.floor() as i64;
        let need = (CHUNK as f64 * step).ceil() as usize + 2;
        let feed = slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let src = feed.mix(first, need);
        out.clear();
        for i in 0..CHUNK {
            let p = pos + i as f64 * step - first as f64;
            let k = p.floor().max(0.0) as usize;
            let f = (p - k as f64) as f32;
            for c in 0..2 {
                let a = src.get(k * 2 + c).copied().unwrap_or(0.0);
                let b = src.get((k + 1) * 2 + c).copied().unwrap_or(a);
                out.push(a + (b - a) * f);
            }
        }
        pos += CHUNK as f64 * step;
        queue.lock().unwrap_or_else(PoisonError::into_inner).extend(out.iter().copied());
    }
}

/// Without a device: mixes along with the wall clock and drops the sound, so the meters still move.
fn mix_silent(slot: &Mutex<Arc<AudioFeed>>, from: f64, clock: &Arc<Clock>, stop: &Arc<AtomicBool>) {
    let rate = f64::from(slot.lock().unwrap_or_else(PoisonError::into_inner).sample_rate.max(1));
    let mut pos = (from * rate) as i64;
    while !stop.load(Ordering::Acquire) {
        let now = ((from + clock.started.elapsed().as_secs_f64()) * rate) as i64;
        if pos >= now {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let n = usize::try_from(now - pos).unwrap_or(0).min(CHUNK * 4);
        let feed = slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let _ = feed.mix(pos, n);
        pos = pos.saturating_add(n as i64);
    }
}
