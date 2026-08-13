//! Offline block render engine: 64-sample blocks split at event boundaries,
//! pure voice scheduling (allocation and oldest-note stealing are functions
//! of the event schedule alone), per-track rendering as the parallelism
//! unit and free stems export, f64 master sum in fixed track order, and
//! 32-bit float WAV output via hound.
//!
//! ```
//! use cochlea_score::*;
//! let score = Score::new(SampleRate(48_000), Ppq(960))
//!     .track("lead", Instrument::preset("sine"))
//!     .note("lead", bar(1), Dur::quarter(), Pitch::A4, Vel(96));
//! let rendered = cochlea_render::render(&score).unwrap();
//! assert!(rendered.mix().iter().any(|&s| s != 0.0));
//! ```

mod engine;
mod error;
mod master;
mod schedule;

use std::path::{Component, Path};

use cochlea_score::{SampleRate, Score};
use cochlea_synth::PatchBank;

pub use error::RenderError;

/// The file name a track's stem is written as, or why it can't be written.
///
/// A track name is free-form score data — it can come from a hand-authored
/// RON file or, via `cochlea import`, from a MIDI track-name meta event —
/// so it is *untrusted input* the moment it is used to build a path.
/// `Path::join` replaces the whole base path when handed an absolute
/// argument, so a track named `/etc/foo` or `../../foo` would otherwise
/// escape the stems directory entirely (and, over MCP, escape `--root`
/// while every path *argument* stayed inside it).
///
/// The rule: the composed `<track>.wav` must be an ordinary, *portable*
/// file name — one path component, and one that means the same thing on
/// every host cochlea runs on.
///
/// Every check runs on every platform, not just the one where it bites.
/// That is deliberate, and it is the same commitment as the bit-exact
/// render: a score is portable data, so a score that exports stems on macOS
/// must export the same stems on Windows rather than failing there (or,
/// worse, quietly writing somewhere else). Concretely, that means rejecting
/// on Unix a handful of names Unix itself would accept:
///
/// - `\` — an ordinary character here, a separator on Windows.
/// - `:` — ordinary here; on Windows it names a drive (`C:`) or an NTFS
///   alternate data stream (`x:y`), the latter writing *outside* the
///   directory the caller asked for.
/// - `< > " | ? *` and control characters — illegal in a Win32 file name.
/// - `CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9` — Win32
///   device names, matched before the first dot and *regardless* of
///   extension, so a `NUL` track would write to the null device and vanish
///   while the render reported success.
///
/// A bare `..` is fine and stays allowed: the appended extension makes it
/// `...wav`, an ordinary file, not a parent component.
///
/// This is the single rule behind [`Rendered::write_stems_as`] and both
/// front ends' pre-flight checks (`cochlea render --stems`, the MCP
/// `render_score` tool), so there is one answer to "is this name writable",
/// not three. It bounds the *name*; containment of the resulting path is
/// [`Rendered::write_stems_as`]'s job, because a name alone cannot say
/// whether something at that path is a symlink out of the directory.
///
/// ```
/// assert_eq!(cochlea_render::stem_file_name("lead").unwrap(), "lead.wav");
/// assert!(cochlea_render::stem_file_name("../../escape").is_err());
/// assert!(cochlea_render::stem_file_name("/etc/passwd").is_err());
/// assert!(cochlea_render::stem_file_name("NUL").is_err());
/// ```
pub fn stem_file_name(track: &str) -> Result<String, RenderError> {
    let reject = |reason: &'static str| {
        Err(RenderError::UnwritableStemName {
            name: track.to_owned(),
            reason,
        })
    };
    if track.is_empty() {
        return reject("it is empty");
    }
    if track.contains('\0') {
        return reject("it contains a NUL byte");
    }
    if track.contains('/') || track.contains('\\') {
        return reject("it contains a path separator");
    }
    if track.contains(':') {
        return reject("it contains a colon, which names a drive or a data stream on Windows");
    }
    if track.contains(['<', '>', '"', '|', '?', '*']) {
        return reject("it contains one of <>\"|?*, which no Windows file name may hold");
    }
    if track.contains(char::is_control) {
        return reject("it contains a control character");
    }
    // No trailing space/dot check: Windows strips those from the *end of a
    // component*, and the mandatory `.wav` suffix means the composed name
    // never ends in one. Checking `track` instead would reject `..`, which
    // composes to the perfectly ordinary file `...wav`.
    if is_reserved_device_name(track) {
        return reject("it is a reserved device name on Windows");
    }
    let file = format!("{track}.wav");
    if file.len() > MAX_STEM_FILE_NAME_BYTES {
        return reject("it is too long to be a file name");
    }
    // The structural backstop for anything the character checks can't spell
    // out — `..`-shaped components, a bare root, a Windows drive prefix.
    let mut components = Path::new(&file).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(only)), None) if only == file.as_str() => Ok(file),
        _ => reject("it is not an ordinary relative file name"),
    }
}

/// The longest stem file name we will compose, in bytes. 255 is the
/// per-component limit on ext4, APFS and NTFS alike; bounding it here means
/// a long track name is refused with an actionable message *before* any
/// stem is written, rather than as an `ENAMETOOLONG` partway through the
/// set (see [`Rendered::write_stems_as`]'s all-or-nothing promise).
const MAX_STEM_FILE_NAME_BYTES: usize = 255;

/// Whether `a` and `b` name the same file on *any* host cochlea runs on —
/// the same-path rule every "don't clobber that" guard in both front ends
/// shares.
///
/// Exact equality is not the whole rule, because macOS and Windows are
/// case-insensitive by default: `d/Lead.wav` and `d/lead.wav` are two paths
/// and one file there. That gap was reachable and destructive —
/// `render score.ron --out d/Lead.wav --stems d` with a track named `lead`
/// wrote the mix and then overwrote it with one stem, at exit 0. The
/// stem-*set* check in [`Rendered::write_stems_as`] already folds case for
/// exactly this reason; this is the same rule, applied between a stem and
/// everything else a command writes or reads.
///
/// It is deliberately enforced everywhere rather than only where it bites,
/// which is the trade [`stem_file_name`] already makes: a score is portable
/// data, and two outputs that would collide on a colleague's laptop are
/// better refused here than silently merged there. The cost is that on Linux
/// two genuinely distinct paths differing only by case are now refused as a
/// pair.
///
/// Callers pass paths they have already resolved as far as they can (the CLI
/// canonicalizes through the parent, the MCP server hands over canonical
/// paths), so this compares *where things land*, not how they were spelled.
///
/// Case folding is Unicode-aware but *not* normalization-aware: NFC and NFD
/// spellings of the same name still compare distinct here, the same known
/// limit [`Rendered::write_stems_as`] documents for the stem set.
///
/// ```
/// use std::path::Path;
/// use cochlea_render::same_target_file;
/// assert!(same_target_file(Path::new("d/mix.wav"), Path::new("d/mix.wav")));
/// assert!(same_target_file(Path::new("d/Mix.wav"), Path::new("d/mix.wav")));
/// assert!(!same_target_file(Path::new("d/mix.wav"), Path::new("e/mix.wav")));
/// ```
pub fn same_target_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if a.parent() != b.parent() {
        return false;
    }
    match (a.file_name(), b.file_name()) {
        (Some(x), Some(y)) => {
            x.to_string_lossy().to_lowercase() == y.to_string_lossy().to_lowercase()
        }
        _ => false,
    }
}

/// Win32 device names, which resolve to a device from *any* directory and
/// are matched on the portion before the first dot, ignoring the extension
/// — so `NUL`, `NUL.wav` and `NUL.foo.wav` are all the null device.
fn is_reserved_device_name(track: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
        "LPT9",
    ];
    let base = track.split('.').next().unwrap_or(track);
    RESERVED.contains(&base.to_ascii_uppercase().as_str())
}

/// A completed render: per-track stems plus the mix, all interleaved
/// stereo f32 at the score's sample rate.
///
/// The mix is *defined* as the f64 sum of the stems' stored f32 values in
/// fixed track order, passed through the score's master stage (gain +
/// limiter — a no-op for the default master), converted back to f32. For
/// a score without a master section the invariant "mix == sum of stems"
/// therefore holds byte-for-byte exactly as before, and is tested; with a
/// master, the mix is `master(Σ stems)` and the stems stay pre-master.
pub struct Rendered {
    sample_rate: SampleRate,
    stems: Vec<(String, Vec<f32>)>,
    mix: Vec<f32>,
}

impl Rendered {
    /// The stereo mix, interleaved L/R.
    pub fn mix(&self) -> &[f32] {
        &self.mix
    }

    /// Per-track stems in score track order, interleaved L/R, all the same
    /// length as the mix.
    ///
    /// The names are the score's track names verbatim — free-form data, not
    /// sanitized identifiers. If you build file paths from them, go through
    /// [`stem_file_name`] rather than joining them directly; a track name
    /// can be spelled as an absolute path, and `Path::join` would honour it.
    pub fn stems(&self) -> impl Iterator<Item = (&str, &[f32])> {
        self.stems.iter().map(|(n, s)| (n.as_str(), s.as_slice()))
    }

    /// One track's stem by name.
    pub fn stem(&self, track: &str) -> Option<&[f32]> {
        self.stems
            .iter()
            .find(|(n, _)| n == track)
            .map(|(_, s)| s.as_slice())
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Render length in frames (samples per channel).
    pub fn frames(&self) -> u64 {
        (self.mix.len() / 2) as u64
    }

    /// Writes the mix as a 32-bit float stereo WAV — the render verbatim,
    /// lossless. For a smaller, ordinary integer-PCM file, see
    /// [`Rendered::write_wav_as`].
    pub fn write_wav(&self, path: impl AsRef<Path>) -> Result<(), RenderError> {
        write_wav(
            path.as_ref(),
            self.sample_rate,
            &self.mix,
            WavBitDepth::Float32,
        )
    }

    /// Writes the mix as a stereo WAV in the chosen PCM encoding (see
    /// [`WavBitDepth`]). `Float32` is the render's exact ground truth;
    /// `Int24`/`Int16` are deterministically quantized for a small, ordinary
    /// file a human or another tool can open.
    pub fn write_wav_as(
        &self,
        path: impl AsRef<Path>,
        depth: WavBitDepth,
    ) -> Result<(), RenderError> {
        write_wav(path.as_ref(), self.sample_rate, &self.mix, depth)
    }

    /// Writes one `<track>.wav` per stem into `dir` (created if missing), as
    /// 32-bit float. See [`Rendered::write_stems_as`] for other encodings.
    pub fn write_stems(&self, dir: impl AsRef<Path>) -> Result<(), RenderError> {
        self.write_stems_as(dir, WavBitDepth::Float32)
    }

    /// Writes one `<track>.wav` per stem into `dir` (created if missing), in
    /// the chosen PCM encoding.
    ///
    /// Two things are settled before any stem is written, so that a score
    /// which cannot be exported cleanly leaves no stems behind rather than a
    /// half-written set. (Only stems: a mix the caller already wrote through
    /// [`Rendered::write_wav_as`] is its own write and stays on disk.)
    ///
    /// 1. **Every name** is checked against [`stem_file_name`], and the set
    ///    is checked for names that differ only by case — those are distinct
    ///    tracks to a score but one file on macOS and Windows, so writing
    ///    them would silently drop a stem.
    /// 2. **Every path** is checked for containment *after* resolving what
    ///    is already on disk. A well-formed name is not enough: if
    ///    `dir/lead.wav` already exists as a symlink pointing elsewhere,
    ///    `File::create` follows it and the stem lands outside the directory
    ///    the caller named — the same trap `resolve_write` was hardened
    ///    against on the MCP side. A lexical `starts_with` cannot see that;
    ///    canonicalizing can. The presence test is `symlink_metadata`, so a
    ///    *broken* link — one whose target does not exist yet, which
    ///    `exists()` reports as nothing at all while `File::create` still
    ///    follows it — is caught too, and refused outright.
    ///
    /// A symlink that stays *inside* `dir` is fine and is left alone — the
    /// rule is containment, not a ban on links.
    pub fn write_stems_as(
        &self,
        dir: impl AsRef<Path>,
        depth: WavBitDepth,
    ) -> Result<(), RenderError> {
        let dir = dir.as_ref();
        let files = self
            .stems
            .iter()
            .map(|(track, stem)| Ok((track.as_str(), stem_file_name(track)?, stem)))
            .collect::<Result<Vec<_>, RenderError>>()?;

        // Case-insensitive collision: `Lead` and `lead` are two tracks and
        // two valid names, but one file on any case-insensitive volume.
        // (Unicode normalization — NFC vs NFD spellings of the same name —
        // would need a dependency this workspace does not carry, so that
        // narrower collision is out of scope and documented as such.)
        for (i, (track, file, _)) in files.iter().enumerate() {
            if let Some((other, _, _)) = files[..i]
                .iter()
                .find(|(_, earlier, _)| earlier.to_lowercase() == file.to_lowercase())
            {
                return Err(RenderError::CollidingStemNames {
                    first: (*other).to_owned(),
                    second: (*track).to_owned(),
                });
            }
        }

        std::fs::create_dir_all(dir)?;
        // Resolve the directory once, so containment is judged against where
        // it actually lands rather than how the caller spelled it.
        let root = std::fs::canonicalize(dir)?;
        let mut paths = Vec::with_capacity(files.len());
        for (track, file, stem) in files {
            let path = root.join(&file);
            // Only something already sitting at this path can redirect the
            // write; a name that is not there yet resolves to exactly this
            // path.
            //
            // `symlink_metadata`, not `exists()`: `exists()` *follows* the
            // link, so a link pointing at a file that does not exist yet
            // reads as "nothing here" and skipped this check entirely —
            // while `File::create` follows it all the same and creates the
            // stem at the far end, outside the directory (and, over MCP,
            // outside `--root`), at exit 0. Reproduced against 0.7.0, which
            // closed the same hole for a link whose target already existed.
            if std::fs::symlink_metadata(&path).is_ok() {
                let landed =
                    std::fs::canonicalize(&path).map_err(|_| RenderError::UnwritableStemName {
                        name: track.to_owned(),
                        // A link we cannot resolve is a link we cannot vouch
                        // for: refuse rather than guess where the write lands.
                        reason: "a broken link sits at its path in the stems directory",
                    })?;
                if !landed.starts_with(&root) {
                    return Err(RenderError::UnwritableStemName {
                        name: track.to_owned(),
                        reason: "a link at its path in the stems directory points outside it",
                    });
                }
            }
            paths.push((path, stem));
        }

        for (path, stem) in paths {
            write_wav(&path, self.sample_rate, stem, depth)?;
        }
        Ok(())
    }
}

/// PCM encoding for WAV output. The render's ground truth is the f32 mix, so
/// `Float32` is the default and the only lossless choice; `Int24`/`Int16`
/// exist to hand a small, ordinary file to a human or a tool that wants
/// integer PCM. Integer conversion is deterministic: clamp to `[-1, 1]`,
/// scale to full range, round to nearest (no dither — dither would trade
/// determinism for a lower noise floor, the wrong trade for a byte-exact test
/// engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WavBitDepth {
    /// 32-bit IEEE float — the mix verbatim (default, lossless).
    #[default]
    Float32,
    /// 24-bit signed integer PCM.
    Int24,
    /// 16-bit signed integer PCM.
    Int16,
}

impl WavBitDepth {
    /// The canonical selector name (`"float"`/`"24"`/`"16"`), the inverse of
    /// [`FromStr`](std::str::FromStr) — `d.canonical_str().parse() == Ok(d)`
    /// for every variant.
    pub const fn canonical_str(self) -> &'static str {
        match self {
            WavBitDepth::Float32 => "float",
            WavBitDepth::Int24 => "24",
            WavBitDepth::Int16 => "16",
        }
    }
}

impl std::fmt::Display for WavBitDepth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.canonical_str())
    }
}

impl std::str::FromStr for WavBitDepth {
    type Err = String;

    /// Parse a `--bits`-style selector, case-insensitively: `float`/`f32`/`32`
    /// → [`Self::Float32`], `24` → [`Self::Int24`], `16` → [`Self::Int16`].
    /// The one place every front door (CLI, MCP, Python) resolves the encoding
    /// name, so they can't drift; surrounding whitespace is tolerated.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "float" | "f32" | "32" => Ok(WavBitDepth::Float32),
            "24" => Ok(WavBitDepth::Int24),
            "16" => Ok(WavBitDepth::Int16),
            other => Err(format!(
                "unknown bit depth {other:?} (expected float, 24, or 16)"
            )),
        }
    }
}

fn write_wav(
    path: &Path,
    sample_rate: SampleRate,
    samples: &[f32],
    depth: WavBitDepth,
) -> Result<(), RenderError> {
    let (bits_per_sample, sample_format) = match depth {
        WavBitDepth::Float32 => (32, hound::SampleFormat::Float),
        WavBitDepth::Int24 => (24, hound::SampleFormat::Int),
        WavBitDepth::Int16 => (16, hound::SampleFormat::Int),
    };
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sample_rate.0,
        bits_per_sample,
        sample_format,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    match depth {
        WavBitDepth::Float32 => {
            for &s in samples {
                writer.write_sample(s)?;
            }
        }
        WavBitDepth::Int24 => {
            for &s in samples {
                writer.write_sample(quantize_int(s, 23))?;
            }
        }
        WavBitDepth::Int16 => {
            for &s in samples {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "quantize_int clamps into the 16-bit range, so the cast is exact"
                )]
                writer.write_sample(quantize_int(s, 15) as i16)?;
            }
        }
    }
    writer.finalize()?;
    Ok(())
}

/// Deterministic float→signed-integer PCM: clamp `s` to `[-1, 1]`, scale by
/// `2^frac_bits`, round to nearest, and clamp into the signed range
/// `[-2^frac_bits, 2^frac_bits - 1]`. Returned as `i32` (the 24-bit path
/// writes it directly; the 16-bit path narrows the already-clamped value).
fn quantize_int(s: f32, frac_bits: u32) -> i32 {
    let scale = f64::from(1u32 << frac_bits);
    let scaled = (f64::from(s).clamp(-1.0, 1.0) * scale).round();
    let clamped = scaled.clamp(-scale, scale - 1.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped into i32-representable signed range above"
    )]
    {
        clamped as i32
    }
}

fn render_inner(score: &Score, bank: &PatchBank, parallel: bool) -> Result<Rendered, RenderError> {
    let schedule = schedule::compile(score, bank)?;
    let stems = engine::render_stems(&schedule, score, parallel);
    let mut mix64 = engine::sum_stems_f64(&stems, schedule.total_samples);
    master::process(&mut mix64, schedule.sample_rate, score.master());
    let mix = engine::quantize(&mix64);
    Ok(Rendered {
        sample_rate: schedule.sample_rate,
        stems: schedule
            .tracks
            .iter()
            .map(|t| t.name.clone())
            .zip(stems)
            .collect(),
        mix,
    })
}

/// Renders a score with the shipped presets, tracks in parallel.
pub fn render(score: &Score) -> Result<Rendered, RenderError> {
    render_inner(score, &PatchBank::presets(), true)
}

/// Renders with a custom [`PatchBank`] (the `Instrument::custom` path).
pub fn render_with(score: &Score, bank: &PatchBank) -> Result<Rendered, RenderError> {
    render_inner(score, bank, true)
}

/// Single-threaded render — exists so the determinism test can assert
/// `parallel == serial` byte-for-byte, and for callers that want to bound
/// CPU use.
pub fn render_serial(score: &Score) -> Result<Rendered, RenderError> {
    render_inner(score, &PatchBank::presets(), false)
}
