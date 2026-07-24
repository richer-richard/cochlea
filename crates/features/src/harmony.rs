//! Harmony: a chord timeline and per-section key, both read off the same
//! chroma the global [`crate::key`] estimate already computes — the two
//! questions ("what's the progression" / "what key is the bridge in") the
//! single global key can't answer.
//!
//! ## Chord detection
//!
//! Each ~250 ms frame's 12-bin chroma is matched against a bank of chord
//! templates (major/minor/dominant-7th/major-7th/minor-7th/diminished/
//! augmented/sus4, in all twelve roots) by cosine similarity. The templates
//! are L2-normalized binary pitch-class masks, which makes the score prefer
//! the *simplest* chord that fits: a bare triad scores higher against the
//! triad mask (norm √3) than against a 7th mask (norm √4) when the seventh's
//! pitch class carries no energy, so a plain major chord doesn't get reported
//! as a major-7th just because the triad is a subset of it. A frame whose best
//! match is below [`MIN_CHORD_COSINE`], or which has no in-range spectral
//! energy at all, is left unlabeled (a gap in the timeline — silence or an
//! atonal passage), not forced onto the nearest chord. Adjacent equal labels
//! merge into spans; runs shorter than [`HarmonyOpts::min_chord_ms`] are
//! smoothed away so momentary spectral wobble doesn't shatter a held chord
//! into a flicker of neighbors.
//!
//! ## Per-section key
//!
//! Section boundaries come from the caller as plain milliseconds (the
//! self-similarity structure detector supplies them, but this module never
//! sees a [`crate::StructureReport`] type — it takes a `&[f64]`, honoring the
//! dependency-direction law). Chroma is summed over each section's frames and
//! run through the *same* Krumhansl-Schmuckler correlation as the global key
//! ([`crate::key::correlate`]), so "the key of bars 17–32" is measured
//! exactly the way "the key of the whole file" is — only the time span
//! differs.
//!
//! Determinism: chroma uses `libm::log2` (never std), the template match is
//! fixed-order integer/float arithmetic, and std `sqrt` is the workspace's one
//! exempt transcendental (IEEE-exact everywhere).

use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::key;
use crate::report::{Mode, PitchClass};
use crate::stft::Stft;
use crate::structure::{StructureOpts, detect_structure};

/// A frame whose best chord match scores below this cosine similarity is left
/// unlabeled rather than forced onto the nearest chord. Tuned so a clearly
/// voiced triad/seventh (typically 0.7–0.95 against its own template) is kept
/// while a wash of inharmonic or single-note energy is not.
const MIN_CHORD_COSINE: f64 = 0.6;

/// A chord template is only *considered* if every one of its tones carries at
/// least this fraction of the frame's peak chroma bin. This is what stops a
/// single strong note (whose spectral leakage sprays a little energy into
/// neighboring pitch classes) from being read as a chord: a lone tone has one
/// dominant pitch class, so no triad — which needs three tones actually
/// sounding — can pass the gate. It also naturally keeps a bare triad from
/// matching a seventh whose extra tone isn't present.
const CHORD_TONE_PRESENCE: f64 = 0.4;

/// When two chord templates fit nearly as well, prefer the *simpler* one (the
/// triad over the seventh that contains it) unless the richer chord scores
/// more than this much higher. Rendered harmonic audio always carries some
/// energy at the upper partials — the 7th harmonic of a triad's root lands a
/// minor seventh up — so a bare triad reliably activates the matching seventh
/// template a little; this margin keeps that from being read as a genuine
/// seventh chord while still reporting a seventh when its tone is prominent.
const SIMPLICITY_MARGIN: f64 = 0.06;

/// The quality of a detected chord, relative to its root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    /// Major triad `{0, 4, 7}`.
    Major,
    /// Minor triad `{0, 3, 7}`.
    Minor,
    /// Dominant seventh `{0, 4, 7, 10}`.
    Dominant7,
    /// Major seventh `{0, 4, 7, 11}`.
    Major7,
    /// Minor seventh `{0, 3, 7, 10}`.
    Minor7,
    /// Diminished triad `{0, 3, 6}`.
    Diminished,
    /// Augmented triad `{0, 4, 8}`.
    Augmented,
    /// Suspended fourth `{0, 5, 7}`.
    Sus4,
}

impl ChordQuality {
    /// The chord tones as semitone offsets above the root.
    const fn intervals(self) -> &'static [usize] {
        match self {
            ChordQuality::Major => &[0, 4, 7],
            ChordQuality::Minor => &[0, 3, 7],
            ChordQuality::Dominant7 => &[0, 4, 7, 10],
            ChordQuality::Major7 => &[0, 4, 7, 11],
            ChordQuality::Minor7 => &[0, 3, 7, 10],
            ChordQuality::Diminished => &[0, 3, 6],
            ChordQuality::Augmented => &[0, 4, 8],
            ChordQuality::Sus4 => &[0, 5, 7],
        }
    }

    /// Lead-sheet suffix appended to the root name (`""` for a plain major
    /// triad, so `C` major reads as `C`; `m` for minor, `7` for dominant
    /// seventh, and so on).
    fn suffix(self) -> &'static str {
        match self {
            ChordQuality::Major => "",
            ChordQuality::Minor => "m",
            ChordQuality::Dominant7 => "7",
            ChordQuality::Major7 => "maj7",
            ChordQuality::Minor7 => "m7",
            ChordQuality::Diminished => "dim",
            ChordQuality::Augmented => "aug",
            ChordQuality::Sus4 => "sus4",
        }
    }

    /// Every quality the detector considers, in template-preference order
    /// (ties break toward the earlier entry — simpler triads before their
    /// seventh extensions).
    const ALL: [ChordQuality; 8] = [
        ChordQuality::Major,
        ChordQuality::Minor,
        ChordQuality::Diminished,
        ChordQuality::Augmented,
        ChordQuality::Sus4,
        ChordQuality::Dominant7,
        ChordQuality::Major7,
        ChordQuality::Minor7,
    ];
}

/// One contiguous span over which a single chord is held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChordSpan {
    /// Start time, milliseconds.
    pub start_ms: f64,
    /// End time, milliseconds (one chord-frame past the last frame of the
    /// span, so consecutive spans tile with no gap where chords are adjacent).
    pub end_ms: f64,
    /// Chord root pitch class.
    pub root: PitchClass,
    /// Chord quality.
    pub quality: ChordQuality,
    /// Lead-sheet symbol, e.g. `"C"`, `"Am"`, `"G7"`, `"Fmaj7"`.
    pub symbol: String,
    /// Mean cosine match strength over the span, `0.0..=1.0`.
    pub confidence: f64,
}

/// The key of one structural section.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SectionKey {
    /// Start time, milliseconds.
    pub start_ms: f64,
    /// End time, milliseconds.
    pub end_ms: f64,
    /// Estimated tonic pitch class for this section.
    pub tonic: PitchClass,
    /// Estimated mode for this section.
    pub mode: Mode,
    /// Krumhansl-Schmuckler correlation for this section's chroma (same scale
    /// as [`crate::KeyReport::confidence`]).
    pub confidence: f64,
}

/// Chord timeline plus per-section key. Plain struct, no own schema version —
/// embedded into [`crate::Report`] (mirrors [`crate::StructureReport`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarmonyReport {
    /// Detected chord spans, ascending in time, with gaps where no chord was
    /// confidently present (silence or atonal passages).
    pub chords: Vec<ChordSpan>,
    /// Per-section key estimates, one per structural section, ascending.
    pub sections: Vec<SectionKey>,
    /// Fraction of analyzed (non-silent) time covered by a confident chord,
    /// `0.0..=1.0` — a rough "how tonal/harmonic is this" summary.
    pub chord_coverage: f64,
}

impl HarmonyReport {
    fn empty() -> HarmonyReport {
        HarmonyReport {
            chords: Vec::new(),
            sections: Vec::new(),
            chord_coverage: 0.0,
        }
    }
}

/// Tunables for [`analyze_harmony`]. Chainable-setter style, matching
/// [`crate::StructureOpts`]/[`crate::SegmentOpts`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HarmonyOpts {
    /// Length of each chord-analysis frame, milliseconds. Default `250.0` —
    /// short enough to catch a chord change on a beat, long enough to average
    /// out per-note attack transients.
    pub chord_frame_ms: f64,
    /// Shortest chord span kept, milliseconds; shorter runs are smoothed into
    /// their neighbors so momentary spectral wobble doesn't flicker a held
    /// chord. Default `500.0`.
    pub min_chord_ms: f64,
}

impl Default for HarmonyOpts {
    fn default() -> Self {
        Self {
            chord_frame_ms: 250.0,
            min_chord_ms: 500.0,
        }
    }
}

impl HarmonyOpts {
    /// Override the chord-analysis frame length, milliseconds.
    #[must_use]
    pub fn with_chord_frame_ms(mut self, ms: f64) -> Self {
        self.chord_frame_ms = ms;
        self
    }

    /// Override the minimum kept chord-span length, milliseconds.
    #[must_use]
    pub fn with_min_chord_ms(mut self, ms: f64) -> Self {
        self.min_chord_ms = ms;
        self
    }
}

/// Analyze `audio` into a chord timeline and per-section key. Standalone
/// entry point: computes its own chroma STFT and section boundaries. Inside
/// [`crate::probe`], [`analyze_from_parts`] is used instead so the STFT and
/// the structure boundaries are shared rather than recomputed.
pub fn analyze_harmony(audio: &Audio, opts: &HarmonyOpts) -> HarmonyReport {
    let mono = audio.mono();
    if mono.len() < key::FFT_SIZE || audio.sample_rate == 0 {
        return HarmonyReport::empty();
    }
    let stft = Stft::compute(&mono, audio.sample_rate, key::FFT_SIZE, key::HOP);
    let structure = detect_structure(audio, &StructureOpts::default());
    analyze_from_parts(&stft, &structure.boundaries_ms, audio.duration_ms(), opts)
}

/// Harmony off an already-computed chroma-grade STFT and precomputed section
/// boundaries (milliseconds, ascending, excluding 0 and the end). This is the
/// shared-work path [`crate::probe`] uses. `duration_ms` bounds the last
/// section and the timeline.
pub(crate) fn analyze_from_parts(
    stft: &Stft,
    boundaries_ms: &[f64],
    duration_ms: f64,
    opts: &HarmonyOpts,
) -> HarmonyReport {
    if stft.magnitudes.is_empty() || stft.sample_rate == 0 {
        return HarmonyReport::empty();
    }
    let frame_chroma = per_frame_chroma(stft);
    let chords = detect_chords(&frame_chroma, opts, duration_ms);
    let sections = section_keys(&frame_chroma, boundaries_ms, duration_ms);
    let chord_coverage = coverage(&chords, duration_ms);
    HarmonyReport {
        chords,
        sections,
        chord_coverage,
    }
}

/// One raw (un-normalized) 12-bin chroma vector per STFT frame, plus the
/// frame's center time in milliseconds. Same pitch-class mapping as
/// [`crate::key`] and [`crate::structure`]: each in-range bin's center
/// frequency maps to the nearest equal-tempered pitch class.
struct FrameChroma {
    /// `chroma[frame][pitch_class]`, raw magnitude sums.
    chroma: Vec<[f64; 12]>,
    /// `center_ms[frame]`, the frame's center time.
    center_ms: Vec<f64>,
}

fn per_frame_chroma(stft: &Stft) -> FrameChroma {
    let sr = f64::from(stft.sample_rate);
    let mut chroma = Vec::with_capacity(stft.magnitudes.len());
    let mut center_ms = Vec::with_capacity(stft.magnitudes.len());
    for (t, frame) in stft.magnitudes.iter().enumerate() {
        let mut c = [0.0f64; 12];
        for (bin, &mag) in frame.iter().enumerate() {
            let hz = stft.bin_hz(bin);
            if !(key::CHROMA_MIN_HZ..=key::CHROMA_MAX_HZ).contains(&hz) {
                continue;
            }
            let midi_float = 69.0 + 12.0 * libm::log2(hz / 440.0);
            let pitch_class = midi_float.round().rem_euclid(12.0) as usize;
            c[pitch_class] += f64::from(mag);
        }
        chroma.push(c);
        center_ms.push((t as f64 * key::HOP as f64 + key::FFT_SIZE as f64 / 2.0) / sr * 1000.0);
    }
    FrameChroma { chroma, center_ms }
}

/// A per-chord-frame label: `Some((root, quality, cosine))` or `None` (a gap).
type ChordLabel = Option<(usize, ChordQuality, f64)>;

/// Match one chord frame's chroma against the template bank, returning the
/// best `(root, quality, cosine)` if it clears [`MIN_CHORD_COSINE`], else
/// `None`. Cosine against L2-normalized binary masks prefers the simplest
/// chord that fits (see the module docs).
fn best_chord(chroma: &[f64; 12]) -> ChordLabel {
    let norm: f64 = chroma.iter().map(|x| x * x).sum::<f64>().sqrt();
    let max: f64 = chroma.iter().copied().fold(0.0, f64::max);
    if norm <= 0.0 || max <= 0.0 {
        return None;
    }
    // A chord tone counts as sounding only above this absolute floor.
    let present_floor = CHORD_TONE_PRESENCE * max;

    // Score every template that passes the presence gate, then pick — see
    // below — the *simplest* chord within `SIMPLICITY_MARGIN` of the best.
    let mut scored: Vec<(usize, ChordQuality, f64)> = Vec::new();
    for &quality in &ChordQuality::ALL {
        let intervals = quality.intervals();
        let mask_norm = (intervals.len() as f64).sqrt();
        for root in 0..12 {
            // Every tone of the chord must actually be present — this is the
            // guard that keeps a lone note (or a note plus leakage) from being
            // reported as a chord.
            if !intervals
                .iter()
                .all(|&iv| chroma[(root + iv) % 12] >= present_floor)
            {
                continue;
            }
            let dot: f64 = intervals.iter().map(|&iv| chroma[(root + iv) % 12]).sum();
            let cosine = dot / (norm * mask_norm);
            if cosine >= MIN_CHORD_COSINE {
                scored.push((root, quality, cosine));
            }
        }
    }

    let best_cosine = scored.iter().map(|&(_, _, c)| c).fold(f64::MIN, f64::max);
    if best_cosine < MIN_CHORD_COSINE {
        return None;
    }
    // Among templates within `SIMPLICITY_MARGIN` of the best, take the one
    // with the fewest tones (a triad over the seventh that contains it); break
    // remaining ties by higher cosine. `ChordQuality::ALL` lists triads first,
    // so equal-tone-count ties resolve to the earlier-listed quality.
    scored
        .into_iter()
        .filter(|&(_, _, c)| c >= best_cosine - SIMPLICITY_MARGIN)
        .min_by(|a, b| {
            let tones = |q: ChordQuality| q.intervals().len();
            tones(a.1).cmp(&tones(b.1)).then(b.2.total_cmp(&a.2))
        })
}

/// Bucket per-STFT-frame chroma into fixed-width chord frames, label each,
/// smooth away sub-`min_chord_ms` runs, and coalesce into spans.
fn detect_chords(fc: &FrameChroma, opts: &HarmonyOpts, duration_ms: f64) -> Vec<ChordSpan> {
    if !opts.chord_frame_ms.is_finite() || opts.chord_frame_ms < 1.0 || duration_ms <= 0.0 {
        return Vec::new();
    }
    let frame_ms = opts.chord_frame_ms;
    let n_frames = (duration_ms / frame_ms).ceil().max(1.0) as usize;

    // Sum each STFT frame's chroma into the chord frame its center falls in.
    let mut buckets = vec![[0.0f64; 12]; n_frames];
    for (chroma, &c_ms) in fc.chroma.iter().zip(fc.center_ms.iter()) {
        let idx = ((c_ms / frame_ms).floor().max(0.0) as usize).min(n_frames - 1);
        for (slot, &v) in buckets[idx].iter_mut().zip(chroma.iter()) {
            *slot += v;
        }
    }

    let mut labels: Vec<ChordLabel> = buckets.iter().map(best_chord).collect();
    let min_run = ((opts.min_chord_ms / frame_ms).round().max(1.0)) as usize;
    smooth_runs(&mut labels, min_run);

    coalesce(&labels, frame_ms, duration_ms)
}

/// Absorb runs shorter than `min_run` frames into the preceding run (or, for
/// a short leading run, the following one), so a held chord isn't shattered by
/// a one-frame wobble. A run of `None` (a genuine gap) is preserved — silence
/// stays silence — only *labeled* short runs are absorbed. Deterministic,
/// single left-to-right pass over run-length-encoded segments.
fn smooth_runs(labels: &mut [ChordLabel], min_run: usize) {
    if labels.is_empty() || min_run <= 1 {
        return;
    }
    // Run-length encode into (label, start, len).
    let mut runs: Vec<(ChordLabel, usize, usize)> = Vec::new();
    for (i, &label) in labels.iter().enumerate() {
        match runs.last_mut() {
            Some((prev, _, len)) if same_label(prev, &label) => *len += 1,
            _ => runs.push((label, i, 1)),
        }
    }

    // Reassign short *labeled* runs to a neighbor's label (previous first,
    // else next). Gaps (None) are never absorbed.
    for r in 0..runs.len() {
        let (label, start, len) = runs[r];
        if label.is_none() || len >= min_run {
            continue;
        }
        let replacement = runs[..r]
            .iter()
            .rev()
            .find_map(|(l, _, _)| *l)
            .or_else(|| runs[r + 1..].iter().find_map(|(l, _, _)| *l));
        if let Some(new_label) = replacement {
            for slot in &mut labels[start..start + len] {
                *slot = Some(new_label);
            }
        }
    }
}

/// Two labels are "the same" for run-merging when both are gaps, or both name
/// the same root+quality (the cosine strength is ignored — it's per-frame).
fn same_label(a: &ChordLabel, b: &ChordLabel) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some((ra, qa, _)), Some((rb, qb, _))) => ra == rb && qa == qb,
        _ => false,
    }
}

/// Coalesce a per-frame label sequence into [`ChordSpan`]s (dropping gaps),
/// each span's confidence the mean cosine over its frames.
fn coalesce(labels: &[ChordLabel], frame_ms: f64, duration_ms: f64) -> Vec<ChordSpan> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < labels.len() {
        let Some((root, quality, _)) = labels[i] else {
            i += 1;
            continue;
        };
        let mut j = i;
        let mut sum = 0.0;
        let mut count = 0usize;
        while j < labels.len() {
            match labels[j] {
                Some((r, q, c)) if r == root && q == quality => {
                    sum += c;
                    count += 1;
                    j += 1;
                }
                _ => break,
            }
        }
        let start_ms = i as f64 * frame_ms;
        let end_ms = (j as f64 * frame_ms).min(duration_ms);
        let root_pc = PitchClass::ALL[root];
        spans.push(ChordSpan {
            start_ms,
            end_ms,
            root: root_pc,
            quality,
            symbol: format!("{}{}", root_pc.name(), quality.suffix()),
            confidence: if count > 0 { sum / count as f64 } else { 0.0 },
        });
        i = j;
    }
    spans
}

/// Per-section key: split `[0, duration]` at `boundaries_ms`, sum each
/// section's chroma, and run [`key::correlate`]. A section with no chroma
/// energy is skipped (no meaningful key). Always at least one section for
/// non-empty input (the whole file, when no boundaries are given).
fn section_keys(fc: &FrameChroma, boundaries_ms: &[f64], duration_ms: f64) -> Vec<SectionKey> {
    if duration_ms <= 0.0 || fc.chroma.is_empty() {
        return Vec::new();
    }
    // Build ascending section edges [0, b0, b1, ..., duration], ignoring any
    // boundary that isn't strictly inside (0, duration).
    let mut edges = vec![0.0f64];
    for &b in boundaries_ms {
        if b > 0.0 && b < duration_ms && b > *edges.last().unwrap() {
            edges.push(b);
        }
    }
    edges.push(duration_ms);

    let mut sections = Vec::new();
    for w in edges.windows(2) {
        let (start, end) = (w[0], w[1]);
        let mut acc = [0.0f64; 12];
        for (chroma, &c_ms) in fc.chroma.iter().zip(fc.center_ms.iter()) {
            if c_ms >= start && c_ms < end {
                for (slot, &v) in acc.iter_mut().zip(chroma.iter()) {
                    *slot += v;
                }
            }
        }
        if acc.iter().all(|&v| v == 0.0) {
            continue;
        }
        let (tonic, mode, confidence) = key::correlate(&acc);
        sections.push(SectionKey {
            start_ms: start,
            end_ms: end,
            tonic,
            mode,
            confidence,
        });
    }
    sections
}

/// Fraction of the whole duration covered by a confident chord span.
fn coverage(chords: &[ChordSpan], duration_ms: f64) -> f64 {
    if duration_ms <= 0.0 {
        return 0.0;
    }
    let covered: f64 = chords
        .iter()
        .map(|c| (c.end_ms - c.start_ms).max(0.0))
        .sum();
    (covered / duration_ms).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a clean chroma vector with unit energy in the given pitch
    /// classes (root-relative offsets applied to `root`).
    fn chroma_of(root: usize, offsets: &[usize]) -> [f64; 12] {
        let mut c = [0.0f64; 12];
        for &iv in offsets {
            c[(root + iv) % 12] = 1.0;
        }
        c
    }

    #[test]
    fn templates_recover_their_own_chords() {
        for root in 0..12 {
            for &(quality, offsets) in &[
                (ChordQuality::Major, &[0, 4, 7][..]),
                (ChordQuality::Minor, &[0, 3, 7][..]),
                (ChordQuality::Dominant7, &[0, 4, 7, 10][..]),
                (ChordQuality::Major7, &[0, 4, 7, 11][..]),
                (ChordQuality::Minor7, &[0, 3, 7, 10][..]),
                (ChordQuality::Diminished, &[0, 3, 6][..]),
                (ChordQuality::Sus4, &[0, 5, 7][..]),
            ] {
                let chroma = chroma_of(root, offsets);
                let (r, q, cos) = best_chord(&chroma).expect("a clean chord must be detected");
                assert_eq!(r, root, "root for {quality:?} at {root}");
                assert_eq!(q, quality, "quality at root {root}");
                assert!(cos > 0.99, "an exact template match should score ~1.0");
            }
        }
    }

    #[test]
    fn a_bare_triad_is_not_over_read_as_a_seventh() {
        // C-E-G with no seventh present: the L2-normalized templates must
        // prefer the plain triad over Cmaj7/C7 (both supersets of it).
        let (root, quality, _) = best_chord(&chroma_of(0, &[0, 4, 7])).unwrap();
        assert_eq!((root, quality), (0, ChordQuality::Major));
    }

    #[test]
    fn a_seventh_present_is_read_as_a_seventh() {
        // Add the b7 (Bb over C): now the dominant-seventh template wins.
        let (root, quality, _) = best_chord(&chroma_of(0, &[0, 4, 7, 10])).unwrap();
        assert_eq!((root, quality), (0, ChordQuality::Dominant7));
    }

    #[test]
    fn silence_and_noise_are_unlabeled() {
        assert!(best_chord(&[0.0; 12]).is_none(), "silence has no chord");
        // A single pitch class isn't a chord — it shouldn't clear the floor
        // against any triad template.
        let mut single = [0.0; 12];
        single[0] = 1.0;
        assert!(best_chord(&single).is_none(), "one note is not a chord");
    }

    #[test]
    fn short_runs_are_smoothed_into_held_chords() {
        let c = Some((0, ChordQuality::Major, 0.9));
        let g = Some((7, ChordQuality::Major, 0.9));
        // C C C G C C C  — the lone G is a one-frame flicker.
        let mut labels = vec![c, c, c, g, c, c, c];
        smooth_runs(&mut labels, 2);
        assert!(
            labels.iter().all(|l| same_label(l, &c)),
            "a one-frame flicker below min_run must be absorbed: {labels:?}"
        );
    }

    #[test]
    fn gaps_are_never_absorbed_by_smoothing() {
        let c = Some((0, ChordQuality::Major, 0.9));
        // A genuine one-frame silence between two chords stays a gap.
        let mut labels = vec![c, c, None, c, c];
        smooth_runs(&mut labels, 2);
        assert_eq!(labels[2], None, "a silence gap must survive smoothing");
    }

    #[test]
    fn section_keys_split_at_boundaries() {
        // Two frames of C-major chroma then two of A-minor-flavored chroma;
        // a boundary between them yields two keyed sections.
        let fc = FrameChroma {
            chroma: vec![
                chroma_of(0, &[0, 4, 7]),
                chroma_of(0, &[0, 4, 7]),
                chroma_of(9, &[0, 3, 7]),
                chroma_of(9, &[0, 3, 7]),
            ],
            center_ms: vec![100.0, 300.0, 700.0, 900.0],
        };
        let sections = section_keys(&fc, &[500.0], 1000.0);
        assert_eq!(sections.len(), 2, "one boundary → two sections");
        assert!(sections[0].start_ms == 0.0 && sections[0].end_ms == 500.0);
        assert!(sections[1].start_ms == 500.0 && sections[1].end_ms == 1000.0);
    }
}
