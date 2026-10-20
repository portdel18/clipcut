//! Time stretching that keeps the pitch (WSOLA: waveform-similarity overlap-add), for clips played
//! faster or slower with Maintain Audio Pitch.
//!
//! The output is built from Hann-windowed frames of `n` samples, one every `hop = n / 2` output
//! samples (the windows sum to one). Frame `k` covers output `[k·hop, k·hop + n)` and is read from
//! the source at `k·hop·speed + δ_k`: its nominal place plus a shift within `±tolerance` that
//! makes its first half match the natural continuation of frame `k − 1` best (normalised
//! cross-correlation, a coarse search on a 4× decimated mono signal refined at full rate). Nothing
//! is resampled, so the pitch and the timbre stay as they were: voices sound like themselves, not
//! like a phase vocoder's.
//!
//! Each shift depends on the previous one. [`Wsola::shifts`] continues from a known shift, so a
//! stream read block by block (playback, export) is the same as one read whole; a reader starting
//! somewhere new begins a few frames early with a shift of zero.

/// Frames a fresh chain starts before the first one wanted, so the wanted ones are aligned.
pub const WARM_UP: i64 = 8;

/// The WSOLA settings for one sample rate.
#[derive(Clone, Debug)]
pub struct Wsola {
    n: usize,
    hop: usize,
    tolerance: usize,
    window: Vec<f32>,
}

impl Wsola {
    /// 32 ms frames, 8 ms of search either way.
    pub fn new(sample_rate: u32) -> Self {
        let sr = f64::from(sample_rate.clamp(8_000, 384_000));
        let n = (((sr * 0.032) as usize) / 8 * 8).max(64);
        let hop = n / 2;
        let tolerance = (((sr * 0.008) as usize) / 4 * 4).max(8);
        let window = (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32).collect();
        Self { n, hop, tolerance, window }
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Frames that touch output samples `[o0, o0 + len)`: `k0..=k1`.
    pub fn frames_for(&self, o0: i64, len: usize) -> (i64, i64) {
        let hop = self.hop as i64;
        let o1 = o0.saturating_add(len as i64);
        // frame k touches them when k·hop < o1 and k·hop + n > o0
        (o0.saturating_sub(self.n as i64).div_euclid(hop) + 1, (o1 - 1).div_euclid(hop))
    }

    /// Nominal source start of frame `k` at `speed` (output and source counted from the clip's start).
    pub fn nominal(&self, k: i64, speed: f64) -> i64 {
        ((k as f64) * self.hop as f64 * speed).round() as i64
    }

    /// The source samples frames `from.0 + 1 ..= k1` read: `[lo, lo + len)`.
    pub fn span(&self, from: i64, k1: i64, speed: f64) -> (i64, usize) {
        let lo = self.nominal(from, speed) - self.tolerance as i64 - self.n as i64;
        let hi = self.nominal(k1, speed) + (self.n + self.tolerance) as i64 + self.n as i64;
        (lo, usize::try_from(hi - lo).unwrap_or(0))
    }

    /// The shifts of frames `from.0 + 1 ..= k1`, continuing from frame `from.0`'s shift `from.1`.
    /// `mono` holds the source (channels summed) from sample `lo` on (see [`Wsola::span`]);
    /// samples outside it count as silence.
    pub fn shifts(&self, speed: f64, from: (i64, i64), k1: i64, mono: &[f32], lo: i64) -> Vec<i64> {
        let at = |i: i64| -> f32 { usize::try_from(i - lo).ok().and_then(|i| mono.get(i)).copied().unwrap_or(0.0) };
        // the 4× decimated signal for the coarse search
        let dec: Vec<f32> = mono.chunks(4).map(|c| c.iter().sum::<f32>() * 0.25).collect();
        let dat = |i: i64| -> f32 { usize::try_from((i - lo).div_euclid(4)).ok().and_then(|i| dec.get(i)).copied().unwrap_or(0.0) };
        let half = self.n - self.hop;
        let tol = self.tolerance as i64;
        let mut out = Vec::with_capacity(usize::try_from(k1 - from.0).unwrap_or(0));
        let (mut prev_k, mut prev_d) = from;
        while prev_k < k1 {
            let k = prev_k + 1;
            // where frame k-1 would carry on, and where frame k would start
            let target = self.nominal(prev_k, speed) + prev_d + self.hop as i64;
            let nom = self.nominal(k, speed);
            let score = |d: i64, step: usize, get: &dyn Fn(i64) -> f32| -> f32 {
                let (mut xy, mut yy) = (0f32, 1e-9f32);
                for i in (0..half).step_by(step) {
                    let x = get(target + i as i64);
                    let y = get(nom + d + i as i64);
                    xy += x * y;
                    yy += y * y;
                }
                xy / yy.sqrt()
            };
            // coarse, every 4th shift on the decimated signal; ties keep the smaller shift
            let mut best = (0i64, f32::MIN);
            for d in (0..=tol).step_by(4).flat_map(|d| [d, -d]) {
                let s = score(d, 4, &dat);
                if s > best.1 + 1e-6 {
                    best = (d, s);
                }
            }
            // refined at full rate around it
            let centre = best.0;
            let mut fine = (centre, score(centre, 1, &at));
            for d in (centre - 3..=centre + 3).filter(|d| d.abs() <= tol && *d != centre) {
                let s = score(d, 1, &at);
                if s > fine.1 + 1e-6 {
                    fine = (d, s);
                }
            }
            out.push(fine.0);
            prev_k = k;
            prev_d = fine.0;
        }
        out
    }

    /// Output samples `[o0, o0 + len)` of one channel: frames `k0..` with `shift(k)` read from
    /// `src` (that channel from source sample `lo` on).
    pub fn render(&self, speed: f64, o0: i64, len: usize, shift: &dyn Fn(i64) -> i64, src: &[f32], lo: i64) -> Vec<f32> {
        let mut out = vec![0f32; len];
        let (k0, k1) = self.frames_for(o0, len);
        let hop = self.hop as i64;
        for k in k0..=k1 {
            let start = k * hop;
            let a = self.nominal(k, speed) + shift(k);
            for (i, w) in self.window.iter().enumerate() {
                let o = start + i as i64;
                let Some(dst) = usize::try_from(o - o0).ok().and_then(|j| out.get_mut(j)) else { continue };
                let s = usize::try_from(a + i as i64 - lo).ok().and_then(|j| src.get(j)).copied().unwrap_or(0.0);
                *dst += w * s;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders `[o0, o0 + len)` of `x` at `speed` with a fresh chain, as a reader starting there.
    fn stretch(w: &Wsola, x: &[f32], speed: f64, o0: i64, len: usize, from: Option<(i64, i64)>) -> (Vec<f32>, (i64, i64)) {
        let (k0, k1) = w.frames_for(o0, len);
        let from = from.unwrap_or((k0 - WARM_UP, 0));
        let shifts = w.shifts(speed, from, k1, x, 0);
        let first = from.0 + 1;
        let shift = |k: i64| if k == from.0 { from.1 } else { usize::try_from(k - first).ok().and_then(|i| shifts.get(i)).copied().unwrap_or(0) };
        let out = w.render(speed, o0, len, &shift, x, 0);
        (out, (k1, shifts.last().copied().unwrap_or(from.1)))
    }

    fn zero_crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|p| p[0] < 0.0 && p[1] >= 0.0).count()
    }

    #[test]
    fn keeps_the_pitch_and_changes_the_length() {
        let sr = 48_000;
        let w = Wsola::new(sr);
        let x: Vec<f32> = (0..sr as usize * 4).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.5).collect();
        for speed in [0.5, 0.75, 1.5, 2.0, 3.0] {
            // one second of output from the middle
            let o0 = sr as i64 / 2;
            let (y, _) = stretch(&w, &x, speed, o0, sr as usize / 2, None);
            let hz = zero_crossings(&y) as f64 * 2.0;
            assert!((hz - 440.0).abs() < 8.0, "{speed}x: {hz} Hz");
            let rms = (y.iter().map(|v| v * v).sum::<f32>() / y.len() as f32).sqrt();
            assert!((rms - 0.354).abs() < 0.04, "{speed}x: level {rms}");
            // smooth: no clicks where frames join (a sine's second difference stays small)
            let rough = y.windows(3).map(|p| (p[0] - 2.0 * p[1] + p[2]).abs()).fold(0f32, f32::max);
            assert!(rough < 0.05, "{speed}x: a click of {rough}");
        }
    }

    #[test]
    fn blocks_continue_the_chain_exactly() {
        let sr = 48_000;
        let w = Wsola::new(sr);
        // a chirp-ish signal, so alignments differ frame to frame
        let x: Vec<f32> = (0..sr as usize * 3)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (2.0 * std::f32::consts::PI * (200.0 + 300.0 * t) * t).sin() * 0.4 + 0.1 * (t * 1234.5).sin()
            })
            .collect();
        let speed = 1.5;
        let (whole, _) = stretch(&w, &x, speed, 1000, 8192, None);
        let mut blocks = Vec::new();
        let mut chain = None;
        let mut memo: Vec<(i64, i64)> = Vec::new();
        for b in 0..8 {
            let o0 = 1000 + b * 1024;
            let (k0, k1) = w.frames_for(o0, 1024);
            // the shifts of this block's frames: known ones from the memo, the rest continued
            let from = chain.unwrap_or((k0 - WARM_UP, 0));
            let more = w.shifts(speed, from, k1, &x, 0);
            for (i, d) in more.iter().enumerate() {
                memo.push((from.0 + 1 + i as i64, *d));
            }
            let shift = |k: i64| memo.iter().find(|(kk, _)| *kk == k).map_or(0, |(_, d)| *d);
            blocks.extend(w.render(speed, o0, 1024, &shift, &x, 0));
            chain = Some((k1, shift(k1)));
        }
        let worst = whole.iter().zip(&blocks).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-6, "block by block differs by {worst}");
    }

    #[test]
    fn silence_and_edges_are_quiet() {
        let w = Wsola::new(48_000);
        let x = vec![0f32; 48_000];
        let (y, _) = stretch(&w, &x, 2.0, 0, 4800, None);
        assert!(y.iter().all(|v| *v == 0.0));
        // a frame range starting before the clip reads silence there, no panic
        let (y, _) = stretch(&w, &x, 0.5, -5000, 100, None);
        assert_eq!(y.len(), 100);
        assert_eq!(w.frames_for(0, 1), (-1, 0));
    }
}
