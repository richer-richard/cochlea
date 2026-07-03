//! The tempo map: step changes at ticks, and the exact rational tick→sample
//! conversion — rounding rules 2 and 3 of `docs/determinism.md`.

use fenestra_anim::{Rounding, mul_div};

use crate::time::{Ppq, SampleRate, Ticks};

/// One tempo step: from `at` onward, quarters last `npq` nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TempoStep {
    pub at: Ticks,
    pub npq: u64,
}

/// A compiled tempo map: sorted steps with precomputed integer anchors, so
/// `sample_at` rounds exactly once per lookup and error never accumulates
/// with tick count (only bounded ±0.5-sample rounding per tempo *change*).
///
/// Built by [`Score::tempo_map`](crate::Score::tempo_map); the constructor
/// guarantees a step at tick 0 and strictly increasing step ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempoMap {
    segs: Vec<Seg>,
    ppq: Ppq,
    sample_rate: SampleRate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seg {
    start_tick: Ticks,
    /// Nanoseconds per quarter note in this segment.
    npq: u64,
    /// `sample_at(start_tick)`, computed left-to-right.
    anchor_sample: u64,
    /// `ns_at(start_tick)`, computed left-to-right.
    anchor_ns: u64,
}

impl TempoMap {
    /// `steps` must be sorted by tick, deduplicated, and start at tick 0 —
    /// the score builder enforces that before calling.
    pub(crate) fn new(ppq: Ppq, sample_rate: SampleRate, steps: &[TempoStep]) -> TempoMap {
        assert!(
            steps.first().is_some_and(|s| s.at == Ticks::ZERO),
            "tempo map needs a step at tick 0"
        );
        let mut segs: Vec<Seg> = Vec::with_capacity(steps.len());
        for step in steps {
            let (anchor_sample, anchor_ns) = match segs.last() {
                None => (0, 0),
                Some(prev) => {
                    let dt = step.at.0 - prev.start_tick.0;
                    (
                        prev.anchor_sample + convert(dt, prev.npq, sample_rate, ppq),
                        prev.anchor_ns + ns_of(dt, prev.npq, ppq),
                    )
                }
            };
            segs.push(Seg {
                start_tick: step.at,
                npq: step.npq,
                anchor_sample,
                anchor_ns,
            });
        }
        TempoMap {
            segs,
            ppq,
            sample_rate,
        }
    }

    fn seg_at_tick(&self, t: Ticks) -> &Seg {
        let i = self.segs.partition_point(|s| s.start_tick <= t);
        &self.segs[i - 1]
    }

    /// Tick → sample index: THE conversion, applied once at event-schedule
    /// time (rounding rule 2). Exact rational within a segment; each tempo
    /// change contributes at most one ±0.5-sample anchor rounding, so error
    /// is bounded by the number of tempo changes, never by tick count.
    pub fn sample_at(&self, t: Ticks) -> u64 {
        let seg = self.seg_at_tick(t);
        seg.anchor_sample + convert(t.0 - seg.start_tick.0, seg.npq, self.sample_rate, self.ppq)
    }

    /// Sample index → the tick that sample lies inside (rounding rule 3:
    /// `Floor`). Used for automation evaluation at block starts. Clamped so
    /// boundary rounding never leaks a tick into the next tempo segment.
    pub fn tick_at(&self, sample: u64) -> Ticks {
        let i = self.segs.partition_point(|s| s.anchor_sample <= sample);
        let seg = &self.segs[i - 1];
        let dt = mul_div(
            sample - seg.anchor_sample,
            u64::from(self.ppq.0) * 1_000_000_000,
            seg.npq * u64::from(self.sample_rate.0),
            Rounding::Floor,
        );
        let tick = seg.start_tick.0 + dt;
        match self.segs.get(i) {
            Some(next) => Ticks(tick.min(next.start_tick.0 - 1)),
            None => Ticks(tick),
        }
    }

    /// Tick → elapsed nanoseconds. Same anchored-rational scheme.
    pub fn ns_at(&self, t: Ticks) -> u64 {
        let seg = self.seg_at_tick(t);
        seg.anchor_ns + ns_of(t.0 - seg.start_tick.0, seg.npq, self.ppq)
    }

    /// Tick → elapsed milliseconds, as a float for reports and tolerances
    /// (derived from the exact integer nanoseconds, never accumulated).
    pub fn ms_at(&self, t: Ticks) -> f64 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "render lengths sit far below f64's 2^53 integer range in ns"
        )]
        let ns = self.ns_at(t) as f64;
        ns / 1_000_000.0
    }

    /// Nanoseconds per quarter in force at `t`.
    pub fn npq_at(&self, t: Ticks) -> u64 {
        self.seg_at_tick(t).npq
    }
}

/// `Δticks → Δsamples` within one tempo segment:
/// `Δticks · npq · sr / (ppq · 1e9)`, rounded to nearest once.
///
/// Range: `npq ≤ 6e10` (BPM ≥ 1) and `sr ≤ 192_000` keep `npq · sr` under
/// `1.2e16 < u64::MAX`; the product with `Δticks` widens to u128 inside
/// `mul_div`.
fn convert(dticks: u64, npq: u64, sample_rate: SampleRate, ppq: Ppq) -> u64 {
    mul_div(
        dticks,
        npq * u64::from(sample_rate.0),
        u64::from(ppq.0) * 1_000_000_000,
        Rounding::Round,
    )
}

/// `Δticks → Δnanoseconds` within one segment: `Δticks · npq / ppq`.
fn ns_of(dticks: u64, npq: u64, ppq: Ppq) -> u64 {
    mul_div(dticks, npq, u64::from(ppq.0), Rounding::Round)
}
