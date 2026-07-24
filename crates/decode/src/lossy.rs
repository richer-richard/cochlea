//! Lossy decode (mp3, ogg/vorbis) via symphonia — the probe path's window
//! into the formats real-world audio actually ships in.
//!
//! **Contract, stated plainly**: lossy audio is *analysis input*, never
//! render ground truth. There is no bit-exactness to preserve — the codec
//! already threw the original samples away — so unlike the `flac` module
//! this one makes no sample-exactness claim. What it does keep: decoding
//! the same file with the same cochlea build yields the same `f32` buffer
//! every time (symphonia is pure Rust and our feature set disables its
//! `opt-simd` runtime dispatch), so probe reports over lossy input are
//! reproducible even though they aren't "the" samples in any exact sense.
//!
//! Sample-format handling: lossy decoders emit whatever layout their
//! synthesis produces (f32 for the mp3 and vorbis bundles today); the
//! match below converts the common integer/float layouts and refuses
//! anything else loudly ([`DecodeError::UnexpectedSampleFormat`]) instead
//! of guessing a normalization. Non-finite decoded samples are rejected
//! like the WAV path rejects them — poison for analyzers.

use std::fs::File;
use std::path::Path;

use cochlea_features::Audio;
use symphonia::core::audio::{Audio as _, GenericAudioBufferRef};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::TrackType;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;

use crate::error::DecodeError;

pub(crate) fn decode(
    path: &Path,
    extension_hint: &str,
    limit: Option<u64>,
) -> Result<Audio, DecodeError> {
    let file = File::open(path).map_err(|source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(extension_hint);

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
    // Unlike FLAC's STREAMINFO, lossy headers may omit shape fields the
    // packets themselves carry — fall back to the first decoded buffer's
    // spec instead of failing on a missing declaration.
    let declared_rate = codec_params.sample_rate;
    let declared_channels = codec_params.channels.as_ref().map(|c| c.count() as u16);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_rate = declared_rate;
    let mut channels = declared_channels;

    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;

        let rate = decoded.spec().rate();
        let ch = decoded.spec().channels().count() as u16;
        match (sample_rate, channels) {
            (None, _) | (_, None) => {
                sample_rate = Some(rate);
                channels = Some(ch);
            }
            (Some(r), Some(c)) => {
                // One fixed spec per stream, same rule as the FLAC path.
                if rate != r || ch != c {
                    return Err(DecodeError::InconsistentStream);
                }
            }
        }

        append_interleaved(&decoded, ch, &mut samples)?;

        // Refuse a decompression bomb as it accumulates, before the whole
        // buffer materializes — the overshoot past the cap is at most one
        // decoded packet.
        if let Some(limit) = limit
            && samples.len() as u64 > limit
        {
            return Err(DecodeError::TooLong {
                samples: samples.len() as u64,
                limit,
            });
        }
    }

    let (Some(sample_rate), Some(channels)) = (sample_rate, channels) else {
        // No header declaration and zero decodable packets.
        return Err(DecodeError::MissingStreamInfo);
    };

    if let Some(index) = samples.iter().position(|s| !s.is_finite()) {
        return Err(DecodeError::NonFiniteSample { index });
    }

    Ok(Audio {
        samples,
        channels,
        sample_rate,
    })
}

/// Transpose one decoded (planar) buffer into interleaved f32, converting
/// from whichever sample layout the decoder produced.
fn append_interleaved(
    decoded: &GenericAudioBufferRef<'_>,
    channels: u16,
    samples: &mut Vec<f32>,
) -> Result<(), DecodeError> {
    let channels = usize::from(channels);
    macro_rules! transpose {
        ($buf:expr, $convert:expr) => {{
            let buf = $buf;
            for frame in 0..buf.frames() {
                for ch in 0..channels {
                    let plane = buf
                        .plane(ch)
                        .expect("ch is within 0..channels, matching buf.num_planes()");
                    samples.push($convert(plane[frame]));
                }
            }
        }};
    }
    match decoded {
        GenericAudioBufferRef::F32(buf) => transpose!(buf, |v: f32| v),
        #[expect(
            clippy::cast_possible_truncation,
            reason = "f64 -> f32 narrowing is the crate's output format"
        )]
        GenericAudioBufferRef::F64(buf) => transpose!(buf, |v: f64| v as f32),
        GenericAudioBufferRef::S16(buf) => transpose!(buf, |v: i16| f32::from(v) / 32_768.0),
        GenericAudioBufferRef::S32(buf) => {
            transpose!(buf, |v: i32| (f64::from(v) / 2_147_483_648.0) as f32)
        }
        _ => return Err(DecodeError::UnexpectedSampleFormat),
    }
    Ok(())
}
