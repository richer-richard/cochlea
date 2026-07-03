//! The block engine: renders each track independently (the parallelism
//! unit and the free stems export), then sums stems at f64 in fixed track
//! order. Voices tick sample-by-sample — fundsp block `process()` is
//! banned (docs/determinism.md).

use cochlea_score::{Param, SampleRate, Score, TempoMap};
use cochlea_synth::{Voice, VoiceCtx};
use rayon::prelude::*;

use crate::schedule::{Schedule, TrackPlan};

/// Maximum samples between control-rate automation updates. Blocks also
/// split at every note on/off/steal boundary, so note timing stays
/// sample-accurate; only *automation* is quantized to this (~1.3 ms at
/// 48 kHz — documented in docs/plan.md).
const MAX_BLOCK: u64 = 64;

/// One sounding note. `key` is the steal order: oldest `(on, index)` first
/// — a total order derived from the schedule alone (invariant 7).
struct ActiveVoice {
    key: (u64, usize),
    end: u64,
    voice: Voice,
}

/// Renders one track to an interleaved stereo f32 stem of `total` samples.
///
/// Precision path, in order: voices tick f32 (fundsp) → f64 voice sum →
/// f32 insert chain (fundsp) → f64 gain/pan → f32 stem.
pub(crate) fn render_track(
    plan: &TrackPlan,
    map: &TempoMap,
    sr: SampleRate,
    total: u64,
) -> Vec<f32> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "total is capped at one hour of samples; times 2 fits usize"
    )]
    let mut stem = vec![0.0f32; (total * 2) as usize];
    let mut inserts: Vec<Box<dyn fundsp::audiounit::AudioUnit>> =
        plan.inserts.iter().map(|i| i.unit(sr)).collect();

    // Block boundaries: every voice birth and death, in one sorted pass.
    let mut boundaries: Vec<u64> = plan
        .notes
        .iter()
        .flat_map(|n| [n.on, n.off, n.end])
        .filter(|&b| b < total)
        .collect();
    boundaries.push(total);
    boundaries.sort_unstable();
    boundaries.dedup();

    let polyphony = match plan.patch.polyphony() {
        cochlea_score::Polyphony::Mono => 1,
        cochlea_score::Polyphony::Poly(n) => usize::from(n.max(1)),
    };
    let mut active: Vec<ActiveVoice> = Vec::with_capacity(polyphony);
    let mut next_note = 0usize;

    let gain_auto = plan.automation.iter().find(|a| a.param == Param::GAIN);
    let pan_auto = plan.automation.iter().find(|a| a.param == Param::PAN);
    let voice_autos: Vec<_> = plan
        .automation
        .iter()
        .filter(|a| a.param != Param::GAIN && a.param != Param::PAN)
        .collect();

    let mut cursor = 0u64;
    for &boundary in &boundaries {
        // Retire voices whose lifetime ends here; start ones born here.
        // (Boundaries are exactly the on/off/end samples, so nothing is
        // missed between them.)
        active.retain(|v| v.end > cursor);
        while next_note < plan.notes.len() && plan.notes[next_note].on == cursor {
            let n = plan.notes[next_note];
            if active.len() == polyphony {
                // Steal the oldest note: smallest (on, index).
                let oldest = active
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.key)
                    .map(|(i, _)| i)
                    .expect("pool is non-empty when full");
                active.remove(oldest);
            }
            let voice = plan.patch.voice(&VoiceCtx {
                pitch: n.pitch,
                vel: n.vel,
                sample_rate: sr,
                note_len_samples: n.off - n.on,
                seed: n.seed,
            });
            active.push(ActiveVoice {
                key: (n.on, next_note),
                end: n.end,
                voice,
            });
            next_note += 1;
        }

        // Render [cursor, boundary) in control-rate chunks.
        while cursor < boundary {
            let block_end = (cursor + MAX_BLOCK).min(boundary);
            // Automation is sampled once per block, at the block-start tick
            // (rounding rule 3: Floor).
            let tick = map.tick_at(cursor);
            let gain = f64::from(gain_auto.map_or(1.0, |a| a.value_at(tick)));
            let pan = f64::from(pan_auto.map_or(0.0, |a| a.value_at(tick)).clamp(-1.0, 1.0));
            // Constant-power pan (libm at control rate).
            let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
            let (pan_l, pan_r) = (libm::cos(angle), libm::sin(angle));
            for auto in &voice_autos {
                let value = auto.value_at(tick);
                for v in &mut active {
                    for (param, shared) in &v.voice.controls {
                        if *param == auto.param {
                            shared.set_value(value);
                        }
                    }
                }
            }

            for s in cursor..block_end {
                let (mut l, mut r) = (0.0f64, 0.0f64);
                // Fixed summation order: allocation order in the pool.
                for v in &mut active {
                    let mut out = [0.0f32; 2];
                    v.voice.unit.tick(&[], &mut out);
                    l += f64::from(out[0]);
                    r += f64::from(out[1]);
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "insert chain runs at fundsp's f32 interface; documented precision hop"
                )]
                let (mut xl, mut xr) = (l as f32, r as f32);
                for ins in &mut inserts {
                    let mut out = [0.0f32; 2];
                    ins.tick(&[xl, xr], &mut out);
                    (xl, xr) = (out[0], out[1]);
                }
                let (ol, or) = (f64::from(xl) * gain * pan_l, f64::from(xr) * gain * pan_r);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "f64 bus to f32 stem is rounding rule 4"
                )]
                {
                    stem[(s * 2) as usize] = ol as f32;
                    stem[(s * 2 + 1) as usize] = or as f32;
                }
            }
            cursor = block_end;
        }
    }
    stem
}

/// Sums stems (their stored f32 values) at f64 in fixed track order — the
/// mix is *defined* as this sum, so `mix == Σ stems` holds byte-for-byte.
pub(crate) fn sum_stems(stems: &[Vec<f32>], total: u64) -> Vec<f32> {
    #[expect(clippy::cast_possible_truncation, reason = "capped render length")]
    let len = (total * 2) as usize;
    let mut mix = vec![0.0f32; len];
    for (i, out) in mix.iter_mut().enumerate() {
        let mut acc = 0.0f64;
        for stem in stems {
            acc += f64::from(stem[i]);
        }
        #[expect(clippy::cast_possible_truncation, reason = "rounding rule 4")]
        {
            *out = acc as f32;
        }
    }
    mix
}

pub(crate) fn render_stems(schedule: &Schedule, score: &Score, parallel: bool) -> Vec<Vec<f32>> {
    let map = score.tempo_map();
    let (sr, total) = (schedule.sample_rate, schedule.total_samples);
    if parallel {
        schedule
            .tracks
            .par_iter()
            .map(|plan| render_track(plan, &map, sr, total))
            .collect()
    } else {
        schedule
            .tracks
            .iter()
            .map(|plan| render_track(plan, &map, sr, total))
            .collect()
    }
}
