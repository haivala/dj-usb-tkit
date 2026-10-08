//! Audio-frame indexes for the ANLZ `PVBR` (MP3) and `PVB2` (FLAC) chunks.
//!
//! These parsers intentionally accept only files whose frame layout can be
//! reproduced deterministically. An unsupported or malformed file gets a
//! [`SeekIndexSkip`] reason instead of a guessed index. The rules are in
//! docs/WAVEFORMS.md ("Seek-index chunks").

use std::path::Path;

const PVBR_ENTRIES: usize = 400;
pub(super) const PVBR_PAYLOAD_LEN: usize = PVBR_ENTRIES * 4 + 4;
const PVB2_ENTRIES: usize = 400;
const MP3_SAMPLES_PER_FRAME: u32 = 1152;

/// Appended to every "seek data not added" log line: each one is a file
/// layout the parsers don't cover yet, worth a bug report.
pub(super) const SEEK_SKIP_REPORT_HINT: &str = "please report this file (format details, not \
     the audio) so its seek data can be supported";

/// The seek index a source audio file gets.
#[derive(Debug)]
pub(super) enum SeekIndex {
    /// The 1604-byte `PVBR` payload (400 offsets + total samples).
    Pvbr(Vec<u8>),
    Pvb2(Pvb2Chunk),
}

#[derive(Debug)]
pub(super) struct Pvb2Chunk {
    /// The 20 bytes after the common 12-byte chunk header.
    pub header: Vec<u8>,
    pub payload: Vec<u8>,
}

/// Why a file gets no seek index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SeekIndexSkip {
    Unreadable,
    BadId3v2,
    NoAudioFrames,
    UnsupportedMpegVersion,
    LostSync,
    TruncatedFrame,
    MixedSampleRate,
    ApeOrLyrics3Tag,
    VbriHeader,
    UnknownEncoder,
    XingCountMismatch,
    NotFlac,
    BadStreamInfo,
    UnknownTotalSamples,
    FlacFrameSequence,
    TooLarge,
}

impl SeekIndexSkip {
    pub(super) fn describe(self) -> &'static str {
        match self {
            Self::Unreadable => "audio file could not be read",
            Self::BadId3v2 => "malformed ID3v2 tag",
            Self::NoAudioFrames => "no MPEG audio frames",
            Self::UnsupportedMpegVersion => "MPEG-2/2.5 or not Layer III",
            Self::LostSync => "unexplained bytes between MPEG frames",
            Self::TruncatedFrame => "truncated last MPEG frame",
            Self::MixedSampleRate => "sample rate changes mid-file",
            Self::ApeOrLyrics3Tag => "APE or Lyrics3 tag",
            Self::VbriHeader => "VBRI header",
            Self::UnknownEncoder => "unknown encoder in Xing/Info header",
            Self::XingCountMismatch => "Xing/Info frame count disagrees with the file",
            Self::NotFlac => "no fLaC marker",
            Self::BadStreamInfo => "malformed FLAC metadata",
            Self::UnknownTotalSamples => "FLAC STREAMINFO has no sample count",
            Self::FlacFrameSequence => "FLAC frames are not a continuous sequence",
            Self::TooLarge => "file too large for the index",
        }
    }
}

/// The seek index for an MP3 or FLAC; `Ok(None)` for any other format,
/// which keeps the empty `PVBR` and gets no `PVB2`.
pub(super) fn seek_index_for_audio(path: &Path) -> Result<Option<SeekIndex>, SeekIndexSkip> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let read = || std::fs::read(path).map_err(|_| SeekIndexSkip::Unreadable);
    match extension.as_str() {
        "mp3" => parse_mp3_pvbr(&read()?).map(|payload| Some(SeekIndex::Pvbr(payload))),
        "flac" => parse_flac_pvb2(&read()?).map(|chunk| Some(SeekIndex::Pvb2(chunk))),
        _ => Ok(None),
    }
}

#[derive(Clone, Copy, Debug)]
struct Mp3Frame {
    offset: usize,
    length: usize,
    bitrate_kbps: u16,
    sample_rate: u32,
}

fn parse_mp3_pvbr(bytes: &[u8]) -> Result<Vec<u8>, SeekIndexSkip> {
    let start = id3v2_end(bytes)?;
    let end = id3v1_start(bytes);
    if start >= end {
        return Err(SeekIndexSkip::NoAudioFrames);
    }
    if has_ape_or_lyrics3_tag(&bytes[..end]) {
        return Err(SeekIndexSkip::ApeOrLyrics3Tag);
    }

    let mut frames = Vec::new();
    let mut offset = start;
    while offset < end {
        let Some(frame) = parse_mpeg1_layer3_frame(bytes, offset) else {
            // Encoders pad the tail with zeros or 0xaa; anything else is a
            // layout we can't reproduce.
            if bytes[offset..end]
                .iter()
                .all(|byte| *byte == 0 || *byte == 0xaa)
            {
                break;
            }
            return Err(if is_other_mpeg_frame(bytes, offset) {
                SeekIndexSkip::UnsupportedMpegVersion
            } else {
                SeekIndexSkip::LostSync
            });
        };
        if offset + frame.length > end {
            return Err(SeekIndexSkip::TruncatedFrame);
        }
        frames.push(frame);
        offset += frame.length;
    }
    let Some(&first) = frames.first() else {
        return Err(SeekIndexSkip::NoAudioFrames);
    };
    if frames
        .iter()
        .any(|frame| frame.sample_rate != first.sample_rate)
    {
        return Err(SeekIndexSkip::MixedSampleRate);
    }

    let header = mp3_header_frame(&bytes[first.offset..first.offset + first.length])?;
    let counted: &[Mp3Frame] = match header {
        Some(header) if header.skip => &frames[1..],
        _ => &frames,
    };
    if counted.is_empty() {
        return Err(SeekIndexSkip::NoAudioFrames);
    }
    if let Some(declared) = header.and_then(|header| header.declared_frames) {
        let counted_len = counted.len() as u64;
        // Xing/Info conventionally counts the audio frames after its own
        // header; rekordbox nevertheless counts a LAME/iTunes header frame.
        let header_counted = header.is_some_and(|header| !header.skip);
        let declared = u64::from(declared);
        if declared != counted_len && !(header_counted && declared + 1 == counted_len) {
            return Err(SeekIndexSkip::XingCountMismatch);
        }
    }

    // CBR vs VBR is decided on the audio frames: a header frame that is
    // counted is still not audio, and may carry its own bitrate.
    let audio = match header {
        Some(_) => &frames[1..],
        None => &frames[..],
    };
    let is_vbr = audio
        .iter()
        .any(|frame| frame.bitrate_kbps != audio[0].bitrate_kbps);

    let total_samples = u32::try_from(counted.len())
        .ok()
        .and_then(|count| count.checked_mul(MP3_SAMPLES_PER_FRAME))
        .ok_or(SeekIndexSkip::TooLarge)?;
    let base = counted[0].offset;
    let mut payload = vec![0u8; PVBR_PAYLOAD_LEN];
    if is_vbr {
        let count = counted.len();
        for i in 0..PVBR_ENTRIES {
            let frame_index = ((i + 1) * count / PVBR_ENTRIES).saturating_sub(8);
            let relative = counted[frame_index.min(count - 1)].offset - base;
            let relative = u32::try_from(relative).map_err(|_| SeekIndexSkip::TooLarge)?;
            payload[i * 4..i * 4 + 4].copy_from_slice(&relative.to_be_bytes());
        }
    }
    payload[PVBR_ENTRIES * 4..].copy_from_slice(&total_samples.to_be_bytes());
    Ok(payload)
}

fn id3v2_end(bytes: &[u8]) -> Result<usize, SeekIndexSkip> {
    if bytes.get(0..3) != Some(b"ID3") {
        return Ok(0);
    }
    let header = bytes.get(0..10).ok_or(SeekIndexSkip::BadId3v2)?;
    if header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return Err(SeekIndexSkip::BadId3v2);
    }
    let size = header[6..10]
        .iter()
        .fold(0usize, |value, byte| (value << 7) | usize::from(*byte));
    let footer = usize::from(header[5] & 0x10 != 0) * 10;
    Some(10 + size + footer)
        .filter(|end| *end <= bytes.len())
        .ok_or(SeekIndexSkip::BadId3v2)
}

fn id3v1_start(bytes: &[u8]) -> usize {
    let candidate = bytes.len().saturating_sub(128);
    if bytes.get(candidate..candidate + 3) == Some(b"TAG") {
        candidate
    } else {
        bytes.len()
    }
}

/// An APEv2 footer or a Lyrics3v2 end marker right before ID3v1 (or the end).
fn has_ape_or_lyrics3_tag(bytes: &[u8]) -> bool {
    let ends_with_at = |marker: &[u8], from_end: usize| {
        bytes.len() >= from_end && bytes[bytes.len() - from_end..].starts_with(marker)
    };
    ends_with_at(b"APETAGEX", 32) || ends_with_at(b"LYRICS200", 9)
}

/// An MPEG audio frame sync that isn't MPEG-1 Layer III.
fn is_other_mpeg_frame(bytes: &[u8], offset: usize) -> bool {
    bytes
        .get(offset..offset + 2)
        .is_some_and(|sync| sync[0] == 0xff && sync[1] & 0xe0 == 0xe0)
}

fn parse_mpeg1_layer3_frame(bytes: &[u8], offset: usize) -> Option<Mp3Frame> {
    let header = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
    if header & 0xffe0_0000 != 0xffe0_0000
        || (header >> 19) & 0x3 != 0x3
        || (header >> 17) & 0x3 != 0x1
    {
        return None;
    }
    const BITRATES: [u16; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 32_000, 0];
    let bitrate_kbps = BITRATES[((header >> 12) & 0xf) as usize];
    let sample_rate = SAMPLE_RATES[((header >> 10) & 0x3) as usize];
    if bitrate_kbps == 0 || sample_rate == 0 {
        return None;
    }
    let padding = ((header >> 9) & 1) as usize;
    let length = 144_000 * usize::from(bitrate_kbps) / sample_rate as usize + padding;
    Some(Mp3Frame {
        offset,
        length,
        bitrate_kbps,
        sample_rate,
    })
}

#[derive(Clone, Copy, Debug)]
struct XingHeader {
    /// rekordbox leaves this header frame out of the counted frames.
    skip: bool,
    declared_frames: Option<u32>,
}

/// The first frame's Xing/Info header, if it has one. Unknown encoder
/// strings and VBRI headers are rejected.
fn mp3_header_frame(frame: &[u8]) -> Result<Option<XingHeader>, SeekIndexSkip> {
    let truncated = SeekIndexSkip::TruncatedFrame;
    let read_u32 = |at: usize| -> Result<u32, SeekIndexSkip> {
        frame
            .get(at..at + 4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()))
            .ok_or(truncated)
    };
    let header = read_u32(0)?;
    let has_crc = ((header >> 16) & 1) == 0;
    let mono = ((header >> 6) & 0x3) == 0x3;
    let xing_offset = 4 + usize::from(has_crc) * 2 + if mono { 17 } else { 32 };

    // VBRI sits at a fixed 32 bytes after the frame header.
    if frame.get(36..40) == Some(b"VBRI") {
        return Err(SeekIndexSkip::VbriHeader);
    }
    let signature = frame.get(xing_offset..xing_offset + 4).ok_or(truncated)?;
    if signature != b"Xing" && signature != b"Info" {
        return Ok(None);
    }

    let flags = read_u32(xing_offset + 4)?;
    let mut cursor = xing_offset + 8;
    let declared_frames = if flags & 0x1 != 0 {
        cursor += 4;
        Some(read_u32(cursor - 4)?)
    } else {
        None
    };
    if flags & 0x2 != 0 {
        cursor += 4;
    }
    if flags & 0x4 != 0 {
        cursor += 100;
    }
    if flags & 0x8 != 0 {
        cursor += 4;
    }
    let encoder = frame.get(cursor..cursor + 9).ok_or(truncated)?;
    let skip = if encoder.starts_with(b"LAME") || encoder.starts_with(b"iTunes") {
        false
    } else if encoder.starts_with(b"Lavc") || encoder.starts_with(b"Lame") {
        true
    } else {
        return Err(SeekIndexSkip::UnknownEncoder);
    };
    Ok(Some(XingHeader {
        skip,
        declared_frames,
    }))
}

#[derive(Clone, Copy, Debug)]
struct FlacFrame {
    first_sample: u64,
    offset: usize,
    block_size: u32,
}

#[derive(Clone, Copy, Debug)]
struct FlacFrameHeader {
    blocking_strategy: bool,
    number: u64,
    block_size: u32,
}

fn parse_flac_pvb2(bytes: &[u8]) -> Result<Pvb2Chunk, SeekIndexSkip> {
    let marker = id3v2_end(bytes)?;
    if bytes.get(marker..marker + 4) != Some(b"fLaC") {
        return Err(SeekIndexSkip::NotFlac);
    }
    let mut cursor = marker + 4;
    let mut total_samples = None;
    loop {
        let block_header = bytes
            .get(cursor..cursor + 4)
            .ok_or(SeekIndexSkip::BadStreamInfo)?;
        let is_last = block_header[0] & 0x80 != 0;
        let block_type = block_header[0] & 0x7f;
        let length = (usize::from(block_header[1]) << 16)
            | (usize::from(block_header[2]) << 8)
            | usize::from(block_header[3]);
        let data_start = cursor + 4;
        let block = bytes
            .get(data_start..data_start + length)
            .ok_or(SeekIndexSkip::BadStreamInfo)?;
        if block_type == 0 {
            if block.len() != 34 {
                return Err(SeekIndexSkip::BadStreamInfo);
            }
            let packed = u64::from_be_bytes(block[10..18].try_into().unwrap());
            total_samples = Some(packed & 0x0f_ffff_ffff);
        }
        cursor = data_start + length;
        if is_last {
            break;
        }
    }
    let total_samples = total_samples.ok_or(SeekIndexSkip::BadStreamInfo)?;
    if total_samples == 0 {
        return Err(SeekIndexSkip::UnknownTotalSamples);
    }
    let first_audio_offset = cursor;
    let first_header =
        parse_flac_frame_header(bytes, cursor).ok_or(SeekIndexSkip::FlacFrameSequence)?;
    if first_header.number != 0 {
        return Err(SeekIndexSkip::FlacFrameSequence);
    }
    let strategy = first_header.blocking_strategy;
    let nominal_block_size = first_header.block_size;
    let mut frames = vec![FlacFrame {
        first_sample: 0,
        offset: cursor,
        block_size: first_header.block_size,
    }];
    let mut search = cursor + 2;

    // FLAC frames carry no length, so the next frame is the next header that
    // passes its CRC-8 and continues the frame/sample sequence.
    while let Some(relative) = bytes[search.min(bytes.len())..]
        .iter()
        .position(|byte| *byte == 0xff)
    {
        let candidate = search + relative;
        search = candidate + 1;
        let Some(header) = parse_flac_frame_header(bytes, candidate) else {
            continue;
        };
        let last = frames[frames.len() - 1];
        let expected_first = last.first_sample + u64::from(last.block_size);
        let sequence_matches = header.blocking_strategy == strategy
            && if strategy {
                header.number == expected_first
            } else {
                header.number == frames.len() as u64
                    && expected_first == (frames.len() as u64) * u64::from(nominal_block_size)
            };
        if !sequence_matches {
            continue;
        }
        frames.push(FlacFrame {
            first_sample: expected_first,
            offset: candidate,
            block_size: header.block_size,
        });
        search = candidate + 2;
    }
    let last = frames[frames.len() - 1];
    if last.first_sample + u64::from(last.block_size) != total_samples {
        return Err(SeekIndexSkip::FlacFrameSequence);
    }

    let selected: Vec<FlacFrame> = if frames.len() <= PVB2_ENTRIES {
        frames[..frames.len() - 1].to_vec()
    } else {
        (0..PVB2_ENTRIES)
            .map(|i| {
                let target = (i as u64) * total_samples / PVB2_ENTRIES as u64;
                let index = frames
                    .partition_point(|frame| frame.first_sample <= target)
                    .saturating_sub(1);
                frames[index]
            })
            .collect()
    };

    let entry_count = u32::try_from(selected.len()).map_err(|_| SeekIndexSkip::TooLarge)?;
    let mut header = Vec::with_capacity(20);
    header.extend_from_slice(&0u32.to_be_bytes());
    header.extend_from_slice(&total_samples.to_be_bytes());
    header.extend_from_slice(&entry_count.to_be_bytes());
    header.extend_from_slice(&20u32.to_be_bytes());
    let mut payload = Vec::with_capacity(selected.len() * 20);
    for frame in selected {
        payload.extend_from_slice(&frame.first_sample.to_be_bytes());
        payload.extend_from_slice(&((frame.offset - first_audio_offset) as u64).to_be_bytes());
        payload.extend_from_slice(&frame.block_size.to_be_bytes());
    }
    Ok(Pvb2Chunk { header, payload })
}

fn parse_flac_frame_header(bytes: &[u8], offset: usize) -> Option<FlacFrameHeader> {
    let fixed = bytes.get(offset..offset + 4)?;
    if fixed[0] != 0xff || fixed[1] & 0xfe != 0xf8 || fixed[3] & 0x01 != 0 {
        return None;
    }
    let blocking_strategy = fixed[1] & 1 != 0;
    let block_size_code = fixed[2] >> 4;
    let sample_rate_code = fixed[2] & 0x0f;
    let channel_assignment = fixed[3] >> 4;
    let sample_size_code = (fixed[3] >> 1) & 0x07;
    if block_size_code == 0
        || sample_rate_code == 0x0f
        || channel_assignment > 10
        || matches!(sample_size_code, 3 | 7)
    {
        return None;
    }
    let (number, number_len) = decode_utf8_uint(bytes.get(offset + 4..)?)?;
    let mut cursor = offset + 4 + number_len;
    let block_size = match block_size_code {
        1 => 192,
        2..=5 => 576u32 << (block_size_code - 2),
        6 => {
            let value = u32::from(*bytes.get(cursor)?);
            cursor += 1;
            value + 1
        }
        7 => {
            let value = u32::from(u16::from_be_bytes(
                bytes.get(cursor..cursor + 2)?.try_into().ok()?,
            ));
            cursor += 2;
            value + 1
        }
        _ => 256u32 << (block_size_code - 8),
    };
    cursor += match sample_rate_code {
        12 => 1,
        13 | 14 => 2,
        _ => 0,
    };
    let stored_crc = *bytes.get(cursor)?;
    if flac_crc8(bytes.get(offset..cursor)?) != stored_crc {
        return None;
    }
    Some(FlacFrameHeader {
        blocking_strategy,
        number,
        block_size,
    })
}

fn decode_utf8_uint(bytes: &[u8]) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    let length: usize = match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        0xf8..=0xfb => 5,
        0xfc..=0xfd => 6,
        0xfe => 7,
        _ => return None,
    };
    let mut value = u64::from(first & (0x7f >> (length - 1)));
    for byte in bytes.get(1..length)? {
        if byte & 0xc0 != 0x80 {
            return None;
        }
        value = (value << 6) | u64::from(byte & 0x3f);
    }
    Some((value, length))
}

fn flac_crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    // MPEG-1 Layer III, 44.1 kHz, no padding, joint stereo, no CRC.
    const KBPS_128: u8 = 0x90;
    const KBPS_320: u8 = 0xe0;

    fn mp3_frame(bitrate_byte: u8) -> Vec<u8> {
        let header = [0xff, 0xfb, bitrate_byte, 0x64];
        let frame = parse_mpeg1_layer3_frame(&header, 0).expect("valid test header");
        let mut bytes = vec![0u8; frame.length];
        bytes[..4].copy_from_slice(&header);
        bytes
    }

    /// A 320 kbps header frame with a Xing/Info tag at the stereo offset.
    fn xing_frame(signature: &[u8; 4], declared_frames: u32, encoder: &[u8]) -> Vec<u8> {
        let mut frame = mp3_frame(KBPS_320);
        frame[36..40].copy_from_slice(signature);
        frame[40..44].copy_from_slice(&1u32.to_be_bytes());
        frame[44..48].copy_from_slice(&declared_frames.to_be_bytes());
        frame[48..48 + encoder.len()].copy_from_slice(encoder);
        frame
    }

    fn file(frames: &[Vec<u8>]) -> Vec<u8> {
        frames.concat()
    }

    fn total(payload: &[u8]) -> u32 {
        u32::from_be_bytes(payload[PVBR_ENTRIES * 4..].try_into().unwrap())
    }

    fn entry(payload: &[u8], i: usize) -> u32 {
        u32::from_be_bytes(payload[i * 4..i * 4 + 4].try_into().unwrap())
    }

    #[test]
    fn cbr_mp3_has_zero_offsets_and_counted_sample_total() {
        let payload = parse_mp3_pvbr(&file(&vec![mp3_frame(KBPS_320); 3])).unwrap();
        assert_eq!(&payload[..PVBR_ENTRIES * 4], &[0; PVBR_ENTRIES * 4]);
        assert_eq!(total(&payload), 3 * 1152);
    }

    #[test]
    fn vbr_entries_follow_the_minus_eight_frame_rule() {
        let frames: Vec<Vec<u8>> = (0..1_000)
            .map(|i| mp3_frame(if i % 3 == 0 { KBPS_128 } else { KBPS_320 }))
            .collect();
        let offsets: Vec<u32> = frames
            .iter()
            .scan(0u32, |offset, frame| {
                let start = *offset;
                *offset += frame.len() as u32;
                Some(start)
            })
            .collect();
        let payload = parse_mp3_pvbr(&file(&frames)).unwrap();
        for i in 0..PVBR_ENTRIES {
            let frame_index = ((i + 1) * 1_000 / PVBR_ENTRIES).saturating_sub(8);
            assert_eq!(entry(&payload, i), offsets[frame_index], "entry {i}");
        }
        assert_eq!(total(&payload), 1_000 * 1152);
    }

    #[test]
    fn lame_header_frame_is_counted_but_not_used_for_cbr_detection() {
        // The Info frame runs at 320 kbps over 128 kbps CBR audio.
        let mut frames = vec![xing_frame(b"Info", 4, b"LAME3.100")];
        frames.extend(vec![mp3_frame(KBPS_128); 4]);
        let payload = parse_mp3_pvbr(&file(&frames)).unwrap();
        assert_eq!(&payload[..PVBR_ENTRIES * 4], &[0; PVBR_ENTRIES * 4]);
        assert_eq!(total(&payload), 5 * 1152);
    }

    #[test]
    fn lavc_header_frame_is_not_counted() {
        let mut frames = vec![xing_frame(b"Info", 4, b"Lavc61.19")];
        frames.extend(vec![mp3_frame(KBPS_128); 4]);
        assert_eq!(total(&parse_mp3_pvbr(&file(&frames)).unwrap()), 4 * 1152);
    }

    #[test]
    fn tags_and_tail_fill_are_accepted() {
        let mut bytes = b"ID3\x04\x00\x00\x00\x00\x00\x05".to_vec();
        bytes.extend_from_slice(&[0; 5]);
        bytes.extend(file(&vec![mp3_frame(KBPS_320); 2]));
        bytes.extend_from_slice(&[0xaa; 40]);
        let mut id3v1 = vec![0u8; 128];
        id3v1[..3].copy_from_slice(b"TAG");
        bytes.extend(id3v1);
        assert_eq!(total(&parse_mp3_pvbr(&bytes).unwrap()), 2 * 1152);
    }

    #[test]
    fn unreproducible_mp3_layouts_are_skipped_with_a_reason() {
        let cbr = || vec![mp3_frame(KBPS_320); 3];

        let mut truncated = file(&cbr());
        truncated.truncate(truncated.len() - 10);
        truncated.extend_from_slice(&[0xff; 4]);
        assert_eq!(
            parse_mp3_pvbr(&truncated),
            Err(SeekIndexSkip::TruncatedFrame)
        );

        let mut junk = cbr();
        junk.insert(1, vec![0x12]);
        assert_eq!(parse_mp3_pvbr(&file(&junk)), Err(SeekIndexSkip::LostSync));

        let mut mpeg2 = mp3_frame(KBPS_320);
        mpeg2[1] = 0xf3;
        assert_eq!(
            parse_mp3_pvbr(&mpeg2),
            Err(SeekIndexSkip::UnsupportedMpegVersion)
        );

        let mut unknown = vec![xing_frame(b"Xing", 3, b"GOGO-2.39")];
        unknown.extend(cbr());
        assert_eq!(
            parse_mp3_pvbr(&file(&unknown)),
            Err(SeekIndexSkip::UnknownEncoder)
        );

        let mut mismatch = vec![xing_frame(b"Info", 9, b"LAME3.100")];
        mismatch.extend(cbr());
        assert_eq!(
            parse_mp3_pvbr(&file(&mismatch)),
            Err(SeekIndexSkip::XingCountMismatch)
        );

        let mut vbri = cbr();
        vbri[0][36..40].copy_from_slice(b"VBRI");
        assert_eq!(parse_mp3_pvbr(&file(&vbri)), Err(SeekIndexSkip::VbriHeader));

        let mut ape = file(&cbr());
        let mut footer = vec![0u8; 32];
        footer[..8].copy_from_slice(b"APETAGEX");
        ape.extend(footer);
        assert_eq!(parse_mp3_pvbr(&ape), Err(SeekIndexSkip::ApeOrLyrics3Tag));
    }

    fn flac_fixture() -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio/formats/track_format_flac.flac"),
        )
        .unwrap()
    }

    #[test]
    fn flac_fixture_produces_consistent_pvb2_entries() {
        let chunk = parse_flac_pvb2(&flac_fixture()).expect("parse FLAC");
        assert_eq!(chunk.header.len(), 20);
        let count = u32::from_be_bytes(chunk.header[12..16].try_into().unwrap()) as usize;
        assert_eq!(chunk.payload.len(), count * 20);
        assert_eq!(
            u64::from_be_bytes(chunk.header[4..12].try_into().unwrap()),
            88_200
        );
        assert_eq!(count, 19);
        assert_eq!(&chunk.payload[0..16], &[0; 16]);
        let absolute_offsets = [
            8_286u64, 9_531, 10_783, 12_047, 13_303, 14_566, 15_839, 17_109, 18_359, 19_613,
            20_868, 22_129, 23_384, 24_640, 25_903, 27_157, 28_423, 29_680, 30_921,
        ];
        for (entry, expected_absolute) in chunk.payload.chunks_exact(20).zip(absolute_offsets) {
            let relative = u64::from_be_bytes(entry[8..16].try_into().unwrap());
            assert_eq!(relative, expected_absolute - absolute_offsets[0]);
        }
        for pair in chunk
            .payload
            .chunks_exact(20)
            .collect::<Vec<_>>()
            .windows(2)
        {
            let first_sample = u64::from_be_bytes(pair[0][0..8].try_into().unwrap());
            let next_sample = u64::from_be_bytes(pair[1][0..8].try_into().unwrap());
            let block_size = u32::from_be_bytes(pair[0][16..20].try_into().unwrap());
            assert_eq!(next_sample, first_sample + u64::from(block_size));
        }
    }

    #[test]
    fn damaged_flac_gets_no_index() {
        // A broken CRC on the second frame's header breaks the sequence.
        let mut bad_frame = flac_fixture();
        bad_frame[9_531 + 2] ^= 0x01;
        assert_eq!(
            parse_flac_pvb2(&bad_frame).map(|_| ()),
            Err(SeekIndexSkip::FlacFrameSequence)
        );

        // STREAMINFO's total samples end at file offset 25 (no ID3v2 here).
        let mut wrong_total = flac_fixture();
        assert_eq!(&wrong_total[0..4], b"fLaC");
        wrong_total[25] ^= 0x01;
        assert_eq!(
            parse_flac_pvb2(&wrong_total).map(|_| ()),
            Err(SeekIndexSkip::FlacFrameSequence)
        );
    }

    #[test]
    fn ffmpeg_vbr_fixture_produces_offsets_and_skips_its_header_frame() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/embedded/track_embedded.mp3");
        let payload = parse_mp3_pvbr(&std::fs::read(path).unwrap()).expect("parse VBR MP3");
        assert!(payload[..PVBR_ENTRIES * 4].iter().any(|byte| *byte != 0));
        assert!(total(&payload) > 0);
        assert_eq!(total(&payload) % 1152, 0);
    }
}
