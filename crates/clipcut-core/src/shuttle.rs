//! Shuttle sound: the sequence played faster (L pressed again: 1.25×, 1.5×… 8×) with its pitch
//! kept, the same time-stretch Maintain Audio Pitch uses (`filmcraft_audio_dsp::stretch`). The
//! mix is pulled from the [`AudioFeed`] in one contiguous run, as at normal speed (its effects see
//! one stream), and kept long enough for the stretch to read around each frame.

use std::collections::VecDeque;

use filmcraft_audio_dsp::stretch::Wsola;

use crate::AudioFeed;

/// Frame shifts kept for the frames still overlapping the next block.
const KEPT: usize = 64;

/// The sequence's sound from one place on, at `speed` × with its pitch kept, block by block.
pub struct Stretcher {
    w: Wsola,
    speed: f64,
    /// Sequence sample under output sample 0.
    origin: i64,
    /// The mix from sequence sample `start` on, per side, and the two summed.
    start: i64,
    left: Vec<f32>,
    right: Vec<f32>,
    mono: Vec<f32>,
    /// The last frames' shifts, oldest first.
    shifts: VecDeque<(i64, i64)>,
    /// The next output sample.
    next: i64,
}

impl Stretcher {
    /// Plays from sequence sample `from` at `speed` (kept within 0.25–8).
    pub fn new(sample_rate: u32, from: i64, speed: f64) -> Self {
        let speed = if speed.is_finite() { speed.clamp(0.25, 8.0) } else { 1.0 };
        Self {
            w: Wsola::new(sample_rate),
            speed,
            origin: from,
            start: from,
            left: Vec::new(),
            right: Vec::new(),
            mono: Vec::new(),
            shifts: VecDeque::new(),
            next: 0,
        }
    }

    /// The next `n` output samples, interleaved L R L R.
    pub fn next(&mut self, feed: &AudioFeed, n: usize) -> Vec<f32> {
        let n = n.min(1 << 16);
        let (k0, k1) = self.w.frames_for(self.next, n);
        // carry on from the last frame whose shift is known (a fresh start begins unshifted)
        let from = self.shifts.back().copied().filter(|(k, _)| *k >= k0 - 1).unwrap_or((k0 - 1, 0));
        let (lo, len) = self.w.span(from.0.min(k0), k1, self.speed);
        self.cover(feed, self.origin.saturating_add(lo), len);
        let rel = self.start - self.origin;
        if k1 > from.0 {
            let more = self.w.shifts(self.speed, from, k1, &self.mono, rel);
            if self.shifts.back().is_none_or(|(k, _)| *k != from.0) {
                self.shifts.push_back(from);
            }
            for (i, d) in more.into_iter().enumerate() {
                self.shifts.push_back((from.0 + 1 + i as i64, d));
            }
        }
        while self.shifts.len() > KEPT {
            self.shifts.pop_front();
        }
        let shifts = &self.shifts;
        let shift = |k: i64| shifts.iter().rev().find(|(kk, _)| *kk == k).map_or(0, |(_, d)| *d);
        let l = self.w.render(self.speed, self.next, n, &shift, &self.left, rel);
        let r = self.w.render(self.speed, self.next, n, &shift, &self.right, rel);
        self.next = self.next.saturating_add(n as i64);
        let mut out = Vec::with_capacity(n * 2);
        for (a, b) in l.iter().zip(&r) {
            out.push(*a);
            out.push(*b);
        }
        out
    }

    /// Makes the kept mix cover sequence samples `[lo, lo + len)`: mixes on from where it ends,
    /// and lets go of what lies before `lo`.
    fn cover(&mut self, feed: &AudioFeed, lo: i64, len: usize) {
        let end = self.start.saturating_add(self.left.len() as i64);
        let want = lo.saturating_add(len as i64);
        if want > end {
            let n = usize::try_from(want - end).unwrap_or(0).min(1 << 20);
            let mix = feed.mix(end, n);
            for p in mix.chunks_exact(2) {
                self.left.push(p[0]);
                self.right.push(p[1]);
                self.mono.push(p[0] + p[1]);
            }
        }
        if lo > self.start {
            let drop = usize::try_from(lo - self.start).unwrap_or(0).min(self.left.len());
            self.left.drain(..drop);
            self.right.drain(..drop);
            self.mono.drain(..drop);
            self.start += drop as i64;
        }
    }
}
