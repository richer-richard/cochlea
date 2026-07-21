//! Analysis overlays as plain data — like [`crate::Marker`], this crate
//! never sees score or feature-report types; callers with analysis context
//! (`cli`, `mcp`) translate their reports into sample offsets and Hz
//! before handing them over (`docs/plan.md`'s dependency-direction law).

/// Analysis marks to draw over a spectrogram in
/// [`render_annotated`](crate::render_annotated): beat-grid ticks along
/// the top, onset ticks along the bottom, and detected-pitch line segments
/// on the frequency axis. All positions are sample offsets into the same
/// audio the [`crate::MelSpec`] was computed from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// Beat positions (sample offsets) — drawn as short ticks hanging from
    /// the top edge, so the detected grid reads at a glance without
    /// covering the image the way full-height lines would.
    pub beats: Vec<u64>,
    /// Onset positions (sample offsets) — short ticks rising from the
    /// bottom edge of the spectrogram region.
    pub onsets: Vec<u64>,
    /// Detected-pitch segments `(start_sample, end_sample, f0_hz)` — each
    /// drawn as a horizontal line at its f0's mel band across its time
    /// range. A vibrato or glide draws at its segment's single f0 (the
    /// pitch report's per-run median); that's the report being visualized,
    /// not a rendering shortcut.
    pub pitch: Vec<(u64, u64, f64)>,
}

impl Overlay {
    /// An overlay with nothing in it (what [`crate::render_png`] passes).
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether every layer is empty.
    pub fn is_empty(&self) -> bool {
        self.beats.is_empty() && self.onsets.is_empty() && self.pitch.is_empty()
    }
}
