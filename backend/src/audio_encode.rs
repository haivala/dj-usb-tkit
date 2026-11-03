//! Transcodes one audio file into AIFF, WAV or MP3 320 CBR.
//!
//! The source is decoded at full resolution (every channel, the source's own
//! sample rate and bit depth) and streamed into the target writer, so memory
//! stays bounded whatever the track length. PCM targets keep the source's
//! rate and depth (16 or 24 bit); MP3 is always 320 kbit/s CBR, resampled by
//! LAME to 44.1 or 48 kHz when the source rate isn't an MPEG-1 rate.
//!
//! Tags and cover art are written afterwards with lofty from values the
//! caller passes in (the app's own track data, not the source file's tags).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use mp3lame_encoder::{Bitrate, Builder, FlushGap, InterleavedPcm, MonoPcm, Quality};
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// MP3 decoders output this many samples of their own filter delay before
/// the first encoded sample (the "529" of the LAME/gapless docs).
const MP3_DECODER_DELAY_SAMPLES: u32 = 529;
const MP3_BITRATE_KBPS: u32 = 320;

/// A transcode target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranscodeFormat {
    Aiff,
    Wav,
    Mp3,
}

impl TranscodeFormat {
    pub const ALL: [TranscodeFormat; 3] = [Self::Aiff, Self::Wav, Self::Mp3];

    /// Stable id stored in the DB and used on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aiff => "aiff",
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.as_str().eq_ignore_ascii_case(value.trim()))
    }

    pub fn extension(self) -> &'static str {
        self.as_str()
    }

    /// What the library shows for this transcode.
    pub fn label(self) -> &'static str {
        match self {
            Self::Aiff => "AIFF",
            Self::Wav => "WAV",
            Self::Mp3 => "MP3 320",
        }
    }
}

/// Tag values written into the transcode.
#[derive(Debug, Clone, Default)]
pub struct TranscodeTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    /// JPEG or PNG bytes for the front cover.
    pub artwork: Option<Vec<u8>>,
}

/// Facts about a finished transcode.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscodeOutput {
    pub sample_rate: u32,
    /// PCM bit depth; `None` for MP3.
    pub bit_depth: Option<u8>,
    pub channels: u16,
    pub bitrate_kbps: u32,
    pub duration_ms: u64,
    /// How much later the audio starts in the transcode than in the source,
    /// as an untrimmed decoder (and the CDJ) plays it. 0 for PCM.
    pub offset_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscodeError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for TranscodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

fn failed(err: impl std::fmt::Display) -> TranscodeError {
    TranscodeError::Failed(err.to_string())
}

/// Transcodes `src` into `dst` (which is created or replaced). `dst` must
/// carry the target's extension, so lofty can tell the file type when it
/// writes the tags. On error or cancel, `dst` may be left half-written; the
/// caller writes to a temporary name and renames on success.
pub fn transcode_file(
    src: &Path,
    dst: &Path,
    format: TranscodeFormat,
    tags: &TranscodeTags,
    cancel: &AtomicBool,
) -> Result<TranscodeOutput, TranscodeError> {
    let mut source = PcmSource::open(src)?;
    let channels = source.channels;
    let sample_rate = source.sample_rate;
    let mut sink = match format {
        TranscodeFormat::Aiff | TranscodeFormat::Wav => {
            let bits = if source.bits_per_sample <= 16 { 16 } else { 24 };
            Sink::Pcm(PcmWriter::create(dst, format, channels, bits, sample_rate)?)
        }
        TranscodeFormat::Mp3 => Sink::Mp3(Box::new(Mp3Writer::create(dst, channels, sample_rate)?)),
    };

    let mut frames: u64 = 0;
    source.for_each_chunk(cancel, |samples| {
        frames += (samples.len() / usize::from(channels)) as u64;
        sink.write(samples)
    })?;

    let mut output = sink.finish()?;
    output.duration_ms = frames * 1000 / u64::from(sample_rate.max(1));
    write_tags(dst, tags)?;
    Ok(output)
}

enum Sink {
    Pcm(PcmWriter),
    Mp3(Box<Mp3Writer>),
}

impl Sink {
    fn write(&mut self, samples: &[i32]) -> Result<(), TranscodeError> {
        match self {
            Self::Pcm(writer) => writer.write(samples),
            Self::Mp3(writer) => writer.write(samples),
        }
    }

    fn finish(self) -> Result<TranscodeOutput, TranscodeError> {
        match self {
            Self::Pcm(writer) => writer.finish(),
            Self::Mp3(writer) => writer.finish(),
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding

struct PcmSource {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u32,
    buffer: Option<SampleBuffer<i32>>,
    /// The first packet, decoded by `open` to learn the stream's layout (some
    /// containers, like ALAC in MP4, only state it in the decoded audio).
    primed: bool,
}

impl PcmSource {
    fn open(path: &Path) -> Result<Self, TranscodeError> {
        let media = crate::symphonia_decoder::open_media_source(path).map_err(failed)?;
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                MediaSourceStream::new(media, Default::default()),
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(failed)?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| failed("no audio track"))?;
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(failed)?;
        let bits_per_sample = track
            .codec_params
            .bits_per_sample
            .or(decoder.codec_params().bits_per_sample)
            .or_else(|| alac_cookie_bit_depth(&track.codec_params))
            .unwrap_or(16);
        let mut source = Self {
            format,
            decoder,
            track_id,
            channels: 0,
            sample_rate: 0,
            bits_per_sample,
            buffer: None,
            primed: false,
        };
        let spec = source
            .decode_next()?
            .ok_or_else(|| failed("the file has no audio"))?;
        source.channels = u16::try_from(spec.channels.count()).map_err(failed)?;
        source.sample_rate = spec.rate;
        if source.channels == 0 || source.sample_rate == 0 {
            return Err(failed("unknown channel count or sample rate"));
        }
        source.primed = true;
        Ok(source)
    }

    /// Decodes the next packet of the track into `self.buffer`. `None` at
    /// the end of the stream.
    fn decode_next(&mut self) -> Result<Option<SignalSpec>, TranscodeError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(err) => return Err(failed(err)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(err) => return Err(failed(err)),
            };
            let spec = *decoded.spec();
            if self
                .buffer
                .as_ref()
                .is_none_or(|buf| buf.capacity() < decoded.capacity())
            {
                self.buffer = Some(SampleBuffer::<i32>::new(decoded.capacity() as u64, spec));
            }
            if let Some(buf) = self.buffer.as_mut() {
                buf.copy_interleaved_ref(decoded);
            }
            return Ok(Some(spec));
        }
    }

    /// Calls `on_chunk` with each decoded packet as interleaved, full-scale
    /// i32 samples. Checks `cancel` between packets.
    fn for_each_chunk(
        &mut self,
        cancel: &AtomicBool,
        mut on_chunk: impl FnMut(&[i32]) -> Result<(), TranscodeError>,
    ) -> Result<(), TranscodeError> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(TranscodeError::Cancelled);
            }
            if !std::mem::take(&mut self.primed) {
                let Some(spec) = self.decode_next()? else {
                    return Ok(());
                };
                if spec.channels.count() != usize::from(self.channels)
                    || spec.rate != self.sample_rate
                {
                    return Err(failed(
                        "the stream changes its channel count or sample rate",
                    ));
                }
            }
            if let Some(buf) = self.buffer.as_ref()
                && !buf.samples().is_empty()
            {
                on_chunk(buf.samples())?;
            }
        }
    }
}

/// ALAC in MP4 states its bit depth only in the codec's "magic cookie"
/// (`ALACSpecificConfig`: frame length u32, version u8, bit depth u8, ...),
/// which symphonia reads but doesn't expose.
fn alac_cookie_bit_depth(params: &symphonia::core::codecs::CodecParameters) -> Option<u32> {
    if params.codec != symphonia::core::codecs::CODEC_TYPE_ALAC {
        return None;
    }
    let cookie = params.extra_data.as_deref()?;
    if cookie.len() != 24 && cookie.len() != 48 {
        return None;
    }
    Some(u32::from(cookie[5])).filter(|bits| (1..=32).contains(bits))
}

// ---------------------------------------------------------------------------
// AIFF / WAV

struct PcmWriter {
    file: BufWriter<File>,
    format: TranscodeFormat,
    channels: u16,
    bits: u16,
    sample_rate: u32,
    frames: u64,
    data_bytes: u64,
    scratch: Vec<u8>,
}

/// Where the size fields sit in the headers `PcmWriter` writes.
const AIFF_FORM_SIZE_AT: u64 = 4;
const AIFF_COMM_FRAMES_AT: u64 = 22;
const AIFF_SSND_SIZE_AT: u64 = 42;
const WAV_RIFF_SIZE_AT: u64 = 4;
const WAV_DATA_SIZE_AT: u64 = 40;

impl PcmWriter {
    fn create(
        path: &Path,
        format: TranscodeFormat,
        channels: u16,
        bits: u16,
        sample_rate: u32,
    ) -> Result<Self, TranscodeError> {
        let mut file = BufWriter::new(File::create(path).map_err(failed)?);
        let block_align = channels * (bits / 8);
        let header = match format {
            TranscodeFormat::Aiff => {
                let mut h = Vec::with_capacity(54);
                h.extend_from_slice(b"FORM");
                h.extend_from_slice(&0u32.to_be_bytes());
                h.extend_from_slice(b"AIFF");
                h.extend_from_slice(b"COMM");
                h.extend_from_slice(&18u32.to_be_bytes());
                h.extend_from_slice(&channels.to_be_bytes());
                h.extend_from_slice(&0u32.to_be_bytes());
                h.extend_from_slice(&bits.to_be_bytes());
                h.extend_from_slice(&extended_80(sample_rate));
                h.extend_from_slice(b"SSND");
                h.extend_from_slice(&0u32.to_be_bytes());
                h.extend_from_slice(&0u32.to_be_bytes()); // offset
                h.extend_from_slice(&0u32.to_be_bytes()); // block size
                h
            }
            TranscodeFormat::Wav => {
                let mut h = Vec::with_capacity(44);
                h.extend_from_slice(b"RIFF");
                h.extend_from_slice(&0u32.to_le_bytes());
                h.extend_from_slice(b"WAVE");
                h.extend_from_slice(b"fmt ");
                h.extend_from_slice(&16u32.to_le_bytes());
                h.extend_from_slice(&1u16.to_le_bytes()); // plain PCM, never extensible
                h.extend_from_slice(&channels.to_le_bytes());
                h.extend_from_slice(&sample_rate.to_le_bytes());
                h.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
                h.extend_from_slice(&block_align.to_le_bytes());
                h.extend_from_slice(&bits.to_le_bytes());
                h.extend_from_slice(b"data");
                h.extend_from_slice(&0u32.to_le_bytes());
                h
            }
            TranscodeFormat::Mp3 => return Err(failed("MP3 is not a PCM format")),
        };
        file.write_all(&header).map_err(failed)?;
        Ok(Self {
            file,
            format,
            channels,
            bits,
            sample_rate,
            frames: 0,
            data_bytes: 0,
            scratch: Vec::new(),
        })
    }

    fn write(&mut self, samples: &[i32]) -> Result<(), TranscodeError> {
        let bytes_per_sample = usize::from(self.bits / 8);
        let shift = 32 - u32::from(self.bits);
        let big_endian = self.format == TranscodeFormat::Aiff;
        self.scratch.clear();
        self.scratch.reserve(samples.len() * bytes_per_sample);
        for &sample in samples {
            let value = sample >> shift;
            let bytes = value.to_be_bytes();
            // The low `bytes_per_sample` bytes of the big-endian i32.
            let low = &bytes[4 - bytes_per_sample..];
            if big_endian {
                self.scratch.extend_from_slice(low);
            } else {
                self.scratch.extend(low.iter().rev());
            }
        }
        self.file.write_all(&self.scratch).map_err(failed)?;
        self.data_bytes += self.scratch.len() as u64;
        self.frames += (samples.len() / usize::from(self.channels)) as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<TranscodeOutput, TranscodeError> {
        let pad = self.data_bytes % 2;
        if pad == 1 {
            self.file.write_all(&[0]).map_err(failed)?;
        }
        let too_big = || failed("the transcode would be larger than 4 GB");
        let mut patch = |at: u64, value: u64, big_endian: bool| -> Result<(), TranscodeError> {
            let value = u32::try_from(value).map_err(|_| too_big())?;
            let bytes = if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            };
            self.file.seek(SeekFrom::Start(at)).map_err(failed)?;
            self.file.write_all(&bytes).map_err(failed)
        };
        match self.format {
            TranscodeFormat::Aiff => {
                patch(AIFF_FORM_SIZE_AT, 4 + 26 + 16 + self.data_bytes + pad, true)?;
                patch(AIFF_COMM_FRAMES_AT, self.frames, true)?;
                patch(AIFF_SSND_SIZE_AT, 8 + self.data_bytes, true)?;
            }
            _ => {
                patch(WAV_RIFF_SIZE_AT, 4 + 24 + 8 + self.data_bytes + pad, false)?;
                patch(WAV_DATA_SIZE_AT, self.data_bytes, false)?;
            }
        }
        self.file.flush().map_err(failed)?;
        Ok(TranscodeOutput {
            sample_rate: self.sample_rate,
            bit_depth: u8::try_from(self.bits).ok(),
            channels: self.channels,
            bitrate_kbps: self.sample_rate * u32::from(self.channels) * u32::from(self.bits) / 1000,
            duration_ms: 0,
            offset_ms: 0.0,
        })
    }
}

/// An integer sample rate as the 80-bit IEEE extended float AIFF's COMM
/// chunk stores.
fn extended_80(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let leading = rate.leading_zeros();
    let exponent = 16383 + 31 - leading as u16;
    let mantissa = u64::from(rate) << (32 + leading);
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

// ---------------------------------------------------------------------------
// MP3

struct Mp3Writer {
    encoder: mp3lame_encoder::Encoder,
    file: BufWriter<File>,
    channels: u16,
    out_rate: u32,
    buf: Vec<u8>,
}

/// The MPEG-1 rate LAME encodes to: the source rate when it is one, else
/// 44.1 kHz for the 44.1 kHz family and 48 kHz for the rest.
fn mp3_output_rate(source_rate: u32) -> u32 {
    match source_rate {
        32_000 | 44_100 | 48_000 => source_rate,
        rate if rate % 11_025 == 0 => 44_100,
        _ => 48_000,
    }
}

impl Mp3Writer {
    fn create(path: &Path, channels: u16, sample_rate: u32) -> Result<Self, TranscodeError> {
        if channels > 2 {
            return Err(failed(
                "MP3 transcodes support mono and stereo sources only",
            ));
        }
        let out_rate = mp3_output_rate(sample_rate);
        let mut builder = Builder::new().ok_or_else(|| failed("LAME could not start"))?;
        builder.set_num_channels(channels as u8).map_err(failed)?;
        builder.set_sample_rate(sample_rate).map_err(failed)?;
        builder
            .set_output_sample_rate(std::num::NonZeroU32::new(out_rate))
            .map_err(failed)?;
        builder.set_brate(Bitrate::Kbps320).map_err(failed)?;
        builder.set_quality(Quality::NearBest).map_err(failed)?;
        builder.set_to_write_vbr_tag(true).map_err(failed)?;
        let encoder = builder.build().map_err(failed)?;
        let file = BufWriter::new(File::create(path).map_err(failed)?);
        Ok(Self {
            encoder,
            file,
            channels,
            out_rate,
            buf: Vec::new(),
        })
    }

    fn write(&mut self, samples: &[i32]) -> Result<(), TranscodeError> {
        let frames = samples.len() / usize::from(self.channels);
        self.buf.clear();
        self.buf.reserve(frames * 5 / 4 + 7200);
        let result = if self.channels == 1 {
            self.encoder.encode_to_vec(MonoPcm(samples), &mut self.buf)
        } else {
            self.encoder
                .encode_to_vec(InterleavedPcm(samples), &mut self.buf)
        };
        result.map_err(|err| failed(format!("MP3 encoding failed: {err:?}")))?;
        self.file.write_all(&self.buf).map_err(failed)
    }

    fn finish(mut self) -> Result<TranscodeOutput, TranscodeError> {
        self.buf.clear();
        self.buf.reserve(7200);
        self.encoder
            .flush_to_vec::<FlushGap>(&mut self.buf)
            .map_err(|err| failed(format!("MP3 encoding failed: {err:?}")))?;
        self.file.write_all(&self.buf).map_err(failed)?;

        // LAME left a placeholder frame at the start for its Info tag; fill
        // it in now that the stream is complete.
        let mut tag = Vec::with_capacity(self.encoder.lame_tag_size());
        self.encoder
            .lame_tag_encode_to_vec(&mut tag)
            .ok_or_else(|| failed("LAME produced no Info tag"))?;
        self.file.seek(SeekFrom::Start(0)).map_err(failed)?;
        self.file.write_all(&tag).map_err(failed)?;
        self.file.flush().map_err(failed)?;

        let encoder_delay = lame_tag_encoder_delay(&tag)
            .ok_or_else(|| failed("LAME Info tag has no encoder delay"))?;
        let offset_samples = encoder_delay + MP3_DECODER_DELAY_SAMPLES;
        Ok(TranscodeOutput {
            sample_rate: self.out_rate,
            bit_depth: None,
            channels: self.channels,
            bitrate_kbps: MP3_BITRATE_KBPS,
            duration_ms: 0,
            offset_ms: f64::from(offset_samples) * 1000.0 / f64::from(self.out_rate),
        })
    }
}

/// The encoder delay (in samples) from a LAME Info/Xing frame: 12 bits, 21
/// bytes after the `LAME` version string.
fn lame_tag_encoder_delay(frame: &[u8]) -> Option<u32> {
    let at = frame.windows(4).position(|w| w == b"LAME")? + 21;
    let bytes = frame.get(at..at + 2)?;
    Some((u32::from(bytes[0]) << 4) | (u32::from(bytes[1]) >> 4))
}

// ---------------------------------------------------------------------------
// Tags

fn write_tags(path: &Path, tags: &TranscodeTags) -> Result<(), TranscodeError> {
    let mut tag = Tag::new(TagType::Id3v2);
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    if let Some(title) = text(&tags.title) {
        tag.set_title(title);
    }
    if let Some(artist) = text(&tags.artist) {
        tag.set_artist(artist);
    }
    if let Some(album) = text(&tags.album) {
        tag.set_album(album);
    }
    if let Some(genre) = text(&tags.genre) {
        tag.set_genre(genre);
    }
    if let Some(year) = tags.year.filter(|year| *year > 0) {
        tag.set_year(year);
    }
    if let Some(number) = tags.track_number.filter(|n| *n > 0) {
        tag.set_track(number);
    }
    if let Some(bpm) = tags.bpm.filter(|bpm| *bpm > 0.0) {
        tag.insert_text(ItemKey::IntegerBpm, format!("{}", bpm.round() as u32));
    }
    if let Some(key) = text(&tags.key) {
        tag.insert_text(ItemKey::InitialKey, key);
    }
    if let Some(artwork) = tags.artwork.as_ref().filter(|bytes| !bytes.is_empty()) {
        let mime = if artwork.starts_with(&[0x89, b'P', b'N', b'G']) {
            MimeType::Png
        } else {
            MimeType::Jpeg
        };
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(mime),
            None,
            artwork.clone(),
        ));
    }
    if tag.is_empty() {
        return Ok(());
    }
    tag.save_to_path(path, WriteOptions::default())
        .map_err(|err| failed(format!("writing tags failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_80_matches_known_aiff_rates() {
        assert_eq!(
            extended_80(44_100),
            [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            extended_80(48_000),
            [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn mp3_output_rate_picks_an_mpeg1_rate() {
        assert_eq!(mp3_output_rate(44_100), 44_100);
        assert_eq!(mp3_output_rate(48_000), 48_000);
        assert_eq!(mp3_output_rate(88_200), 44_100);
        assert_eq!(mp3_output_rate(96_000), 48_000);
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats")
            .join(name)
    }

    /// Every sample of `path`, as the transcoder's own decoder reads it.
    fn decode_all(path: &Path) -> (Vec<i32>, u16, u32, u32) {
        let mut source = PcmSource::open(path).expect("open decoded file");
        let mut samples = Vec::new();
        source
            .for_each_chunk(&AtomicBool::new(false), |chunk| {
                samples.extend_from_slice(chunk);
                Ok(())
            })
            .expect("decode");
        (
            samples,
            source.channels,
            source.sample_rate,
            source.bits_per_sample,
        )
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn pcm_transcodes_are_sample_identical_to_the_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, depth) in [
            ("track_format_flac.flac", 16),
            ("track_format_flac_24bit.flac", 24),
            ("track_format_alac.m4a", 24),
        ] {
            let src = fixture(name);
            let (source_samples, channels, rate, _) = decode_all(&src);
            for format in [TranscodeFormat::Aiff, TranscodeFormat::Wav] {
                let dst = dir.path().join(format!("{name}.{}", format.extension()));
                let out =
                    transcode_file(&src, &dst, format, &TranscodeTags::default(), &no_cancel())
                        .expect("transcode");
                assert_eq!(out.sample_rate, rate, "{name} → {format:?}");
                assert_eq!(out.bit_depth, Some(depth), "{name} → {format:?}");
                assert_eq!(out.channels, channels, "{name} → {format:?}");
                assert_eq!(out.offset_ms, 0.0);
                let (out_samples, out_channels, out_rate, out_bits) = decode_all(&dst);
                assert_eq!(
                    (out_channels, out_rate),
                    (channels, rate),
                    "{name} → {format:?}"
                );
                assert_eq!(out_bits, u32::from(depth), "{name} → {format:?}");
                assert!(
                    out_samples == source_samples,
                    "{name} → {format:?}: samples differ"
                );
            }
        }
    }

    /// Deterministic noise bursts: unlike a tone, they correlate at exactly
    /// one lag.
    fn write_noise_wav(path: &Path, seconds: u32) {
        let rate = 44_100u32;
        let mut writer =
            PcmWriter::create(path, TranscodeFormat::Wav, 2, 16, rate).expect("create wav");
        let mut state: u32 = 0x1234_5678;
        let mut samples = Vec::new();
        for frame in 0..rate * seconds {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let burst = (frame / 2205) % 3 == 0;
            let value = if burst { (state as i32) >> 2 } else { 0 };
            samples.push(value);
            samples.push(value);
        }
        writer.write(&samples).expect("write wav");
        writer.finish().expect("finish wav");
    }

    fn mono_f32(samples: &[i32], channels: u16) -> Vec<f32> {
        samples
            .chunks(usize::from(channels))
            .map(|frame| frame.iter().map(|s| *s as f32).sum::<f32>() / frame.len() as f32)
            .collect()
    }

    #[test]
    fn mp3_offset_matches_where_the_decoded_audio_lands() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("noise.wav");
        write_noise_wav(&src, 3);
        let dst = dir.path().join("noise.mp3");
        let out = transcode_file(
            &src,
            &dst,
            TranscodeFormat::Mp3,
            &TranscodeTags::default(),
            &no_cancel(),
        )
        .expect("transcode");
        assert_eq!(out.sample_rate, 44_100);
        assert_eq!(out.bitrate_kbps, 320);
        assert_eq!(out.bit_depth, None);
        assert_eq!(out.duration_ms, 3000);

        let (source, source_channels, _, _) = decode_all(&src);
        let (encoded, encoded_channels, _, _) = decode_all(&dst);
        let source = mono_f32(&source, source_channels);
        let encoded = mono_f32(&encoded, encoded_channels);
        let window = 44_100;
        let best_lag = (0..4_000usize)
            .max_by(|a, b| {
                let score = |lag: usize| -> f64 {
                    source[..window]
                        .iter()
                        .zip(&encoded[lag..lag + window])
                        .map(|(x, y)| f64::from(*x) * f64::from(*y))
                        .sum()
                };
                score(*a).total_cmp(&score(*b))
            })
            .expect("lag");
        let measured_ms = best_lag as f64 * 1000.0 / 44_100.0;
        assert!(
            (measured_ms - out.offset_ms).abs() < 0.1,
            "stored offset {} ms, decoded audio lands at {measured_ms} ms",
            out.offset_ms
        );

        // CBR: an Info (not Xing) tag, and the size 320 kbit/s gives.
        let bytes = std::fs::read(&dst).expect("read mp3");
        assert!(bytes.windows(4).any(|w| w == b"Info"));
        let expected = 3.0 * 320_000.0 / 8.0;
        let size = bytes.len() as f64;
        assert!((size - expected).abs() / expected < 0.05, "size {size}");
    }

    #[test]
    fn tags_and_cover_come_from_the_given_values() {
        use lofty::file::TaggedFileExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let tags = TranscodeTags {
            title: Some("Made Up Title".into()),
            artist: Some("Imaginary Artist".into()),
            album: Some("No Such Album".into()),
            genre: Some("Techno".into()),
            year: Some(2021),
            track_number: Some(4),
            bpm: Some(127.9),
            key: Some("8A".into()),
            artwork: Some(vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]),
        };
        for format in TranscodeFormat::ALL {
            let dst = dir.path().join(format!("tagged.{}", format.extension()));
            transcode_file(
                &fixture("track_format_flac.flac"),
                &dst,
                format,
                &tags,
                &no_cancel(),
            )
            .expect("transcode");
            let file = lofty::read_from_path(&dst).expect("read tags");
            let tag = file.primary_tag().expect("tag");
            assert_eq!(tag.title().as_deref(), Some("Made Up Title"), "{format:?}");
            assert_eq!(
                tag.artist().as_deref(),
                Some("Imaginary Artist"),
                "{format:?}"
            );
            assert_eq!(tag.album().as_deref(), Some("No Such Album"), "{format:?}");
            assert_eq!(tag.year(), Some(2021), "{format:?}");
            assert_eq!(
                tag.get_string(&ItemKey::IntegerBpm),
                Some("128"),
                "{format:?}"
            );
            assert_eq!(
                tag.get_string(&ItemKey::InitialKey),
                Some("8A"),
                "{format:?}"
            );
            assert_eq!(tag.pictures().len(), 1, "{format:?}");
            // Tags must not break the audio.
            let (samples, _, _, _) = decode_all(&dst);
            assert!(!samples.is_empty(), "{format:?}");
        }
    }

    #[test]
    fn cancel_stops_before_the_first_packet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = transcode_file(
            &fixture("track_format_flac.flac"),
            &dir.path().join("x.wav"),
            TranscodeFormat::Wav,
            &TranscodeTags::default(),
            &AtomicBool::new(true),
        )
        .expect_err("cancelled");
        assert_eq!(err, TranscodeError::Cancelled);
    }

    #[test]
    fn format_round_trips_through_its_id() {
        for format in TranscodeFormat::ALL {
            assert_eq!(TranscodeFormat::parse(format.as_str()), Some(format));
        }
        assert_eq!(TranscodeFormat::parse("flac"), None);
    }
}
