//! A minimal, streaming, symphonia-backed `rodio::Source` used by the playback worker
//! (`player.rs`) instead of `rodio::Decoder`.
//!
//! Why this exists: `rodio::Decoder::new(reader)` wraps any `Read + Seek` in rodio's own
//! private `ReadSeekSource`, whose `MediaSource::byte_len()` unconditionally returns `None`
//! (see rodio 0.20.1's `src/decoder/read_seek_source.rs`). Several symphonia format
//! demuxers — notably FLAC's, unconditionally regardless of seek mode — require a known
//! byte length to seek at all, and return `SeekError::Unseekable` without one. That makes
//! `rodio::Decoder`'s `try_seek` silently unable to do anything but the slowest possible
//! fallback for those formats, no matter what feature flags are enabled.
//!
//! This wraps symphonia's own `FormatReader`/`Decoder` directly (the same public APIs
//! rodio's own private decoder uses) over a `MediaSource` that reports a real byte length
//! (`file.metadata()?.len()`), which is all several demuxers need to seek efficiently. rodio
//! is still used for everything else (`Sink`/`OutputStream`/`cpal` audio output) — only its
//! decoder layer is bypassed here.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError as RodioSeekError;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder as CodecDecoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, SeekedTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{self, Time};

// Decode errors are not considered fatal — retry on the next packet, up to a point.
const MAX_DECODE_RETRIES: usize = 3;

/// A `symphonia::core::io::MediaSource` that reports a real byte length, unlike rodio's
/// private `ReadSeekSource`. This is the one thing that needs to differ from rodio's own
/// decoder for demuxer-level seeking to work for every format symphonia supports.
struct FileMediaSource {
    file: File,
    len: Option<u64>,
}

impl FileMediaSource {
    fn new(file: File) -> Self {
        let len = file.metadata().ok().map(|m| m.len());
        Self { file, len }
    }
}

impl MediaSource for FileMediaSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

impl Read for FileMediaSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for FileMediaSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

/// Opens `path` as a symphonia `MediaSource`. Every symphonia reader in the backend goes
/// through here, so they all handle the same files.
///
/// AIFF/AIFF-C files whose `SSND` chunk has a non-zero `offset` (padding between the chunk
/// header and the first sample; some encoders write a few KB of it) are rejected by
/// symphonia 0.5 ("No support for AIFF block-aligned data"). For those, the file is served
/// through `AiffSsndOffsetSource`, which hides the padding so the file reads like one with
/// `offset == 0`.
pub(crate) fn open_media_source(path: &Path) -> std::io::Result<Box<dyn MediaSource>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if let Some(layout) = AiffSsndOffsetSource::detect(&mut file, len) {
        file.seek(SeekFrom::Start(0))?;
        return Ok(Box::new(AiffSsndOffsetSource::new(file, len, layout)));
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(Box::new(FileMediaSource::new(file)))
}

/// Where an AIFF file's `SSND` padding sits, from `AiffSsndOffsetSource::detect`.
struct AiffSsndLayout {
    form_size: u32,
    ssnd_pos: u64,
    ssnd_size: u32,
    offset: u32,
}

/// A view of an AIFF file with its `SSND` padding cut out: the FORM and SSND sizes shrink
/// by the padding, the SSND `offset`/`blockSize` fields read as 0, and everything after
/// them is shifted back over the padding. Positions are in that view.
struct AiffSsndOffsetSource {
    file: File,
    /// Up to here the view is the file itself (apart from `patches`).
    split: u64,
    /// Bytes skipped at `split`: the padding.
    skip: u64,
    len: u64,
    /// Rewritten big-endian u32 fields: (position, value).
    patches: [(u64, [u8; 4]); 4],
    pos: u64,
}

impl AiffSsndOffsetSource {
    /// Walks the chunks of a FORM AIFF/AIFC file and returns its SSND layout when the
    /// `offset` is non-zero; `None` for anything else (including other formats).
    fn detect(file: &mut File, len: u64) -> Option<AiffSsndLayout> {
        let mut header = [0u8; 12];
        file.seek(SeekFrom::Start(0)).ok()?;
        file.read_exact(&mut header).ok()?;
        if &header[0..4] != b"FORM" || !(&header[8..12] == b"AIFF" || &header[8..12] == b"AIFC") {
            return None;
        }
        let form_size = u32::from_be_bytes(header[4..8].try_into().ok()?);
        let mut pos = 12u64;
        while pos + 8 <= len {
            let mut chunk = [0u8; 8];
            file.seek(SeekFrom::Start(pos)).ok()?;
            file.read_exact(&mut chunk).ok()?;
            let size = u32::from_be_bytes(chunk[4..8].try_into().ok()?);
            if &chunk[0..4] == b"SSND" {
                let mut fields = [0u8; 4];
                file.read_exact(&mut fields).ok()?;
                let offset = u32::from_be_bytes(fields);
                // The padding must fit in the chunk, after the 8 bytes of offset/blockSize.
                if offset == 0 || u64::from(offset) + 8 > u64::from(size) {
                    return None;
                }
                return Some(AiffSsndLayout {
                    form_size,
                    ssnd_pos: pos,
                    ssnd_size: size,
                    offset,
                });
            }
            pos += 8 + u64::from(size) + u64::from(size & 1);
        }
        None
    }

    fn new(file: File, len: u64, layout: AiffSsndLayout) -> Self {
        let AiffSsndLayout {
            form_size,
            ssnd_pos,
            ssnd_size,
            offset,
        } = layout;
        Self {
            file,
            split: ssnd_pos + 16,
            skip: u64::from(offset),
            len: len - u64::from(offset),
            patches: [
                (4, form_size.saturating_sub(offset).to_be_bytes()),
                (ssnd_pos + 4, (ssnd_size - offset).to_be_bytes()),
                (ssnd_pos + 8, [0; 4]),
                (ssnd_pos + 12, [0; 4]),
            ],
            pos: 0,
        }
    }
}

impl MediaSource for AiffSsndOffsetSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl Read for AiffSsndOffsetSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let start = self.pos;
        let (real, max) = if start < self.split {
            (start, (self.split - start) as usize)
        } else {
            (start + self.skip, (self.len - start) as usize)
        };
        let want = buf.len().min(max);
        self.file.seek(SeekFrom::Start(real))?;
        let n = self.file.read(&mut buf[..want])?;
        for (at, bytes) in &self.patches {
            for (i, byte) in bytes.iter().enumerate() {
                let p = at + i as u64;
                if p >= start && p < start + n as u64 {
                    buf[(p - start) as usize] = *byte;
                }
            }
        }
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for AiffSsndOffsetSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let next = next.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start")
        })?;
        self.pos = next;
        Ok(next)
    }
}

#[derive(Debug)]
pub struct DecoderError(pub String);

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DecoderError {}

fn to_decoder_error(err: SymphoniaError) -> DecoderError {
    DecoderError(err.to_string())
}

/// A streaming, seekable audio source backed directly by symphonia. Produces `i16` samples,
/// matching rodio's own decoder convention, so it's a drop-in for `sink.append(...)`.
pub struct SeekableSymphoniaSource {
    decoder: Box<dyn CodecDecoder>,
    format: Box<dyn FormatReader>,
    current_frame_offset: usize,
    total_duration: Option<Time>,
    buffer: SampleBuffer<i16>,
    spec: SignalSpec,
}

impl SeekableSymphoniaSource {
    pub fn open(path: &Path) -> Result<Self, DecoderError> {
        let source = open_media_source(path).map_err(|err| DecoderError(err.to_string()))?;
        let mss = MediaSourceStream::new(source, Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|v| v.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();
        let mut probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(to_decoder_error)?;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| DecoderError("no track with a supported codec".to_string()))?
            .clone();

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(to_decoder_error)?;
        let total_duration = track
            .codec_params
            .time_base
            .zip(track.codec_params.n_frames)
            .map(|(base, frames)| base.calc_time(frames));

        let mut decode_errors: usize = 0;
        let decoded = loop {
            let packet = match probed.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Err(DecoderError("audio file produced no samples".to_string()));
                }
                Err(err) => return Err(to_decoder_error(err)),
            };
            if packet.track_id() != track.id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(decoded) => break decoded,
                Err(SymphoniaError::DecodeError(_)) => {
                    decode_errors += 1;
                    if decode_errors > MAX_DECODE_RETRIES {
                        return Err(DecoderError(
                            "too many consecutive decode errors".to_string(),
                        ));
                    }
                    continue;
                }
                Err(err) => return Err(to_decoder_error(err)),
            }
        };

        let spec = *decoded.spec();
        let buffer = Self::to_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            format: probed.format,
            current_frame_offset: 0,
            total_duration,
            buffer,
            spec,
        })
    }

    fn to_buffer(decoded: AudioBufferRef, spec: &SignalSpec) -> SampleBuffer<i16> {
        let duration = units::Duration::from(decoded.capacity() as u64);
        let mut buffer = SampleBuffer::<i16>::new(duration, *spec);
        buffer.copy_interleaved_ref(decoded);
        buffer
    }

    fn refine_position(&mut self, seek_res: SeekedTo) -> Result<(), DecoderError> {
        let mut samples_to_pass = seek_res.required_ts.saturating_sub(seek_res.actual_ts);
        let packet = loop {
            let candidate = self.format.next_packet().map_err(to_decoder_error)?;
            if candidate.dur() > samples_to_pass {
                break candidate;
            }
            samples_to_pass -= candidate.dur();
        };

        let mut decoded = self.decoder.decode(&packet);
        for _ in 0..MAX_DECODE_RETRIES {
            if decoded.is_err() {
                let packet = self.format.next_packet().map_err(to_decoder_error)?;
                decoded = self.decoder.decode(&packet);
            } else {
                break;
            }
        }
        let decoded = decoded.map_err(to_decoder_error)?;
        self.spec = *decoded.spec();
        self.buffer = Self::to_buffer(decoded, &self.spec);
        self.current_frame_offset = samples_to_pass as usize * self.channels() as usize;
        Ok(())
    }
}

impl Iterator for SeekableSymphoniaSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.current_frame_offset >= self.buffer.samples().len() {
            let packet = self.format.next_packet().ok()?;
            let mut decoded = self.decoder.decode(&packet);
            for _ in 0..MAX_DECODE_RETRIES {
                if decoded.is_err() {
                    let packet = self.format.next_packet().ok()?;
                    decoded = self.decoder.decode(&packet);
                } else {
                    break;
                }
            }
            let decoded = decoded.ok()?;
            self.spec = *decoded.spec();
            self.buffer = Self::to_buffer(decoded, &self.spec);
            self.current_frame_offset = 0;
        }

        let sample = *self.buffer.samples().get(self.current_frame_offset)?;
        self.current_frame_offset += 1;
        Some(sample)
    }
}

impl Source for SeekableSymphoniaSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.buffer.samples().len())
    }

    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }

    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
            .map(|Time { seconds, frac }| Duration::new(seconds, (frac * 1e9) as u32))
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), RodioSeekError> {
        let seek_beyond_end = self
            .total_duration()
            .is_some_and(|dur| dur.saturating_sub(pos).as_millis() < 1);

        let time: Time = if seek_beyond_end {
            let time = self.total_duration.expect("checked by seek_beyond_end");
            skip_back_a_tiny_bit(time)
        } else {
            pos.as_secs_f64().into()
        };

        let to_skip = self.current_frame_offset % self.channels().max(1) as usize;

        let seek_res = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: None,
                },
            )
            .map_err(|err| RodioSeekError::Other(Box::new(to_decoder_error(err))))?;

        self.refine_position(seek_res)
            .map_err(|err| RodioSeekError::Other(Box::new(err)))?;
        self.current_frame_offset += to_skip;

        Ok(())
    }
}

fn skip_back_a_tiny_bit(
    Time {
        mut seconds,
        mut frac,
    }: Time,
) -> Time {
    frac -= 0.0001;
    if frac < 0.0 {
        seconds = seconds.saturating_sub(1);
        frac = 1.0 - frac;
    }
    Time { seconds, frac }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats/track_format_wav.wav")
    }

    fn flac_fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats/track_format_flac.flac")
    }

    /// A 16-bit stereo AIFF-C `sowt` file whose SSND chunk has `offset` bytes of padding
    /// (0xEE) before `frames` frames of samples.
    fn write_padded_aifc(path: &Path, offset: u32, frames: u32) {
        let data_len = frames * 4;
        let ssnd_size = 8 + offset + data_len;
        let comm: Vec<u8> = [
            &2u16.to_be_bytes()[..],
            &frames.to_be_bytes(),
            &16u16.to_be_bytes(),
            // 44100 as an 80-bit extended float.
            &[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0],
            b"sowt",
            &[0, 0],
        ]
        .concat();
        let mut body = Vec::new();
        body.extend_from_slice(b"AIFC");
        body.extend_from_slice(b"FVER");
        body.extend_from_slice(&4u32.to_be_bytes());
        body.extend_from_slice(&0xA280_5140u32.to_be_bytes());
        body.extend_from_slice(b"COMM");
        body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        body.extend_from_slice(&comm);
        body.extend_from_slice(b"SSND");
        body.extend_from_slice(&ssnd_size.to_be_bytes());
        body.extend_from_slice(&offset.to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend(std::iter::repeat_n(0xEE, offset as usize));
        for i in 0..frames {
            let v = ((i % 200) as i16 - 100) * 100;
            body.extend_from_slice(&v.to_le_bytes());
            body.extend_from_slice(&v.to_le_bytes());
        }
        let mut file = b"FORM".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_be_bytes());
        file.extend_from_slice(&body);
        std::fs::write(path, file).unwrap();
    }

    #[test]
    fn aiff_with_ssnd_padding_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("padded.aiff");
        write_padded_aifc(&path, 2280, 44_100);

        let mut source = SeekableSymphoniaSource::open(&path).expect("padded AIFF should open");
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 44_100);
        // The first samples are the audio, not the 0xEE padding.
        let first: Vec<i16> = source.by_ref().take(4).collect();
        assert_eq!(first, vec![-10_000, -10_000, -9_900, -9_900]);
        assert_eq!(source.count(), 44_100 * 2 - 4);
    }

    #[test]
    fn aiff_without_padding_is_read_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.aiff");
        write_padded_aifc(&path, 0, 100);
        let mut file = File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        assert!(AiffSsndOffsetSource::detect(&mut file, len).is_none());
        assert!(SeekableSymphoniaSource::open(&path).is_ok());
    }

    #[test]
    fn decoder_error_display_shows_message() {
        let err = DecoderError("boom".to_string());
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn skip_back_a_tiny_bit_subtracts_small_fraction() {
        let t = Time {
            seconds: 10,
            frac: 0.5,
        };
        let result = skip_back_a_tiny_bit(t);
        assert_eq!(result.seconds, 10);
        assert!((result.frac - 0.4999).abs() < 1e-9);
    }

    #[test]
    fn skip_back_a_tiny_bit_borrows_a_second_when_frac_underflows() {
        let t = Time {
            seconds: 5,
            frac: 0.00005,
        };
        let result = skip_back_a_tiny_bit(t);
        assert_eq!(result.seconds, 4);
        assert!(result.frac > 1.0);
    }

    #[test]
    fn skip_back_a_tiny_bit_saturates_at_zero_seconds() {
        let t = Time {
            seconds: 0,
            frac: 0.00001,
        };
        let result = skip_back_a_tiny_bit(t);
        assert_eq!(result.seconds, 0);
    }

    #[test]
    fn seekable_symphonia_source_exposes_stream_metadata() {
        let decoder =
            SeekableSymphoniaSource::open(&wav_fixture_path()).expect("should decode wav fixture");
        assert!(decoder.channels() >= 1);
        assert!(decoder.sample_rate() > 0);
        assert!(decoder.current_frame_len().unwrap_or(0) > 0);
    }

    #[test]
    fn seekable_symphonia_source_iterates_samples_across_packet_boundaries() {
        let decoder = SeekableSymphoniaSource::open(&flac_fixture_path())
            .expect("should decode flac fixture");
        // Pull far more samples than a single packet holds to force at least
        // one internal packet refill (Iterator::next's buffer-exhausted branch).
        let samples: Vec<i16> = decoder.take(200_000).collect();
        assert!(!samples.is_empty());
    }

    #[test]
    fn seekable_symphonia_source_open_errors_for_non_audio_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not_audio.wav");
        std::fs::write(&path, b"definitely not audio data").unwrap();
        let Err(err) = SeekableSymphoniaSource::open(&path) else {
            panic!("expected an error for a non-audio file");
        };
        assert!(!err.0.is_empty());
    }
}
