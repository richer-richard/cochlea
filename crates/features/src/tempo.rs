//! Tempo (speed) tracking: autocorrelation of the onset-strength envelope
//! over a BPM search range, a log-Gaussian tempo preference to steer away
//! from octave errors, and an Ellis-style ("Beat Tracking by Dynamic
//! Programming", Ellis 2007) dynamic-programming beat grid. Implemented
//! from the paper's publicly described method, not from any GPL codebase.
//!
//! **Tempo vs rhythm.** This module answers one question only: *how fast is
//! the pulse, and how periodic is the envelope at that pulse?* Whether the
//! onsets form a rhythm an agent should trust — whether they actually land
//! on that pulse's grid — is the [`crate::rhythm`] module's question. A
//! drum solo can hold a rock-steady tempo while its rhythm turns
//! unrecognizable; a rubato ballad can have clear rhythmic figures over an
//! unsteady pulse. The two are reported independently so an agent can tell
//! those situations apart.
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
//!    lag is the tempo estimate. Other prominent peaks are reported as
//!    [`TempoCandidate`]s so a caller can weigh the octave alternatives
//!    itself instead of trusting the prior blindly.
//! 4. Dynamic-programming beat grid at that period: greedily gridding on
//!    the estimated period would drift whenever the true period isn't an
//!    exact integer number of frames (it essentially never is), so instead
//!    each frame's cumulative score rewards a strong local onset *and*
//!    penalizes its interval from the previous chosen beat for deviating
//!    from the estimated period, letting real onsets pull the grid back
//!    into alignment every beat instead of accumulating drift.
//! 5. Windowed re-estimation for [`TempoReport::stability`]: the same
//!    detector over non-overlapping thirds/quarters of the envelope, scored
//!    by how many windows agree (mod octave) with the whole-buffer answer.

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
/// cumulative score, with the flux envelope normalized to unit standard
/// deviation first (see [`beat_grid`]) so this weight has stable units —
/// Ellis 2007 calibrates its α against a normalized envelope for the same
/// reason. Calibration (this module's fixtures): a half-period snap costs
/// `400 * ln(0.5)^2 ≈ 192`, far above a normalized onset peak (~10), so
/// the grid can't degenerate into "every onset is a beat"; a ±5% interval
/// flex costs ~1, far below one, so the grid still follows human timing
/// and re-locks every beat instead of accumulating drift.
const BEAT_DP_LAMBDA: f64 = 400.0;

/// Maximum [`TempoCandidate`]s reported (including the winner).
const MAX_CANDIDATES: usize = 3;
/// Candidate peaks closer than this fraction of an already-accepted peak's
/// lag are suppressed as duplicates of the same periodicity (adjacent-lag
/// shoulders of one autocorrelation peak, not genuinely distinct tempos).
const CANDIDATE_MIN_LAG_SEPARATION: f64 = 0.10;

/// Windows for the [`TempoReport::stability`] re-estimation. Each window
/// must still fit the full lag search range with margin, so short buffers
/// degrade to fewer windows (and below two usable windows, stability is
/// `None` — see [`estimate_from_parts`]).
const STABILITY_WINDOWS: usize = 4;
/// A window's locally-best lag agrees with the whole-buffer lag when it is
/// within this relative distance after octave folding (x2 / x0.5 are the
/// same pulse heard at a different metrical level, not a tempo change).
const STABILITY_LAG_TOL: f64 = 0.05;

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

/// One tempo hypothesis: a prominent autocorrelation peak in the search
/// range. The winner (the report's `bpm`) is always `candidates[0]`; the
/// rest are the strongest *distinct* alternatives (usually the winner's
/// half- or double-tempo octave), so a caller facing an ambiguous groove
/// can weigh both readings itself instead of trusting the octave prior
/// blindly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TempoCandidate {
    /// This hypothesis' tempo, BPM.
    pub bpm: f64,
    /// This lag's pulse clarity — the same normalized-autocorrelation units
    /// as [`TempoReport::confidence`] (`0.0..=1.0`), *without* the octave
    /// prior, so candidates are directly comparable to each other on the
    /// evidence alone.
    pub salience: f64,
}

/// Tempo (speed) tracking result. Rhythm-level judgments — does the music
/// actually articulate this pulse? — live in [`crate::RhythmReport`], not
/// here (see the module docs on the tempo/rhythm split).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoReport {
    /// Estimated tempo, BPM. `None` for degenerate input (empty/too-short
    /// audio, or an onset-strength envelope with no measurable energy).
    pub bpm: Option<f64>,
    /// Pulse clarity at the winning lag, `0.0..=1.0`: the envelope's
    /// normalized autocorrelation `ac[lag] / ac[0]` (length-unbiased). A
    /// perfectly periodic envelope scores near `1.0`; an aperiodic one near
    /// `0.0`. Unlike a mass-fraction normalization, this does *not* punish
    /// music for carrying several metrical levels at once (16ths + quarters
    /// + bars all raise `ac[beat lag]` together). See [`estimate_tempo`].
    pub confidence: f64,
    /// The winning tempo plus the strongest distinct alternatives
    /// (typically octave relatives), strongest-first; `candidates[0].bpm ==
    /// bpm` whenever `bpm` is `Some`. At most three entries. Empty exactly
    /// when `bpm` is `None`.
    pub candidates: Vec<TempoCandidate>,
    /// Fraction of analysis windows whose locally-best tempo agrees (mod
    /// octave) with the whole-buffer tempo, `0.0..=1.0` — a steady-tempo
    /// piece (even one whose *rhythm* goes wild, e.g. a drum solo) scores
    /// near `1.0`; a piece that actually changes speed scores lower. `None`
    /// when the buffer is too short to form at least two windows that each
    /// fit the lag search range.
    pub stability: Option<f64>,
    /// Beat grid, milliseconds, ascending. Empty when `bpm` is `None`.
    pub beats_ms: Vec<f64>,
}

fn degenerate_report() -> TempoReport {
    TempoReport {
        bpm: None,
        confidence: 0.0,
        candidates: Vec::new(),
        stability: None,
        beats_ms: Vec::new(),
    }
}

/// Estimate tempo, alternatives, stability, and a beat grid for `audio`
/// (see the module docs for the pipeline).
///
/// **`confidence`** (pulse clarity): at the winning lag, `ac[lag] /
/// ac[0]`, where `ac[0] = sum(flux[t]^2)` is the envelope's total energy
/// and the truncated-sum `ac[lag]` is rescaled by `n / (n - lag)` so long
/// lags aren't penalized just for summing fewer terms. Bounded to
/// `0.0..=1.0` (Cauchy-Schwarz, then a defensive clamp for the unbias
/// rescale). The octave *prior* still picks which lag wins, but it does not
/// leak into the confidence value — confidence reports the evidence, the
/// prior only breaks octave ties.
///
/// Degenerate input (empty/near-empty audio, a zero sample rate, or an
/// onset-strength envelope with no measurable energy) returns `bpm: None`,
/// `confidence: 0.0`, no candidates, `stability: None`, `beats_ms: []` —
/// never panics.
pub fn estimate_tempo(audio: &Audio, opts: &TempoOpts) -> TempoReport {
    let mono = audio.mono();
    if audio.sample_rate == 0 || mono.len() < onsets::FFT_SIZE {
        return degenerate_report();
    }
    let stft = Stft::compute(&mono, audio.sample_rate, onsets::FFT_SIZE, onsets::HOP);
    let onset_report = onsets::analyze_stft(&stft);
    estimate_from_parts(&stft, &onset_report, audio.sample_rate, opts)
}

/// The estimator over caller-provided shared parts (the onsets-grade STFT
/// and its onset report) — `probe()` computes both exactly once and feeds
/// every consumer, instead of this module recomputing them.
pub(crate) fn estimate_from_parts(
    stft: &Stft,
    onset_report: &OnsetsReport,
    sample_rate: u32,
    opts: &TempoOpts,
) -> TempoReport {
    if sample_rate == 0 || stft.magnitudes.len() < 3 {
        return degenerate_report();
    }
    // Tempo is the rate of *events*. An envelope can be periodic with no
    // events at all — a sustained tone's window-sliding flux ripple is
    // genuinely periodic and measured pulse clarity 0.99 before this gate
    // — but with fewer than two detected onsets there is no pulse to name
    // a speed for, so the honest answer is "no tempo", not a confident
    // BPM of the measurement artifact.
    if onset_report.count < 2 {
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

    let Some(picked) = pick_tempo_peaks(&flux, min_lag, max_lag, frame_rate) else {
        return degenerate_report();
    };
    let period_frames = picked.winner_lag;
    let confidence = picked.winner_salience;

    let bpm = frame_rate * 60.0 / period_frames as f64;
    let candidates: Vec<TempoCandidate> = picked
        .peaks
        .iter()
        .map(|&(lag, salience)| TempoCandidate {
            bpm: frame_rate * 60.0 / lag as f64,
            salience,
        })
        .collect();

    let beats_ms: Vec<f64> = beat_grid(&flux, period_frames, BEAT_DP_LAMBDA)
        .into_iter()
        .map(|t| frame_center_ms(t, sample_rate))
        .collect();

    let stability = stability_of(&flux, min_lag, max_lag, frame_rate, period_frames);

    TempoReport {
        bpm: Some(bpm),
        confidence,
        candidates,
        stability,
        beats_ms,
    }
}

/// The winning lag plus up to [`MAX_CANDIDATES`] distinct peak lags, each
/// with its pulse-clarity salience.
struct PickedPeaks {
    winner_lag: usize,
    winner_salience: f64,
    /// `(lag, salience)`, winner first, then alternatives strongest-first.
    peaks: Vec<(usize, f64)>,
}

/// Direct-summation autocorrelation of `flux` over `[min_lag, max_lag]`
/// (fixed ascending summation order — deterministic). The *winner* is the
/// prior-weighted best lag; saliences are unweighted pulse clarity
/// (`ac[lag] / ac[0]`, length-unbiased — see [`estimate_tempo`]).
/// Alternatives are the strongest weighted local maxima at least
/// [`CANDIDATE_MIN_LAG_SEPARATION`] away from every already-accepted lag.
/// Returns `None` if the envelope has no energy at all.
fn pick_tempo_peaks(
    flux: &[f64],
    min_lag: usize,
    max_lag: usize,
    frame_rate: f64,
) -> Option<PickedPeaks> {
    let n = flux.len();
    // Mean-removed autocorrelation (autocovariance): without mean removal,
    // any envelope with a large DC component — a sustained tone's residual
    // flux ripple, a dense wash — autocorrelates near 1.0 at *every* lag
    // and reads as maximal pulse clarity (measured 0.996 on a plain sine
    // before this fix). Periodicity of the *variation* is what a pulse is.
    let mean = flux.iter().sum::<f64>() / n as f64;
    let centered: Vec<f64> = flux.iter().map(|v| v - mean).collect();
    let ac0: f64 = centered.iter().map(|v| v * v).sum();
    if ac0 <= f64::EPSILON {
        return None;
    }

    let lag_count = max_lag - min_lag + 1;
    let mut clarity = vec![0.0f64; lag_count]; // unbiased ac[lag] / ac[0]
    let mut weighted = vec![0.0f64; lag_count]; // clarity * octave prior
    for (i, lag) in (min_lag..=max_lag).enumerate() {
        let mut sum = 0.0f64;
        for t in 0..n - lag {
            sum += centered[t] * centered[t + lag];
        }
        // Length-unbias the truncated sum, then normalize by the envelope's
        // own variance; clamp into range (covariance can go negative — an
        // anti-correlated lag is simply zero clarity — and the unbias
        // rescale can overshoot 1.0 by a hair for near-perfect periodicity).
        let c = ((sum * n as f64) / (ac0 * (n - lag) as f64)).clamp(0.0, 1.0);
        clarity[i] = c;
        let bpm = frame_rate * 60.0 / lag as f64;
        weighted[i] = c * tempo_prior_weight(bpm);
    }

    // Winner: best weighted value anywhere (not just at a local maximum, so
    // a monotone edge case still yields an answer).
    let mut best_i = 0usize;
    for (i, &w) in weighted.iter().enumerate() {
        if w > weighted[best_i] {
            best_i = i;
        }
    }
    if weighted[best_i] <= 0.0 {
        return None;
    }

    // Alternatives: weighted local maxima, strongest-first, suppressing
    // lags within CANDIDATE_MIN_LAG_SEPARATION of any accepted lag.
    let mut maxima: Vec<usize> = (0..lag_count)
        .filter(|&i| {
            let left_ok = i == 0 || weighted[i] >= weighted[i - 1];
            let right_ok = i + 1 == lag_count || weighted[i] > weighted[i + 1];
            left_ok && right_ok && weighted[i] > 0.0
        })
        .collect();
    maxima.sort_by(|&a, &b| weighted[b].total_cmp(&weighted[a]).then(a.cmp(&b)));

    let mut accepted: Vec<usize> = vec![best_i];
    for &i in &maxima {
        if accepted.len() >= MAX_CANDIDATES {
            break;
        }
        let lag = min_lag + i;
        let distinct = accepted.iter().all(|&j| {
            let a_lag = (min_lag + j) as f64;
            (lag as f64 - a_lag).abs() / a_lag > CANDIDATE_MIN_LAG_SEPARATION
        });
        if distinct {
            accepted.push(i);
        }
    }

    Some(PickedPeaks {
        winner_lag: min_lag + best_i,
        winner_salience: clarity[best_i],
        peaks: accepted
            .into_iter()
            .map(|i| (min_lag + i, clarity[i]))
            .collect(),
    })
}

/// Windowed tempo agreement (see [`TempoReport::stability`]): re-run the
/// lag search over up to [`STABILITY_WINDOWS`] non-overlapping chunks of
/// the envelope, and score the fraction whose best lag matches
/// `global_lag` within [`STABILITY_LAG_TOL`] after octave folding.
/// `None` below two usable windows.
fn stability_of(
    flux: &[f64],
    min_lag: usize,
    max_lag: usize,
    frame_rate: f64,
    global_lag: usize,
) -> Option<f64> {
    let n = flux.len();
    // Each window must comfortably fit the search range: the same
    // `n <= max_lag + 1` degeneracy rule the whole-buffer path uses, plus
    // one extra lag of margin.
    let min_window = max_lag + 2;
    let window_count = (n / min_window).min(STABILITY_WINDOWS);
    if window_count < 2 {
        return None;
    }
    let window_len = n / window_count;

    let mut agree = 0usize;
    for w in 0..window_count {
        let chunk = &flux[w * window_len..(w + 1) * window_len];
        let Some(picked) = pick_tempo_peaks(chunk, min_lag, max_lag, frame_rate) else {
            continue; // a silent window neither agrees nor disagrees loudly
        };
        if lags_agree_mod_octave(picked.winner_lag as f64, global_lag as f64) {
            agree += 1;
        }
    }
    Some(agree as f64 / window_count as f64)
}

/// Whether two lags describe the same pulse within [`STABILITY_LAG_TOL`],
/// treating x2 / x0.5 (and x4 / x0.25) octave relatives as agreement — a
/// window that locks onto the half-time or double-time level of the same
/// groove hasn't detected a speed change.
fn lags_agree_mod_octave(a: f64, b: f64) -> bool {
    for fold in [0.25, 0.5, 1.0, 2.0, 4.0] {
        let folded = a * fold;
        if (folded - b).abs() / b <= STABILITY_LAG_TOL {
            return true;
        }
    }
    false
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

    // Normalize the envelope to unit standard deviation so `lambda` has
    // stable units (see [`BEAT_DP_LAMBDA`]): raw spectral-flux magnitudes
    // scale with signal level and FFT size, and an unnormalized envelope
    // makes the period penalty negligible — the "beat grid" then snaps to
    // every onset (measured: an eighth-note track's grid landed on all the
    // eighths, and even random onsets got a flattering grid).
    let mean = flux.iter().sum::<f64>() / n as f64;
    let variance = flux.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
    if variance <= f64::EPSILON {
        return Vec::new();
    }
    let inv_std = 1.0 / variance.sqrt();
    let flux: Vec<f64> = flux.iter().map(|v| v * inv_std).collect();
    let flux = flux.as_slice();

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
