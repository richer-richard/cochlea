//! FLAC decode via symphonia's bundled FLAC reader + decoder
//! (`symphonia-bundle-flac`, pulled in by the `symphonia` crate's `flac`
//! feature — no other format/codec feature is enabled, so this is the only
//! decode path symphonia can take here).
//!
//! FLAC is lossless by spec: its LPC prediction and Rice coding are pure
//! integer arithmetic, so a correct decoder reconstructs the *exact*
//! original PCM integers, no floating point involved. The only place this
//! module could still diverge from a WAV twin's decoded samples is in how
//! those raw integers get normalized to `f32`.
//!
//! **The non-obvious part, found by testing against a real WAV twin rather
//! than trusting either side's docs**: `symphonia-bundle-flac`'s decoder
//! doesn't emit samples at their true bit depth. Per its own source
//! comment (`decoder.rs`, `decode_inner`): "the decoder uses a 32bit
//! sample format as a common denominator... shift all samples in the
//! output buffer so that regardless the encoded bits/sample, the output is
//! always 32bits/sample" — i.e. every sample is left-justified into the
//! full `i32` range (`raw << (32 - bits_per_sample)`) before it ever
//! reaches this crate. `bits_per_sample` (from the codec params) is
//! informational metadata only; it is *not* a scaling factor to divide by.
//!
//! Dividing the left-justified value by `2^31` unconditionally —
//! `cochlea_features::Audio::from_wav`'s own convention, natural value /
//! `2^(bits-1)`, is exactly equal to `(natural << (32-bits)) / 2^31` for
//! any bit depth, since a left-shift by `32-bits` followed by a divide by
//! `2^31` is the same exact power-of-two rescaling as a divide by
//! `2^(bits-1)` — is therefore both simpler *and* the only choice that
//! lands on the same `f32` bits as the WAV path for every depth FLAC
//! supports, not just the four `cochlea_features::Audio::from_wav` handles.
//! `tests/sample_exact.rs` is what actually enforces the match.

use std::fs::File;
use std::path::Path;

use cochlea_features::Audio;
use symphonia::core::audio::{Audio as _, GenericAudioBufferRef};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::TrackType;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;

use crate::error::DecodeError;

/// `i32`'s full range as a divisor, matching symphonia's own left-justified
/// convention (see this module's docs).
const FULL_SCALE_I32: f64 = 2_147_483_648.0;

pub(crate) fn decode(path: &Path, limit: Option<u64>) -> Result<Audio, DecodeError> {
    let file = File::open(path).map_err(|source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("flac");

    let mut format = symphonia::default::get_probe().probe(
        &hint,
        mss,
        Default::default(),
        Default::default(),
    )?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;

    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or(DecodeError::NoAudioTrack)?;
    // Not used for scaling (see module docs) — checked only so a stream
    // missing this STREAMINFO field fails clearly instead of silently.
    if codec_params.bits_per_sample.is_none() {
        return Err(DecodeError::UnknownBitDepth);
    }
    // The returned Audio's shape comes from STREAMINFO, not from whatever
    // packets happen to decode — a truncated/metadata-only stream that
    // yields zero packets previously fabricated an Audio with
    // channels: 0 / sample_rate: 0 that downstream analyzers quietly
    // rendered as an all-null report. Declared shape + empty samples is an
    // truthful "this file contains no audio frames" instead.
    let sample_rate = codec_params
        .sample_rate
        .ok_or(DecodeError::MissingStreamInfo)?;
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count() as u16)
        .ok_or(DecodeError::MissingStreamInfo)?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())?;

    let mut samples = Vec::new();

    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        // Every error here means the file failed to reconstruct the exact
        // PCM it promises (FLAC is lossless by spec) — unlike a playback
        // use case, there's no defensible "skip the bad packet and keep going"
        // for an analysis tool that promises sample-exact decode, so every
        // error is fatal rather than silently dropped.
        let decoded = decoder.decode(&packet)?;

        // The FLAC decoder always produces left-justified 32-bit integer
        // PCM buffers (see module docs) regardless of the stream's true
        // bit depth.
        let GenericAudioBufferRef::S32(buf) = decoded else {
            return Err(DecodeError::UnexpectedSampleFormat);
        };

        // One fixed spec per FLAC stream; a packet disagreeing with
        // STREAMINFO is malformed input, not something to silently adopt.
        if buf.spec().rate() != sample_rate || buf.spec().channels().count() as u16 != channels {
            return Err(DecodeError::InconsistentStream);
        }
        let frames = buf.frames();

        // Planar (per-channel) in the decoded buffer; cochlea_features::Audio
        // wants interleaved, so transpose while normalizing.
        for frame in 0..frames {
            for ch in 0..usize::from(channels) {
                let plane = buf
                    .plane(ch)
                    .expect("ch is within 0..channels, matching buf.num_planes()");
                samples.push((f64::from(plane[frame]) / FULL_SCALE_I32) as f32);
            }
        }

        // Refuse a bomb as it accumulates, before the whole buffer
        // materializes — the overshoot past the cap is at most one packet.
        if let Some(limit) = limit
            && samples.len() as u64 > limit
        {
            return Err(DecodeError::TooLong {
                samples: samples.len() as u64,
                limit,
            });
        }
    }

    Ok(Audio {
        samples,
        channels,
        sample_rate,
    })
}
