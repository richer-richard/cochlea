//! Tempo/beat tracking: autocorrelation of the onset-strength envelope over
//! a BPM search range, a log-Gaussian tempo preference to steer away from
//! octave errors, and an Ellis-style ("Beat Tracking by Dynamic
//! Programming", Ellis 2007) dynamic-programming beat grid. Implemented
//! from the paper's publicly described method, not from any GPL codebase.
//!
//! Pipeline:
//! 1. Reuse `onsets`' spectral-flux onset-strength envelope (same STFT, so
//!    frame timing matches [`crate::OnsetsReport`] exactly).
//! 2. Direct-summation autocorrelation of the envelope over the lag range
//!    implied by `TempoOpts`' BPM bounds (no FFT needed — the search range
//!    is a few hundred lags at most).
//! 3. Weight each lag's autocorrelation by a log-Gaussian centered on a
//!    preferred tempo (Ellis 2007's trick against picking a genuine beat's
//!    double- or half-tempo instead of the beat itself); the best-weighted
//!    lag is the tempo estimate.
//! 4. Dynamic-programming beat grid at that period: greedily gridding on
//!    the estimated period would drift whenever the true period isn't an
//!    exact integer number of frames (it essentially never is), so instead
//!    each frame's cumulative score rewards a strong local onset *and*
//!    penalizes its interval from the previous chosen beat for deviating
//!    from the estimated period, letting real onsets pull the grid back
//!    into alignment every beat instead of accumulating drift.

use crate::audio::Audio;
use crate::onsets;
use crate::report::OnsetsReport;
use crate::stft::Stft;
use serde::{Deserialize, Serialize};

/// Default tempo search range, BPM: covers a slow half-time ballad feel up
/// through double-time dance tempos. Widening this risks the
/// autocorrelation peak landing on a harmonic/subharmonic of the true
/// tempo instead of the tempo itself.
const DEFAULT_MIN_BPM: f64 = 30.0;
const DEFAULT_MAX_BPM: f64 = 300.0;

/// Log-Gaussian tempo-preference center, BPM, and width, octaves — Ellis
/// 2007's idea for biasing the autocorrelation peak search away from octave
/// errors (mistaking a beat's double- or half-tempo for the beat itself)
/// without overriding a genuinely strong peak elsewhere. The center and
/// width are this crate's own calibration (not the paper's published
/// constants): wide enough that a real 70 or 180 BPM piece still wins on
/// its own autocorrelation strength, narrow enough to break a near-tie
/// between a tempo and its octave in the beat's favor.
const TEMPO_PRIOR_CENTER_BPM: f64 = 120.0;
const TEMPO_PRIOR_SIGMA_OCTAVES: f64 = 1.3;

/// Ellis-DP transition penalty weight: each hypothesized beat-to-beat
/// interval costs `-lambda * ln(interval / period)^2` against the
/// cumulative score. Tuned empirically against this module's click-track
/// tests: large enough that the grid doesn't wander off onto an unrelated
/// spurious peak, small enough that a real, strong onset always outweighs
/// blind adherence to the estimated period (letting the grid re-lock onto
/// the true beat every cycle rather than drift, since the estimated period
/// is essentially never an exact whole number of STFT frames).
const BEAT_DP_LAMBDA: f64 = 20.0;

/// `clear_rhythm` thresholds — see [`estimate_tempo`]'s docs for the exact
/// rule. Calibrated on this module's tests: a 120/90 BPM click track
/// measures confidence ~0.11-0.15, a sustained tone (no attacks at all,
/// hence no periodicity worth calling a rhythm) measures ~0.005 — `0.05`
/// sits with a wide margin above the no-rhythm case and comfortably below
/// both click-track measurements.
const CLEAR_RHYTHM_MIN_CONFIDENCE: f64 = 0.05;
const CLEAR_RHYTHM_MIN_ONSET_RATE_PER_S: f64 = 0.5;

/// Tunables for [`estimate_tempo`]. Mirrors [`crate::SegmentOpts`]'s
/// chainable-setter style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoOpts {
    /// Lower bound of the tempo search range, BPM. Default `30.0`.
    pub min_bpm: f64,
    /// Upper bound of the tempo search range, BPM. Default `300.0`.
    pub max_bpm: f64,
}

impl Default for TempoOpts {
    fn default() -> Self {
        Self {
            min_bpm: DEFAULT_MIN_BPM,
            max_bpm: DEFAULT_MAX_BPM,
        }
    }
}

impl TempoOpts {
    /// Override the lower BPM bound.
    #[must_use]
    pub fn with_min_bpm(mut self, min_bpm: f64) -> Self {
        self.min_bpm = min_bpm;
        self
    }

    /// Override the upper BPM bound.
    #[must_use]
    pub fn with_max_bpm(mut self, max_bpm: f64) -> Self {
        self.max_bpm = max_bpm;
        self
    }
}

/// Tempo/beat-tracking result. Plain struct, no own schema version — this
/// is meant to be embedded into a future `Report` schema bump rather than
/// stand alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoReport {
    /// Estimated tempo, BPM. `None` for degenerate input (empty/too-short
    /// audio, or an onset-strength envelope with no measurable energy).
    pub bpm: Option<f64>,
    /// Salience of the winning tempo lag against the envelope
    /// autocorrelation's overall mass, `0.0..=1.0`. See [`estimate_tempo`]
    /// for the exact definition.
    pub confidence: f64,
    /// Whether an agent can trust `bpm`/`beats_ms` as a real, steady pulse
    /// rather than a low-confidence guess. See [`estimate_tempo`] for the
    /// exact rule.
    pub clear_rhythm: bool,
    /// Beat grid, milliseconds, ascending. Empty when `bpm` is `None`.
    pub beats_ms: Vec<f64>,
}

fn degenerate_report() -> TempoReport {
    TempoReport {
        bpm: None,
        confidence: 0.0,
        clear_rhythm: false,
        beats_ms: Vec::new(),
    }
}

/// Estimate tempo and a beat grid for `audio` (see the module docs for the
/// pipeline).
///
/// **`confidence`**: at the winning lag, `weighted = autocorrelation *
/// tempo_prior_weight(bpm)`; `confidence = weighted / mass`, where `mass`
/// is the sum of every non-negative autocorrelation value across the whole
/// search range. Because the prior weight is at most `1.0` and `mass`
/// already includes the winning lag's own (unweighted, non-negative)
/// autocorrelation term, `weighted <= mass` always, so this is bounded to
/// `0.0..=1.0` by construction: it reads as "what fraction of the
/// envelope's total periodic-energy mass, once discounted for how far this
/// tempo sits from the preference center, is concentrated at the winning
/// lag" — sharp, tempo-plausible periodicity scores high; a flat or
/// tempo-implausible autocorrelation profile scores low.
///
/// **`clear_rhythm`**: `true` iff `confidence >= 0.05` *and* the buffer's
/// onset rate (from the same detector as [`crate::OnsetsReport`]) is at
/// least `0.5` onsets/second. Confidence alone isn't enough — a handful of
/// widely-spaced but perfectly regular hits can autocorrelate with high
/// confidence without being a "rhythm" an agent should trust; the onset-
/// rate floor requires the buffer to actually be busy enough to call it
/// one. The confidence floor is deliberately low relative to a strong
/// click track's own measured confidence (~0.11-0.15, see
/// `CLEAR_RHYTHM_MIN_CONFIDENCE`'s docs) — it exists to reject the
/// near-zero confidence a flat/non-periodic envelope produces (~0.005 for
/// a sustained tone), not to grade how clean the rhythm is.
///
/// Degenerate input (empty/near-empty audio, a zero sample rate, or an
/// onset-strength envelope with no measurable energy) returns `bpm: None`,
/// `confidence: 0.0`, `clear_rhythm: false`, `beats_ms: []` — never panics.
pub fn estimate_tempo(audio: &Audio, opts: &TempoOpts) -> TempoReport {
    let mono = audio.mono();
    if audio.sample_rate == 0 || mono.len() < onsets::FFT_SIZE {
        return degenerate_report();
    }
    let stft = Stft::compute(&mono, audio.sample_rate, onsets::FFT_SIZE, onsets::HOP);
    let onset_report = onsets::analyze_stft(&stft);
    let duration_s = mono.len() as f64 / f64::from(audio.sample_rate);
    estimate_from_parts(&stft, &onset_report, duration_s, audio.sample_rate, opts)
}

/// The estimator over caller-provided shared parts (the onsets-grade STFT
/// and its onset report) — `probe()` computes both exactly once and feeds
/// every consumer, instead of this module recomputing an identical STFT
/// and a full second onset-detection pass just to read `count`.
pub(crate) fn estimate_from_parts(
    stft: &Stft,
    onset_report: &OnsetsReport,
    duration_s: f64,
    sample_rate: u32,
    opts: &TempoOpts,
) -> TempoReport {
    if sample_rate == 0 || stft.magnitudes.len() < 3 {
        return degenerate_report();
    }
    let flux = onsets::spectral_flux(stft);
    let frame_rate = f64::from(sample_rate) / onsets::HOP as f64;

    // Reversed bounds are normalized by swapping — a caller writing
    // (min 200, max 100) meant the range 100..=200; the old clamp silently
    // discarded max_bpm and searched a ~1 BPM sliver around min instead.
    // (Non-finite bounds degrade safely through f64::max's NaN-ignoring
    // behavior to a narrow-but-valid range.)
    let (lo, hi) = if opts.min_bpm <= opts.max_bpm {
        (opts.min_bpm, opts.max_bpm)
    } else {
        (opts.max_bpm, opts.min_bpm)
    };
    let min_bpm = lo.max(1.0);
    let max_bpm = hi.max(min_bpm + 1.0);
    let min_lag = ((frame_rate * 60.0 / max_bpm).round() as usize).max(1);
    let max_lag = ((frame_rate * 60.0 / min_bpm).round() as usize).max(min_lag + 1);

    let n = flux.len();
    if n <= max_lag + 1 {
        return degenerate_report();
    }

    let Some((period_frames, confidence)) = best_tempo_lag(&flux, min_lag, max_lag, frame_rate)
    else {
        return degenerate_report();
    };

    let bpm = frame_rate * 60.0 / period_frames as f64;
    let beats_ms: Vec<f64> = beat_grid(&flux, period_frames, BEAT_DP_LAMBDA)
        .into_iter()
        .map(|t| frame_center_ms(t, sample_rate))
        .collect();

    let onset_rate = if duration_s > 0.0 {
        onset_report.count as f64 / duration_s
    } else {
        0.0
    };
    let clear_rhythm = confidence >= CLEAR_RHYTHM_MIN_CONFIDENCE
        && onset_rate >= CLEAR_RHYTHM_MIN_ONSET_RATE_PER_S;

    TempoReport {
        bpm: Some(bpm),
        confidence,
        clear_rhythm,
        beats_ms,
    }
}

/// Direct-summation autocorrelation of `flux` over `[min_lag, max_lag]`
/// (fixed ascending summation order — deterministic), weighted by
/// [`tempo_prior_weight`]. Returns `(best_lag_frames, confidence)`, or
/// `None` if the envelope has no measurable positive autocorrelation mass
/// anywhere in range (silence, or too little signal).
fn best_tempo_lag(
    flux: &[f64],
    min_lag: usize,
    max_lag: usize,
    frame_rate: f64,
) -> Option<(usize, f64)> {
    let n = flux.len();
    let mut ac = vec![0.0f64; max_lag - min_lag + 1];
    let mut best_i = 0usize;
    let mut best_weighted = f64::MIN;

    for (i, lag) in (min_lag..=max_lag).enumerate() {
        let mut sum = 0.0f64;
        for t in 0..n - lag {
            sum += flux[t] * flux[t + lag];
        }
        ac[i] = sum;

        let bpm = frame_rate * 60.0 / lag as f64;
        let weighted = sum * tempo_prior_weight(bpm);
        if weighted > best_weighted {
            best_weighted = weighted;
            best_i = i;
        }
    }

    let mass: f64 = ac.iter().copied().map(|v| v.max(0.0)).sum();
    if mass <= f64::EPSILON {
        return None;
    }

    let confidence = (best_weighted.max(0.0) / mass).clamp(0.0, 1.0);
    Some((min_lag + best_i, confidence))
}

/// Log-Gaussian tempo preference: `exp(-0.5 * (log2(bpm / center) /
/// sigma)^2)`, `1.0` at `TEMPO_PRIOR_CENTER_BPM`, decaying with octave
/// distance from it.
fn tempo_prior_weight(bpm: f64) -> f64 {
    let octaves = libm::log2(bpm / TEMPO_PRIOR_CENTER_BPM);
    let z = octaves / TEMPO_PRIOR_SIGMA_OCTAVES;
    libm::exp(-0.5 * z * z)
}

/// Ellis-style dynamic-programming beat grid at a fixed `period_frames`,
/// operating on the onset-strength envelope `flux`.
///
/// `score[t] = flux[t] + max(0, max_{tau in [t - 2P, t - P/2]}
/// (score[tau] - lambda * ln((t - tau) / P)^2))` — either frame `t` starts
/// a fresh beat sequence (the `0` alternative), or it extends the best-
/// scoring prior beat `tau` within one half- to two-period lookback
/// window, penalized by how far the resulting interval strays from `P` in
/// log space. The beat sequence is recovered by backtracing from the
/// single highest-scoring frame in the whole buffer.
fn beat_grid(flux: &[f64], period_frames: usize, lambda: f64) -> Vec<usize> {
    let n = flux.len();
    if n == 0 || period_frames == 0 {
        return Vec::new();
    }

    let period = period_frames as f64;
    let half_period = ((period_frames as f64 / 2.0).round() as usize).max(1);
    let two_periods = 2 * period_frames;

    let mut score = vec![0.0f64; n];
    let mut back: Vec<Option<usize>> = vec![None; n];

    for t in 0..n {
        let lo = t.saturating_sub(two_periods);
        let hi = t.saturating_sub(half_period);

        let mut best_extra = 0.0f64;
        let mut best_prev = None;
        if lo <= hi && hi < t {
            for (offset, &prev_score) in score[lo..=hi].iter().enumerate() {
                let tau = lo + offset;
                let interval = (t - tau) as f64;
                let log_ratio = libm::log(interval / period);
                let transition = -lambda * log_ratio * log_ratio;
                let candidate = prev_score + transition;
                if candidate > best_extra {
                    best_extra = candidate;
                    best_prev = Some(tau);
                }
            }
        }
        score[t] = flux[t] + best_extra;
        back[t] = best_prev;
    }

    let mut end = 0;
    for (t, &s) in score.iter().enumerate().skip(1) {
        if s > score[end] {
            end = t;
        }
    }

    let mut beats = Vec::new();
    let mut cur = Some(end);
    while let Some(t) = cur {
        beats.push(t);
        cur = back[t];
    }
    beats.reverse();
    beats
}

/// Frame-center time, milliseconds — the same convention
/// [`crate::OnsetsReport::times_ms`] uses, so beat times and onset times
/// are directly comparable.
fn frame_center_ms(frame: usize, sample_rate: u32) -> f64 {
    (frame as f64 * onsets::HOP as f64 + onsets::FFT_SIZE as f64 / 2.0) / f64::from(sample_rate)
        * 1000.0
}
