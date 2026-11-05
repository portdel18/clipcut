//! Speed ramps: a clip whose Time Remapping Speed is keyframed plays each moment of its media at
//! the speed keyed there. The keyframes sit at media times, as every effect's do, so a ramp stays
//! on the footage it was made for through trims and splits. Speed is a percentage of the clip's own
//! speed, eased between keyframes as their interpolation says, and constant before the first and
//! after the last.
//!
//! Timeline time over media `[a, b]` is the integral of `1 / speed`. It is summed over a grid that
//! is fixed in media time (each keyframe segment cut into equal steps, the speed taken at each
//! step's middle), so the time from `a` to `c` is always the time from `a` to `b` plus the time
//! from `b` to `c`. A clip split anywhere carries on exactly where the whole one would have.

use super::{Param, Tick, TrackItem};

/// Slowest and fastest a ramp plays, in percent of the clip's speed.
pub const RAMP_MIN: f64 = 5.0;
pub const RAMP_MAX: f64 = 2000.0;
/// Steps per keyframe segment, and most steps in all (many keyframes get fewer each).
const STEPS: usize = 48;
const BUDGET: usize = 4096;

/// A ramp's speed (media per timeline time) as pieces of media time.
struct Pieces {
    /// Speed before the first keyframe and after the last.
    first: f64,
    last: f64,
    /// `(from, to, speed)` back to back from the first keyframe to the last.
    steps: Vec<(f64, f64, f64)>,
}

impl Pieces {
    fn of(p: &Param, base: f64) -> Option<Pieces> {
        let keys: Vec<f64> = p.keyframes.iter().map(|k| k.time.0 as f64).collect();
        let (lo, hi) = (*keys.first()?, *keys.last()?);
        let speed = |t: f64| {
            let v = p.scalar_at(Tick(t.round() as i64));
            let v = if v.is_finite() { v.clamp(RAMP_MIN, RAMP_MAX) } else { 100.0 };
            base * v / 100.0
        };
        let per = (BUDGET / keys.len().max(1)).clamp(1, STEPS);
        let mut steps = Vec::new();
        for w in keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b <= a {
                continue;
            }
            let len = (b - a) / per as f64;
            for j in 0..per {
                let s0 = a + len * j as f64;
                let s1 = if j + 1 == per { b } else { s0 + len };
                steps.push((s0, s1, speed((s0 + s1) / 2.0)));
            }
        }
        Some(Pieces { first: speed(lo), last: speed(hi), steps })
    }

    /// Every piece as `(from, to, speed)` on the whole media axis, in order.
    fn all(&self) -> impl DoubleEndedIterator<Item = (f64, f64, f64)> + '_ {
        let lo = self.steps.first().map_or(0.0, |s| s.0);
        let hi = self.steps.last().map_or(lo, |s| s.1);
        std::iter::once((f64::NEG_INFINITY, lo, self.first)).chain(self.steps.iter().copied()).chain(std::iter::once((hi, f64::INFINITY, self.last)))
    }

    /// The media time `rel` timeline ticks after (before, when negative) media time `from`.
    fn media_after(&self, from: f64, rel: f64) -> f64 {
        let mut s = from;
        let mut left = rel.abs();
        if rel >= 0.0 {
            for (_, b, v) in self.all().filter(|p| p.1 > from) {
                let dt = (b - s) / v;
                if left <= dt {
                    return s + left * v;
                }
                left -= dt;
                s = b;
            }
        } else {
            for (a, _, v) in self.all().rev().filter(|p| p.0 < from) {
                let dt = (s - a) / v;
                if left <= dt {
                    return s - left * v;
                }
                left -= dt;
                s = a;
            }
        }
        s
    }

    /// Timeline ticks media `[a, b]` plays for.
    fn time_between(&self, a: f64, b: f64) -> f64 {
        self.all().map(|(p0, p1, v)| (p1.min(b) - p0.max(a)).max(0.0) / v).sum()
    }
}

impl TrackItem {
    /// The clip's speed ramp: Time Remapping's Speed keyframed, on a clip playing forward with no
    /// frame hold (ramps leave those alone).
    pub fn ramp(&self) -> Option<&Param> {
        if self.reverse || self.frame_hold.is_some() {
            return None;
        }
        self.effect("time_remap").filter(|e| e.enabled).and_then(|e| e.param("speed")).filter(|p| p.is_animated())
    }

    fn ramp_pieces(&self) -> Option<Pieces> {
        let base = self.speed.abs();
        if !(base.is_finite() && base > 0.0) {
            return None;
        }
        Pieces::of(self.ramp()?, base)
    }

    /// With a ramp, the media time `rel` after (before, when negative) the clip's start.
    pub(crate) fn ramp_media_at(&self, rel: Tick) -> Option<Tick> {
        let p = self.ramp_pieces()?;
        Some(Tick(p.media_after(self.source_in.0 as f64, rel.0 as f64).round() as i64))
    }

    /// With a ramp, how long media `[from, to]` plays on the timeline.
    pub fn ramp_time_between(&self, from: Tick, to: Tick) -> Option<Tick> {
        let p = self.ramp_pieces()?;
        Some(Tick(p.time_between(from.0 as f64, to.0 as f64).round() as i64))
    }

    /// The media the clip shows `rel` timeline ticks after its start, before (negative `rel`) it
    /// too, ramp or not: where its In point goes when it moves by `rel`.
    pub fn media_after_start(&self, rel: Tick) -> Tick {
        self.ramp_media_at(rel).unwrap_or_else(|| self.source_in + Tick((rel.0 as f64 * self.speed.abs()).round() as i64))
    }

    /// The timeline time from the clip's start until it shows media time `m` (negative before its
    /// In point), ramp or not: where a moment of its media plays (reversed clips aside).
    pub fn time_to_media(&self, m: Tick) -> Tick {
        if let Some(p) = self.ramp_pieces() {
            let (a, b) = (self.source_in.0 as f64, m.0 as f64);
            let t = if b >= a { p.time_between(a, b) } else { -p.time_between(b, a) };
            return Tick(t.round() as i64);
        }
        let speed = self.speed.abs().max(1e-6);
        Tick(((m.0 as f64 - self.source_in.0 as f64) / speed).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipId, ItemId, Keyframe, ParamValue, TICKS_PER_SECOND};

    fn sec(s: f64) -> Tick {
        Tick((s * TICKS_PER_SECOND as f64).round() as i64)
    }

    fn ramped(keys: &[(f64, f64)]) -> TrackItem {
        let mut it = TrackItem {
            custom_label_color: None,
            id: ClipId(1),
            item: ItemId(2),
            name: "clip".into(),
            label: crate::Label::Iris,
            start: Tick::ZERO,
            duration: sec(10.0),
            source_in: Tick::ZERO,
            speed: 1.0,
            reverse: false,
            maintain_pitch: false,
            enabled: true,
            link: None,
            group: None,
            effects: crate::effect::intrinsic_video(),
            markers: vec![],
            gain_db: 0.0,
            frame_hold: None,
            scale_to_frame: false,
            essential: None,
            multicam: None,
            time_interpolation: Default::default(),
            hold_filters: false,
            field_options: None,
            source_channels: Vec::new(),
            audio_stream: 0,
            graphic: None,
        };
        let p = it.effect_mut("time_remap").and_then(|e| e.params.get_mut("speed")).unwrap();
        p.keyframes = keys.iter().map(|(t, v)| Keyframe::new(sec(*t), ParamValue::Float(*v))).collect();
        it
    }

    #[test]
    fn a_ramp_plays_each_moment_at_its_speed() {
        // 200 % up to 2 s of media, 50 % from 4 s on, a straight line between
        let it = ramped(&[(2.0, 200.0), (4.0, 50.0)]);
        assert!(it.ramp().is_some());
        // 2 s of media at 200 % take 1 s
        assert!((it.source_time_at(sec(1.0)) - sec(2.0)).abs() < Tick(1000));
        // media 4 s → 6 s at 50 % take 4 s; the ramp between is the integral of 1 / speed
        let between = it.ramp_time_between(sec(2.0), sec(4.0)).unwrap();
        let exact = 2.0 * (4.0f64).ln() / 1.5; // ∫ ds / (2 − 0.75 (s − 2)) over 2 s
        assert!((between.0 as f64 / TICKS_PER_SECOND as f64 - exact).abs() < 1e-3, "{between:?}");
        let at = sec(1.0) + between + sec(4.0);
        assert!((it.source_time_at(at) - sec(6.0)).abs() < Tick(1000));
        // forward and back agree
        for m in [0.5, 2.7, 3.3, 7.0] {
            let t = it.ramp_time_between(Tick::ZERO, sec(m)).unwrap();
            assert!((it.source_time_at(t) - sec(m)).abs() < Tick(1000), "{m}");
        }
        // before the clip's In point (a head extension) at the first keyframe's speed
        assert!((it.media_after_start(-sec(1.0)) - (-sec(2.0))).abs() < Tick(1000));
        assert!((it.time_to_media(sec(6.0)) - at).abs() < Tick(1000));
        assert!((it.time_to_media(-sec(2.0)) + sec(1.0)).abs() < Tick(1000));
        // the clip's media ends where its length runs out
        assert_eq!(it.source_out(), it.source_time_at(it.end()));
    }

    #[test]
    fn a_split_carries_on_where_the_whole_clip_would() {
        let it = ramped(&[(1.0, 300.0), (3.0, 30.0), (5.0, 250.0)]);
        let cut = sec(2.3);
        let mut right = it.clone();
        right.start = cut;
        right.duration = it.duration - cut;
        right.source_in = it.media_after_start(cut);
        for t in [2.5, 4.0, 6.0, 9.9] {
            assert!((right.source_time_at(sec(t)) - it.source_time_at(sec(t))).abs() < Tick(1000), "{t}");
        }
    }

    #[test]
    fn hostile_ramps_stay_in_bounds() {
        // out-of-range and non-finite speeds are held to the limits; keys on one time are skipped
        let it = ramped(&[(1.0, f64::NAN), (1.0, -50.0), (2.0, 1e12), (2.0, 0.0)]);
        let m = it.source_time_at(sec(5.0));
        assert!(m > Tick::ZERO && m < sec(5.0 * RAMP_MAX / 100.0 + 1.0), "{m:?}");
        let many = ramped(&(0..20_000).map(|k| (k as f64 * 0.001, 50.0 + (k % 7) as f64 * 100.0)).collect::<Vec<_>>());
        assert!(many.source_out() > Tick::ZERO);
        // a reversed clip or a frame hold has no ramp
        let mut r = it.clone();
        r.reverse = true;
        assert!(r.ramp().is_none());
    }
}
