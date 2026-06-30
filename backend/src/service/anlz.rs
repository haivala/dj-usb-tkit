//! ANLZ file builders: generate .DAT, .EXT, .2EX analysis files for USB export.
//!
//! Reference: ANLZ export analysis notes from Deep Symmetry.
//!
//! File structure (matching reference exports):
//! - .DAT: PPTH + PVBR + PQTZ + PWAV(400) + PWV2(100) + PCOB(hot) + PCOB(mem)
//! - .EXT: PPTH + PWV3(detail) + PCOB(hot) + PCOB(mem) + PCO2(hot) + PCO2(mem) + PWV5(color detail) + PWV4(color preview 1200)
//! - .2EX: PPTH + PWV7(3-band detail) + PWV6(3-band preview 1200) + PWVC

use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::BackendResult;

use super::usb_vendor_compat::{USB_ANALYSIS_DIR, USB_VENDOR_ROOT_DIR};

/// Waveform data with both amplitude peaks (0-100) and frequency bands (0-5) per bin.
///
/// Frequency band encoding:
///   0 = sub-bass, 1 = bass, 2 = low-mid, 3 = mid, 4 = high-mid, 5 = treble
#[derive(Debug, Clone)]
pub struct WaveformData {
    /// Amplitude peaks per bin, scaled 0-100 (per-track normalized).
    pub peaks: Vec<u8>,
    /// Dominant frequency band per bin, 0-5.
    pub bands: Vec<u8>,
    /// Per-bin low-frequency energy (0-127), shared-reference scaled.
    /// All 3 bands use the same p95 reference so relative balance is preserved.
    /// Used by PWV6/PWV7 (stacked 3-band rendering).
    pub low_energy: Vec<u8>,
    /// Per-bin mid-frequency energy (0-127), shared-reference scaled.
    pub mid_energy: Vec<u8>,
    /// Per-bin high-frequency energy (0-127), shared-reference scaled.
    pub high_energy: Vec<u8>,
    /// Per-bin low-frequency energy (0-127), independently scaled to full range.
    /// Each band uses its own p95 reference so it fills 0-127.
    /// Used by PWV4 (color preview) where each lane needs full dynamic range.
    pub low_energy_full: Vec<u8>,
    /// Per-bin mid-frequency energy (0-127), independently scaled to full range.
    pub mid_energy_full: Vec<u8>,
    /// Per-bin high-frequency energy (0-127), independently scaled to full range.
    pub high_energy_full: Vec<u8>,
    /// Absolute peak level (the max bin level before normalization).
    /// Used by preview writers (PWAV, PWV2, PWV4) for absolute scaling.
    /// Detail writers (PWV3, PWV5) use per-track normalized peaks.
    pub peak_level: f32,
}

impl WaveformData {
    pub fn empty() -> Self {
        Self {
            peaks: Vec::new(),
            bands: Vec::new(),
            low_energy: Vec::new(),
            mid_energy: Vec::new(),
            high_energy: Vec::new(),
            low_energy_full: Vec::new(),
            mid_energy_full: Vec::new(),
            high_energy_full: Vec::new(),
            peak_level: 0.0,
        }
    }

    /// Create WaveformData from amplitude-only peaks (assigns default mid band=3).
    /// 3-band data is synthesized from the single amplitude using a mid-dominant split.
    pub fn from_peaks(peaks: Vec<u8>) -> Self {
        let len = peaks.len();
        // Synthesize plausible 3-band data: most energy in mid, some in high, less in low
        let low_energy: Vec<u8> = peaks
            .iter()
            .map(|&p| ((p as u16 * 40) / 100).min(127) as u8)
            .collect();
        let mid_energy: Vec<u8> = peaks
            .iter()
            .map(|&p| ((p as u16 * 100) / 100).min(127) as u8)
            .collect();
        let high_energy: Vec<u8> = peaks
            .iter()
            .map(|&p| ((p as u16 * 50) / 100).min(127) as u8)
            .collect();
        // For synthetic data, shared and full-range are the same
        let low_energy_full = low_energy.clone();
        let mid_energy_full = mid_energy.clone();
        let high_energy_full = high_energy.clone();
        Self {
            peaks,
            bands: vec![3u8; len],
            low_energy,
            mid_energy,
            high_energy,
            low_energy_full,
            mid_energy_full,
            high_energy_full,
            peak_level: 1.0, // synthetic data assumes normalized
        }
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

pub fn canonical_analysis_bundle_paths(
    usb_root: &Path,
    track_path: &str,
) -> (PathBuf, PathBuf, PathBuf) {
    let hash = usb_analysis_path_hash(track_path);
    let dir_group = format!("P{:03X}", usb_analysis_bucket_from_hash(hash));
    let dir_leaf = format!("{hash:08X}");
    let root = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_ANALYSIS_DIR)
        .join(dir_group)
        .join(dir_leaf);
    (
        root.join("ANLZ0000.DAT"),
        root.join("ANLZ0000.EXT"),
        root.join("ANLZ0000.2EX"),
    )
}

pub(crate) fn usb_analysis_path_hash(track_path: &str) -> u32 {
    let mut hash = 0u32;
    for code_unit in track_path.encode_utf16() {
        let value = code_unit as u32;
        hash = 37_813u32
            .wrapping_mul(23_497u32.wrapping_mul(hash).wrapping_add(value))
            .wrapping_add(value);
    }

    let reduced = ((0xA7C5_075Bu64 * hash as u64) >> 49) as u32;
    hash.wrapping_sub(0x30D43u32.wrapping_mul(reduced))
}

pub(crate) fn usb_analysis_bucket_from_hash(hash: u32) -> u16 {
    let mut bucket = (hash & 0x1) as u16;
    bucket |= ((hash >> 1) & 0x2) as u16;
    bucket |= ((hash >> 4) & 0x4) as u16;
    bucket |= ((hash >> 4) & 0x8) as u16;
    bucket |= ((hash >> 5) & 0x10) as u16;
    bucket |= ((hash >> 8) & 0x20) as u16;
    bucket |= ((hash >> 10) & 0x40) as u16;
    bucket
}

// ---------------------------------------------------------------------------
// Bundle writer
// ---------------------------------------------------------------------------

pub fn write_generated_anlz_bundle(
    waveform: &WaveformData,
    dat_path: &Path,
    ext_path: &Path,
    twoex_path: &Path,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
) -> BackendResult<()> {
    write_generated_anlz_bundle_with_first_beat(
        waveform,
        dat_path,
        ext_path,
        twoex_path,
        track_path,
        bpm,
        duration_ms,
        None,
    )
}

pub fn write_generated_anlz_bundle_with_first_beat(
    waveform: &WaveformData,
    dat_path: &Path,
    ext_path: &Path,
    twoex_path: &Path,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    first_beat_ms_override: Option<u32>,
) -> BackendResult<()> {
    let dat = build_anlz_dat_file_with_first_beat(
        waveform,
        track_path,
        bpm,
        duration_ms,
        first_beat_ms_override,
    );
    let ext = build_anlz_ext_file_with_first_beat(
        waveform,
        track_path,
        bpm,
        duration_ms,
        first_beat_ms_override,
    );
    let twoex = build_anlz_2ex_file(waveform, track_path, duration_ms);
    if let Some(parent) = dat_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write_bytes(dat_path, &dat)?;
    atomic_write_bytes(ext_path, &ext)?;
    atomic_write_bytes(twoex_path, &twoex)?;
    Ok(())
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> BackendResult<()> {
    let parent = path.parent().ok_or_else(|| {
        crate::error::BackendError::Internal("missing parent directory".to_string())
    })?;
    std::fs::create_dir_all(parent)?;
    let tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("anlz"),
        Uuid::now_v7()
    );
    let tmp_path = parent.join(tmp_name);
    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn append_pqt2_chunk(
    file: &mut Vec<u8>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    first_beat_ms: u32,
) {
    let bpm_val = bpm.unwrap_or(120.0);
    if bpm_val <= 0.0 {
        return;
    }
    let dur_ms = duration_ms.unwrap_or(180_000) as f64;
    let beat_interval_ms = 60_000.0 / bpm_val;
    let num_beats = compute_num_beats(dur_ms, beat_interval_ms, first_beat_ms);
    if num_beats == 0 {
        return;
    }
    let tempo_centibpm = (bpm_val * 100.0).round() as u16;

    let mut header = Vec::<u8>::with_capacity(44);
    header.extend_from_slice(&0u32.to_be_bytes());
    header.extend_from_slice(&0x01000002u32.to_be_bytes());
    header.extend_from_slice(&0u32.to_be_bytes());

    let first_beat_num = 1u16;
    let first_time_ms = first_beat_ms;
    let last_beat_index = num_beats.saturating_sub(1);
    let last_beat_num = ((last_beat_index % 4) + 1) as u16;
    let last_time_ms =
        first_beat_ms.saturating_add((last_beat_index as f64 * beat_interval_ms).round() as u32);
    header.extend_from_slice(&first_beat_num.to_be_bytes());
    header.extend_from_slice(&tempo_centibpm.to_be_bytes());
    header.extend_from_slice(&first_time_ms.to_be_bytes());
    header.extend_from_slice(&last_beat_num.to_be_bytes());
    header.extend_from_slice(&tempo_centibpm.to_be_bytes());
    header.extend_from_slice(&last_time_ms.to_be_bytes());

    header.extend_from_slice(&num_beats.to_be_bytes());
    header.extend_from_slice(&0u32.to_be_bytes());
    header.extend_from_slice(&0u32.to_be_bytes());
    header.extend_from_slice(&0u32.to_be_bytes());

    let mut payload = Vec::<u8>::with_capacity((num_beats as usize) * 2);
    for i in 0..num_beats {
        payload.push((i % 4) as u8);
        payload.push(0);
    }

    append_anlz_chunk(file, b"PQT2", &header, &payload);
}

fn normalize_first_beat_ms(first_beat_ms: u32, bpm: Option<f64>) -> u32 {
    let bpm_val = bpm.unwrap_or(120.0);
    if bpm_val <= 0.0 {
        return 0;
    }
    let interval_ms = 60_000.0 / bpm_val;
    if !interval_ms.is_finite() || interval_ms <= 1.0 {
        return 0;
    }
    let wrapped = (first_beat_ms as f64) % interval_ms;
    wrapped.round().max(0.0) as u32
}

fn pssi_xor_mask(len_entries: u16, idx: usize) -> u8 {
    const BASE: [u8; 19] = [
        0xCB, 0xE1, 0xEE, 0xFA, 0xE5, 0xEE, 0xAD, 0xEE, 0xE9, 0xD2, 0xE9, 0xEB, 0xE1, 0xE9, 0xF3,
        0xE8, 0xE9, 0xF4, 0xE1,
    ];
    BASE[idx % BASE.len()].wrapping_add(len_entries as u8)
}

fn append_pssi_chunk(
    file: &mut Vec<u8>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    first_beat_ms: u32,
) {
    let bpm_val = bpm.unwrap_or(120.0);
    if bpm_val <= 0.0 {
        return;
    }
    let dur_ms = duration_ms.unwrap_or(180_000) as f64;
    let beat_interval_ms = 60_000.0 / bpm_val;
    let num_beats = compute_num_beats(dur_ms, beat_interval_ms, first_beat_ms) as u16;
    if num_beats < 8 {
        return;
    }

    // Build a minimal, valid phrase map with deterministic regions:
    // Intro(1), Verse(2), Chorus(9), Outro(10) in mood=2 ("mid").
    let mut starts = [
        1u16,
        (num_beats / 4).max(2),
        (num_beats / 2).max(3),
        ((num_beats * 3) / 4).max(4),
    ];
    for i in 1..starts.len() {
        starts[i] = starts[i].max(starts[i - 1].saturating_add(1));
    }
    let kinds = [1u16, 2u16, 9u16, 10u16];
    let len_entries = starts.len() as u16;

    let mut header = Vec::<u8>::with_capacity(20);
    header.extend_from_slice(&24u32.to_be_bytes()); // len_entry_bytes
    header.extend_from_slice(&len_entries.to_be_bytes()); // len_e
    header.extend_from_slice(&2u16.to_be_bytes()); // mood=mid
    header.extend_from_slice(&[0u8; 6]); // unknown bytes 14-19
    header.extend_from_slice(&num_beats.to_be_bytes()); // end_beat
    header.extend_from_slice(&0u16.to_be_bytes()); // unknown2
    header.push(0); // bank=default
    header.push(0); // unknown3

    let mut payload = Vec::<u8>::with_capacity(starts.len() * 24);
    for (idx, (&beat, &kind)) in starts.iter().zip(kinds.iter()).enumerate() {
        let mut entry = [0u8; 24];
        let entry_index = (idx as u16) + 1;
        entry[0..2].copy_from_slice(&entry_index.to_be_bytes());
        entry[2..4].copy_from_slice(&beat.to_be_bytes());
        entry[4..6].copy_from_slice(&kind.to_be_bytes());
        // b flag at byte 11 = 0 => single auxiliary beat at beat2.
        entry[11] = 0;
        let beat2 = beat.saturating_add(4).min(num_beats);
        entry[12..14].copy_from_slice(&beat2.to_be_bytes());
        // fill flag disabled
        entry[21] = 0;
        payload.extend_from_slice(&entry);
    }

    // Reference exports obfuscate bytes after len_e (absolute byte 18 onward in tag).
    for i in 6..header.len() {
        header[i] ^= pssi_xor_mask(len_entries, i - 6);
    }
    for i in 0..payload.len() {
        payload[i] ^= pssi_xor_mask(len_entries, header.len() - 6 + i);
    }

    append_anlz_chunk(file, b"PSSI", &header, &payload);
}

// ===========================================================================
// PMAI file header (28 bytes)
// ===========================================================================
//
// Offset  Field       Size  Value
// 0-3     magic       4     "PMAI"
// 4-7     len_header  4     0x0000001C (28)
// 8-11    len_file    4     total file length (updated by append_anlz_chunk)
// 12-15   unknown     4     0x00000001
// 16-19   unknown     4     0x00010000
// 20-23   unknown     4     0x00010000
// 24-27   unknown     4     0x00000000

pub(crate) fn build_anlz_file_header() -> Vec<u8> {
    let mut out = vec![0u8; 28];
    out[0..4].copy_from_slice(b"PMAI");
    out[4..8].copy_from_slice(&0x0000001Cu32.to_be_bytes());
    out[8..12].copy_from_slice(&28u32.to_be_bytes()); // updated by append_anlz_chunk
    out[12..16].copy_from_slice(&0x00000001u32.to_be_bytes());
    out[16..20].copy_from_slice(&0x00010000u32.to_be_bytes());
    out[20..24].copy_from_slice(&0x00010000u32.to_be_bytes());
    out[24..28].copy_from_slice(&0x00000000u32.to_be_bytes());
    out
}

// ===========================================================================
// Chunk envelope
// ===========================================================================
//
// Every chunk starts with:
//   0-3:  fourcc      (4-byte tag)
//   4-7:  len_header  (header length including tag/len fields)
//   8-11: len_tag     (total tag length = header + payload)
//   12+:  header-specific content + payload

pub(crate) fn append_anlz_chunk(file: &mut Vec<u8>, tag: &[u8; 4], header: &[u8], payload: &[u8]) {
    let total_len = (12 + header.len() + payload.len()) as u32;
    let header_len = (12 + header.len()) as u32;
    let mut chunk = Vec::with_capacity(total_len as usize);
    chunk.extend_from_slice(tag);
    chunk.extend_from_slice(&header_len.to_be_bytes());
    chunk.extend_from_slice(&total_len.to_be_bytes());
    chunk.extend_from_slice(header);
    chunk.extend_from_slice(payload);
    file.extend_from_slice(&chunk);
    let file_len = file.len() as u32;
    file[8..12].copy_from_slice(&file_len.to_be_bytes());
}

// ===========================================================================
// PPTH — track path (UTF-16BE)
// ===========================================================================
//
// len_header = 0x10 (16)
// Offset 12-15: len_path (4 bytes, length of path data)
// Offset 16+:   path in UTF-16BE with trailing NUL

pub(crate) fn append_ppth_chunk(file: &mut Vec<u8>, path: &str) {
    let Some(ppth_chunk) = build_ppth_chunk(path) else {
        return;
    };
    file.extend_from_slice(&ppth_chunk);
    let file_len = file.len() as u32;
    file[8..12].copy_from_slice(&file_len.to_be_bytes());
}

/// Inject a PPTH chunk into an existing ANLZ file that lacks one.
/// Inserts after the 28-byte PMAI header, before existing chunks.
pub fn inject_ppth_into_anlz(data: &[u8], track_path: &str) -> Vec<u8> {
    if data.len() < 28 {
        return data.to_vec();
    }
    let Some(ppth_buf) = build_ppth_chunk(track_path) else {
        return data.to_vec();
    };

    let mut result = Vec::with_capacity(data.len() + ppth_buf.len());
    result.extend_from_slice(&data[..28]);
    result.extend_from_slice(&ppth_buf);
    result.extend_from_slice(&data[28..]);
    let file_len = result.len() as u32;
    result[8..12].copy_from_slice(&file_len.to_be_bytes());
    result
}

fn build_ppth_chunk(path: &str) -> Option<Vec<u8>> {
    if path.is_empty() {
        return None;
    }
    let utf16: Vec<u16> = path.encode_utf16().collect();
    let path_byte_len = (utf16.len() + 1) * 2;
    let mut header = Vec::with_capacity(4);
    header.extend_from_slice(&(path_byte_len as u32).to_be_bytes());
    let mut payload = Vec::with_capacity(path_byte_len);
    for ch in &utf16 {
        payload.extend_from_slice(&ch.to_be_bytes());
    }
    payload.extend_from_slice(&0u16.to_be_bytes());

    let total_len = (12 + header.len() + payload.len()) as u32;
    let header_len = (12 + header.len()) as u32;
    let mut chunk = Vec::with_capacity(total_len as usize);
    chunk.extend_from_slice(b"PPTH");
    chunk.extend_from_slice(&header_len.to_be_bytes());
    chunk.extend_from_slice(&total_len.to_be_bytes());
    chunk.extend_from_slice(&header);
    chunk.extend_from_slice(&payload);
    Some(chunk)
}

fn read_u32_be_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Ensure an existing ANLZ file has exactly one leading PPTH chunk for the
/// exported USB-relative track path.
///
/// Local analysis cache files are deliberately generated without a PPTH path.
/// On export, older players still expect the path embedded in the analysis
/// file, encoded as UTF-16BE just like DJ-software-authored bundles.
pub fn ensure_ppth_chunk(data: &[u8], track_path: &str) -> Vec<u8> {
    if track_path.is_empty() || data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return data.to_vec();
    }
    let Some(ppth_chunk) = build_ppth_chunk(track_path) else {
        return data.to_vec();
    };

    let mut out = Vec::with_capacity(data.len() + ppth_chunk.len());
    out.extend_from_slice(&data[..28]);
    out.extend_from_slice(&ppth_chunk);

    let mut pos = 28usize;
    while pos < data.len() {
        if pos + 12 > data.len() {
            out.extend_from_slice(&data[pos..]);
            break;
        }
        let Some(header_len) = read_u32_be_at(data, pos + 4).map(|v| v as usize) else {
            out.extend_from_slice(&data[pos..]);
            break;
        };
        let Some(total_len) = read_u32_be_at(data, pos + 8).map(|v| v as usize) else {
            out.extend_from_slice(&data[pos..]);
            break;
        };
        if header_len < 12
            || total_len < header_len
            || total_len == 0
            || pos + total_len > data.len()
        {
            out.extend_from_slice(&data[pos..]);
            break;
        }

        if data.get(pos..pos + 4) != Some(b"PPTH") {
            out.extend_from_slice(&data[pos..pos + total_len]);
        }
        pos += total_len;
    }

    let file_len = out.len() as u32;
    out[8..12].copy_from_slice(&file_len.to_be_bytes());
    out
}

#[cfg(test)]
pub(crate) fn ppth_path_from_anlz(data: &[u8]) -> Option<String> {
    if data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return None;
    }
    let mut pos = 28usize;
    while pos + 12 <= data.len() {
        let header_len = read_u32_be_at(data, pos + 4)? as usize;
        let total_len = read_u32_be_at(data, pos + 8)? as usize;
        if header_len < 12
            || total_len < header_len
            || total_len == 0
            || pos + total_len > data.len()
        {
            return None;
        }
        if data.get(pos..pos + 4) == Some(b"PPTH") {
            let payload = data.get(pos + header_len..pos + total_len)?;
            if payload.len() % 2 != 0 {
                return None;
            }
            let mut units = payload
                .chunks_exact(2)
                .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>();
            if units.last() == Some(&0) {
                units.pop();
            }
            return String::from_utf16(&units).ok();
        }
        pos += total_len;
    }
    None
}

// ===========================================================================
// PVBR — VBR seek index (placeholder)
// ===========================================================================
//
// len_header = 0x10 (16)
// Offset 12-15: unknown (4 bytes, observed 0x00000000)
// Offset 16+:   index data (observed: 1604 bytes of zeros)

fn append_pvbr_chunk(file: &mut Vec<u8>) {
    let header = vec![0u8; 4]; // unknown1 = 0
    let payload = vec![0u8; 1604]; // placeholder zeros
    append_anlz_chunk(file, b"PVBR", &header, &payload);
}

// ===========================================================================
// PQTZ — beat grid
// ===========================================================================
//
// len_header = 0x18 (24)
// Offset 12-13: unknown1 (2 bytes)
// Offset 14-17: unknown2 (4 bytes, observed 0x00080000)
// Offset 18-19: unknown3 (2 bytes)
// Offset 20-23: len_beats (4 bytes)
// Offset 24+:   beat entries (8 bytes each)
//
// Beat entry:
//   0-1: beat_number (u16, 1-4 position in measure)
//   2-3: tempo       (u16, BPM × 100)
//   4-7: time        (u32, milliseconds)

fn append_pqtz_chunk(
    file: &mut Vec<u8>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    first_beat_ms: u32,
) {
    let bpm_val = bpm.unwrap_or(120.0);
    if bpm_val <= 0.0 {
        return;
    }
    let dur_ms = duration_ms.unwrap_or(180_000) as f64;
    let beat_interval_ms = 60_000.0 / bpm_val;
    let num_beats = compute_num_beats(dur_ms, beat_interval_ms, first_beat_ms);
    if num_beats == 0 {
        return;
    }
    let tempo_centibpm = (bpm_val * 100.0).round() as u16;

    // Header content: 12 bytes (offsets 12-23 in chunk)
    let mut header = vec![0u8; 12];
    // [0..2] = unknown1 = 0x0000
    // [2..6] = unknown2 = 0x00080000 in observed reference exports.
    header[2..6].copy_from_slice(&0x00080000u32.to_be_bytes());
    // [6..8] = unknown3 = 0x0000
    // [8..12] = len_beats
    header[8..12].copy_from_slice(&num_beats.to_be_bytes());

    let mut payload = Vec::with_capacity(num_beats as usize * 8);
    for i in 0..num_beats {
        let beat_num = ((i % 4) + 1) as u16;
        let time_ms = first_beat_ms.saturating_add((i as f64 * beat_interval_ms).round() as u32);
        payload.extend_from_slice(&beat_num.to_be_bytes());
        payload.extend_from_slice(&tempo_centibpm.to_be_bytes());
        payload.extend_from_slice(&time_ms.to_be_bytes());
    }
    append_anlz_chunk(file, b"PQTZ", &header, &payload);
}

fn estimate_first_beat_ms(
    waveform: &WaveformData,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
) -> u32 {
    let bpm_val = bpm.unwrap_or(120.0);
    let dur_ms = duration_ms.unwrap_or(180_000) as f64;
    if bpm_val <= 0.0 || waveform.peaks.is_empty() || dur_ms <= 0.0 {
        return 0;
    }
    let interval = 60_000.0 / bpm_val;
    if interval <= 1.0 {
        return 0;
    }
    let bins = waveform.peaks.len();
    let step = (interval / 96.0).max(1.0);
    let tol = interval * 0.22;
    let series_full: Vec<f64> = waveform.peaks.iter().map(|&v| v as f64).collect();
    let series_low: Vec<f64> = if waveform.low_energy.len() == bins {
        waveform.low_energy.iter().map(|&v| v as f64).collect()
    } else {
        Vec::new()
    };
    let series_mid: Vec<f64> = if waveform.mid_energy.len() == bins {
        waveform.mid_energy.iter().map(|&v| v as f64).collect()
    } else {
        Vec::new()
    };
    let series_high: Vec<f64> = if waveform.high_energy.len() == bins {
        waveform.high_energy.iter().map(|&v| v as f64).collect()
    } else {
        Vec::new()
    };

    let salience = |series: &[f64]| -> f64 {
        if series.is_empty() {
            return 0.0;
        }
        let mean = series.iter().sum::<f64>() / series.len() as f64;
        let var = series
            .iter()
            .map(|v| {
                let d = *v - mean;
                d * d
            })
            .sum::<f64>()
            / series.len() as f64;
        var.sqrt()
    };

    let mut weights: Vec<(&[f64], f64)> = vec![(&series_full, salience(&series_full))];
    if !series_low.is_empty() {
        weights.push((&series_low, salience(&series_low)));
    }
    if !series_mid.is_empty() {
        weights.push((&series_mid, salience(&series_mid)));
    }
    if !series_high.is_empty() {
        weights.push((&series_high, salience(&series_high)));
    }
    let total_w = weights.iter().map(|(_, w)| *w).sum::<f64>().max(1.0);
    for (_, w) in &mut weights {
        *w /= total_w;
    }

    let mut scored = Vec::<(f64, f64)>::new();
    let mut best_score = f64::MIN;
    let mut phase = 0.0f64;
    while phase < interval {
        let mut combined = 0.0f64;
        for (series, weight) in &weights {
            let mut score = 0.0f64;
            for (i, amp) in series.iter().enumerate() {
                if *amp <= 0.0 {
                    continue;
                }
                let t = (i as f64 * dur_ms) / bins as f64;
                let mut m = (t - phase) % interval;
                if m < 0.0 {
                    m += interval;
                }
                let dist = m.min(interval - m);
                if dist <= tol {
                    let proximity = 1.0 - (dist / tol);
                    score += amp * proximity;
                }
            }
            combined += score * *weight;
        }
        if combined > best_score {
            best_score = combined;
        }
        scored.push((phase, combined));
        phase += step;
    }
    // Bias to the earliest near-optimal phase so we do not pick a later equivalent beat.
    let near_best = best_score * 0.98;
    let min_first_phase = (interval * 0.08).max(step);
    let mut chosen = scored
        .iter()
        .filter(|(_, score)| *score >= near_best)
        .map(|(phase, _)| *phase)
        .filter(|phase| *phase >= min_first_phase)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .or_else(|| {
            scored
                .iter()
                .filter(|(_, score)| *score >= near_best)
                .map(|(phase, _)| *phase)
                .filter(|phase| *phase > 0.0)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        })
        .unwrap_or(0.0);
    if chosen > 0.0 && chosen < interval {
        chosen = chosen.min(interval - chosen);
    }
    chosen.round().max(0.0) as u32
}

fn compute_num_beats(duration_ms: f64, beat_interval_ms: f64, first_beat_ms: u32) -> u32 {
    if beat_interval_ms <= 0.0 || duration_ms <= 0.0 {
        return 1;
    }
    let first = first_beat_ms as f64;
    let base = if duration_ms > first {
        ((duration_ms - first) / beat_interval_ms).floor() as u32 + 1
    } else {
        1
    };
    // Reference-style behavior: when phase lands very early in the beat period, include
    // one trailing beat marker at the end of the grid.
    if first_beat_ms > 0 && first <= beat_interval_ms * 0.20 {
        base.saturating_add(1)
    } else {
        base
    }
}

// ===========================================================================
// PCOB — cue list (empty placeholder)
// ===========================================================================
//
// len_header = 0x18 (24)
// Offset 12-15: type       (4 bytes, 0=memory points, 1=hot cues)
// Offset 16-17: lencues    (2 bytes, 0)
// Offset 18-19: unk        (2 bytes, 0)
// Offset 20-23: memory_count (4 bytes, observed 0xFFFFFFFF for empty)

fn append_empty_pcob_chunk(file: &mut Vec<u8>, cue_type: u32) {
    let mut header = vec![0u8; 12];
    header[0..4].copy_from_slice(&cue_type.to_be_bytes()); // type
    // header[4..6] = lencues = 0
    // header[6..8] = unk = 0
    header[8..12].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes()); // memory_count
    append_anlz_chunk(file, b"PCOB", &header, &[]);
}

// ===========================================================================
// PCO2 — extended cue list (empty placeholder)
// ===========================================================================
//
// len_header = 0x14 (20)
// Offset 12-15: type       (4 bytes, 0=memory points, 1=hot cues)
// Offset 16-17: lencues    (2 bytes, 0)
// Offset 18-19: unknown    (2 bytes, 0)

fn append_empty_pco2_chunk(file: &mut Vec<u8>, cue_type: u32) {
    let mut header = vec![0u8; 8];
    header[0..4].copy_from_slice(&cue_type.to_be_bytes()); // type
    // header[4..6] = lencues = 0
    // header[6..8] = unknown = 0
    append_anlz_chunk(file, b"PCO2", &header, &[]);
}

// ===========================================================================
// PWVC — waveform color settings
// ===========================================================================
//
// len_header = 0x0E (14)
// Content observed: 00 00 00 50 00 5f 00 64 (8 bytes at offsets 12-19)
// Possibly: unknown(2) + 3 × u16 color thresholds

fn append_pwvc_chunk(file: &mut Vec<u8>) {
    // 14 - 12 = 2 bytes header content, + 6 bytes payload
    let header: [u8; 2] = [0x00, 0x00];
    let payload: [u8; 6] = [0x00, 0x50, 0x00, 0x5F, 0x00, 0x64];
    append_anlz_chunk(file, b"PWVC", &header, &payload);
}

// ===========================================================================
// Resampling helpers
// ===========================================================================

/// Resample peaks (0-100) to height values (0-31) at target count.
pub(crate) fn peaks_to_levels(peaks: &[u8], count: usize) -> Vec<u8> {
    if peaks.is_empty() || count == 0 {
        return vec![0u8; count];
    }
    (0..count)
        .map(|i| {
            let idx = i * peaks.len() / count;
            let v = peaks[idx.min(peaks.len().saturating_sub(1))];
            ((u16::from(v) * 31) / 100) as u8
        })
        .collect::<Vec<_>>()
}

/// Resample peaks to absolute heights (0..=max_height) at target count.
/// Uses only audio-derived absolute peak level per window, no extra gain.
fn peaks_to_absolute_levels(
    peaks: &[u8],
    count: usize,
    peak_level: f32,
    max_height: u8,
) -> Vec<u8> {
    if peaks.is_empty() || count == 0 {
        return vec![0u8; count];
    }
    (0..count)
        .map(|i| {
            let (start, end) = resample_window(i, peaks.len(), count);
            let max_abs = peaks[start..end]
                .iter()
                .map(|&v| (v as f32 / 100.0) * peak_level)
                .fold(0.0f32, |acc, v| acc.max(v));
            (max_abs * max_height as f32)
                .round()
                .clamp(0.0, max_height as f32) as u8
        })
        .collect::<Vec<_>>()
}

/// Resample frequency bands to target count (nearest-neighbor).
pub(crate) fn bands_to_levels(bands: &[u8], count: usize) -> Vec<u8> {
    if bands.is_empty() || count == 0 {
        return vec![3u8; count]; // default mid band
    }
    (0..count)
        .map(|i| {
            let idx = i * bands.len() / count;
            bands[idx.min(bands.len().saturating_sub(1))].min(5)
        })
        .collect::<Vec<_>>()
}

fn resample_window(i: usize, src_len: usize, count: usize) -> (usize, usize) {
    let start = i * src_len / count;
    let mut end = ((i + 1) * src_len) / count;
    if end <= start {
        end = (start + 1).min(src_len);
    }
    (start.min(src_len.saturating_sub(1)), end.min(src_len))
}

/// Resample 3-band energy values (0-127) to target count using transient-preserving
/// window aggregation (upper percentile + max), avoiding nearest-neighbor flattening.
fn resample_energy(energy: &[u8], count: usize) -> Vec<u8> {
    if energy.is_empty() || count == 0 {
        return vec![0u8; count];
    }
    (0..count)
        .map(|i| {
            let (start, end) = resample_window(i, energy.len(), count);
            let mut window: Vec<u8> = energy[start..end].to_vec();
            window.sort_unstable();
            let max_v = *window.last().unwrap_or(&0);
            let p85_idx = ((window.len().saturating_sub(1)) as f32 * 0.85).round() as usize;
            let p85 = window[p85_idx.min(window.len().saturating_sub(1))];
            (((u16::from(max_v) * 3) + (u16::from(p85) * 2)) / 5).min(127) as u8
        })
        .collect()
}

/// Compute detail entry count from duration.
///
/// Reference exports consistently use a slightly longer detail stream than a
/// plain `round(duration_seconds * 150)` calculation. The observed fit on
/// reference USBs is `ceil(duration_seconds * 150) + 4`, which aligns both
/// PWV5 and PWV7 header entry counts with device-working exports.
fn detail_entry_count(duration_ms: Option<u64>) -> u32 {
    let duration_secs = duration_ms.unwrap_or(180_000) as f64 / 1000.0;
    ((duration_secs * 150.0).ceil() as u32)
        .saturating_add(4)
        .max(400)
}

/// Map frequency band (0-5) to a "whiteness" value (0-7) for PWAV encoding.
/// Reference data shows whiteness values distributed 0-5 matching band values,
/// where lower bands (bass) = less white, higher bands (treble) = whiter.
fn band_to_whiteness(band: u8) -> u8 {
    band.min(5)
}

// ===========================================================================
// .DAT — PPTH + PVBR + PQTZ + PWAV + PWV2 + PCOB(hot) + PCOB(mem)
// ===========================================================================
//
// PWAV: 400 entries, 1 byte each
//   len_header = 0x14 (20)
//   Offset 12-15: len_preview (u32, = 400)
//   Offset 16-19: unknown (u32, observed 0x00100000)
//   Each byte: bits 5-7 = whiteness (0-7), bits 0-4 = height (0-31)
//
// PWV2: 100 entries, 1 byte each
//   len_header = 0x14 (20)
//   Offset 12-15: len_preview (u32, = 100)
//   Offset 16-19: unknown (u32, observed 0x00100000)
//   Each byte: bits 0-3 = height (0-15), bits 4-7 = 0

/// PWAV preview blend weights (normalized by PWAV_BLEND_DENOM):
/// fuse amplitude envelope with low/mid energy envelope for reference-export shape.
const PWAV_BLEND_PEAK_WEIGHT: f32 = 1.0;
const PWAV_BLEND_LOW_WEIGHT: f32 = 1.2;
const PWAV_BLEND_MID_WEIGHT: f32 = 0.6;
const PWAV_BLEND_DENOM: f32 =
    PWAV_BLEND_PEAK_WEIGHT + PWAV_BLEND_LOW_WEIGHT + PWAV_BLEND_MID_WEIGHT;
const PWAV_BLEND_GAMMA: f32 = 0.85;
const PWAV_OUTPUT_GAIN: f32 = 0.85;
const PWAV_OUTPUT_OFFSET: f32 = 0.0;
/// Reference export profile: max height = 25, mean ≈ 18.4 (not full 31-range).
const PWAV_OUTPUT_CAP: f32 = 25.0;

fn preview_header_magic() -> u32 {
    0x00010000
}

pub fn build_anlz_dat_file(
    waveform: &WaveformData,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
) -> Vec<u8> {
    build_anlz_dat_file_with_first_beat(waveform, track_path, bpm, duration_ms, None)
}

fn build_anlz_dat_file_with_first_beat(
    waveform: &WaveformData,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    first_beat_ms_override: Option<u32>,
) -> Vec<u8> {
    let mut file = build_anlz_file_header();
    let first_beat_ms_raw = first_beat_ms_override
        .unwrap_or_else(|| estimate_first_beat_ms(waveform, bpm, duration_ms));
    let first_beat_ms = normalize_first_beat_ms(first_beat_ms_raw, bpm);

    // 1. PPTH
    append_ppth_chunk(&mut file, track_path);

    // 2. PVBR — VBR seek index placeholder
    append_pvbr_chunk(&mut file);

    // 3. PQTZ — beat grid
    append_pqtz_chunk(&mut file, bpm, duration_ms, first_beat_ms);

    // 4. PWAV — 400-entry waveform preview (absolute scaling, full 5-bit headroom)
    {
        let count = 400u32;
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&count.to_be_bytes()); // len_preview
        header.extend_from_slice(&preview_header_magic().to_be_bytes()); // unknown (reference export)
        let levels =
            peaks_to_absolute_levels(&waveform.peaks, count as usize, waveform.peak_level, 31);
        let low_preview = resample_energy(&waveform.low_energy, count as usize)
            .into_iter()
            .map(|v| ((v as f32) * 31.0 / 127.0).round().clamp(0.0, 31.0) as u8)
            .collect::<Vec<_>>();
        let mid_preview = resample_energy(&waveform.mid_energy, count as usize)
            .into_iter()
            .map(|v| ((v as f32) * 31.0 / 127.0).round().clamp(0.0, 31.0) as u8)
            .collect::<Vec<_>>();
        let freq_bands = bands_to_levels(&waveform.bands, count as usize);
        let payload: Vec<u8> = levels
            .iter()
            .zip(low_preview.iter())
            .zip(mid_preview.iter())
            .zip(freq_bands.iter())
            .map(|(((&height, &low), &mid), &band)| {
                let blended = ((height as f32 * PWAV_BLEND_PEAK_WEIGHT)
                    + (low as f32 * PWAV_BLEND_LOW_WEIGHT)
                    + (mid as f32 * PWAV_BLEND_MID_WEIGHT))
                    / PWAV_BLEND_DENOM;
                let h = ((blended.clamp(0.0, 31.0) / 31.0).powf(PWAV_BLEND_GAMMA) * 31.0)
                    .round()
                    .mul_add(PWAV_OUTPUT_GAIN, PWAV_OUTPUT_OFFSET)
                    .clamp(0.0, PWAV_OUTPUT_CAP) as u8;
                (band_to_whiteness(band) << 5) | (h & 0x1F)
            })
            .collect();
        append_anlz_chunk(&mut file, b"PWAV", &header, &payload);
    }

    // 5. PWV2 — 100-entry tiny waveform preview (absolute scaling, cap 15)
    {
        let count = 100u32;
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&count.to_be_bytes()); // len_preview
        header.extend_from_slice(&preview_header_magic().to_be_bytes()); // unknown (reference export)
        let payload =
            peaks_to_absolute_levels(&waveform.peaks, count as usize, waveform.peak_level, 15);
        append_anlz_chunk(&mut file, b"PWV2", &header, &payload);
    }

    // 6. PCOB — empty hot cue list (type=1)
    append_empty_pcob_chunk(&mut file, 1);

    // 7. PCOB — empty memory point list (type=0)
    append_empty_pcob_chunk(&mut file, 0);

    file
}

// ===========================================================================
// .EXT — PPTH + PWV3 + PCOB(hot) + PCOB(mem) + PCO2(hot) + PCO2(mem) + PQT2 + PWV5 + PWV4 + PSSI
// ===========================================================================
//
// PWV3: detail waveform, 1 byte per entry
//   len_header = 0x18 (24)
//   Offset 12-15: len_entry_bytes (u32, = 1)
//   Offset 16-19: len_entries (u32)
//   Offset 20-23: unknown (u32, observed 0x00960000)
//   Each byte: bits 5-7 = whiteness (reference: always 7), bits 0-4 = height (0-31)
//   Entry count = duration_seconds × 150
//
// PWV5: color detail waveform, 2 bytes per entry (BE u16)
//   len_header = 0x18 (24)
//   Offset 12-15: len_entry_bytes (u32, = 2)
//   Offset 16-19: len_entries (u32)
//   Offset 20-23: unknown (u32, observed 0x00960305)
//   Bit layout: R(3) | G(3) | B(3) | Height(5) | unused(2)
//   Entry count = same as PWV3
//
// PWV4: color preview waveform, 6 bytes per entry
//   len_header = 0x18 (24)
//   Offset 12-15: len_entry_bytes (u32, = 6)
//   Offset 16-19: len_entries (u32, = 1200)
//   Offset 20-23: unknown (u32, observed 0x00000000)

pub fn build_anlz_ext_file(
    waveform: &WaveformData,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
) -> Vec<u8> {
    build_anlz_ext_file_with_first_beat(waveform, track_path, bpm, duration_ms, None)
}

fn build_anlz_ext_file_with_first_beat(
    waveform: &WaveformData,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    first_beat_ms_override: Option<u32>,
) -> Vec<u8> {
    let mut file = build_anlz_file_header();
    let first_beat_ms_raw = first_beat_ms_override
        .unwrap_or_else(|| estimate_first_beat_ms(waveform, bpm, duration_ms));
    let first_beat_ms = normalize_first_beat_ms(first_beat_ms_raw, bpm);

    let detail_count = detail_entry_count(duration_ms);

    // 1. PPTH
    append_ppth_chunk(&mut file, track_path);

    // 2. PWV3 — mono detail waveform (1 byte per entry)
    {
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&1u32.to_be_bytes()); // len_entry_bytes
        header.extend_from_slice(&detail_count.to_be_bytes());
        header.extend_from_slice(&0x00960000u32.to_be_bytes());
        let levels = peaks_to_levels(&waveform.peaks, detail_count as usize);
        let freq_bands = bands_to_levels(&waveform.bands, detail_count as usize);
        let payload: Vec<u8> = levels
            .iter()
            .zip(freq_bands.iter())
            .map(|(&height, &band)| {
                let whiteness = band_to_whiteness(band) << 5;
                whiteness | (height & 0x1F)
            })
            .collect();
        append_anlz_chunk(&mut file, b"PWV3", &header, &payload);
    }

    // 3. PCOB — empty hot cue list (type=1)
    append_empty_pcob_chunk(&mut file, 1);

    // 4. PCOB — empty memory point list (type=0)
    append_empty_pcob_chunk(&mut file, 0);

    // 5. PCO2 — empty extended hot cue list (type=1)
    append_empty_pco2_chunk(&mut file, 1);

    // 6. PCO2 — empty extended memory point list (type=0)
    append_empty_pco2_chunk(&mut file, 0);

    // 7. PQT2 — extended beat grid
    append_pqt2_chunk(&mut file, bpm, duration_ms, first_beat_ms);

    // 8. PWV5 — color detail waveform (2 bytes per entry)
    let levels = peaks_to_levels(&waveform.peaks, detail_count as usize);
    let lows = resample_energy(&waveform.low_energy, detail_count as usize);
    let mids = resample_energy(&waveform.mid_energy, detail_count as usize);
    let highs = resample_energy(&waveform.high_energy, detail_count as usize);
    {
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&2u32.to_be_bytes()); // len_entry_bytes
        header.extend_from_slice(&detail_count.to_be_bytes());
        header.extend_from_slice(&0x00960305u32.to_be_bytes());
        let mut payload = Vec::<u8>::with_capacity((detail_count * 2) as usize);
        for (((&height, &low), &mid), &high) in levels
            .iter()
            .zip(lows.iter())
            .zip(mids.iter())
            .zip(highs.iter())
        {
            let low_half = u16::from(low) / 2;
            let mid_contrast = u16::from(mid).saturating_sub(low_half);
            let high_contrast = u16::from(high).saturating_sub(low_half);
            let r3 = u16::from((u16::from(low) / 8).saturating_sub(3).min(7) as u8);
            let g3 = u16::from(((mid_contrast / 12) + 3).min(7) as u8);
            let b3 = u16::from(((high_contrast / 4) + 5).min(7) as u8);
            let h5 = u16::from((((u16::from(height) * 22) / 25).saturating_sub(4)).min(31) as u8);
            let packed: u16 = (r3 << 13) | (g3 << 10) | (b3 << 7) | (h5 << 2);
            payload.extend_from_slice(&packed.to_be_bytes());
        }
        append_anlz_chunk(&mut file, b"PWV5", &header, &payload);
    }

    // 9. PWV4 — color preview waveform (6 bytes per entry, 1200 entries)
    // NXS2-style six-lane preview payload (per dysentery#9):
    //   d0 = absolute amplitude (0-127)
    //   d1 = luminance boost factor (colors *= d1/127); reference profile: d0+d1 ≈ 255
    //   d2 = inverse intensity for blue/mono waveform (0-127)
    //   d3 = red channel / low frequency (0-127)
    //   d4 = green channel / mid frequency (0-127)
    //   d5 = blue channel + front waveform height (0-127)
    {
        let preview_count = 1200u32;
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&6u32.to_be_bytes()); // len_entry_bytes
        header.extend_from_slice(&preview_count.to_be_bytes());
        header.extend_from_slice(&0u32.to_be_bytes());
        let preview_levels = peaks_to_absolute_levels(
            &waveform.peaks,
            preview_count as usize,
            waveform.peak_level,
            127,
        );
        // Use independently-scaled full-range band data for PWV4.
        // Each band fills 0-127 using its own p95 reference (set in analysis stage).
        let lows = resample_energy(&waveform.low_energy_full, preview_count as usize);
        let mids = resample_energy(&waveform.mid_energy_full, preview_count as usize);
        let highs = resample_energy(&waveform.high_energy_full, preview_count as usize);
        let mut payload = Vec::<u8>::with_capacity((preview_count * 6) as usize);
        for i in 0..preview_count as usize {
            let b0 = preview_levels[i];
            // d1: luminance boost — inverse of amplitude, floor 128 (reference pattern)
            let b1 = if b0 == 0 {
                0u8
            } else {
                255u8.saturating_sub(b0).max(128)
            };
            let b2 = (((u16::from(lows[i]) + u16::from(mids[i])) * 3 / 4).saturating_sub(12))
                .min(127) as u8;
            let b3 = lows[i];
            let b4 = mids[i];
            let b5 = highs[i];
            payload.extend_from_slice(&[b0, b1, b2, b3, b4, b5]);
        }
        append_anlz_chunk(&mut file, b"PWV4", &header, &payload);
    }

    // 10. PSSI — song structure / phrase analysis (reference exports place this in .EXT)
    append_pssi_chunk(&mut file, bpm, duration_ms, first_beat_ms);

    file
}

// ===========================================================================
// .2EX — PPTH + PWV7 + PWV6 + PWVC
// ===========================================================================
//
// PWV7: 3-band detail waveform, 3 bytes per entry
//   len_header = 0x18 (24)
//   Offset 12-15: len_entry_bytes (u32, = 3)
//   Offset 16-19: len_entries (u32)
//   Offset 20-23: unknown (u32, observed 0x00960000)
//   Each entry: [mid_height, high_height, low_height]
//   Entry count = duration_seconds × 150
//
// PWV6: 3-band preview waveform, 3 bytes per entry
//   len_header = 0x14 (20)
//   Offset 12-15: len_entry_bytes (u32, = 3)
//   Offset 16-19: len_entries (u32, = 1200)
//   Each entry: [mid_height, high_height, low_height]
//   Display: stacked vertically — lows (dark blue) + mids (amber) + highs (white)

pub fn build_anlz_2ex_file(
    waveform: &WaveformData,
    track_path: &str,
    duration_ms: Option<u64>,
) -> Vec<u8> {
    let mut file = build_anlz_file_header();

    let detail_count = detail_entry_count(duration_ms);

    // 1. PPTH
    append_ppth_chunk(&mut file, track_path);

    // 2. PWV7 — 3-band detail waveform (3 bytes per entry)
    //    Lane order: [mid, high, low]
    {
        let mids = resample_energy(&waveform.mid_energy, detail_count as usize);
        let highs = resample_energy(&waveform.high_energy, detail_count as usize);
        let lows = resample_energy(&waveform.low_energy, detail_count as usize);
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&3u32.to_be_bytes()); // len_entry_bytes
        header.extend_from_slice(&detail_count.to_be_bytes());
        header.extend_from_slice(&0x00960000u32.to_be_bytes());
        let mut payload = Vec::<u8>::with_capacity((detail_count * 3) as usize);
        for i in 0..detail_count as usize {
            let low = (((u16::from(lows[i]) * 5) + u16::from(mids[i])) / 8 + 3).min(127) as u8;
            let mid = (((u16::from(mids[i]) * 10) + u16::from(highs[i])) / 16 + 3).min(127) as u8;
            let high = (((u16::from(highs[i]) * 5) / 4) + (u16::from(lows[i]) / 8))
                .saturating_sub(u16::from(mids[i]) / 8 + 13)
                .min(127) as u8;
            payload.extend_from_slice(&[mid, high, low]);
        }
        append_anlz_chunk(&mut file, b"PWV7", &header, &payload);
    }

    // 3. PWV6 — 3-band preview waveform (3 bytes per entry, 1200 entries)
    //    Lane order: [mid, high, low]
    //    Note: PWV6 has len_header=0x14 (20), no unknown field at offset 20-23
    {
        let preview_count = 1200u32;
        let mids = resample_energy(&waveform.mid_energy, preview_count as usize);
        let highs = resample_energy(&waveform.high_energy, preview_count as usize);
        let lows = resample_energy(&waveform.low_energy, preview_count as usize);
        let mut header = Vec::<u8>::new();
        header.extend_from_slice(&3u32.to_be_bytes()); // len_entry_bytes
        header.extend_from_slice(&preview_count.to_be_bytes());
        // No unknown field — header is only 8 bytes (total header_len = 20)
        let mut payload = Vec::<u8>::with_capacity((preview_count * 3) as usize);
        for i in 0..preview_count as usize {
            let low = (u16::from(lows[i]) / 2).saturating_sub(1).min(127) as u8;
            let mid = (((u16::from(mids[i]) * 11) + u16::from(highs[i])) / 32)
                .saturating_add(4)
                .min(127) as u8;
            let high = ((u16::from(highs[i]) * 3) / 4).saturating_sub(1).min(127) as u8;
            payload.extend_from_slice(&[mid, high, low]);
        }
        append_anlz_chunk(&mut file, b"PWV6", &header, &payload);
    }

    // 4. PWVC — waveform color settings
    append_pwvc_chunk(&mut file);

    file
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_analysis_path_hash_matches_known_sample() {
        let track_path = "/Contents/Artist/Album/track-001.mp3";
        let hash = usb_analysis_path_hash(track_path);
        assert_eq!(hash, 0x0002_F0E4);
        assert_eq!(usb_analysis_bucket_from_hash(hash), 0x02E);
    }

    #[test]
    fn canonical_analysis_bundle_paths_match_known_usb_sample() {
        let usb = Path::new("/mnt/usb");
        let track_path = "/Contents/Artist/Album/track-001.mp3";
        let (dat, ext, twoex) = canonical_analysis_bundle_paths(usb, track_path);
        assert_eq!(
            dat,
            Path::new("/mnt/usb/PIONEER/USBANLZ/P02E/0002F0E4/ANLZ0000.DAT")
        );
        assert_eq!(
            ext,
            Path::new("/mnt/usb/PIONEER/USBANLZ/P02E/0002F0E4/ANLZ0000.EXT")
        );
        assert_eq!(
            twoex,
            Path::new("/mnt/usb/PIONEER/USBANLZ/P02E/0002F0E4/ANLZ0000.2EX")
        );
    }

    #[test]
    fn usb_analysis_path_hash_matches_fixture_unicode_paths() {
        let app_path = "/Contents/Fixture Ö Artist/Fixture Ä Album/01 - Fixture Å Track.flac";
        let rb_collision_path =
            "/Contents/Fixture Ö Artist/Fixture Ä Album/01 - Fixture Å Track-1.flac";

        let app_hash = usb_analysis_path_hash(app_path);
        assert_eq!(app_hash, 0x0000_03B9);
        assert_eq!(usb_analysis_bucket_from_hash(app_hash), 0x019);

        let rb_hash = usb_analysis_path_hash(rb_collision_path);
        assert_eq!(rb_hash, 0x0000_8501);
        assert_eq!(usb_analysis_bucket_from_hash(rb_hash), 0x001);
    }
    use tempfile::tempdir;

    fn read_u32_be(data: &[u8], offset: usize) -> u32 {
        u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    }

    fn collect_chunk_tags(data: &[u8]) -> Vec<String> {
        let mut pos = 28;
        let mut tags = Vec::new();
        while pos + 12 <= data.len() {
            tags.push(String::from_utf8_lossy(&data[pos..pos + 4]).to_string());
            let total_len = read_u32_be(data, pos + 8) as usize;
            if total_len == 0 {
                break;
            }
            pos += total_len;
        }
        tags
    }

    fn verify_anlz_structure(data: &[u8], label: &str) {
        assert!(data.len() >= 28, "{label}: file too short for PMAI header");
        assert_eq!(&data[0..4], b"PMAI", "{label}: missing PMAI magic");

        let file_len_field = read_u32_be(data, 8) as usize;
        assert_eq!(
            file_len_field,
            data.len(),
            "{label}: file-level length field ({file_len_field}) != actual size ({})",
            data.len()
        );

        let mut pos = 28;
        let mut chunk_count = 0;
        while pos + 12 <= data.len() {
            let tag = &data[pos..pos + 4];
            let header_len = read_u32_be(data, pos + 4) as usize;
            let total_len = read_u32_be(data, pos + 8) as usize;

            assert!(
                header_len >= 12,
                "{label}: chunk {:?} header_len ({header_len}) < minimum 12",
                String::from_utf8_lossy(tag)
            );
            assert!(
                total_len >= header_len,
                "{label}: chunk {:?} total_len ({total_len}) < header_len ({header_len})",
                String::from_utf8_lossy(tag)
            );
            assert!(
                pos + total_len <= data.len(),
                "{label}: chunk {:?} at {pos} extends past file end ({} + {total_len} > {})",
                String::from_utf8_lossy(tag),
                pos,
                data.len()
            );

            pos += total_len;
            chunk_count += 1;
        }
        assert!(
            chunk_count > 0,
            "{label}: no chunks found after PMAI header"
        );
        assert_eq!(
            pos,
            data.len(),
            "{label}: {pos} bytes consumed but file is {} bytes",
            data.len()
        );
    }

    fn find_chunk_payload<'a>(data: &'a [u8], wanted: &str) -> Option<&'a [u8]> {
        let mut pos = 28;
        while pos + 12 <= data.len() {
            let tag = &data[pos..pos + 4];
            let header_len = read_u32_be(data, pos + 4) as usize;
            let total_len = read_u32_be(data, pos + 8) as usize;
            if tag == wanted.as_bytes() {
                return Some(&data[pos + header_len..pos + total_len]);
            }
            if total_len == 0 {
                break;
            }
            pos += total_len;
        }
        None
    }

    // --- DAT file ---

    #[test]
    fn dat_has_correct_chunk_order() {
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "/Contents/Test/track.mp3",
            Some(120.0),
            Some(30_000),
        );
        let tags = collect_chunk_tags(&dat);
        assert_eq!(
            tags,
            vec!["PPTH", "PVBR", "PQTZ", "PWAV", "PWV2", "PCOB", "PCOB"],
            "DAT chunk order must match reference"
        );
    }

    #[test]
    fn dat_without_path_skips_ppth() {
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            Some(120.0),
            Some(30_000),
        );
        let tags = collect_chunk_tags(&dat);
        assert_eq!(tags[0], "PVBR");
    }

    #[test]
    fn dat_pqtz_beat_count_matches_bpm_and_duration() {
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            Some(120.0),
            Some(30_000),
        );
        let pqtz = find_chunk_payload(&dat, "PQTZ").expect("PQTZ chunk");
        assert_eq!(pqtz.len(), 61 * 8, "120 BPM × 30s = 61 beats × 8 bytes");
    }

    #[test]
    fn dat_pqtz_beat_entries_have_correct_format() {
        let dat = build_anlz_dat_file_with_first_beat(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            Some(120.0),
            Some(10_000),
            Some(0),
        );
        let pqtz = find_chunk_payload(&dat, "PQTZ").expect("PQTZ chunk");
        assert_eq!(pqtz.len(), 21 * 8);

        let beat_num = u16::from_be_bytes([pqtz[0], pqtz[1]]);
        let tempo = u16::from_be_bytes([pqtz[2], pqtz[3]]);
        let time = u32::from_be_bytes([pqtz[4], pqtz[5], pqtz[6], pqtz[7]]);
        assert_eq!(beat_num, 1);
        assert_eq!(tempo, 12000);
        assert_eq!(time, 0);

        // Fifth beat: beat_number=1 (wraps 4→1), time ≈ 2000ms
        let beat5 = &pqtz[4 * 8..5 * 8];
        assert_eq!(u16::from_be_bytes([beat5[0], beat5[1]]), 1);
        assert_eq!(
            u32::from_be_bytes([beat5[4], beat5[5], beat5[6], beat5[7]]),
            2000
        );
    }

    #[test]
    fn dat_pwav_encoding() {
        let dat = build_anlz_dat_file(&WaveformData::from_peaks(vec![100; 100]), "", None, None);
        let pwav = find_chunk_payload(&dat, "PWAV").expect("PWAV");
        assert_eq!(pwav.len(), 400);
        for &b in pwav {
            let height = b & 0x1F;
            let whiteness = (b >> 5) & 0x07;
            assert!(height <= 31);
            assert!(whiteness <= 7);
        }
    }

    #[test]
    fn dat_pwv2_encoding() {
        let dat = build_anlz_dat_file(&WaveformData::from_peaks(vec![100; 100]), "", None, None);
        let pwv2 = find_chunk_payload(&dat, "PWV2").expect("PWV2");
        assert_eq!(pwv2.len(), 100);
        for &b in pwv2 {
            let height = b & 0x0F;
            let upper = (b >> 4) & 0x0F;
            assert!(height <= 15, "PWV2 height must be 0-15");
            assert_eq!(upper, 0, "PWV2 upper nibble must be 0");
        }
    }

    #[test]
    fn preview_resampling_preserves_transient_spikes() {
        let mut peaks = vec![8u8; 400];
        for i in (0..400).step_by(50) {
            peaks[i] = 100;
        }
        let out = peaks_to_absolute_levels(&peaks, 100, 1.0, 31);
        let strong_bins = out.iter().filter(|&&v| v >= 20).count();
        assert!(
            strong_bins >= 6,
            "expected preserved spikes, got {strong_bins} strong bins"
        );
    }

    #[test]
    fn energy_resampling_preserves_window_peaks() {
        let mut e = vec![5u8; 1200];
        for i in (20..1200).step_by(100) {
            e[i] = 120;
        }
        let out = resample_energy(&e, 120);
        let bright_bins = out.iter().filter(|&&v| v >= 60).count();
        assert!(
            bright_bins >= 8,
            "expected bright peak bins, got {bright_bins}"
        );
    }

    #[test]
    fn dat_pcob_chunks_present() {
        let dat = build_anlz_dat_file(&WaveformData::from_peaks(vec![128; 100]), "", None, None);
        let tags = collect_chunk_tags(&dat);
        let pcob_count = tags.iter().filter(|t| *t == "PCOB").count();
        assert_eq!(pcob_count, 2, "DAT should have 2 empty PCOB chunks");
    }

    // --- EXT file ---

    #[test]
    fn ext_has_correct_chunk_order() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "/Contents/Test/track.mp3",
            None,
            Some(30_000),
        );
        let tags = collect_chunk_tags(&ext);
        assert_eq!(
            tags,
            vec![
                "PPTH", "PWV3", "PCOB", "PCOB", "PCO2", "PCO2", "PQT2", "PWV5", "PWV4", "PSSI",
            ],
            "EXT chunk order must match reference"
        );
    }

    #[test]
    fn ext_pwv3_whiteness_uses_frequency_band_bits() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![80; 100]),
            "",
            None,
            Some(10_000),
        );
        let pwv3 = find_chunk_payload(&ext, "PWV3").expect("PWV3");
        let mut saw_non7 = false;
        for &b in pwv3 {
            let whiteness = (b >> 5) & 0x07;
            assert!(whiteness <= 5, "PWV3 whiteness out of expected range");
            if whiteness != 7 {
                saw_non7 = true;
            }
        }
        assert!(
            saw_non7,
            "PWV3 should carry varying whiteness/frequency bands"
        );
    }

    #[test]
    fn ext_pwv3_entry_count_scales_with_duration() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            None,
            Some(30_000),
        );
        let pwv3 = find_chunk_payload(&ext, "PWV3").expect("PWV3");
        assert_eq!(
            pwv3.len(),
            4504,
            "30s detail count follows ceil(duration × 150) + 4"
        );
    }

    #[test]
    fn ext_pwv5_same_entry_count_as_pwv3() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            None,
            Some(30_000),
        );
        let pwv3 = find_chunk_payload(&ext, "PWV3").expect("PWV3");
        let pwv5 = find_chunk_payload(&ext, "PWV5").expect("PWV5");
        assert_eq!(pwv5.len(), pwv3.len() * 2, "PWV5 = 2 bytes per PWV3 entry");
    }

    #[test]
    fn ext_pwv4_has_1200_entries() {
        let ext = build_anlz_ext_file(&WaveformData::from_peaks(vec![128; 100]), "", None, None);
        let pwv4 = find_chunk_payload(&ext, "PWV4").expect("PWV4");
        assert_eq!(pwv4.len(), 7200, "1200 × 6 bytes");
    }

    #[test]
    fn ext_pwv4_uses_nxs2_style_support_and_band_lanes() {
        // Bass-dominant input: high low energy, moderate mid, low high
        let waveform = WaveformData {
            peaks: vec![80; 200],
            bands: vec![0; 200],
            low_energy: vec![100; 200],
            mid_energy: vec![30; 200],
            high_energy: vec![10; 200],
            low_energy_full: vec![127; 200],
            mid_energy_full: vec![38; 200],
            high_energy_full: vec![13; 200],
            peak_level: 1.0,
        };
        let ext = build_anlz_ext_file(&waveform, "", None, Some(30_000));
        let pwv4 = find_chunk_payload(&ext, "PWV4").expect("PWV4");
        assert_eq!(pwv4.len(), 7200, "1200 × 6 bytes");

        // Check all entries stay in the expected NXS2-style lane ranges.
        let mut checked = 0;
        for entry in pwv4.chunks(6) {
            let b0 = entry[0]; // amplitude lane
            let b1 = entry[1]; // support/luminance lane
            let b2 = entry[2]; // background lane
            let b3 = entry[3]; // low lane
            let b4 = entry[4]; // mid
            let b5 = entry[5]; // front/intensity lane

            assert!(b0 <= 127, "byte0 out of range: {b0}");
            assert!(b2 <= 127, "byte2 out of range: {b2}");
            assert!(b3 <= 127, "byte3 out of range: {b3}");
            assert!(b4 <= 127, "byte4 out of range: {b4}");
            assert!(b5 <= 127, "byte5 out of range: {b5}");

            assert!(b1 >= 96, "support lane should keep a high baseline");
            if b3 > 0 || b4 > 0 || b5 > 0 {
                assert!(
                    b3 > b4,
                    "low ({b3}) should exceed mid ({b4}) for bass-dominant input"
                );
                assert!(b5 > 0, "front lane ({b5}) should remain populated");
                checked += 1;
            }
        }
        assert!(checked > 0, "should have non-zero frequency entries");
    }

    // --- 2EX file ---

    #[test]
    fn twoex_has_correct_chunk_order() {
        let twoex = build_anlz_2ex_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "/Contents/Test/track.mp3",
            Some(30_000),
        );
        let tags = collect_chunk_tags(&twoex);
        assert_eq!(
            tags,
            vec!["PPTH", "PWV7", "PWV6", "PWVC"],
            "2EX chunk order must match reference"
        );
    }

    #[test]
    fn twoex_pwv7_is_3_bytes_per_entry() {
        let twoex =
            build_anlz_2ex_file(&WaveformData::from_peaks(vec![128; 100]), "", Some(30_000));
        let pwv7 = find_chunk_payload(&twoex, "PWV7").expect("PWV7");
        assert_eq!(pwv7.len(), 4504 * 3);
    }

    #[test]
    fn twoex_pwv6_has_1200_entries() {
        let twoex = build_anlz_2ex_file(&WaveformData::from_peaks(vec![128; 100]), "", None);
        let pwv6 = find_chunk_payload(&twoex, "PWV6").expect("PWV6");
        assert_eq!(pwv6.len(), 1200 * 3, "1200 × 3 bytes");
    }

    #[test]
    fn twoex_pwv7_bass_dominant_input_uses_mid_high_low_order() {
        let waveform = WaveformData {
            peaks: vec![80; 100],
            bands: vec![0; 100],
            // Bass-dominant: high low energy, moderate mid, low high
            low_energy: vec![100; 100],
            mid_energy: vec![25; 100],
            high_energy: vec![10; 100],
            low_energy_full: vec![100; 100],
            mid_energy_full: vec![25; 100],
            high_energy_full: vec![10; 100],
            peak_level: 1.0,
        };
        let twoex = build_anlz_2ex_file(&waveform, "", Some(10_000));
        let pwv7 = find_chunk_payload(&twoex, "PWV7").expect("PWV7");
        for chunk in pwv7.chunks(3) {
            let (mid, high, low) = (chunk[0], chunk[1], chunk[2]);
            assert!(low >= mid, "bass input: low ({low}) should >= mid ({mid})");
            assert!(
                low >= high,
                "bass input: low ({low}) should >= high ({high})"
            );
        }
    }

    fn pulsed_waveform_for_band(
        bins: usize,
        bpm: f64,
        duration_ms: u64,
        first_beat_ms: u32,
        band: u8,
    ) -> WaveformData {
        let mut peaks = vec![5u8; bins];
        let mut low = vec![3u8; bins];
        let mut mid = vec![3u8; bins];
        let mut high = vec![3u8; bins];
        let interval = 60_000.0 / bpm;
        let mut t = first_beat_ms as f64;
        while t < duration_ms as f64 {
            let idx = ((t / duration_ms as f64) * bins as f64).round() as usize;
            if idx < bins {
                peaks[idx] = 90;
                match band {
                    0 => low[idx] = 110,
                    1 => mid[idx] = 110,
                    _ => high[idx] = 110,
                }
            }
            t += interval;
        }
        WaveformData {
            peaks,
            bands: vec![3; bins],
            low_energy: low.clone(),
            mid_energy: mid.clone(),
            high_energy: high.clone(),
            low_energy_full: low,
            mid_energy_full: mid,
            high_energy_full: high,
            peak_level: 1.0,
        }
    }

    #[test]
    fn estimate_first_beat_handles_low_band_driven_beats() {
        let wf = pulsed_waveform_for_band(6000, 140.0, 180_000, 52, 0);
        let got = estimate_first_beat_ms(&wf, Some(140.0), Some(180_000));
        assert!(got.abs_diff(52) <= 20, "expected ~52ms, got {got}ms");
    }

    #[test]
    fn estimate_first_beat_handles_mid_band_driven_beats() {
        let wf = pulsed_waveform_for_band(6000, 128.0, 210_000, 120, 1);
        let got = estimate_first_beat_ms(&wf, Some(128.0), Some(210_000));
        assert!(got.abs_diff(120) <= 25, "expected ~120ms, got {got}ms");
    }

    #[test]
    fn estimate_first_beat_handles_high_band_driven_beats() {
        let wf = pulsed_waveform_for_band(6000, 90.0, 200_000, 165, 2);
        let got = estimate_first_beat_ms(&wf, Some(90.0), Some(200_000));
        assert!(got.abs_diff(165) <= 25, "expected ~165ms, got {got}ms");
    }

    // --- Structure consistency ---

    #[test]
    fn dat_structure_consistent() {
        verify_anlz_structure(
            &build_anlz_dat_file(
                &WaveformData::from_peaks(vec![128; 512]),
                "",
                Some(120.0),
                Some(60_000),
            ),
            "DAT",
        );
    }

    #[test]
    fn ext_structure_consistent() {
        verify_anlz_structure(
            &build_anlz_ext_file(
                &WaveformData::from_peaks(vec![128; 512]),
                "",
                None,
                Some(60_000),
            ),
            "EXT",
        );
    }

    #[test]
    fn twoex_structure_consistent() {
        verify_anlz_structure(
            &build_anlz_2ex_file(&WaveformData::from_peaks(vec![128; 512]), "", Some(60_000)),
            "2EX",
        );
    }

    #[test]
    fn all_files_consistent_with_empty_peaks() {
        verify_anlz_structure(
            &build_anlz_dat_file(&WaveformData::empty(), "", None, None),
            "DAT-empty",
        );
        verify_anlz_structure(
            &build_anlz_ext_file(&WaveformData::empty(), "", None, None),
            "EXT-empty",
        );
        verify_anlz_structure(
            &build_anlz_2ex_file(&WaveformData::empty(), "", None),
            "2EX-empty",
        );
    }

    // --- PPTH ---

    #[test]
    fn ppth_encodes_utf16be_path() {
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 10]),
            "/Contents/Test/track.mp3",
            None,
            None,
        );
        let ppth = find_chunk_payload(&dat, "PPTH").expect("PPTH chunk");
        let u16s: Vec<u16> = ppth
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        let decoded = String::from_utf16(&u16s[..u16s.len() - 1]).expect("valid UTF-16");
        assert_eq!(decoded, "/Contents/Test/track.mp3");
    }

    #[test]
    fn ppth_encodes_unicode_usb_path_as_utf16be() {
        let path = "/Contents/Fixture Ö Artist/Fixture Ä Album/03 - Entä jos Fixture.flac";
        let dat = build_anlz_dat_file(&WaveformData::from_peaks(vec![128; 10]), path, None, None);
        assert_eq!(ppth_path_from_anlz(&dat).as_deref(), Some(path));
    }

    #[test]
    fn ensure_ppth_chunk_inserts_unicode_path_before_existing_chunks() {
        let path = "/Contents/Fixture Ö Artist/Fixture Ä Album/05 - Mitä Fixture.flac";
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 10]),
            "",
            Some(120.0),
            Some(30_000),
        );
        assert_eq!(
            collect_chunk_tags(&dat).first().map(String::as_str),
            Some("PVBR")
        );

        let fixed = ensure_ppth_chunk(&dat, path);
        let tags = collect_chunk_tags(&fixed);
        assert_eq!(tags.first().map(String::as_str), Some("PPTH"));
        assert_eq!(tags.get(1).map(String::as_str), Some("PVBR"));
        assert_eq!(ppth_path_from_anlz(&fixed).as_deref(), Some(path));
    }

    // --- Bundle writer ---

    #[test]
    fn write_bundle_creates_three_files() {
        let dir = tempdir().unwrap();
        let dat = dir.path().join("ANLZ0000.DAT");
        let ext = dir.path().join("ANLZ0000.EXT");
        let twoex = dir.path().join("ANLZ0000.2EX");

        write_generated_anlz_bundle(
            &WaveformData::from_peaks(vec![128; 100]),
            &dat,
            &ext,
            &twoex,
            "",
            None,
            None,
        )
        .unwrap();

        assert!(dat.is_file());
        assert!(ext.is_file());
        assert!(twoex.is_file());
        assert_eq!(&std::fs::read(&dat).unwrap()[0..4], b"PMAI");
        assert_eq!(&std::fs::read(&ext).unwrap()[0..4], b"PMAI");
        assert_eq!(&std::fs::read(&twoex).unwrap()[0..4], b"PMAI");
    }

    #[test]
    fn from_peaks_assigns_default_mid_band() {
        let waveform = WaveformData::from_peaks(vec![50, 75, 100]);
        assert_eq!(waveform.bands, vec![3, 3, 3]);
    }
}
