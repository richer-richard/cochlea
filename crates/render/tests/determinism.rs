//! P2 determinism gates: repeat renders are byte-equal, parallel equals
//! serial, the mix is byte-equal to the f64 sum of the stems, voice
//! stealing is a pure function of the schedule, and a golden hash pins the
//! pinned-target output.

use std::sync::Arc;

use cochlea_render::{render, render_serial, render_with};
use cochlea_score::*;
use cochlea_synth::{Adsr, Patch, PatchBank, Voice, VoiceCtx};

/// FNV-1a over the f32 bit stream — enough to pin bytes in a test without
/// pulling a hash dependency.
fn pcm_hash(samples: &[f32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &s in samples {
        for b in s.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

fn busy_score() -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .tempo(Ticks(0), Bpm(132.0))
        .tempo(bar(2), Bpm(97.5))
        .track("lead", Instrument::preset("saw_lead"))
        .insert("lead", Insert::preset("reverb"))
        .note("lead", bar(1).beat(1), Dur::quarter(), Pitch::A4, Vel(96))
        .note("lead", bar(1).beat(2), Dur::eighth(), Pitch::CS5, Vel(88))
        .note("lead", bar(1).beat(3), Dur::half(), Pitch::E5, Vel(100))
        .automate(
            "lead",
            Param::CUTOFF_HZ,
            keys![(bar(1), 500.0, ease_in_out()), (bar(2), 6_000.0)],
        )
        .track("bass", Instrument::preset("square_bass"))
        .note("bass", bar(1), Dur::half(), Pitch::A2, Vel(110))
        .note("bass", bar(1).beat(3), Dur::half(), Pitch::E2, Vel(104))
        .track("hat", Instrument::preset("noise_hat"))
        .note("hat", bar(1).beat(1), Dur::sixteenth(), Pitch::A4, Vel(90))
        .note("hat", bar(1).beat(2), Dur::sixteenth(), Pitch::A4, Vel(70))
        .note("hat", bar(1).beat(3), Dur::sixteenth(), Pitch::A4, Vel(90))
        .track("keys", Instrument::preset("chord_pad"))
        .note("keys", bar(1), Dur::whole(), Pitch::A3, Vel(80))
        .note("keys", bar(1), Dur::whole(), Pitch::CS4, Vel(80))
        .note("keys", bar(1), Dur::whole(), Pitch::E4, Vel(80))
        .automate("keys", Param::GAIN, keys![(bar(1), 0.8), (bar(2), 0.5)])
        .automate("keys", Param::PAN, keys![(bar(1), -0.4), (bar(2), 0.4)])
        .track("pluck", Instrument::preset("pluck"))
        .note("pluck", bar(1).beat(2), Dur::quarter(), Pitch::A5, Vel(84))
        .track("bell", Instrument::preset("sine"))
        .note("bell", bar(1).beat(4), Dur::quarter(), Pitch::A6, Vel(60))
}

#[test]
fn the_same_score_renders_byte_identically_twice() {
    let score = busy_score();
    let a = render(&score).unwrap();
    let b = render(&score).unwrap();
    assert_eq!(a.mix().len(), b.mix().len());
    assert_eq!(pcm_hash(a.mix()), pcm_hash(b.mix()));
    for ((na, sa), (nb, sb)) in a.stems().zip(b.stems()) {
        assert_eq!(na, nb);
        assert_eq!(pcm_hash(sa), pcm_hash(sb), "stem {na} differs");
    }
    assert!(a.mix().iter().any(|&s| s != 0.0), "render is not silence");
}

#[test]
fn parallel_render_equals_serial_render_byte_for_byte() {
    let score = busy_score();
    let par = render(&score).unwrap();
    let ser = render_serial(&score).unwrap();
    assert_eq!(par.mix(), ser.mix());
    for ((na, sa), (nb, sb)) in par.stems().zip(ser.stems()) {
        assert_eq!(na, nb);
        assert_eq!(sa, sb, "stem {na} differs between parallel and serial");
    }
}

#[test]
fn the_mix_is_byte_equal_to_the_f64_sum_of_stems_in_track_order() {
    let score = busy_score();
    let rendered = render(&score).unwrap();
    let stems: Vec<&[f32]> = rendered.stems().map(|(_, s)| s).collect();
    for (i, &m) in rendered.mix().iter().enumerate() {
        let sum: f64 = stems.iter().map(|s| f64::from(s[i])).sum();
        #[expect(clippy::cast_possible_truncation, reason = "the tested rounding rule")]
        let expected = sum as f32;
        assert_eq!(m.to_bits(), expected.to_bits(), "sample {i}");
    }
}

/// A deliberately simple deterministic patch for the stealing test: a sine
/// with a plain ADSR, either mono or 16-voice.
struct TestTone(Polyphony);

impl Patch for TestTone {
    fn name(&self) -> &'static str {
        "test_tone"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        self.0
    }

    fn release_secs(&self) -> f64 {
        0.1
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        use fundsp::prelude64 as fd;
        let adsr = Adsr {
            attack: 0.01,
            decay: 0.05,
            sustain: 0.8,
            release: 0.1,
        };
        let len = ctx.note_len_secs();
        let amp = f64::from(ctx.amp()) * 0.3;
        #[expect(clippy::cast_possible_truncation, reason = "audio-band frequency")]
        let f = ctx.pitch.hz() as f32;
        let graph =
            (fd::sine_hz(f) * fd::envelope(move |t| adsr.value(t, len) * amp)) >> fd::pan(0.0);
        let mut unit: Box<dyn fundsp::audiounit::AudioUnit> = Box::new(graph);
        unit.set_sample_rate(f64::from(ctx.sample_rate.0));
        Voice {
            unit,
            controls: Vec::new(),
        }
    }
}

/// Voice stealing is pure schedule math: on a mono patch, the moment note 2
/// starts, note 1's voice is removed outright — so from that sample on, the
/// stem is byte-equal to a render where note 2 plays alone (same seed).
#[test]
fn mono_voice_stealing_is_a_pure_function_of_the_schedule() {
    let two_notes = |poly| {
        let bank = PatchBank::presets().with_patch("test", Arc::new(TestTone(poly)));
        // Note 1: beats 1..3. Note 2: beats 2..4 — overlap on beat 2.
        let score = Score::new(SampleRate(48_000), Ppq(960))
            .track("t", Instrument::custom("test"))
            .note("t", bar(1).beat(1), Dur::half(), Pitch::A4, Vel(96))
            .note("t", bar(1).beat(2), Dur::half(), Pitch::E5, Vel(96));
        render_with(&score, &bank).unwrap()
    };
    let mono = two_notes(Polyphony::Mono);
    let poly = two_notes(Polyphony::Poly(16));

    // Note 2 starts at beat 2 = 24_000 samples (120 BPM default).
    let steal_at = 24_000 * 2; // interleaved index
    assert_eq!(
        &mono.stem("t").unwrap()[..steal_at],
        &poly.stem("t").unwrap()[..steal_at],
        "before the steal, mono and poly are identical"
    );
    assert_ne!(
        &mono.stem("t").unwrap()[steal_at..],
        &poly.stem("t").unwrap()[steal_at..],
        "after the steal they must differ (poly keeps note 1 ringing)"
    );

    // The decisive purity check: mono-after-steal == note 2 rendered alone,
    // byte for byte. Voice 2's seed depends on its note index, so the solo
    // score gets a placeholder first note far away to preserve indexing.
    let bank = PatchBank::presets().with_patch("test", Arc::new(TestTone(Polyphony::Mono)));
    let solo = Score::new(SampleRate(48_000), Ppq(960))
        .track("t", Instrument::custom("test"))
        .note("t", bar(100), Dur::sixteenth(), Pitch::A4, Vel(96))
        .note("t", bar(1).beat(2), Dur::half(), Pitch::E5, Vel(96));
    let solo = render_with(&solo, &bank).unwrap();
    let mono_stem = mono.stem("t").unwrap();
    let solo_stem = solo.stem("t").unwrap();
    let end = mono_stem.len().min(solo_stem.len());
    assert_eq!(
        &mono_stem[steal_at..end],
        &solo_stem[steal_at..end],
        "after the steal, the mono stem is exactly note 2 alone"
    );
}

#[test]
fn wav_round_trips_through_hound() {
    let dir = std::env::temp_dir().join("cochlea-render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let score = busy_score();
    let rendered = render(&score).unwrap();
    let path = dir.join("mix.wav");
    rendered.write_wav(&path).unwrap();
    let mut reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 48_000);
    let back: Vec<f32> = reader.samples::<f32>().map(Result::unwrap).collect();
    assert_eq!(back, rendered.mix(), "WAV bytes round-trip exactly");

    rendered.write_stems(dir.join("stems")).unwrap();
    assert!(dir.join("stems/lead.wav").exists());
}

/// Golden PCM hash of the committed example score.
///
/// The constant was blessed on aarch64-macos, and the first Tier 1 CI run
/// (x86_64-linux, 2026-07-03, run 28659358254) produced the same bytes —
/// empirical confirmation that the libm + pure-arithmetic construction is
/// byte-identical across these architectures. If a future change moves it
/// on one platform only, split the constant per-platform and re-bless from
/// the pinned target: the Tier 1 contract is defined by x86_64-linux, not
/// by a dev machine (docs/determinism.md).
#[test]
#[cfg(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "macos"),
))]
fn golden_pcm_hash_of_the_committed_example() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/scores/first_light.ron"
    ))
    .unwrap();
    let score = Score::from_ron(&text).unwrap();
    let rendered = render(&score).unwrap();
    let hash = pcm_hash(rendered.mix());
    assert_eq!(
        hash, GOLDEN_MIX_HASH,
        "golden mismatch: got {hash:#018x} — a deliberate DSP change re-blesses this constant"
    );
}

#[cfg(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "macos"),
))]
const GOLDEN_MIX_HASH: u64 = 0x3DC3_B57D_406D_E89F; // blessed 2026-07-03, rustc 1.95.0

/// Golden PCM hash of the `drum_groove` demo (`demos/drum_groove`) — Wave
/// 2's rhythm-analysis showcase. Extends Tier 1 coverage to a DSP
/// combination the original golden doesn't exercise: `pluck` +
/// `noise_hat` + `chord_pad` together, with a `reverb` insert on one
/// track. See [`golden_pcm_hash_of_the_committed_example`]'s docs on the
/// re-blessing process.
///
/// Blessed on aarch64-macos only so far (2026-07-09) — not yet confirmed
/// cross-arch by a Tier 1 CI run the way the first-light golden above was;
/// update this note once CI runs it on x86_64-linux.
#[test]
#[cfg(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "macos"),
))]
fn golden_pcm_hash_of_the_drum_groove_demo() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demos/drum_groove/score.ron"
    ))
    .unwrap();
    let score = Score::from_ron(&text).unwrap();
    let rendered = render(&score).unwrap();
    let hash = pcm_hash(rendered.mix());
    assert_eq!(
        hash, DRUM_GROOVE_GOLDEN_MIX_HASH,
        "golden mismatch: got {hash:#018x} — a deliberate DSP change re-blesses this constant"
    );
}

#[cfg(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "macos"),
))]
const DRUM_GROOVE_GOLDEN_MIX_HASH: u64 = 0xBECB_EB55_B4F8_DD00; // blessed 2026-07-09, aarch64-macos, rustc 1.95.0
