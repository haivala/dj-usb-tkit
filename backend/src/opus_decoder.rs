//! An Opus decoder for symphonia, backed by libopus.
//!
//! symphonia 0.5 reads Ogg Opus files (its Ogg demuxer maps the stream and fills the codec
//! parameters from the `OpusHead` packet) but has no Opus decoder. `OpusDecoder` fills that
//! gap and is registered next to symphonia's own decoders in
//! `symphonia_decoder::codecs`, so playback, analysis and transcodes all decode Opus the same
//! way they decode every other format.
//!
//! What the decoder does on top of libopus, from the Ogg Opus spec (RFC 7845):
//! - drops the stream's pre-skip (the encoder's warm-up samples, `OpusHead` bytes 10..12)
//!   from the start of the decoded audio, so the first sample out is the first sample of
//!   the track;
//! - applies the output gain (`OpusHead` bytes 16..18, Q7.8 dB).
//!
//! Only mono and stereo streams (channel mapping family 0, or family 1 with up to two
//! channels) are decoded; multichannel Opus is refused when the decoder is made.

use std::sync::Mutex;

use symphonia::core::audio::{
    AsAudioBufferRef, AudioBuffer, AudioBufferRef, Channels, Signal, SignalSpec,
};
use symphonia::core::codecs::{
    CODEC_TYPE_OPUS, CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult,
};
use symphonia::core::errors::{Error, Result, unsupported_error};
use symphonia::core::formats::Packet;

/// Opus always decodes at 48 kHz.
const SAMPLE_RATE: u32 = 48_000;

/// The longest Opus packet: 120 ms at 48 kHz.
const MAX_PACKET_FRAMES: usize = 5_760;

/// The Opus decoder symphonia's codec registry hands out for `CODEC_TYPE_OPUS`.
pub(crate) struct OpusDecoder {
    params: CodecParameters,
    /// libopus's decoder is `Send` but not `Sync`; symphonia's `Decoder` must be both. Only
    /// ever used through `&mut self` (`Mutex::get_mut`), so the lock is never taken.
    opus: Mutex<opus::Decoder>,
    channels: usize,
    /// Pre-skip frames still to drop from the start of the decoded audio.
    skip: usize,
    /// Interleaved libopus output for one packet.
    scratch: Vec<f32>,
    buf: AudioBuffer<f32>,
}

/// The fields of an `OpusHead` identification packet the decoder needs.
#[derive(Debug, PartialEq)]
struct OpusHead {
    channels: u8,
    pre_skip: u16,
    /// Q7.8 dB.
    output_gain: i16,
}

impl OpusHead {
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 19 || &data[..8] != b"OpusHead" {
            return None;
        }
        Some(Self {
            channels: data[9],
            pre_skip: u16::from_le_bytes([data[10], data[11]]),
            output_gain: i16::from_le_bytes([data[16], data[17]]),
        })
    }
}

impl OpusDecoder {
    fn spec(&self) -> SignalSpec {
        let channels = if self.channels == 1 {
            Channels::FRONT_LEFT
        } else {
            Channels::FRONT_LEFT | Channels::FRONT_RIGHT
        };
        SignalSpec::new(SAMPLE_RATE, channels)
    }

    fn decode_packet(&mut self, data: &[u8]) -> Result<()> {
        let opus = self.opus.get_mut().unwrap_or_else(|err| err.into_inner());
        let frames = opus
            .decode_float(data, &mut self.scratch, false)
            .map_err(|_| Error::DecodeError("opus: invalid packet"))?;
        let skip = self.skip.min(frames);
        self.skip -= skip;

        self.buf.clear();
        self.buf.render_reserved(Some(frames - skip));
        let channels = self.channels;
        let samples = &self.scratch[skip * channels..frames * channels];
        for ch in 0..channels {
            let plane = self.buf.chan_mut(ch);
            for (out, frame) in plane.iter_mut().zip(samples.chunks_exact(channels)) {
                *out = frame[ch];
            }
        }
        Ok(())
    }
}

impl Decoder for OpusDecoder {
    fn try_new(params: &CodecParameters, _options: &DecoderOptions) -> Result<Self> {
        let head = params
            .extra_data
            .as_deref()
            .and_then(OpusHead::parse)
            .ok_or(Error::DecodeError("opus: missing or invalid OpusHead"))?;
        let opus_channels = match head.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            _ => return unsupported_error("opus: only mono and stereo are supported"),
        };
        let mut opus = opus::Decoder::new(SAMPLE_RATE, opus_channels)
            .map_err(|_| Error::DecodeError("opus: decoder init failed"))?;
        if head.output_gain != 0 {
            opus.set_gain(i32::from(head.output_gain))
                .map_err(|_| Error::DecodeError("opus: invalid output gain"))?;
        }
        let channels = usize::from(head.channels);
        let mut decoder = Self {
            params: params.clone(),
            opus: Mutex::new(opus),
            channels,
            skip: usize::from(head.pre_skip),
            scratch: vec![0.0; MAX_PACKET_FRAMES * channels],
            buf: AudioBuffer::unused(),
        };
        decoder.buf = AudioBuffer::new(MAX_PACKET_FRAMES as u64, decoder.spec());
        Ok(decoder)
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[CodecDescriptor {
            codec: CODEC_TYPE_OPUS,
            short_name: "opus",
            long_name: "Opus (libopus)",
            inst_func: |params, options| Ok(Box::new(OpusDecoder::try_new(params, options)?)),
        }]
    }

    fn reset(&mut self) {
        // After a seek the stream continues mid-way: there is no pre-skip left to drop.
        if let Ok(opus) = self.opus.get_mut() {
            let _ = opus.reset_state();
        }
        self.skip = 0;
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        if let Err(err) = self.decode_packet(&packet.data) {
            self.buf.clear();
            return Err(err);
        }
        Ok(self.buf.as_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buf.as_audio_buffer_ref()
    }
}

/// Frames to add to a seek target in an Opus stream: its packet timestamps count the
/// pre-skip the decoder drops, so time `t` in the decoded audio sits `pre_skip` frames
/// later in the stream. Zero for any other codec.
pub(crate) fn seek_offset_frames(params: &CodecParameters) -> u64 {
    if params.codec != CODEC_TYPE_OPUS {
        return 0;
    }
    params
        .extra_data
        .as_deref()
        .and_then(OpusHead::parse)
        .map_or(0, |head| u64::from(head.pre_skip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::probe::Hint;

    /// Decodes the 1 s mono Ogg Opus fixture (pre-skip 312, 51 packets of 20 ms, all in
    /// one Ogg page) and returns the frame count of each decoded packet.
    fn decoded_packet_frames() -> Vec<usize> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats/track_format_ogg_opus.ogg");
        let media = crate::symphonia_decoder::open_media_source(&path).expect("open");
        let mut format = symphonia::default::get_probe()
            .format(
                Hint::new().with_extension("ogg"),
                MediaSourceStream::new(media, Default::default()),
                &FormatOptions::default(),
                &Default::default(),
            )
            .expect("probe")
            .format;
        let track = format.default_track().expect("track").clone();
        assert_eq!(track.codec_params.codec, CODEC_TYPE_OPUS);
        let mut decoder = crate::symphonia_decoder::codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .expect("an Opus decoder is registered");
        let mut frames = Vec::new();
        while let Ok(packet) = format.next_packet() {
            let decoded = decoder.decode(&packet).expect("decode");
            assert_eq!(decoded.spec().rate, SAMPLE_RATE);
            assert_eq!(decoded.spec().channels.count(), 1);
            frames.push(decoded.frames());
        }
        frames
    }

    #[test]
    fn the_pre_skip_is_dropped_from_the_start() {
        let frames = decoded_packet_frames();
        assert_eq!(frames.len(), 51);
        // The first packet loses its 312 warm-up frames; the rest are whole.
        assert_eq!(frames[0], 960 - 312);
        assert!(frames[1..].iter().all(|&n| n == 960), "{frames:?}");
    }

    #[test]
    fn opus_head_fields_are_read() {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 2]); // version, channels
        head.extend_from_slice(&312u16.to_le_bytes()); // pre-skip
        head.extend_from_slice(&44_100u32.to_le_bytes()); // input rate
        head.extend_from_slice(&(-256i16).to_le_bytes()); // gain: -1 dB
        head.push(0); // mapping family
        assert_eq!(
            OpusHead::parse(&head),
            Some(OpusHead {
                channels: 2,
                pre_skip: 312,
                output_gain: -256,
            })
        );
        assert_eq!(OpusHead::parse(&head[..18]), None);
        assert_eq!(OpusHead::parse(b"OpusTags..........."), None);
    }
}
