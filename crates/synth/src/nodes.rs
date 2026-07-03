//! In-repo fundsp `AudioNode`s: counter-keyed noise, Karplus-Strong, and a
//! Schroeder reverb. These exist because the fundsp equivalents fail the
//! determinism contract: `noise()`/`Pluck` replay a stateful funutd RNG, and
//! every fundsp stereo reverb is built on `fdn()`, whose `Feedback` core
//! silently sets thread-global x86 FTZ (`docs/determinism.md`). Everything
//! here is pure arithmetic per tick; libm only at construction.

use fundsp::prelude::{AudioNode, Frame, U0, U1, U2};

use crate::rng::crng_f32;

/// White noise as a pure function of `(seed, index)` — random access,
/// invariant 3. The index is the sample position since voice start.
#[derive(Clone)]
pub struct CounterNoise {
    seed: u64,
    index: u64,
}

impl CounterNoise {
    pub fn new(seed: u64) -> CounterNoise {
        CounterNoise { seed, index: 0 }
    }
}

impl AudioNode for CounterNoise {
    const ID: u64 = 0xC0C4_0001;
    type Inputs = U0;
    type Outputs = U1;

    fn reset(&mut self) {
        self.index = 0;
    }

    fn tick(&mut self, _input: &Frame<f32, U0>) -> Frame<f32, U1> {
        let v = crng_f32(self.seed, self.index);
        self.index += 1;
        [v].into()
    }
}

/// Karplus-Strong plucked string: a delay line excited by counter-RNG
/// noise, averaged pairwise with a decay factor per pass. Stateful (it is a
/// fold) but fully determined by `(seed, freq, t60, amp, sample_rate)`.
///
/// The delay length quantizes pitch to `round(sr / freq)` samples (≈1.5
/// cents at A4/48 kHz) — inside the Tier 2 pitch tolerance; fractional
/// tuning is a phase-2 refinement.
#[derive(Clone)]
pub struct KarplusStrong {
    seed: u64,
    freq: f64,
    t60: f64,
    amp: f32,
    sample_rate: f64,
    line: Vec<f32>,
    pos: usize,
    decay_half: f32,
}

impl KarplusStrong {
    pub fn new(seed: u64, freq: f64, t60: f64, amp: f32) -> KarplusStrong {
        let mut ks = KarplusStrong {
            seed,
            freq,
            t60,
            amp,
            sample_rate: 44_100.0,
            line: Vec::new(),
            pos: 0,
            decay_half: 0.5,
        };
        ks.rebuild();
        ks
    }

    /// Fills the delay line from the counter RNG (mean-corrected so the
    /// string has no DC) and derives the per-period decay from t60.
    fn rebuild(&mut self) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "validated frequency/sample-rate ranges keep this in 2..=6000"
        )]
        let len = (libm::round(self.sample_rate / self.freq) as usize).max(2);
        let mut line: Vec<f32> = (0..len)
            .map(|i| crng_f32(self.seed, i as u64) * self.amp)
            .collect();
        #[expect(clippy::cast_precision_loss, reason = "delay lines are short")]
        let mean = line.iter().map(|&v| f64::from(v)).sum::<f64>() / len as f64;
        #[expect(clippy::cast_possible_truncation, reason = "mean of f32s fits f32")]
        let mean = mean as f32;
        for v in &mut line {
            *v -= mean;
        }
        // Amplitude decays roughly by `decay` once per period; -60 dB after
        // t60 seconds = freq * t60 periods.
        let periods = (self.freq * self.t60).max(1.0);
        #[expect(clippy::cast_possible_truncation, reason = "decay factor is in (0, 1]")]
        let decay = libm::pow(1e-3, 1.0 / periods) as f32;
        self.line = line;
        self.pos = 0;
        self.decay_half = 0.5 * decay;
    }
}

impl AudioNode for KarplusStrong {
    const ID: u64 = 0xC0C4_0002;
    type Inputs = U0;
    type Outputs = U1;

    fn reset(&mut self) {
        self.rebuild();
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild();
        }
    }

    fn tick(&mut self, _input: &Frame<f32, U0>) -> Frame<f32, U1> {
        let len = self.line.len();
        let a = self.line[self.pos];
        let b = self.line[(self.pos + 1) % len];
        let out = self.decay_half * (a + b);
        self.line[self.pos] = out;
        self.pos = (self.pos + 1) % len;
        [a].into()
    }
}

/// One damped feedback comb filter (Freeverb-style).
#[derive(Clone)]
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    filt: f32,
    feedback: f32,
    damp: f32,
}

impl Comb {
    fn new(len: usize, feedback: f32, damp: f32) -> Comb {
        Comb {
            buf: vec![0.0; len],
            pos: 0,
            filt: 0.0,
            feedback,
            damp,
        }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let out = self.buf[self.pos];
        self.filt = out * (1.0 - self.damp) + self.filt * self.damp;
        self.buf[self.pos] = x + self.filt * self.feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
        self.filt = 0.0;
    }
}

/// One Schroeder allpass diffuser.
#[derive(Clone)]
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Allpass {
        Allpass {
            buf: vec![0.0; len],
            pos: 0,
        }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.pos];
        let out = delayed - x;
        self.buf[self.pos] = x + delayed * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }
}

/// Classic Freeverb comb tunings at 44.1 kHz, scaled to the actual rate at
/// construction; the right channel runs `STEREO_SPREAD` samples longer.
const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

/// A stereo Schroeder/Freeverb reverb with the dry/wet mix baked in:
/// 8 damped combs + 4 allpasses per channel. Pure arithmetic per tick.
#[derive(Clone)]
pub struct SchroederReverb {
    feedback: f32,
    damp: f32,
    wet: f32,
    sample_rate: f64,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl SchroederReverb {
    /// `feedback` sets the tail length (0.84 ≈ two seconds), `damp` the
    /// high-frequency decay, `wet` the mix (dry passes at unity).
    pub fn new(feedback: f32, damp: f32, wet: f32) -> SchroederReverb {
        let mut r = SchroederReverb {
            feedback,
            damp,
            wet,
            sample_rate: 44_100.0,
            combs: [Vec::new(), Vec::new()],
            allpasses: [Vec::new(), Vec::new()],
        };
        r.rebuild();
        r
    }

    fn rebuild(&mut self) {
        let scale = self.sample_rate / 44_100.0;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "tunings are a few thousand samples at any valid rate"
        )]
        let scaled = |len: usize| ((len as f64 * scale) as usize).max(1);
        for (ch, spread) in [(0, 0), (1, STEREO_SPREAD)] {
            self.combs[ch] = COMB_TUNINGS
                .iter()
                .map(|&len| Comb::new(scaled(len + spread), self.feedback, self.damp))
                .collect();
            self.allpasses[ch] = ALLPASS_TUNINGS
                .iter()
                .map(|&len| Allpass::new(scaled(len + spread)))
                .collect();
        }
    }
}

impl AudioNode for SchroederReverb {
    const ID: u64 = 0xC0C4_0003;
    type Inputs = U2;
    type Outputs = U2;

    fn reset(&mut self) {
        for ch in 0..2 {
            for c in &mut self.combs[ch] {
                c.clear();
            }
            for a in &mut self.allpasses[ch] {
                a.clear();
            }
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild();
        }
    }

    fn tick(&mut self, input: &Frame<f32, U2>) -> Frame<f32, U2> {
        let mut out = [0.0f32; 2];
        // Mono-sum feed, per-channel decorrelated tails (Freeverb topology).
        let feed = (input[0] + input[1]) * 0.5 * 0.015;
        for ch in 0..2 {
            // Fixed summation order: comb index order.
            let mut acc = 0.0f32;
            for comb in &mut self.combs[ch] {
                acc += comb.tick(feed);
            }
            for ap in &mut self.allpasses[ch] {
                acc = ap.tick(acc);
            }
            out[ch] = input[ch] + acc * self.wet;
        }
        out.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_noise_is_reproducible_and_reset_replays() {
        let mut n1 = CounterNoise::new(9);
        let mut n2 = CounterNoise::new(9);
        let a: Vec<f32> = (0..256).map(|_| n1.tick(&Frame::default())[0]).collect();
        let b: Vec<f32> = (0..256).map(|_| n2.tick(&Frame::default())[0]).collect();
        assert_eq!(a, b);
        n1.reset();
        let c: Vec<f32> = (0..256).map(|_| n1.tick(&Frame::default())[0]).collect();
        assert_eq!(a, c);
    }

    #[test]
    fn karplus_strong_rings_then_decays() {
        let mut ks = KarplusStrong::new(3, 220.0, 0.5, 0.8);
        ks.set_sample_rate(48_000.0);
        let early: f32 = (0..4_800)
            .map(|_| ks.tick(&Frame::default())[0].abs())
            .fold(0.0, f32::max);
        // Skip to 2 x t60, where the string sits far below -60 dB of its
        // excitation, then measure the residual peak.
        for _ in 0..(48_000 - 4_800) {
            ks.tick(&Frame::default());
        }
        let mut late_peak = 0.0f32;
        for _ in 0..24_000 {
            late_peak = late_peak.max(ks.tick(&Frame::default())[0].abs());
        }
        assert!(early > 0.05, "string should ring: {early}");
        assert!(
            late_peak < early * 1e-3,
            "string should decay: early {early}, late {late_peak}"
        );
    }

    #[test]
    fn reverb_impulse_decays_below_audibility_within_tail() {
        let mut r = SchroederReverb::new(0.84, 0.2, 0.3);
        r.set_sample_rate(48_000.0);
        let first = r.tick(&[1.0, 1.0].into());
        assert!(first[0].abs() > 0.5, "dry passes at unity");
        let mut peak_after_tail = 0.0f32;
        for i in 0..(3 * 48_000) {
            let out = r.tick(&[0.0, 0.0].into());
            if i > 2 * 48_000 {
                peak_after_tail = peak_after_tail.max(out[0].abs().max(out[1].abs()));
            }
        }
        assert!(
            peak_after_tail < 1e-4,
            "tail should be inaudible after 2 s: {peak_after_tail}"
        );
    }

    #[test]
    fn reverb_is_deterministic_across_instances() {
        let run = || {
            let mut r = SchroederReverb::new(0.84, 0.2, 0.3);
            r.set_sample_rate(48_000.0);
            (0..10_000)
                .map(|i| {
                    let x = if i < 100 { 0.5 } else { 0.0 };
                    let out = r.tick(&[x, x].into());
                    out[0].to_bits() ^ out[1].to_bits().rotate_left(1)
                })
                .fold(0u32, u32::wrapping_add)
        };
        assert_eq!(run(), run());
    }
}
