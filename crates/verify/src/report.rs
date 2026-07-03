//! The verification report: a schema-versioned, serde-JSON list of check
//! outcomes, each with a stable machine code alongside human-readable text.

/// Schema version of [`VerifyReport`]'s JSON form. Bump and document here
/// on any breaking change to the report shape (mirrors
/// `cochlea_features::SCHEMA_VERSION`'s convention).
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// The result of running a [`crate::Verifier`]: `schema_version: 1`,
/// `passed` iff every entry in `checks` passed, in the order the checks
/// were queued.
#[derive(Debug, serde::Serialize)]
pub struct VerifyReport {
    /// Report schema version. Currently always `1`.
    pub schema_version: u32,
    /// `true` iff every check in `checks` passed.
    pub passed: bool,
    /// One entry per queued check, in queue order.
    pub checks: Vec<CheckResult>,
}

/// One check's outcome. `kind` is a stable machine code safe to match or
/// group on; `assertion`/`expected`/`actual` are free-form human-readable
/// text (not meant to be re-parsed — `passed` is the machine-readable
/// verdict, `detail` carries structured-ish extra context like "why" a
/// check failed to even run).
#[derive(Debug, serde::Serialize)]
pub struct CheckResult {
    /// Stable machine code: one of `"integrated_lufs"`, `"true_peak_below"`,
    /// `"onset_at"`, `"pitch_matches_score"`, `"monotone"`,
    /// `"no_discontinuity"`, `"silent_after"`.
    pub kind: &'static str,
    /// Human-readable statement of what was asserted.
    pub assertion: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable rendering of what was expected.
    pub expected: String,
    /// Human-readable rendering of what was measured.
    pub actual: String,
    /// Extra context: set when a check couldn't fully run as asserted
    /// (an unknown track, missing automation, notes skipped as too short
    /// for a stable pitch estimate, undefined/silent loudness, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CheckResult {
    /// The one failure mode every track-scoped check must produce instead
    /// of panicking: `track` doesn't name a track on the render/score.
    pub(crate) fn unknown_track(kind: &'static str, assertion: String, track: &str) -> CheckResult {
        CheckResult {
            kind,
            assertion,
            passed: false,
            expected: format!("a track named {track:?}"),
            actual: "no such track".to_string(),
            detail: Some(format!("unknown track {track:?}")),
        }
    }
}
