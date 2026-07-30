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

use rodio::source::SeekError as RodioSeekError;
use rodio::Source;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder as CodecDecoder, DecoderOptions, CODEC_TYPE_NULL};
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
        let file = File::open(path).map_err(|err| DecoderError(err.to_string()))?;
        let mss = MediaSourceStream::new(
            Box::new(FileMediaSource::new(file)) as Box<dyn MediaSource>,
            Default::default(),
        );

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|v| v.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions { enable_gapless: true, ..Default::default() };
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
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: None })
            .map_err(|err| RodioSeekError::Other(Box::new(to_decoder_error(err))))?;

        self.refine_position(seek_res)
            .map_err(|err| RodioSeekError::Other(Box::new(err)))?;
        self.current_frame_offset += to_skip;

        Ok(())
    }
}

fn skip_back_a_tiny_bit(Time { mut seconds, mut frac }: Time) -> Time {
    frac -= 0.0001;
    if frac < 0.0 {
        seconds = seconds.saturating_sub(1);
        frac = 1.0 - frac;
    }
    Time { seconds, frac }
}
