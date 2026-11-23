//! Reading ANLZ analysis files: chunk payloads and the waveform previews
//! the track tables draw, from the library's or the stick's files.

use crate::service::WAVEFORM_PREVIEW_BINS;
use std::path::{Path, PathBuf};

pub(crate) fn load_waveform_preview_from_analysis_path(path: &Path) -> Option<Vec<u8>> {
    for candidate in waveform_preview_candidates(path) {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        if let Some(peaks) = extract_waveform_preview_from_anlz_bytes(&bytes, WAVEFORM_PREVIEW_BINS)
            && !peaks.is_empty()
        {
            return Some(peaks);
        }
    }
    None
}

/// The ANLZ files `load_waveform_preview_from_analysis_path` tries, in order.
pub(crate) fn waveform_preview_candidates(path: &Path) -> Vec<PathBuf> {
    let base = path.to_path_buf();
    let mut candidates = Vec::<PathBuf>::new();
    let ext = base
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();

    if ext == "EXT" {
        candidates.push(base.clone());
        candidates.push(base.with_extension("2EX"));
        candidates.push(base.with_extension("DAT"));
    } else if ext == "2EX" {
        candidates.push(base.clone());
        candidates.push(base.with_extension("EXT"));
        candidates.push(base.with_extension("DAT"));
    } else if ext == "DAT" {
        candidates.push(base.with_extension("EXT"));
        candidates.push(base.with_extension("2EX"));
        candidates.push(base.clone());
    } else {
        candidates.push(base.with_extension("EXT"));
        candidates.push(base.with_extension("2EX"));
        candidates.push(base.with_extension("DAT"));
        candidates.push(base.clone());
    }
    candidates
}

/// Extract the raw PWV4 payload from a desktop library ANLZ `.EXT` file (no conversion).
/// Returns the 1200 × 6 byte payload, or None if the file/chunk is absent.
pub(crate) fn read_pwv4_from_anlz(dat_path: &Path) -> Option<Vec<u8>> {
    read_ext_chunk(dat_path, "PWV4")
}

/// Extract the raw PWV5 colour-detail waveform payload from a desktop library
/// ANLZ `.EXT` file (no conversion). 2 bytes/entry, BE u16:
/// `R(3) | G(3) | B(3) | Height(5) | _(2)`. Entry count is duration-derived
/// (`ceil(duration_s * 150) + 4`), so this can be tens of thousands of entries.
pub(crate) fn read_pwv5_from_anlz(dat_path: &Path) -> Option<Vec<u8>> {
    read_ext_chunk(dat_path, "PWV5").filter(|p| p.len() >= 4)
}

/// The raw `tag` chunk payload from the `.EXT` sibling of `dat_path` (or
/// `dat_path` itself when it already is the `.EXT`).
fn read_ext_chunk(dat_path: &Path, tag: &str) -> Option<Vec<u8>> {
    let ext_path = if dat_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ext"))
    {
        dat_path.to_path_buf()
    } else {
        dat_path.with_extension("EXT")
    };
    ext_chunk_from_bytes(&std::fs::read(&ext_path).ok()?, tag)
}

/// The raw `tag` chunk payload from an ANLZ file's bytes.
fn ext_chunk_from_bytes(bytes: &[u8], tag: &str) -> Option<Vec<u8>> {
    let container = find_anlz_chunk_payload(bytes, "PMAI").unwrap_or(bytes);
    find_anlz_chunk_payload(container, tag).map(<[u8]>::to_vec)
}

/// [`read_pwv5_from_anlz`] from `.EXT` bytes already read.
pub(crate) fn pwv5_from_ext_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    ext_chunk_from_bytes(bytes, "PWV5").filter(|p| p.len() >= 4)
}

pub(crate) fn extract_waveform_preview_from_anlz_bytes(
    bytes: &[u8],
    bins: usize,
) -> Option<Vec<u8>> {
    if bytes.len() < 16 {
        return None;
    }

    let container = find_anlz_chunk_payload(bytes, "PMAI").unwrap_or(bytes);
    // Prefer single-byte-per-level formats (PWV3, PWAV, PWV2) because
    // downsample_waveform_payload extracts amplitude via `b & 0x1F` per byte.
    // Multi-byte formats (PWV5=2B, PWV4=6B per level) need dedicated decoders
    // and would produce garbage with byte-level extraction.
    let preferred_tags = ["PWV3", "PWAV", "PWV2"];
    for tag in preferred_tags {
        if let Some(payload) = find_anlz_chunk_payload(container, tag) {
            let peaks = downsample_waveform_payload(payload, bins.max(1));
            if !peaks.is_empty() {
                return Some(peaks);
            }
        }
    }
    None
}

pub(crate) fn find_anlz_chunk_payload<'a>(bytes: &'a [u8], wanted_tag: &str) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + 12 <= bytes.len() {
        let tag_bytes = &bytes[offset..offset + 4];
        let tag_ok = tag_bytes
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !tag_ok {
            offset += 1;
            continue;
        }

        let Ok(tag) = std::str::from_utf8(tag_bytes) else {
            offset += 1;
            continue;
        };

        let header_len = u32::from_be_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let total_len = u32::from_be_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]) as usize;

        if header_len < 12 || total_len < header_len || offset + total_len > bytes.len() {
            offset += 1;
            continue;
        }

        if tag == wanted_tag {
            let start = offset + header_len;
            let end = offset + total_len;
            if start < end {
                return Some(&bytes[start..end]);
            }
        }

        offset += total_len;
    }
    None
}

pub(crate) fn downsample_waveform_payload(payload: &[u8], bins: usize) -> Vec<u8> {
    if payload.is_empty() || bins == 0 {
        return Vec::new();
    }

    let mut peaks = Vec::with_capacity(bins);
    let n = payload.len();
    for i in 0..bins {
        let start = i * n / bins;
        let mut end = ((i + 1) * n / bins).max(start + 1);
        if end > n {
            end = n;
        }
        let mut peak = 0u8;
        let mut sum = 0u32;
        let mut count = 0u32;
        for &b in &payload[start..end] {
            // Extract amplitude from lower 5 bits.  PWAV packs as ((7<<5)|v),
            // PWV3 as ((6<<5)|v), PWV2 stores raw 0-31 — all use b & 0x1F.
            let v = b & 0x1F;
            if v > peak {
                peak = v;
            }
            sum += u32::from(v);
            count += 1;
        }
        let avg = if count > 0 {
            sum as f32 / count as f32
        } else {
            0.0
        };
        // Blend average energy with peak to avoid a "brick wall" look.
        let level = (avg * 0.75) + (f32::from(peak) * 0.25);
        let percent = ((level * 100.0) / 31.0).round().clamp(0.0, 100.0) as u8;
        peaks.push(percent);
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pwav_encode_decode_roundtrip_preserves_values() {
        // Simulate PWAV encoding with band=7 (legacy) — decoder extracts low 5 bits regardless of band
        let original_peaks: Vec<u8> = vec![0, 25, 50, 75, 100];
        let levels: Vec<u8> = original_peaks
            .iter()
            .map(|&v| ((u16::from(v) * 31) / 100) as u8)
            .collect();
        let encoded: Vec<u8> = levels.iter().map(|&v| (7u8 << 5) | (v & 0x1F)).collect();

        // Decode with downsample_waveform_payload (same bin count = no resampling)
        let decoded = downsample_waveform_payload(&encoded, encoded.len());

        // Each decoded value should be the 5-bit level scaled back to 0-100
        for (i, &dec) in decoded.iter().enumerate() {
            let expected_level = levels[i];
            let expected_percent = ((f32::from(expected_level) * 100.0) / 31.0)
                .round()
                .clamp(0.0, 100.0) as u8;
            assert_eq!(
                dec, expected_percent,
                "PWAV roundtrip mismatch at index {i}: level={expected_level}, expected={expected_percent}, got={dec}"
            );
        }
    }

    #[test]
    fn pwv3_encode_decode_roundtrip_preserves_values() {
        // PWV3 encoding: peaks_to_levels then pack as ((6<<5) | (v&0x1F))
        let original_peaks: Vec<u8> = vec![10, 30, 60, 90, 100];
        let levels: Vec<u8> = original_peaks
            .iter()
            .map(|&v| ((u16::from(v) * 31) / 100) as u8)
            .collect();
        let encoded: Vec<u8> = levels.iter().map(|&v| (6u8 << 5) | (v & 0x1F)).collect();

        let decoded = downsample_waveform_payload(&encoded, encoded.len());

        for (i, &dec) in decoded.iter().enumerate() {
            let expected_level = levels[i];
            let expected_percent = ((f32::from(expected_level) * 100.0) / 31.0)
                .round()
                .clamp(0.0, 100.0) as u8;
            assert_eq!(
                dec, expected_percent,
                "PWV3 roundtrip mismatch at index {i}: level={expected_level}, expected={expected_percent}, got={dec}"
            );
        }
    }

    #[test]
    fn anlz_dat_file_roundtrip_extracts_correct_preview() {
        use crate::service::anlz::{WaveformData, build_anlz_dat_file};

        let peaks: Vec<u8> = (0..100).map(|i| (i * 100 / 99).min(100) as u8).collect();
        let dat_bytes = build_anlz_dat_file(
            &WaveformData::from_peaks(peaks),
            "",
            None,
            30_000,
            None,
            &[],
        );

        // extract_waveform_preview_from_anlz_bytes should find PWAV and decode it
        let preview = extract_waveform_preview_from_anlz_bytes(&dat_bytes, 50);
        assert!(preview.is_some(), "should extract preview from DAT file");
        let preview = preview.unwrap();
        assert_eq!(preview.len(), 50);
        // Values should be in 0-100 range and not all zero
        assert!(
            preview.iter().any(|&v| v > 0),
            "preview should have non-zero values"
        );
        assert!(
            preview.iter().all(|&v| v <= 100),
            "all values should be <= 100"
        );
    }
}
