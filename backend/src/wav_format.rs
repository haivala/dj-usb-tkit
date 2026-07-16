//! Minimal RIFF/WAVE `fmt ` chunk parsing.
//!
//! Some Pioneer CDJs reject WAV files whose `fmt ` chunk uses the
//! `WAVE_FORMAT_EXTENSIBLE` tag (0xFFFE), even when the wrapped audio is
//! plain PCM within spec. `lofty`'s decoder-facing properties don't expose
//! the raw format tag, so this module parses just enough of the RIFF
//! structure to detect the issue and, where safe, rewrite the header to a
//! standard PCM/IEEE-float `fmt ` chunk without touching sample data.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{BackendError, BackendResult};

const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavFmtInfo {
    pub format_tag: u16,
    pub sub_format_tag: Option<u16>,
    pub channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    pub fmt_chunk_offset: u64,
    pub fmt_chunk_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormatIssue {
    /// WAVE_FORMAT_EXTENSIBLE wrapping plain PCM or IEEE float data - safe to
    /// losslessly rewrite to a standard `fmt ` chunk.
    ExtensiblePcm,
    /// WAVE_FORMAT_EXTENSIBLE wrapping some other subformat - not safe to
    /// rewrite automatically.
    ExtensibleOther,
}

impl WavFormatIssue {
    pub fn as_db_str(self) -> &'static str {
        match self {
            WavFormatIssue::ExtensiblePcm => "extensible_pcm",
            WavFormatIssue::ExtensibleOther => "extensible_other",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "extensible_pcm" => Some(WavFormatIssue::ExtensiblePcm),
            "extensible_other" => Some(WavFormatIssue::ExtensibleOther),
            _ => None,
        }
    }
}

/// Walks RIFF chunks looking for `fmt `, reading only the bytes needed
/// (never the `data` chunk's audio payload).
pub fn parse_wav_fmt(path: &Path) -> io::Result<Option<WavFmtInfo>> {
    let mut reader = BufReader::new(File::open(path)?);

    let mut riff_header = [0u8; 12];
    if reader.read_exact(&mut riff_header).is_err() {
        return Ok(None);
    }
    if &riff_header[0..4] != b"RIFF" || &riff_header[8..12] != b"WAVE" {
        return Ok(None);
    }

    let mut offset: u64 = 12;
    loop {
        let mut chunk_header = [0u8; 8];
        if reader.read_exact(&mut chunk_header).is_err() {
            return Ok(None);
        }
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_le_bytes(chunk_header[4..8].try_into().unwrap());

        if chunk_id == b"fmt " {
            let read_len = (chunk_size as usize).min(40);
            let mut body = vec![0u8; read_len];
            if reader.read_exact(&mut body).is_err() || body.len() < 16 {
                return Ok(None);
            }
            let format_tag = u16::from_le_bytes(body[0..2].try_into().unwrap());
            let channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let byte_rate = u32::from_le_bytes(body[8..12].try_into().unwrap());
            let block_align = u16::from_le_bytes(body[12..14].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(body[14..16].try_into().unwrap());
            let sub_format_tag = if format_tag == WAVE_FORMAT_EXTENSIBLE && body.len() >= 26 {
                Some(u16::from_le_bytes(body[24..26].try_into().unwrap()))
            } else {
                None
            };

            return Ok(Some(WavFmtInfo {
                format_tag,
                sub_format_tag,
                channels,
                sample_rate,
                byte_rate,
                block_align,
                bits_per_sample,
                fmt_chunk_offset: offset,
                fmt_chunk_size: chunk_size,
            }));
        }

        let padded_size = u64::from(chunk_size) + (chunk_size % 2) as u64;
        if reader.seek_relative(padded_size as i64).is_err() {
            return Ok(None);
        }
        offset += 8 + padded_size;
    }
}

pub fn classify(info: &WavFmtInfo) -> Option<WavFormatIssue> {
    if info.format_tag != WAVE_FORMAT_EXTENSIBLE {
        return None;
    }
    match info.sub_format_tag {
        Some(WAVE_FORMAT_PCM) | Some(WAVE_FORMAT_IEEE_FLOAT) => Some(WavFormatIssue::ExtensiblePcm),
        _ => Some(WavFormatIssue::ExtensibleOther),
    }
}

/// Detects a WAVE_FORMAT_EXTENSIBLE issue for the given path, or `None` if
/// the file isn't a parseable WAV or its `fmt ` chunk isn't extensible.
pub fn detect_wav_format_issue(path: &Path) -> Option<WavFormatIssue> {
    let info = parse_wav_fmt(path).ok().flatten()?;
    classify(&info)
}

/// Rewrites an extensible-PCM WAV at `source` to a standard-PCM/IEEE-float
/// WAV at `target`. Only the `fmt ` chunk is replaced - every other byte
/// (including sample data) is streamed through verbatim, so this is a
/// lossless, header-only fix. Returns an error if `source` doesn't classify
/// as `WavFormatIssue::ExtensiblePcm`.
pub fn rewrite_extensible_to_pcm(source: &Path, target: &Path) -> BackendResult<()> {
    let info = parse_wav_fmt(source)?.ok_or_else(|| {
        BackendError::Internal(format!(
            "rewrite_extensible_to_pcm: no fmt chunk found in {}",
            source.display()
        ))
    })?;

    let sub_format_tag = match classify(&info) {
        Some(WavFormatIssue::ExtensiblePcm) => info.sub_format_tag.unwrap_or(WAVE_FORMAT_PCM),
        _ => {
            return Err(BackendError::Internal(format!(
                "rewrite_extensible_to_pcm: {} is not a safely-convertible extensible WAV",
                source.display()
            )));
        }
    };

    let mut src = File::open(source)?;

    let mut new_fmt_body = Vec::with_capacity(16);
    new_fmt_body.extend_from_slice(&sub_format_tag.to_le_bytes());
    new_fmt_body.extend_from_slice(&info.channels.to_le_bytes());
    new_fmt_body.extend_from_slice(&info.sample_rate.to_le_bytes());
    new_fmt_body.extend_from_slice(&info.byte_rate.to_le_bytes());
    new_fmt_body.extend_from_slice(&info.block_align.to_le_bytes());
    new_fmt_body.extend_from_slice(&info.bits_per_sample.to_le_bytes());

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = File::create(target)?;

    // 12-byte RIFF/WAVE header; the size field is a placeholder, patched below.
    out.write_all(b"RIFF\0\0\0\0WAVE")?;

    out.write_all(b"fmt ")?;
    out.write_all(&(new_fmt_body.len() as u32).to_le_bytes())?;
    out.write_all(&new_fmt_body)?;

    // Stream every remaining chunk after the old fmt chunk, verbatim.
    let after_fmt_offset = info.fmt_chunk_offset
        + 8
        + u64::from(info.fmt_chunk_size)
        + (info.fmt_chunk_size % 2) as u64;
    src.seek(SeekFrom::Start(after_fmt_offset))?;
    io::copy(&mut src, &mut out)?;

    let total_len = out.stream_position()?;
    let riff_size = (total_len - 8) as u32;
    out.seek(SeekFrom::Start(4))?;
    out.write_all(&riff_size.to_le_bytes())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn le16(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }
    fn le32(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    fn pcm_subformat_guid(sub_format_tag: u16) -> [u8; 16] {
        let mut guid = [0u8; 16];
        guid[0..2].copy_from_slice(&le16(sub_format_tag));
        guid[8..16].copy_from_slice(&[0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);
        guid
    }

    fn make_wav(fmt_body: &[u8], extra_chunks: &[(&[u8; 4], &[u8])], data: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&le32(fmt_body.len() as u32));
        body.extend_from_slice(fmt_body);
        if fmt_body.len() % 2 == 1 {
            body.push(0);
        }
        for (id, chunk_data) in extra_chunks {
            body.extend_from_slice(*id);
            body.extend_from_slice(&le32(chunk_data.len() as u32));
            body.extend_from_slice(chunk_data);
            if chunk_data.len() % 2 == 1 {
                body.push(0);
            }
        }
        body.extend_from_slice(b"data");
        body.extend_from_slice(&le32(data.len() as u32));
        body.extend_from_slice(data);
        if data.len() % 2 == 1 {
            body.push(0);
        }

        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&le32(body.len() as u32));
        out.extend_from_slice(&body);
        out
    }

    fn make_plain_pcm_fmt() -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&le16(WAVE_FORMAT_PCM));
        fmt.extend_from_slice(&le16(1)); // mono
        fmt.extend_from_slice(&le32(44_100));
        fmt.extend_from_slice(&le32(44_100 * 2));
        fmt.extend_from_slice(&le16(2));
        fmt.extend_from_slice(&le16(16));
        fmt
    }

    fn make_extensible_fmt(sub_format_tag: u16) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&le16(WAVE_FORMAT_EXTENSIBLE));
        fmt.extend_from_slice(&le16(1)); // mono
        fmt.extend_from_slice(&le32(44_100));
        fmt.extend_from_slice(&le32(44_100 * 2));
        fmt.extend_from_slice(&le16(2));
        fmt.extend_from_slice(&le16(16));
        fmt.extend_from_slice(&le16(22)); // cbSize
        fmt.extend_from_slice(&le16(16)); // validBitsPerSample
        fmt.extend_from_slice(&le32(0x4)); // channel mask (front center)
        fmt.extend_from_slice(&pcm_subformat_guid(sub_format_tag));
        fmt
    }

    #[test]
    fn classify_none_for_plain_pcm() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plain.wav");
        std::fs::write(&path, make_wav(&make_plain_pcm_fmt(), &[], &[0u8; 20])).unwrap();
        assert_eq!(detect_wav_format_issue(&path), None);
    }

    #[test]
    fn classify_extensible_pcm() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ext_pcm.wav");
        std::fs::write(
            &path,
            make_wav(&make_extensible_fmt(WAVE_FORMAT_PCM), &[], &[0u8; 20]),
        )
        .unwrap();
        assert_eq!(
            detect_wav_format_issue(&path),
            Some(WavFormatIssue::ExtensiblePcm)
        );
    }

    #[test]
    fn classify_extensible_float() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ext_float.wav");
        std::fs::write(
            &path,
            make_wav(
                &make_extensible_fmt(WAVE_FORMAT_IEEE_FLOAT),
                &[],
                &[0u8; 20],
            ),
        )
        .unwrap();
        assert_eq!(
            detect_wav_format_issue(&path),
            Some(WavFormatIssue::ExtensiblePcm)
        );
    }

    #[test]
    fn classify_extensible_other() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ext_other.wav");
        std::fs::write(
            &path,
            make_wav(&make_extensible_fmt(0x0006), &[], &[0u8; 20]),
        )
        .unwrap();
        assert_eq!(
            detect_wav_format_issue(&path),
            Some(WavFormatIssue::ExtensibleOther)
        );
    }

    #[test]
    fn classify_none_for_non_wav() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("not_a_wav.txt");
        std::fs::write(&path, b"hello world").unwrap();
        assert_eq!(detect_wav_format_issue(&path), None);
    }

    #[test]
    fn rewrite_produces_standard_pcm_header_and_preserves_bytes() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.wav");
        let data = (0u8..40).collect::<Vec<_>>();
        std::fs::write(
            &source,
            make_wav(
                &make_extensible_fmt(WAVE_FORMAT_PCM),
                &[(b"LIST", b"some list chunk data")],
                &data,
            ),
        )
        .unwrap();

        let target = dir.path().join("target.wav");
        rewrite_extensible_to_pcm(&source, &target).unwrap();

        let info = parse_wav_fmt(&target).unwrap().unwrap();
        assert_eq!(info.format_tag, WAVE_FORMAT_PCM);
        assert_eq!(info.fmt_chunk_size, 16);
        assert_eq!(info.channels, 1);
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(classify(&info), None);

        let out_bytes = std::fs::read(&target).unwrap();
        // RIFF size field must match the actual remaining length.
        let riff_size = u32::from_le_bytes(out_bytes[4..8].try_into().unwrap());
        assert_eq!(riff_size as usize, out_bytes.len() - 8);
        // LIST chunk and data must be preserved verbatim.
        assert!(out_bytes.windows(4).any(|w| w == b"LIST"));
        assert!(out_bytes.ends_with(&data));
    }

    #[test]
    fn rewrite_rejects_extensible_other() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.wav");
        std::fs::write(
            &source,
            make_wav(&make_extensible_fmt(0x0006), &[], &[0u8; 20]),
        )
        .unwrap();
        let target = dir.path().join("target.wav");
        let err = rewrite_extensible_to_pcm(&source, &target).expect_err("should reject");
        assert!(err.to_string().contains("not a safely-convertible"));
    }

    #[test]
    fn db_str_round_trip() {
        assert_eq!(
            WavFormatIssue::from_db_str(WavFormatIssue::ExtensiblePcm.as_db_str()),
            Some(WavFormatIssue::ExtensiblePcm)
        );
        assert_eq!(
            WavFormatIssue::from_db_str(WavFormatIssue::ExtensibleOther.as_db_str()),
            Some(WavFormatIssue::ExtensibleOther)
        );
        assert_eq!(WavFormatIssue::from_db_str("bogus"), None);
    }
}
