//! AI noise reduction: DeepFilterNet 3 (Schröter et al.; the authors' Rust release, MIT OR
//! Apache-2.0, model included), a speech model that keeps voices and takes out everything else:
//! steady hiss and hum, but also traffic, wind and clatter, which the engine's own DeNoise cannot
//! tell from speech. It is too heavy for the engine itself, so ClipCut adds it to the engine's audio
//! effects ([`filmcraft_audio_dsp::register_effect`]) as `ai_denoise`, and clips use it like any
//! other effect: it plays, scrubs and exports as the clip's sound, the engine compensating its delay.
//!
//! The model works at 48 kHz in frames of 480 samples: this buffers whole frames, so the delay is
//! one frame plus the model's look-ahead. At another sample rate, or with more than two channels,
//! the sound goes through as it is. A model that fails (it never should) lets the sound through too,
//! in step, rather than stop playback. The model can't move between threads, so each runs on a
//! worker thread of its own; idle workers are kept and reused, so it is loaded once, not on each seek.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, Once, PoisonError};

use df::tract::{DfParams, DfTract, RuntimeParams};
use filmcraft_audio_dsp::{AudioEffect, Category, EffectInfo, ParamSpec, Unit};
use ndarray::Array2;

/// The engine effect's id.
pub(crate) const ID: &str = "ai_denoise";
const PARAMS: &[ParamSpec] = &[ParamSpec::new("amount", "Amount", 0.0, 100.0, 100.0, Unit::Percent)];
/// The model's sample rate.
const RATE: f32 = 48_000.0;
/// Idle workers kept per channel count.
const KEEP: usize = 8;

/// The least limit: below 0.01 dB the model copies its input straight out, without its delay,
/// which would put the sound out of step with the latency the engine makes up for.
const LEAST_DB: f32 = 0.02;

/// How far the noise is taken down at `amount` (0…100 %): all the way at 100 %, next to nothing
/// (but still through the model, in step) at 0.
fn limit_db(amount: f32) -> f32 {
    if !amount.is_finite() {
        return 100.0;
    }
    if amount >= 99.5 { 100.0 } else { (amount.clamp(0.0, 100.0) * 0.6).max(LEAST_DB) }
}

/// Adds AI noise reduction to the engine's audio effects (once).
pub(crate) fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        filmcraft_audio_dsp::register_effect(EffectInfo {
            id: ID,
            name: "AI Noise Reduction",
            category: Category::Restoration,
            params: PARAMS,
            create: |sr, ch| Box::new(AiDenoise::new(sr, ch)),
        });
    });
}

enum Job {
    /// A frame (channels one after the other) and the noise limit (dB).
    Frame(Vec<f32>, f32),
    /// Start again from silence.
    Reset,
}

/// A thread with its own model (the model can't move between threads; the effect can). Frames go
/// in and come back cleaned, or `None` when the model failed.
struct Worker {
    tx: Sender<Job>,
    rx: Receiver<Option<Vec<f32>>>,
    hop: usize,
    delay: usize,
}

impl Worker {
    /// A worker for `channels` channels, or `None` when the model can't be loaded.
    fn spawn(channels: usize) -> Option<Worker> {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Option<Vec<f32>>>();
        let (ready_tx, ready) = channel::<Option<(usize, usize)>>();
        std::thread::Builder::new()
            .name("ClipCut AI noise reduction".into())
            .spawn(move || {
                let load = || catch_unwind(|| DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(channels)).ok()).ok().flatten();
                let Some(pristine) = load() else {
                    let _ = ready_tx.send(None);
                    return;
                };
                // a fresh state is a copy of the model as loaded (cheap; loading takes 0.2 s)
                let mut m = pristine.clone();
                let (hop, delay) = (m.hop_size, m.hop_size.saturating_mul(m.lookahead.saturating_add(1)));
                if ready_tx.send(Some((hop, delay))).is_err() {
                    return;
                }
                let mut applied = f32::NAN;
                while let Ok(job) = jobs.recv() {
                    match job {
                        Job::Reset => {
                            m = pristine.clone();
                            applied = f32::NAN;
                        }
                        Job::Frame(frame, lim) => {
                            let out = catch_unwind(AssertUnwindSafe(|| {
                                if lim != applied {
                                    m.set_atten_lim(lim);
                                    applied = lim;
                                }
                                let input = Array2::from_shape_vec((channels, hop), frame).ok()?;
                                let mut out = Array2::<f32>::zeros((channels, hop));
                                m.process(input.view(), out.view_mut()).ok()?;
                                out.iter().all(|v| v.is_finite()).then(|| out.into_raw_vec())
                            }))
                            .ok()
                            .flatten();
                            if done.send(out).is_err() {
                                return;
                            }
                        }
                    }
                }
            })
            .ok()?;
        let (hop, delay) = ready.recv().ok().flatten()?;
        Some(Worker { tx, rx, hop, delay })
    }
}

/// Idle workers, by channel count (one or two).
fn pool() -> &'static [Mutex<Vec<Worker>>; 2] {
    static POOL: [Mutex<Vec<Worker>>; 2] = [Mutex::new(Vec::new()), Mutex::new(Vec::new())];
    &POOL
}

/// A worker for `channels`: an idle one (made fresh), or a new one.
fn lease(channels: usize) -> Option<Worker> {
    let slot = pool().get(channels.checked_sub(1)?)?;
    let idle = slot.lock().unwrap_or_else(PoisonError::into_inner).pop();
    match idle {
        Some(w) => w.tx.send(Job::Reset).is_ok().then_some(w),
        None => Worker::spawn(channels),
    }
}

fn give_back(channels: usize, w: Worker) {
    if let Some(slot) = channels.checked_sub(1).and_then(|k| pool().get(k)) {
        let mut v = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if v.len() < KEEP {
            v.push(w);
        }
    }
}

pub(crate) struct AiDenoise {
    worker: Option<Worker>,
    channels: usize,
    hop: usize,
    /// The model's own delay (samples) after a frame goes in.
    delay: usize,
    amount: f32,
    /// The frame being filled, per channel.
    frame: Vec<Vec<f32>>,
    /// Output waiting to go out, per channel (starts with one frame of silence).
    out: Vec<VecDeque<f32>>,
    /// The last `delay` samples in, per channel: the sound as it was, in step, if the model fails.
    dry: Vec<VecDeque<f32>>,
}

impl AiDenoise {
    pub(crate) fn new(sample_rate: f32, channels: usize) -> Self {
        let worker = if (sample_rate - RATE).abs() < 0.5 { lease(channels) } else { None };
        let (hop, delay) = worker.as_ref().map_or((0, 0), |w| (w.hop, w.delay));
        let ch = channels.max(1);
        let mut s = AiDenoise {
            worker,
            channels: ch,
            hop,
            delay,
            amount: 100.0,
            frame: vec![Vec::with_capacity(hop); ch],
            out: vec![VecDeque::new(); ch],
            dry: vec![VecDeque::new(); ch],
        };
        s.clear();
        s
    }

    /// Empty buffers: the frame of silence the output starts with, and silence before the sound.
    fn clear(&mut self) {
        for f in &mut self.frame {
            f.clear();
        }
        for q in &mut self.out {
            q.clear();
            q.extend(std::iter::repeat_n(0.0, self.hop));
        }
        for d in &mut self.dry {
            d.clear();
            d.extend(std::iter::repeat_n(0.0, self.delay));
        }
    }

    /// Runs the model on the full frame; on failure the sound as it was, in step.
    fn run(&mut self) {
        let (ch, hop) = (self.channels, self.hop);
        let mut dry = vec![Vec::with_capacity(hop); ch];
        for (c, d) in self.dry.iter_mut().enumerate() {
            d.extend(self.frame.get(c).map_or(&[][..], Vec::as_slice));
            for _ in 0..hop {
                if let Some(v) = dry.get_mut(c) {
                    v.push(d.pop_front().unwrap_or(0.0));
                }
            }
        }
        let lim = limit_db(self.amount);
        let wet = self.worker.as_ref().and_then(|w| {
            w.tx.send(Job::Frame(self.frame.concat(), lim)).ok()?;
            w.rx.recv().ok().flatten()
        });
        match wet {
            Some(out) if out.len() == ch * hop => {
                for (c, q) in self.out.iter_mut().enumerate() {
                    q.extend(out.get(c * hop..(c + 1) * hop).unwrap_or(&[]));
                }
            }
            _ => {
                // the model failed: from now on the sound goes through as it is
                self.worker = None;
                for (c, q) in self.out.iter_mut().enumerate() {
                    q.extend(dry.get(c).map_or(&[][..], Vec::as_slice));
                }
            }
        }
        for f in &mut self.frame {
            f.clear();
        }
    }
}

impl Drop for AiDenoise {
    fn drop(&mut self) {
        if let Some(w) = self.worker.take() {
            give_back(self.channels, w);
        }
    }
}

impl AudioEffect for AiDenoise {
    fn id(&self) -> &'static str {
        ID
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if id != "amount" {
            return false;
        }
        self.amount = if value.is_finite() { value.clamp(0.0, 100.0) } else { 100.0 };
        true
    }
    fn param(&self, id: &str) -> Option<f32> {
        (id == "amount").then_some(self.amount)
    }
    fn reset(&mut self) {
        // a fresh model state, and the frame of silence the output starts with
        if let Some(w) = &self.worker
            && w.tx.send(Job::Reset).is_err()
        {
            self.worker = None;
        }
        self.clear();
    }
    fn latency(&self) -> usize {
        if self.hop == 0 { 0 } else { self.hop + self.delay }
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        // nothing to do: another sample rate, more channels than the model takes, or no model
        if self.hop == 0 || channels.len() != self.channels {
            return;
        }
        let n = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        for i in 0..n {
            for (c, x) in channels.iter().enumerate() {
                if let (Some(f), Some(v)) = (self.frame.get_mut(c), x.get(i)) {
                    f.push(*v);
                }
            }
            if self.frame.first().is_some_and(|f| f.len() >= self.hop) {
                self.run();
            }
            for (c, x) in channels.iter_mut().enumerate() {
                if let Some(v) = x.get_mut(i) {
                    *v = self.out.get_mut(c).and_then(VecDeque::pop_front).unwrap_or(0.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sentence by the engine's own voice at 48 kHz (peak 0.3), and the noise (white, `snr` dB
    /// below the voice) to put over it.
    fn speech(snr: f64) -> (Vec<f32>, Vec<f32>) {
        let v = filmcraft_tts::voice("basic-male").unwrap();
        let a = v
            .synthesize(
                "The council approved the new transit plan. [pause 1s] Residents said it was long overdue. [pause 1s] Work starts in the spring.",
                &Default::default(),
            )
            .unwrap();
        let k = 48_000.0 / f64::from(a.sample_rate);
        let n = (a.samples.len() as f64 * k) as usize;
        let at = |j: usize| a.samples.get(j).copied().unwrap_or(0.0);
        let clean: Vec<f32> = (0..n)
            .map(|i| {
                let p = i as f64 / k;
                let j = p as usize;
                at(j) + (at(j + 1) - at(j)) * (p - j as f64) as f32
            })
            .collect();
        let peak = clean.iter().fold(0f32, |m, v| m.max(v.abs()));
        let clean: Vec<f32> = clean.iter().map(|v| v / peak * 0.3).collect();
        let mut r: u64 = 7;
        let noise: Vec<f32> = (0..n)
            .map(|_| {
                r ^= r << 13;
                r ^= r >> 7;
                r ^= r << 17;
                (r as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
            })
            .collect();
        let pw = |x: &[f32]| x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64;
        let g = ((pw(&clean) / pw(&noise)) / 10f64.powf(snr / 10.0)).sqrt() as f32;
        (clean, noise.iter().map(|v| v * g).collect())
    }

    /// `x` through the effect in blocks of 256, as the engine runs it.
    fn through(fx: &mut AiDenoise, x: &[f32]) -> Vec<f32> {
        let mut y = x.to_vec();
        for b in y.chunks_mut(256) {
            fx.process(&mut [b]);
        }
        y
    }

    fn db(x: &[f32]) -> f64 {
        10.0 * (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64).max(1e-20).log10()
    }

    #[test]
    fn takes_the_noise_out_between_words_and_keeps_them_in_step() {
        let (clean, noise) = speech(5.0);
        let x: Vec<f32> = clean.iter().zip(&noise).map(|(c, n)| c + n).collect();
        let mut fx = AiDenoise::new(48_000.0, 1);
        let lat = fx.latency();
        assert_eq!(lat, 1920, "one 480-sample frame and the model's look-ahead");
        let y = through(&mut fx, &x);
        // in step: the clean voice lines up with the output at the reported delay
        let a = 48_000;
        let n = 48_000 * 3;
        let corr = |l: usize| clean[a..a + n].iter().zip(&y[a + l..a + l + n]).map(|(u, v)| f64::from(*u) * f64::from(*v)).sum::<f64>();
        let best = (0..4000).max_by(|p, q| corr(*p).total_cmp(&corr(*q))).unwrap_or(0);
        assert_eq!(best, lat, "the voice comes out {best} samples late, not {lat}");
        // 20 ms frames after the first second: the pauses are the quietest fifth of the voice,
        // the words its loudest half
        let f = 960;
        let fr = |v: &[f32], i: usize| db(&v[i * f..(i + 1) * f]);
        let mut idx: Vec<usize> = (48_000 / f..(clean.len() - lat) / f).collect();
        idx.sort_by(|p, q| fr(&clean, *p).total_cmp(&fr(&clean, *q)));
        let mean = |v: &[f32], set: &[usize], shift: usize| {
            10.0 * (set.iter().map(|i| 10f64.powf(db(&v[i * f + shift..(i + 1) * f + shift]) / 10.0)).sum::<f64>() / set.len() as f64).log10()
        };
        let (pauses, words) = (&idx[..idx.len() / 5], &idx[idx.len() / 2..]);
        let (p0, p1) = (mean(&x, pauses, 0), mean(&y, pauses, lat));
        let (w0, w1) = (mean(&x, words, 0), mean(&y, words, lat));
        assert!(p1 < p0 - 20.0, "between words: {p0:.1} dB → {p1:.1} dB");
        assert!(w1 > w0 - 10.0, "the words: {w0:.1} dB → {w1:.1} dB");
        assert!(w1 > p1 + 25.0, "the words stand far above what is left: {w1:.1} vs {p1:.1} dB");
    }

    #[test]
    fn amount_sets_how_far_the_noise_goes_down() {
        let (clean, noise) = speech(5.0);
        let x: Vec<f32> = clean.iter().zip(&noise).map(|(c, n)| c + n).collect();
        let mut gentle = AiDenoise::new(48_000.0, 1);
        gentle.set_param("amount", 20.0);
        let mut full = AiDenoise::new(48_000.0, 1);
        let (g, a) = (through(&mut gentle, &x), through(&mut full, &x));
        // a stretch of the second pause, aligned
        let lat = full.latency();
        let at = |v: &[f32]| db(&v[48_000 * 3 + lat..48_000 * 3 + lat + 2400]);
        assert!(at(&g) > at(&a) + 6.0, "gentle {:.1} dB, full {:.1} dB", at(&g), at(&a));
        assert!(limit_db(f32::NAN) == 100.0 && limit_db(-5.0) == LEAST_DB && limit_db(0.0) == LEAST_DB && limit_db(100.0) == 100.0);
    }

    #[test]
    fn other_rates_and_channel_counts_go_through_as_they_are() {
        let x: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        for (sr, ch) in [(44_100.0, 1), (48_000.0, 6)] {
            let mut fx = AiDenoise::new(sr, ch);
            assert_eq!(fx.latency(), 0);
            let mut bufs = vec![x.clone(); ch];
            let mut refs: Vec<&mut [f32]> = bufs.iter_mut().map(|b| b.as_mut_slice()).collect();
            fx.process(&mut refs);
            assert!(bufs.iter().all(|b| *b == x), "{sr} Hz × {ch}");
        }
        // stereo works, and a reset starts again from silence
        let mut fx = AiDenoise::new(48_000.0, 2);
        assert_eq!(fx.latency(), 1920);
        let (mut l, mut r) = (x.clone(), x.clone());
        fx.process(&mut [&mut l, &mut r]);
        fx.reset();
        let (mut l2, mut r2) = (x.clone(), x.clone());
        fx.process(&mut [&mut l2, &mut r2]);
        assert_eq!(l, l2, "the same after a reset");
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    }
}
