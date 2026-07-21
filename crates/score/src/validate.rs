//! Static validation: semantic lints over a structurally well-formed score.
//! Structural problems (bad positions, off-grid durations, unknown tracks)
//! are hard errors at build/load time; these lints catch what's *legal but
//! wrong* — and, given a [`Catalog`], what disagrees with the instruments.

use crate::param::Param;
use crate::score::{EaseSpec, Instrument, Score};

/// How many notes an instrument voices at once. Declared by the synth's
/// registry; the score only uses it to lint note overlaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polyphony {
    Mono,
    Poly(u8),
}

/// One automatable parameter as an instrument declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub param: Param,
    /// Human-facing unit, e.g. `"Hz"`, `"linear"`, `"pan"`.
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

/// What validation needs to know about one instrument.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentInfo {
    pub polyphony: Polyphony,
    pub params: Vec<ParamInfo>,
}

/// The synth registry's face toward score validation — keeps this crate
/// free of any synth dependency while letting lints know param ranges and
/// polyphony. `cochlea-synth`'s preset registry implements it.
pub trait Catalog {
    /// Info for a preset name, or `None` if unknown.
    fn instrument(&self, name: &str) -> Option<InstrumentInfo>;
    /// Whether an insert preset name exists.
    fn insert(&self, name: &str) -> bool;
    /// Every instrument name this catalog can resolve, sorted — powers
    /// error messages and the self-describing authoring reference.
    fn instrument_names(&self) -> Vec<String>;
    /// Every insert name this catalog can resolve, sorted.
    fn insert_names(&self) -> Vec<String>;
}

/// Lint severity: `Error` findings fail `cochlea lint` (nonzero exit);
/// warnings inform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Severity {
    Warning,
    Error,
}

/// One lint finding, with a stable machine-readable `code`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LintFinding {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

impl LintFinding {
    fn error(code: &'static str, message: String) -> LintFinding {
        LintFinding {
            severity: Severity::Error,
            code,
            message,
        }
    }

    fn warning(code: &'static str, message: String) -> LintFinding {
        LintFinding {
            severity: Severity::Warning,
            code,
            message,
        }
    }
}

impl Score {
    /// Catalog-independent lints: empty tracks, spring easing on automation
    /// (v1 rejects it — see `docs/plan.md`), custom instrument references.
    pub fn validate_standalone(&self) -> Vec<LintFinding> {
        let mut findings = Vec::new();
        for track in self.tracks() {
            if track.notes.is_empty() && track.automation.is_empty() {
                findings.push(LintFinding::warning(
                    "empty-track",
                    format!("track {:?} has no notes or automation", track.name),
                ));
            }
            if let Instrument::Custom(name) = &track.instrument {
                findings.push(LintFinding::warning(
                    "custom-instrument",
                    format!(
                        "track {:?} uses custom instrument {name:?}: renders only \
                         with a PatchBank supplying it (code-only, not in presets)",
                        track.name
                    ),
                ));
            }
            for auto in &track.automation {
                for key in &auto.keys {
                    if matches!(key.ease, EaseSpec::Spring { .. }) {
                        findings.push(LintFinding::error(
                            "spring-ease",
                            format!(
                                "track {:?}, param {}: spring easing on automation is \
                                 not supported in v1 (springs need seconds grounding \
                                 across tempo changes); use bezier",
                                track.name, auto.param
                            ),
                        ));
                    }
                }
            }
        }
        findings
    }

    /// All lints, including the catalog-backed ones: unknown presets and
    /// params, automation values outside declared ranges, overlapping notes
    /// on mono instruments.
    pub fn validate(&self, catalog: &dyn Catalog) -> Vec<LintFinding> {
        let mut findings = self.validate_standalone();
        for track in self.tracks() {
            for insert in &track.inserts {
                if !catalog.insert(insert.name()) {
                    findings.push(LintFinding::error(
                        "unknown-insert",
                        format!("track {:?}: unknown insert {:?}", track.name, insert.name()),
                    ));
                }
            }
            let Instrument::Preset(preset) = &track.instrument else {
                continue; // custom: already warned; nothing to check against
            };
            let Some(info) = catalog.instrument(preset) else {
                findings.push(LintFinding::error(
                    "unknown-preset",
                    format!("track {:?}: unknown preset {preset:?}", track.name),
                ));
                continue;
            };
            for auto in &track.automation {
                let Some(spec) = info.params.iter().find(|p| p.param == auto.param) else {
                    findings.push(LintFinding::error(
                        "unknown-param",
                        format!(
                            "track {:?}: automation targets {}, which {preset:?} does \
                             not declare",
                            track.name, auto.param
                        ),
                    ));
                    continue;
                };
                for key in &auto.keys {
                    if key.value < spec.min || key.value > spec.max {
                        findings.push(LintFinding::error(
                            "param-range",
                            format!(
                                "track {:?}: {} = {} at tick {} outside declared range \
                                 {}..={} {}",
                                track.name,
                                auto.param,
                                key.value,
                                key.at,
                                spec.min,
                                spec.max,
                                spec.unit
                            ),
                        ));
                    }
                }
            }
            if info.polyphony == Polyphony::Mono {
                let mut notes = track.notes.clone();
                notes.sort_by_key(|n| n.at);
                for pair in notes.windows(2) {
                    if pair[1].at < pair[0].end() {
                        findings.push(LintFinding::warning(
                            "mono-overlap",
                            format!(
                                "track {:?}: {} at tick {} overlaps {} at tick {} on a \
                                 mono instrument — the older note will be stolen",
                                track.name, pair[1].pitch, pair[1].at, pair[0].pitch, pair[0].at
                            ),
                        ));
                    }
                }
            }
        }
        findings
    }
}
