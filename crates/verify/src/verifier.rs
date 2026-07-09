//! The chainable [`Verifier`] builder and its [`VerifyExt`] entry point.

use std::ops::Range;

use cochlea_render::Rendered;
use cochlea_score::{Param, Pos, Score, VerifySpec};

use crate::checks;
use crate::checks::util::resolve_pos;
use crate::report::{CheckResult, SCHEMA_VERSION, VerifyReport};
use crate::types::{BpmTol, Cents, Db, Ms, Tol};

/// Extension trait: `rendered.verify(&score)` starts a chainable
/// [`Verifier`] over a completed render.
pub trait VerifyExt {
    /// Starts a chainable verification run over `self` against `score`.
    fn verify<'a>(&'a self, score: &'a Score) -> Verifier<'a>;
}

impl VerifyExt for Rendered {
    fn verify<'a>(&'a self, score: &'a Score) -> Verifier<'a> {
        Verifier::new(self, score)
    }
}

/// A chainable set of assertions over one render, checked against the
/// score that produced it. Each builder method evaluates its check
/// immediately — checks are read-only over `rendered`/`score`, so there's
/// nothing gained by deferring — and queues the [`CheckResult`];
/// [`Verifier::run`] assembles the final [`VerifyReport`].
///
/// No builder method panics on a bad *track name*: a check naming a track
/// the score/render doesn't have fails with a detail message instead of
/// panicking. A malformed [`Pos`] (e.g. a beat outside the time signature)
/// still panics, matching `cochlea_score::Score`'s own chainable-builder
/// convention — that's an authoring bug at the call site, not a runtime
/// condition a verification report should describe.
pub struct Verifier<'a> {
    rendered: &'a Rendered,
    score: &'a Score,
    checks: Vec<CheckResult>,
}

impl<'a> Verifier<'a> {
    /// Starts a verification run with no checks queued.
    #[must_use]
    pub fn new(rendered: &'a Rendered, score: &'a Score) -> Verifier<'a> {
        Verifier {
            rendered,
            score,
            checks: Vec::new(),
        }
    }

    /// Asserts the mix's integrated (program) loudness is within `tol` LU
    /// of `target` LUFS. Fails (with a detail note) if the mix has no
    /// gated loudness at all (silence).
    #[must_use]
    pub fn integrated_lufs(mut self, target: f64, tol: Tol) -> Self {
        let result = checks::loudness::integrated_lufs(self.rendered, target, tol.0);
        self.checks.push(result);
        self
    }

    /// Asserts the mix's true peak is at or below `dbtp` dBTP. Passes
    /// (with a detail note) if the mix has no measurable peak (silence).
    #[must_use]
    pub fn true_peak_below(mut self, dbtp: f64) -> Self {
        let result = checks::loudness::true_peak_below(self.rendered, dbtp);
        self.checks.push(result);
        self
    }

    /// Asserts `track`'s stem has a detected onset within `tol` of `at`.
    #[must_use]
    pub fn onset_at(mut self, track: &str, at: impl Into<Pos>, tol: Ms) -> Self {
        let ticks = resolve_pos(self.score, at);
        let result = checks::onset::onset_at(self.rendered, self.score, track, ticks, tol.0);
        self.checks.push(result);
        self
    }

    /// Asserts every note on `track` YINs within `tol` of its scored
    /// pitch. **Monophonic tracks only** — see the `pitch_matches_score`
    /// module docs.
    #[must_use]
    pub fn pitch_matches_score(mut self, track: &str, tol: Cents) -> Self {
        let result = checks::pitch::pitch_matches_score(self.rendered, self.score, track, tol.0);
        self.checks.push(result);
        self
    }

    /// Asserts `track`'s authored automation curve for `param` is
    /// monotone (non-strict) over `range`, sampled at block-start ticks
    /// (64 samples — the render engine's automation control rate).
    /// Direction is inferred by comparing the curve's value at
    /// `range.start` vs `range.end`.
    #[must_use]
    pub fn monotone(mut self, track: &str, param: Param, range: Range<Pos>) -> Self {
        let from = resolve_pos(self.score, range.start);
        let to = resolve_pos(self.score, range.end);
        let result = checks::monotone::monotone(self.score, track, &param, from, to, None);
        self.checks.push(result);
        self
    }

    /// Asserts `track`'s stem has no sample-to-sample jump louder than
    /// `-db.0` dBFS, outside a ±10 ms guard window around every note
    /// on/off boundary and ignoring pairs already below -70 dBFS — a click
    /// detector.
    #[must_use]
    pub fn no_discontinuity(mut self, track: &str, db: Db) -> Self {
        let result =
            checks::discontinuity::no_discontinuity(self.rendered, self.score, track, db.0);
        self.checks.push(result);
        self
    }

    /// Asserts the mix's windowed RMS stays below -60 dBFS from `at` to
    /// the end of the render.
    #[must_use]
    pub fn silent_after(mut self, at: impl Into<Pos>) -> Self {
        let ticks = resolve_pos(self.score, at);
        let result = checks::silence::silent_after(self.rendered, self.score, ticks);
        self.checks.push(result);
        self
    }

    /// Asserts the mix's estimated tempo (`cochlea_features::estimate_tempo`)
    /// is within `tol` of `bpm`, searching the detector's default 30–300
    /// BPM range.
    #[must_use]
    pub fn tempo_is(mut self, bpm: f64, tol: BpmTol) -> Self {
        let result = checks::tempo::tempo_is(self.rendered, bpm, tol.0, None, None);
        self.checks.push(result);
        self
    }

    /// [`Self::tempo_is`] with an explicit detector search range — the
    /// escape hatch for fast material, where the octave-error prior would
    /// otherwise favor the half-time subharmonic (structurally so above
    /// ~170 BPM with the default range).
    #[must_use]
    pub fn tempo_is_in_range(mut self, bpm: f64, tol: BpmTol, min_bpm: f64, max_bpm: f64) -> Self {
        let result =
            checks::tempo::tempo_is(self.rendered, bpm, tol.0, Some(min_bpm), Some(max_bpm));
        self.checks.push(result);
        self
    }

    /// Asserts the mix's estimated tempo's trustworthiness
    /// (`cochlea_features::TempoReport::clear_rhythm`) equals `expected`
    /// — pass `true` to assert a clear, steady pulse, or `false` to assert
    /// its absence (a low-confidence or non-rhythmic mix).
    #[must_use]
    pub fn has_clear_rhythm(mut self, expected: bool) -> Self {
        let result = checks::tempo::has_clear_rhythm(self.rendered, expected);
        self.checks.push(result);
        self
    }

    /// Asserts the mix's stereo width
    /// (`cochlea_features::StereoReport::width`, `0.0..=1.0`) falls
    /// within `[min, max]`.
    #[must_use]
    pub fn stereo_width_within(mut self, min: f64, max: f64) -> Self {
        let result = checks::stereo::stereo_width_within(self.rendered, min, max);
        self.checks.push(result);
        self
    }

    /// Asserts the mix's EBU R128 loudness range (LRA) is at or below
    /// `lu` LU.
    #[must_use]
    pub fn lra_below(mut self, lu: f64) -> Self {
        let result = checks::loudness::lra_below(self.rendered, lu);
        self.checks.push(result);
        self
    }

    /// Asserts the mix's detected structural section count
    /// (`cochlea_features::StructureReport::section_count`) falls within
    /// `[min, max]`.
    #[must_use]
    pub fn section_count(mut self, min: usize, max: usize) -> Self {
        let result = checks::structure::section_count(self.rendered, min, max);
        self.checks.push(result);
        self
    }

    /// Queues the check a [`VerifySpec`] (the RON `verify:` data form)
    /// describes — the same checks the typed builder methods run, driven
    /// by data instead of code.
    #[must_use]
    pub fn with_spec(mut self, spec: &VerifySpec) -> Self {
        let result = eval_spec(self.rendered, self.score, spec);
        self.checks.push(result);
        self
    }

    /// [`Verifier::with_spec`] for every spec in `specs`, in order.
    #[must_use]
    pub fn with_specs(mut self, specs: &[VerifySpec]) -> Self {
        for spec in specs {
            let result = eval_spec(self.rendered, self.score, spec);
            self.checks.push(result);
        }
        self
    }

    /// Runs every queued check and assembles the report. Infallible: a
    /// check referencing an unknown track fails with a detail message
    /// rather than panicking (see the type docs).
    #[must_use]
    pub fn run(self) -> VerifyReport {
        let passed = self.checks.iter().all(|c| c.passed);
        VerifyReport {
            schema_version: SCHEMA_VERSION,
            passed,
            checks: self.checks,
        }
    }
}

/// Maps one [`VerifySpec`] variant onto the same internal check the
/// corresponding typed builder method runs.
fn eval_spec(rendered: &Rendered, score: &Score, spec: &VerifySpec) -> CheckResult {
    match spec {
        VerifySpec::IntegratedLufs { target, tol } => {
            checks::loudness::integrated_lufs(rendered, *target, *tol)
        }
        VerifySpec::TruePeakBelow { dbtp } => checks::loudness::true_peak_below(rendered, *dbtp),
        VerifySpec::OnsetAt { track, at, tol_ms } => {
            checks::onset::onset_at(rendered, score, track, *at, *tol_ms)
        }
        VerifySpec::PitchMatchesScore { track, tol_cents } => {
            checks::pitch::pitch_matches_score(rendered, score, track, *tol_cents)
        }
        VerifySpec::Monotone {
            track,
            param,
            from,
            to,
            direction,
        } => checks::monotone::monotone(score, track, param, *from, *to, Some(*direction)),
        VerifySpec::NoDiscontinuity { track, db } => {
            checks::discontinuity::no_discontinuity(rendered, score, track, *db)
        }
        VerifySpec::SilentAfter { at } => checks::silence::silent_after(rendered, score, *at),
        VerifySpec::TempoIs {
            bpm,
            tol_bpm,
            min_bpm,
            max_bpm,
        } => checks::tempo::tempo_is(rendered, *bpm, *tol_bpm, *min_bpm, *max_bpm),
        VerifySpec::HasClearRhythm { expected } => {
            checks::tempo::has_clear_rhythm(rendered, *expected)
        }
        VerifySpec::StereoWidthWithin { min, max } => {
            checks::stereo::stereo_width_within(rendered, *min, *max)
        }
        VerifySpec::LraBelow { lu } => checks::loudness::lra_below(rendered, *lu),
        VerifySpec::SectionCount { min, max } => {
            checks::structure::section_count(rendered, *min, *max)
        }
    }
}
