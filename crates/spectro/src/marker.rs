//! Bar/beat markers as plain data — this crate never sees score types
//! (`docs/plan.md`: "Anything needing score context ... receives plain data
//! (sample/label grids) from the caller, never score types").

/// A single marker: a sample offset and a label. Typically a bar start
/// supplied by a caller that *does* have score context (`cli`/`verify`
/// joining a render against its score); `cochlea-spectro` treats markers as
/// opaque data and never interprets the label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub sample: u64,
    pub label: String,
}

impl Marker {
    pub fn new(sample: u64, label: impl Into<String>) -> Self {
        Self {
            sample,
            label: label.into(),
        }
    }
}
