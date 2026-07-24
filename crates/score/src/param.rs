//! Automatable parameter names — pure vocabulary. The score references
//! params by name; the synth's registry declares which names an instrument
//! actually has, with units and ranges, and validation joins the two.

use std::borrow::Cow;

/// An automatable parameter name. Well-known names are associated
/// constants (`Param::CUTOFF_HZ`); anything else via [`Param::custom`].
/// Snake_case strings in the RON data form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Param(Cow<'static, str>);

impl Param {
    /// Filter cutoff in Hz.
    pub const CUTOFF_HZ: Param = Param(Cow::Borrowed("cutoff_hz"));
    /// Track gain, linear amplitude (1.0 = unity).
    pub const GAIN: Param = Param(Cow::Borrowed("gain"));
    /// Stereo pan, −1.0 (left) ..= 1.0 (right).
    pub const PAN: Param = Param(Cow::Borrowed("pan"));
    /// Timbre brightness — for the FM bell, the modulation index (more
    /// sidebands, brighter/more metallic as it rises).
    pub const BRIGHTNESS: Param = Param(Cow::Borrowed("brightness"));

    /// A parameter name outside the well-known set.
    pub fn custom(name: impl Into<String>) -> Param {
        Param(Cow::Owned(name.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Param {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
