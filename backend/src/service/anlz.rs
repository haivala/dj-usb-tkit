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

use super::anlz_seek::{PVBR_PAYLOAD_LEN, SeekIndex, SeekIndexSkip, seek_index_for_audio};
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

    /// Test fixture: WaveformData from amplitude-only peaks (mid band=3), with
    /// 3-band data derived from the single amplitude using a mid-dominant split.
    #[cfg(test)]
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

/// Output file paths for a generated ANLZ bundle (DAT/EXT/2EX).
pub struct AnlzBundlePaths {
    pub dat_path: PathBuf,
    pub ext_path: PathBuf,
    pub twoex_path: PathBuf,
}

pub fn write_generated_anlz_bundle(
    waveform: &WaveformData,
    paths: &AnlzBundlePaths,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: u64,
) -> BackendResult<()> {
    write_generated_anlz_bundle_with_first_beat(
        waveform,
        paths,
        track_path,
        bpm,
        duration_ms,
        None,
        &[],
    )
}

pub fn write_generated_anlz_bundle_with_first_beat(
    waveform: &WaveformData,
    paths: &AnlzBundlePaths,
    track_path: &str,
    bpm: Option<f64>,
    duration_ms: u64,
    first_beat_ms_override: Option<u32>,
    cues: &[AnlzCue],
) -> BackendResult<()> {
    let dat = build_anlz_dat_file(
        waveform,
        track_path,
        bpm,
        duration_ms,
        first_beat_ms_override,
        cues,
    );
    let ext = build_anlz_ext_file(
        waveform,
        track_path,
        bpm,
        duration_ms,
        first_beat_ms_override,
        cues,
    );
    let twoex = build_anlz_2ex_file(waveform, track_path, duration_ms);
    if let Some(parent) = paths.dat_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write_bytes(&paths.dat_path, &dat)?;
    atomic_write_bytes(&paths.ext_path, &ext)?;
    atomic_write_bytes(&paths.twoex_path, &twoex)?;
    Ok(())
}

pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> BackendResult<()> {
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

// PQT2 — extended beat grid (`.EXT`), as rekordbox writes it (checked against
// 1525 rekordbox bundles):
//
// len_header = 0x38 (56)
// Offset 12-15: 0
// Offset 16-19: 0x01000002
// Offset 20-23: 0
// Offset 24-31: first beat (beat_number u16, tempo u16, time_ms u32), as in PQTZ
// Offset 32-39: last beat, same layout
// Offset 40-43: beat count (= PQTZ's)
// Offset 44-47: checksum: Σ of beat_number + tempo + time_ms over every PQTZ beat
// Offset 48-55: 0
// Offset 56+:   one u16 per beat: the beat time's microseconds (0-999); PQTZ
//               holds the floored milliseconds of the same time.
//
// Before 0.3.7 the app wrote the checksum as 0 and `(i % 4, 0)` as the body.

fn append_pqt2_chunk(file: &mut Vec<u8>, bpm: Option<f64>, duration_ms: u64, first_beat_ms: u32) {
    if let Some(grid) = BeatGrid::from_tempo(bpm, duration_ms, first_beat_ms) {
        append_pqt2_grid(file, &grid);
    }
}

fn append_pqt2_grid(file: &mut Vec<u8>, grid: &BeatGrid) {
    let beats: Vec<GridBeat> = grid.beats().collect();
    let (Some(first), Some(last)) = (beats.first(), beats.last()) else {
        return;
    };
    let mut header = Vec::<u8>::with_capacity(44);
    header.extend_from_slice(&0u32.to_be_bytes());
    header.extend_from_slice(&0x01000002u32.to_be_bytes());
    header.extend_from_slice(&0u32.to_be_bytes());
    for beat in [first, last] {
        header.extend_from_slice(&beat.entry());
    }
    header.extend_from_slice(&grid.num_beats.to_be_bytes());
    header.extend_from_slice(&beat_grid_checksum(&beats).to_be_bytes());
    header.extend_from_slice(&[0u8; 8]);

    let mut payload = Vec::<u8>::with_capacity(beats.len() * 2);
    for beat in &beats {
        payload.extend_from_slice(&beat.micros.to_be_bytes());
    }
    append_anlz_chunk(file, b"PQT2", &header, &payload);
}

/// A constant-tempo beat grid: the one source of the `PQTZ` and `PQT2`
/// chunks, so the two always describe the same beats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BeatGrid {
    /// Position in the bar (1-4) of the first beat.
    first_beat_number: u16,
    tempo_x100: u16,
    first_us: u64,
    interval_us: f64,
    num_beats: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridBeat {
    number: u16,
    tempo_x100: u16,
    time_ms: u32,
    /// The microseconds below `time_ms` (0-999).
    micros: u16,
}

impl GridBeat {
    /// The 8-byte `PQTZ` entry (also `PQT2`'s first/last beat).
    fn entry(&self) -> [u8; 8] {
        let mut entry = [0u8; 8];
        entry[0..2].copy_from_slice(&self.number.to_be_bytes());
        entry[2..4].copy_from_slice(&self.tempo_x100.to_be_bytes());
        entry[4..8].copy_from_slice(&self.time_ms.to_be_bytes());
        entry
    }
}

impl BeatGrid {
    /// The grid analysis and export write; `None` without a known tempo
    /// (never a made-up grid).
    fn from_tempo(bpm: Option<f64>, duration_ms: u64, first_beat_ms: u32) -> Option<Self> {
        let bpm = bpm.filter(|b| *b > 0.0)?;
        let interval_ms = 60_000.0 / bpm;
        let num_beats = compute_num_beats(duration_ms as f64, interval_ms, first_beat_ms);
        (num_beats > 0).then_some(Self {
            first_beat_number: 1,
            tempo_x100: (bpm * 100.0).round() as u16,
            first_us: u64::from(first_beat_ms) * 1000,
            interval_us: interval_ms * 1000.0,
            num_beats,
        })
    }

    fn beats(&self) -> impl Iterator<Item = GridBeat> + '_ {
        (0..self.num_beats).map(move |i| {
            let time_us = self.first_us + (f64::from(i) * self.interval_us).round() as u64;
            GridBeat {
                number: ((u32::from(self.first_beat_number) - 1 + i) % 4 + 1) as u16,
                tempo_x100: self.tempo_x100,
                time_ms: (time_us / 1000) as u32,
                micros: (time_us % 1000) as u16,
            }
        })
    }
}

fn beat_grid_checksum(beats: &[GridBeat]) -> u32 {
    beats.iter().fold(0u32, |sum, beat| {
        sum.wrapping_add(u32::from(beat.number))
            .wrapping_add(u32::from(beat.tempo_x100))
            .wrapping_add(beat.time_ms)
    })
}

/// The `PQTZ` beats of a `.DAT` as (beat_number, tempo, time_ms).
fn pqtz_beats(dat: &[u8]) -> Option<Vec<(u16, u16, u32)>> {
    let (_, range) = anlz_chunk_ranges(dat)?
        .into_iter()
        .find(|(tag, range)| tag == b"PQTZ" && range.len() >= 24)?;
    let header_len = read_u32_be_at(dat, range.start + 4)? as usize;
    let body = dat.get(range.start + header_len..range.end)?;
    Some(
        body.chunks_exact(8)
            .map(|e| {
                (
                    u16::from_be_bytes([e[0], e[1]]),
                    u16::from_be_bytes([e[2], e[3]]),
                    u32::from_be_bytes([e[4], e[5], e[6], e[7]]),
                )
            })
            .collect(),
    )
}

/// Whether an `.EXT`'s `PQT2` checksum matches its `.DAT`'s `PQTZ` beats;
/// `None` when either chunk is missing or a file doesn't walk cleanly.
/// rekordbox's own bundles always match.
pub(crate) fn pqt2_checksum_ok(dat: &[u8], ext: &[u8]) -> Option<bool> {
    let beats = pqtz_beats(dat)?;
    let (_, range) = anlz_chunk_ranges(ext)?
        .into_iter()
        .find(|(tag, range)| tag == b"PQT2" && range.len() >= 56)?;
    let sum = beats.iter().fold(0u32, |sum, (number, tempo, time)| {
        sum.wrapping_add(u32::from(*number))
            .wrapping_add(u32::from(*tempo))
            .wrapping_add(*time)
    });
    Some(read_u32_be_at(ext, range.start + 44)? == sum)
}

/// The constant-tempo grid a `.DAT`'s `PQTZ` describes, to rewrite it (and
/// `PQT2`) in rekordbox's format. `None` unless every beat has the same
/// tempo, the beat numbers cycle through the bar, and every beat lies within
/// 1 ms of the regenerated one -- a grid this app could have written.
pub(crate) fn beat_grid_from_pqtz(dat: &[u8]) -> Option<BeatGrid> {
    let beats = pqtz_beats(dat)?;
    let &(first_number, tempo_x100, first_ms) = beats.first()?;
    let &(_, _, last_ms) = beats.last()?;
    if tempo_x100 == 0 || !(1..=4).contains(&first_number) {
        return None;
    }
    let base = BeatGrid {
        first_beat_number: first_number,
        tempo_x100,
        first_us: u64::from(first_ms) * 1000,
        interval_us: 6_000_000_000.0 / f64::from(tempo_x100),
        num_beats: u32::try_from(beats.len()).ok()?,
    };
    let fits = |grid: &BeatGrid| {
        grid.beats()
            .zip(&beats)
            .all(|(new, &(number, tempo, time))| {
                new.number == number && new.tempo_x100 == tempo && new.time_ms.abs_diff(time) <= 1
            })
    };
    // The tempo field is rounded to 0.01 BPM; when the beats don't follow it
    // exactly, keep their own spacing instead.
    let measured = (beats.len() > 1).then(|| BeatGrid {
        interval_us: f64::from(last_ms - first_ms) * 1000.0 / (beats.len() - 1) as f64,
        ..base
    });
    [Some(base), measured].into_iter().flatten().find(fits)
}

/// What [`upgrade_bundle_beat_grid`] did with a bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridUpgrade {
    Rewritten,
    /// Already in rekordbox's format (or no beat grid at all).
    AlreadyCurrent,
    /// In the old format, but not one constant-tempo grid this app could have
    /// written; left as it is.
    NotRebuildable,
}

/// Rewrite a bundle's `PQTZ` (`.DAT`) and `PQT2` (sibling `.EXT`) in
/// rekordbox's format when either is in the pre-0.3.7 one, from the grid the
/// `.DAT` already holds. Safe to call again: a current bundle is left alone.
/// Shared by the USB repair "Fix Beat Grid" and the cache migration.
pub(crate) fn upgrade_bundle_beat_grid(dat_path: &Path) -> BackendResult<GridUpgrade> {
    let dat = std::fs::read(dat_path)?;
    let ext_path = dat_path.with_extension("EXT");
    let ext = match std::fs::read(&ext_path) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    let old_format = has_misplaced_pqtz_header(&dat)
        || ext
            .as_ref()
            .is_some_and(|ext| pqt2_checksum_ok(&dat, ext) == Some(false));
    if !old_format {
        return Ok(GridUpgrade::AlreadyCurrent);
    }
    let Some(grid) = beat_grid_from_pqtz(&dat) else {
        return Ok(GridUpgrade::NotRebuildable);
    };
    if let Some(patched) = with_beat_grid(&dat, &grid) {
        atomic_write_bytes(dat_path, &patched)?;
    }
    if let Some(ext) = ext
        && let Some(patched) = with_beat_grid(&ext, &grid)
    {
        atomic_write_bytes(&ext_path, &patched)?;
    }
    Ok(GridUpgrade::Rewritten)
}

/// `data` with its `PQTZ` and `PQT2` (whichever it has) rebuilt from `grid`;
/// every other chunk is kept byte for byte. `None` when the file doesn't walk
/// cleanly or has neither chunk.
pub(crate) fn with_beat_grid(data: &[u8], grid: &BeatGrid) -> Option<Vec<u8>> {
    let chunks = anlz_chunk_ranges(data)?;
    if !chunks
        .iter()
        .any(|(tag, _)| tag == b"PQTZ" || tag == b"PQT2")
    {
        return None;
    }
    let mut out = data[..28].to_vec();
    for (tag, range) in chunks {
        match &tag {
            b"PQTZ" => append_pqtz_grid(&mut out, grid),
            b"PQT2" => append_pqt2_grid(&mut out, grid),
            _ => out.extend_from_slice(&data[range]),
        }
    }
    let file_len = out.len() as u32;
    out[8..12].copy_from_slice(&file_len.to_be_bytes());
    Some(out)
}

fn normalize_first_beat_ms(first_beat_ms: u32, bpm: Option<f64>) -> u32 {
    let Some(bpm_val) = bpm.filter(|b| *b > 0.0) else {
        return first_beat_ms;
    };
    let interval_ms = 60_000.0 / bpm_val;
    if !interval_ms.is_finite() || interval_ms <= 1.0 {
        return 0;
    }
    let wrapped = (first_beat_ms as f64) % interval_ms;
    wrapped.round().max(0.0) as u32
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

/// Post-analysis edits to fold into an already-generated ANLZ file.
///
/// `None` fields leave the corresponding chunks untouched; the walk in
/// [`apply_analysis_edits_to_anlz`] only rebuilds chunk types that already
/// exist in the source file, and never touches `PSSI`, waveform, or `PPTH`.
pub struct AnlzAnalysisEdits<'a> {
    /// `Some` ⇒ rebuild `PQTZ`/`PQT2` with this tempo, even with no explicit
    /// `first_beat_ms` (the existing anchor already in the file is reused —
    /// see `apply_analysis_edits_to_anlz`).
    pub bpm: Option<f64>,
    /// The grid is only rebuilt when this is known: its length is the beat count.
    pub duration_ms: Option<u64>,
    /// `Some` ⇒ rebuild `PQTZ`/`PQT2` with this beat-grid anchor (only when
    /// `bpm` and `duration_ms` are known too).
    pub first_beat_ms: Option<u32>,
    /// `Some` ⇒ rebuild `PCOB`/`PCO2` from this cue list (full replace).
    pub cues: Option<&'a [AnlzCue]>,
}

/// Rewrite the beat-grid (`PQTZ`/`PQT2`) and/or cue (`PCOB`/`PCO2`) chunks of an
/// existing `PMAI` file in place, preserving every other chunk and its order.
///
/// Mirrors [`ensure_ppth_chunk`]'s chunk walk. `PSSI` phrase data is copied
/// verbatim — this app never writes one (it doesn't analyze phrases), so any
/// `PSSI` present came from rekordbox.
///
/// The beat grid is rebuilt only when `edits.bpm` and `edits.duration_ms` are
/// known: with an explicit
/// `edits.first_beat_ms`, or with no explicit anchor — in that second case the
/// anchor already embedded in `data` is reused, so a bpm-only correction (a
/// track re-analyzed to the right tempo but with no confident first-beat, or
/// no cue/first-beat edit at all) still lands on disk instead of silently
/// leaving a stale/default beat grid in place.
pub fn apply_analysis_edits_to_anlz(data: &[u8], edits: &AnlzAnalysisEdits<'_>) -> Vec<u8> {
    if data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return data.to_vec();
    }
    // The grid is only rebuilt with a known tempo and length; without them
    // the existing grid chunks are kept as they are.
    let beatgrid_duration_ms = edits
        .duration_ms
        .filter(|_| edits.bpm.is_some_and(|b| b > 0.0));
    let beatgrid_anchor_ms = beatgrid_duration_ms.and_then(|_| {
        edits
            .first_beat_ms
            .or_else(|| read_first_beat_from_anlz(data))
    });
    if beatgrid_anchor_ms.is_none() && edits.cues.is_none() {
        return data.to_vec();
    }

    let rebuilt_first_beat = beatgrid_anchor_ms.map(|raw| normalize_first_beat_ms(raw, edits.bpm));

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..28]);

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

        let fourcc = &data[pos..pos + 4];
        let original = &data[pos..pos + total_len];
        let mut replacement: Option<Vec<u8>> = None;

        if let (Some(first_beat_ms), Some(duration_ms)) = (rebuilt_first_beat, beatgrid_duration_ms)
        {
            if fourcc == b"PQTZ" {
                let mut chunk = Vec::new();
                append_pqtz_chunk(&mut chunk, edits.bpm, duration_ms, first_beat_ms);
                if !chunk.is_empty() {
                    replacement = Some(chunk);
                }
            } else if fourcc == b"PQT2" {
                let mut chunk = Vec::new();
                append_pqt2_chunk(&mut chunk, edits.bpm, duration_ms, first_beat_ms);
                if !chunk.is_empty() {
                    replacement = Some(chunk);
                }
            }
        }

        if let Some(cues) = edits.cues
            && (fourcc == b"PCOB" || fourcc == b"PCO2")
            && let Some(cue_type) = read_u32_be_at(data, pos + 12)
        {
            let mut chunk = Vec::new();
            if fourcc == b"PCOB" {
                append_pcob_chunk(&mut chunk, cue_type, cues);
            } else {
                append_pco2_chunk(&mut chunk, cue_type, cues);
            }
            if !chunk.is_empty() {
                replacement = Some(chunk);
            }
        }

        match replacement {
            Some(chunk) => out.extend_from_slice(&chunk),
            None => out.extend_from_slice(original),
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
                .as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| u16::from_be_bytes(*chunk))
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
// PVBR — MP3 seek index, deliberately left empty
// ===========================================================================
//
// len_header = 0x10 (16)
// Offset 12-15: unknown (4 bytes, 0)
// Offset 16+:   400 u32 byte offsets + u32 total samples (1604 bytes)
//
// All zeros is what rekordbox itself writes for non-MP3 files and, apart from the
// total, for CBR MP3s. Generated bundles keep it empty; the USB repair "Add
// Missing Seek Data" fills it (and `.EXT`'s `PVB2` for FLACs) in on the stick from
// the audio file, via [`with_seek_index`]. The rules are in docs/WAVEFORMS.md
// ("Seek-index chunks").

fn append_pvbr_chunk(file: &mut Vec<u8>) {
    let header = vec![0u8; 4]; // unknown1 = 0
    let payload = vec![0u8; PVBR_PAYLOAD_LEN]; // empty index and total
    append_anlz_chunk(file, b"PVBR", &header, &payload);
}

/// What [`fill_mp3_pvbr`] did with a `.DAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PvbrFill {
    Filled,
    /// The `PVBR` already has a sample total (rekordbox's, or filled before).
    AlreadySet,
    /// Not an MP3: the empty `PVBR` is what rekordbox writes too.
    NotMp3,
    /// The `.DAT` has no well-formed `PVBR` to fill.
    NoPvbrChunk,
    /// An MP3 whose seek index can't be reproduced exactly; the `PVBR`
    /// stays empty rather than guessed.
    Skipped(SeekIndexSkip),
}

/// Fill an empty `PVBR` in a `.DAT` from its source MP3, in place: the 400
/// offsets (zero for CBR) and the sample total rekordbox writes. Only MP3s
/// get one; FLAC's `PVB2` is left to the USB repair (its long-track rule is
/// not exact). Safe to call again.
pub(super) fn fill_mp3_pvbr(dat_path: &Path, audio_path: &Path) -> BackendResult<PvbrFill> {
    let fill = try_fill_mp3_pvbr(dat_path, audio_path);
    match &fill {
        Ok(PvbrFill::Skipped(skip)) => crate::backend_log!(
            Info,
            "anlz",
            "MP3 seek index not written ({}): {}",
            skip.describe(),
            audio_path.display()
        ),
        Err(err) => crate::backend_log!(
            Warn,
            "anlz",
            "MP3 seek index not written ({err}): {}",
            audio_path.display()
        ),
        _ => {}
    }
    fill
}

fn try_fill_mp3_pvbr(dat_path: &Path, audio_path: &Path) -> BackendResult<PvbrFill> {
    let is_mp3 = audio_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp3"));
    if !is_mp3 {
        return Ok(PvbrFill::NotMp3);
    }
    let dat = std::fs::read(dat_path)?;
    match pvbr_total_samples(&dat) {
        Some(0) => {}
        Some(_) => return Ok(PvbrFill::AlreadySet),
        None => return Ok(PvbrFill::NoPvbrChunk),
    }
    let index = match seek_index_for_audio(audio_path) {
        Ok(Some(index @ SeekIndex::Pvbr(_))) => index,
        Ok(_) => return Ok(PvbrFill::NotMp3),
        Err(skip) => return Ok(PvbrFill::Skipped(skip)),
    };
    let Some(patched) = with_seek_index(&dat, &index) else {
        return Ok(PvbrFill::NoPvbrChunk);
    };
    atomic_write_bytes(dat_path, &patched)?;
    Ok(PvbrFill::Filled)
}

/// Every chunk of a `PMAI` file as (fourcc, byte range). `None` unless the
/// whole file walks cleanly, so a patch never lands in a damaged file.
fn anlz_chunk_ranges(data: &[u8]) -> Option<Vec<([u8; 4], std::ops::Range<usize>)>> {
    if data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return None;
    }
    let mut chunks = Vec::new();
    let mut pos = 28usize;
    while pos < data.len() {
        let header_len = read_u32_be_at(data, pos + 4)? as usize;
        let total_len = read_u32_be_at(data, pos + 8)? as usize;
        if header_len < 12 || total_len < header_len || pos + total_len > data.len() {
            return None;
        }
        chunks.push((data[pos..pos + 4].try_into().ok()?, pos..pos + total_len));
        pos += total_len;
    }
    Some(chunks)
}

/// The total sample count in a `.DAT`'s `PVBR`; `None` without a
/// well-formed one. Zero means the seek index is empty.
pub(crate) fn pvbr_total_samples(dat: &[u8]) -> Option<u32> {
    let (_, range) = anlz_chunk_ranges(dat)?
        .into_iter()
        .find(|(tag, range)| tag == b"PVBR" && range.len() == 16 + PVBR_PAYLOAD_LEN)?;
    read_u32_be_at(dat, range.end - 4)
}

/// Whether an `.EXT` has a `PVB2`; `None` when the file doesn't walk cleanly.
pub(crate) fn has_pvb2_chunk(ext: &[u8]) -> Option<bool> {
    anlz_chunk_ranges(ext).map(|chunks| chunks.iter().any(|(tag, _)| tag == b"PVB2"))
}

/// `data` with its seek index set: a `.DAT`'s `PVBR` payload replaced in
/// place, or an `.EXT`'s `PVB2` replaced or appended at the end (where
/// rekordbox writes it). Every other chunk is kept byte for byte. `None` when
/// the file doesn't walk cleanly or a `.DAT` has no `PVBR` to fill.
pub(super) fn with_seek_index(data: &[u8], index: &SeekIndex) -> Option<Vec<u8>> {
    let chunks = anlz_chunk_ranges(data)?;
    match index {
        SeekIndex::Pvbr(payload) => {
            let (_, range) = chunks
                .into_iter()
                .find(|(tag, range)| tag == b"PVBR" && range.len() == 16 + PVBR_PAYLOAD_LEN)?;
            if payload.len() != PVBR_PAYLOAD_LEN {
                return None;
            }
            let mut out = data.to_vec();
            out[range.start + 16..range.end].copy_from_slice(payload);
            Some(out)
        }
        SeekIndex::Pvb2(chunk) => {
            let mut out = data[..28].to_vec();
            for (tag, range) in chunks {
                if &tag != b"PVB2" {
                    out.extend_from_slice(&data[range]);
                }
            }
            append_anlz_chunk(&mut out, b"PVB2", &chunk.header, &chunk.payload);
            Some(out)
        }
    }
}

// ===========================================================================
// PQTZ — beat grid
// ===========================================================================
//
// len_header = 0x18 (24)
// Offset 12-15: unknown1 (4 bytes, 0)
// Offset 16-19: unknown2 (4 bytes, 0x00080000 in every rekordbox export)
// Offset 20-23: len_beats (4 bytes)
// Offset 24+:   beat entries (8 bytes each)
//
// Before 0.3.7 the app wrote unknown2 two bytes early (offsets 14-17). The
// USB repair "Fix Beat Grid" rewrites such grids on the stick
// ([`with_beat_grid`]).
//
// Beat entry:
//   0-1: beat_number (u16, 1-4 position in measure)
//   2-3: tempo       (u16, BPM × 100)
//   4-7: time        (u32, milliseconds)

const PQTZ_UNKNOWN2: u32 = 0x0008_0000;
/// Chunk bytes 12-19 as the app wrote them before 0.3.7 (unknown2 two bytes
/// early).
const PQTZ_HEADER_MISPLACED: [u8; 8] = [0, 0, 0, 0x08, 0, 0, 0, 0];

/// Whether a `.DAT`'s `PQTZ` header has the pre-0.3.7 misplaced layout.
/// Works on the start of the file too: `PQTZ` follows `PPTH` and `PVBR`,
/// about 2 KB in, so diagnostics read only the first [`PQTZ_PROBE_BYTES`].
pub(crate) fn has_misplaced_pqtz_header(dat: &[u8]) -> bool {
    if dat.get(0..4) != Some(b"PMAI") {
        return false;
    }
    let mut pos = 28usize;
    while let (Some(header_len), Some(total_len)) =
        (read_u32_be_at(dat, pos + 4), read_u32_be_at(dat, pos + 8))
    {
        if header_len < 12 || total_len < header_len {
            return false;
        }
        if &dat[pos..pos + 4] == b"PQTZ" {
            return header_len == 24
                && dat.get(pos + 12..pos + 20) == Some(&PQTZ_HEADER_MISPLACED[..]);
        }
        pos += total_len as usize;
    }
    false
}

/// How much of a `.DAT` [`has_misplaced_pqtz_header`] needs to see.
pub(crate) const PQTZ_PROBE_BYTES: u64 = 4096;

fn append_pqtz_chunk(file: &mut Vec<u8>, bpm: Option<f64>, duration_ms: u64, first_beat_ms: u32) {
    if let Some(grid) = BeatGrid::from_tempo(bpm, duration_ms, first_beat_ms) {
        append_pqtz_grid(file, &grid);
    }
}

fn append_pqtz_grid(file: &mut Vec<u8>, grid: &BeatGrid) {
    // Header content: 12 bytes (offsets 12-23 in chunk)
    let mut header = vec![0u8; 12];
    // [0..4] = unknown1 = 0
    header[4..8].copy_from_slice(&PQTZ_UNKNOWN2.to_be_bytes());
    header[8..12].copy_from_slice(&grid.num_beats.to_be_bytes());

    let mut payload = Vec::with_capacity(grid.num_beats as usize * 8);
    for beat in grid.beats() {
        payload.extend_from_slice(&beat.entry());
    }
    append_anlz_chunk(file, b"PQTZ", &header, &payload);
}

fn estimate_first_beat_ms(waveform: &WaveformData, bpm: Option<f64>, duration_ms: u64) -> u32 {
    let Some(bpm_val) = bpm.filter(|b| *b > 0.0) else {
        return 0;
    };
    let dur_ms = duration_ms as f64;
    if waveform.peaks.is_empty() || dur_ms <= 0.0 {
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
// Cue points — PCOB/PCPT (basic) and PCO2/PCP2 (extended, with colour + comment)
// ===========================================================================
//
// PCOB header (len_header = 0x18 / 24):
//   abs 12-15: type          (0 = memory points, 1 = hot cues)
//   abs 16-17: unknown       (0)
//   abs 18-19: len_cues      (entry count)
//   abs 20-23: memory_count  (real Rekordbox always writes 0xFFFFFFFF for
//                             the hot list, regardless of entry count;
//                             for the memory list it writes `count - 1`
//                             when non-empty, 0xFFFFFFFF when empty — the
//                             `count - 1` case is confirmed only from a
//                             single real 1-entry sample)
//   abs 24+  : PCPT entries
//
// PCPT entry (len_header = 0x1C / 28, len_entry = 0x38 / 56, fixed):
//   abs 12-15: hot_cue      (0 = memory; 1 = hot A, 2 = B, …)
//   abs 16-19: status       (always 0, memory and hot alike)
//   abs 20-23: unknown1     (0x00010000, observed constant)
//   abs 24-25: order_first  (memory: 1-based ordinal; hot: 0xFFFF)
//   abs 26-27: order_last   (same as order_first)
//   abs 28   : type         (1 = point)
//   abs 29-31: reserved     (0x0003E8, observed constant, same value PCP2
//                            carries at abs 17-19)
//   abs 32-35: time         (ms)
//   abs 36-39: loop_time    (0xFFFFFFFF, no loop)
//   abs 40-55: padding
//
// PCO2 header (len_header = 0x14 / 20):
//   abs 12-15: type
//   abs 16-17: len_cues
//   abs 18-19: unknown (0)
//   abs 20+  : PCP2 entries
//
// PCP2 entry (len_header = 0x10 / 16, len_entry variable):
//   abs 12-15: hot_cue
//   abs 16   : type (1 = point)
//   abs 17-19: reserved (0x0003E8, observed constant)
//   abs 20-23: time (ms)
//   abs 24-27: loop_time (0xFFFFFFFF)
//   abs 28   : color_id (palette index; 0 for memory)
//   abs 29   : reserved (observed constant `1` on every real sample, all
//              with color_id 0; not cross-checked against a non-zero color)
//   abs 30-35: padding
//   abs 36-37: loop_numerator (0)
//   abs 38-39: loop_denominator (0)
//   abs 40-43: len_comment  ((utf16_units + 1) * 2, or 0)
//   abs 44…  : comment (UTF-16BE, NUL-terminated)
//   then     : color_code, color_red, color_green, color_blue (u8 each)
//   then     : zero padding to a 4-byte multiple

/// A cue point ready for ANLZ encoding. `hot_cue` is 0 for a memory point or the
/// 1-based pad slot (1 = A … 8 = H) for a hot cue.
#[derive(Debug, Clone, Default)]
pub struct AnlzCue {
    pub position_ms: u32,
    pub hot_cue: u32,
    pub color_id: u8,
    pub color_rgb: (u8, u8, u8),
    pub color_code: u8,
    pub comment: String,
}

impl AnlzCue {
    fn is_hot(&self) -> bool {
        self.hot_cue != 0
    }
}

/// Cues of one list type (hot when `hot` is true, memory otherwise), ordered the
/// way Rekordbox writes them: hot cues by pad slot, memory points by time.
fn cues_for_type(cues: &[AnlzCue], hot: bool) -> Vec<&AnlzCue> {
    let mut out: Vec<&AnlzCue> = cues.iter().filter(|c| c.is_hot() == hot).collect();
    if hot {
        out.sort_by_key(|c| c.hot_cue);
    } else {
        out.sort_by_key(|c| c.position_ms);
    }
    out
}

fn build_pcpt_entry(cue: &AnlzCue, hot: bool, ordinal: u16) -> Vec<u8> {
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(&cue.hot_cue.to_be_bytes()); // hot_cue
    header.extend_from_slice(&0u32.to_be_bytes()); // status (always 0, hot and memory alike)
    header.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // unknown1
    let order = if hot { 0xFFFFu16 } else { ordinal };
    header.extend_from_slice(&order.to_be_bytes()); // order_first
    header.extend_from_slice(&order.to_be_bytes()); // order_last

    let mut payload = vec![0u8; 28];
    payload[0] = 1; // type = point
    payload[1..4].copy_from_slice(&[0x00, 0x03, 0xe8]); // reserved (observed constant, 1000)
    payload[4..8].copy_from_slice(&cue.position_ms.to_be_bytes());
    payload[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // loop_time

    let mut entry = Vec::new();
    append_anlz_chunk(&mut entry, b"PCPT", &header, &payload);
    entry
}

fn build_pcp2_entry(cue: &AnlzCue) -> Vec<u8> {
    let header = cue.hot_cue.to_be_bytes().to_vec(); // hot_cue

    let comment_utf16: Vec<u16> = cue.comment.encode_utf16().collect();
    let len_comment = if comment_utf16.is_empty() {
        0u32
    } else {
        ((comment_utf16.len() + 1) * 2) as u32
    };

    let mut payload = vec![0u8; 28]; // abs 16..44 fixed portion
    payload[0] = 1; // type = point (abs 16)
    payload[1..4].copy_from_slice(&[0x00, 0x03, 0xe8]); // reserved (observed constant, 1000; abs 17)
    payload[4..8].copy_from_slice(&cue.position_ms.to_be_bytes()); // time (abs 20)
    payload[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // loop_time (abs 24)
    payload[12] = cue.color_id; // color_id (abs 28)
    // abs 29: observed constant `1` on every real sample (all with color_id
    // 0); not cross-checked against a genuine non-zero-color entry.
    payload[13] = 1;
    // payload[20..22] loop_numerator (abs 36), payload[22..24] loop_denominator
    // (abs 38) stay zero.
    payload[24..28].copy_from_slice(&len_comment.to_be_bytes()); // len_comment (abs 40)

    for unit in &comment_utf16 {
        payload.extend_from_slice(&unit.to_be_bytes());
    }
    if !comment_utf16.is_empty() {
        payload.extend_from_slice(&0u16.to_be_bytes()); // NUL terminator
    }

    payload.push(cue.color_code);
    payload.push(cue.color_rgb.0);
    payload.push(cue.color_rgb.1);
    payload.push(cue.color_rgb.2);

    while !(payload.len() + 16).is_multiple_of(4) {
        payload.push(0);
    }

    let mut entry = Vec::new();
    append_anlz_chunk(&mut entry, b"PCP2", &header, &payload);
    entry
}

/// Decode every cue point from an ANLZ file (`.DAT` or `.EXT`).
///
/// Prefers the extended `PCO2`/`PCP2` chunks (colour + comment); falls back to
/// `PCOB`/`PCPT` (position + hot slot only) for older bundles that lack `PCO2`.
/// Used on USB re-import so cues the app itself exported survive round-trips.
pub fn read_cues_from_anlz(data: &[u8]) -> Vec<AnlzCue> {
    if data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return Vec::new();
    }

    let mut from_pco2: Vec<AnlzCue> = Vec::new();
    let mut from_pcob: Vec<AnlzCue> = Vec::new();

    let mut pos = 28usize;
    while pos + 12 <= data.len() {
        let fourcc = &data[pos..pos + 4];
        let Some(header_len) = read_u32_be_at(data, pos + 4).map(|v| v as usize) else {
            break;
        };
        let Some(total_len) = read_u32_be_at(data, pos + 8).map(|v| v as usize) else {
            break;
        };
        if header_len < 12 || total_len < header_len || pos + total_len > data.len() {
            break;
        }

        if fourcc == b"PCO2" || fourcc == b"PCOB" {
            let body = &data[pos + header_len..pos + total_len];
            let mut sub = 0usize;
            while sub + 12 <= body.len() {
                let sub_fourcc = &body[sub..sub + 4];
                let sub_hlen = read_u32_be_at(body, sub + 4).unwrap_or(0) as usize;
                let sub_tlen = read_u32_be_at(body, sub + 8).unwrap_or(0) as usize;
                if sub_hlen < 12 || sub_tlen < sub_hlen || sub + sub_tlen > body.len() {
                    break;
                }
                let entry = &body[sub..sub + sub_tlen];
                if sub_fourcc == b"PCP2" {
                    if let Some(cue) = parse_pcp2_entry(entry) {
                        from_pco2.push(cue);
                    }
                } else if sub_fourcc == b"PCPT"
                    && let Some(cue) = parse_pcpt_entry(entry)
                {
                    from_pcob.push(cue);
                }
                sub += sub_tlen;
            }
        }
        pos += total_len;
    }

    if !from_pco2.is_empty() {
        from_pco2
    } else {
        from_pcob
    }
}

/// Read the beat-grid anchor (time of the first beat, ms) from an ANLZ file's
/// `PQTZ` (or `PQT2`) chunk. Used on USB re-import to seed `tracks.first_beat_ms`.
pub fn read_first_beat_from_anlz(data: &[u8]) -> Option<u32> {
    if data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return None;
    }
    let mut pos = 28usize;
    while pos + 12 <= data.len() {
        let fourcc = &data[pos..pos + 4];
        let header_len = read_u32_be_at(data, pos + 4)? as usize;
        let total_len = read_u32_be_at(data, pos + 8)? as usize;
        if header_len < 12 || total_len < header_len || pos + total_len > data.len() {
            break;
        }
        if fourcc == b"PQTZ" {
            // payload: 8-byte beat entries (beat_num u16, tempo u16, time u32)
            let body = &data[pos + header_len..pos + total_len];
            if body.len() >= 8 {
                return read_u32_be_at(body, 4);
            }
        }
        if fourcc == b"PQT2" {
            // header carries first_time_ms at chunk-relative offset 28.
            if let Some(t) = read_u32_be_at(data, pos + 28) {
                return Some(t);
            }
        }
        pos += total_len;
    }
    None
}

/// Read the beat-grid tempo (BPM × 100) actually baked into an ANLZ `PQTZ`
/// (or `PQT2`) chunk. Used by diagnostics to detect a bundle whose beat grid
/// has drifted from the track's PDB `tempo_x100` (e.g. a stale/default grid
/// left behind by an earlier degenerate analysis or repair pass — see
/// `docs/DIAGNOSTICS_REPAIRS.md`).
pub fn read_beatgrid_tempo_from_anlz(data: &[u8]) -> Option<u16> {
    if data.len() < 28 || data.get(0..4) != Some(b"PMAI") {
        return None;
    }
    let mut pos = 28usize;
    while pos + 12 <= data.len() {
        let fourcc = &data[pos..pos + 4];
        let header_len = read_u32_be_at(data, pos + 4)? as usize;
        let total_len = read_u32_be_at(data, pos + 8)? as usize;
        if header_len < 12 || total_len < header_len || pos + total_len > data.len() {
            break;
        }
        if fourcc == b"PQTZ" {
            // payload: 8-byte beat entries (beat_num u16, tempo u16, time u32)
            let body = &data[pos + header_len..pos + total_len];
            if body.len() >= 4 {
                return Some(u16::from_be_bytes([body[2], body[3]]));
            }
        }
        if fourcc == b"PQT2" {
            // header content: first_beat_num(u16) tempo(u16) first_time_ms(u32) ...
            // at content offset 12..14/14..16 (chunk-relative offset 26..28).
            if header_len >= 28 {
                return Some(u16::from_be_bytes([data[pos + 26], data[pos + 27]]));
            }
        }
        pos += total_len;
    }
    None
}

fn parse_pcpt_entry(entry: &[u8]) -> Option<AnlzCue> {
    // abs 12-15 hot_cue, abs 32-35 time
    let hot_cue = read_u32_be_at(entry, 12)?;
    let position_ms = read_u32_be_at(entry, 32)?;
    Some(AnlzCue {
        position_ms,
        hot_cue,
        ..AnlzCue::default()
    })
}

fn parse_pcp2_entry(entry: &[u8]) -> Option<AnlzCue> {
    // abs 12-15 hot_cue, abs 20-23 time, abs 28 color_id, abs 40-43 len_comment
    let hot_cue = read_u32_be_at(entry, 12)?;
    let position_ms = read_u32_be_at(entry, 20)?;
    let color_id = entry.get(28).copied().unwrap_or(0);
    let mut comment = String::new();
    if let Some(len_comment) = read_u32_be_at(entry, 40).map(|v| v as usize)
        && len_comment >= 2
        && 44 + len_comment <= entry.len()
    {
        let units: Vec<u16> = entry[44..44 + len_comment]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        comment = String::from_utf16_lossy(&units);
    }
    Some(AnlzCue {
        position_ms,
        hot_cue,
        color_id,
        comment,
        ..AnlzCue::default()
    })
}

fn append_pcob_chunk(file: &mut Vec<u8>, cue_type: u32, cues: &[AnlzCue]) {
    let hot = cue_type == 1;
    let entries = cues_for_type(cues, hot);

    let mut header = vec![0u8; 12];
    header[0..4].copy_from_slice(&cue_type.to_be_bytes()); // type
    if entries.is_empty() {
        // Preserve the historical empty-placeholder bytes exactly.
        header[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // memory_count
        append_anlz_chunk(file, b"PCOB", &header, &[]);
        return;
    }
    let count = entries.len() as u32;
    header[6..8].copy_from_slice(&(count as u16).to_be_bytes()); // len_cues
    // memory_count: real Rekordbox always writes the 0xFFFFFFFF sentinel for
    // the hot list, regardless of count (confirmed on 2-8 hot entries across
    // real reference files). For the memory list, the one real sample we
    // have (a single entry) writes `count - 1`; unconfirmed beyond 1 entry.
    let memory_count = if hot { 0xFFFF_FFFFu32 } else { count - 1 };
    header[8..12].copy_from_slice(&memory_count.to_be_bytes());

    let mut payload = Vec::new();
    for (i, cue) in entries.iter().enumerate() {
        payload.extend_from_slice(&build_pcpt_entry(cue, hot, (i + 1) as u16));
    }
    append_anlz_chunk(file, b"PCOB", &header, &payload);
}

fn append_pco2_chunk(file: &mut Vec<u8>, cue_type: u32, cues: &[AnlzCue]) {
    let hot = cue_type == 1;
    let entries = cues_for_type(cues, hot);

    let mut header = vec![0u8; 8];
    header[0..4].copy_from_slice(&cue_type.to_be_bytes()); // type
    if entries.is_empty() {
        append_anlz_chunk(file, b"PCO2", &header, &[]);
        return;
    }
    header[4..6].copy_from_slice(&(entries.len() as u16).to_be_bytes()); // len_cues

    let mut payload = Vec::new();
    for cue in &entries {
        payload.extend_from_slice(&build_pcp2_entry(cue));
    }
    append_anlz_chunk(file, b"PCO2", &header, &payload);
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
/// `duration_ms` here is the raw, encoder-delay/padding-*unstripped* duration
/// from `detect_track_duration_ms` (Symphonia `n_frames * samples_per_frame /
/// sample_rate` — the MP3 header's declared frame count, not the gapless-
/// trimmed playable length ffmpeg/mutagen would report). Reference RB exports
/// were verified to use exactly `ceil(that_raw_duration_seconds * 150) + 4`
/// entries — a fixed 4-entry tail pad on top of the raw duration, not a
/// rounding artifact. Using a gapless-trimmed duration instead reproduces
/// deltas of 10-140+ entries per track rather than a flat +4; see
/// `docs/WAVEFORMS.md` "Why + 4" for the full derivation.
fn detail_entry_count(duration_ms: u64) -> u32 {
    let duration_secs = duration_ms as f64 / 1000.0;
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
    duration_ms: u64,
    first_beat_ms_override: Option<u32>,
    cues: &[AnlzCue],
) -> Vec<u8> {
    let mut file = build_anlz_file_header();
    let first_beat_ms_raw = first_beat_ms_override
        .unwrap_or_else(|| estimate_first_beat_ms(waveform, bpm, duration_ms));
    let first_beat_ms = normalize_first_beat_ms(first_beat_ms_raw, bpm);

    // 1. PPTH
    append_ppth_chunk(&mut file, track_path);

    // 2. PVBR — MP3 seek index, left empty (see append_pvbr_chunk)
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

    // 6. PCOB — hot cue list (type=1)
    append_pcob_chunk(&mut file, 1, cues);

    // 7. PCOB — memory point list (type=0)
    append_pcob_chunk(&mut file, 0, cues);

    file
}

// ===========================================================================
// .EXT — PPTH + PWV3 + PCOB(hot) + PCOB(mem) + PCO2(hot) + PCO2(mem) + PQT2 + PWV5 + PWV4
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
    duration_ms: u64,
    first_beat_ms_override: Option<u32>,
    cues: &[AnlzCue],
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

    // 3. PCOB — hot cue list (type=1)
    append_pcob_chunk(&mut file, 1, cues);

    // 4. PCOB — memory point list (type=0)
    append_pcob_chunk(&mut file, 0, cues);

    // 5. PCO2 — extended hot cue list (type=1)
    append_pco2_chunk(&mut file, 1, cues);

    // 6. PCO2 — extended memory point list (type=0)
    append_pco2_chunk(&mut file, 0, cues);

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

pub fn build_anlz_2ex_file(waveform: &WaveformData, track_path: &str, duration_ms: u64) -> Vec<u8> {
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
            30_000,
            None,
            &[],
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
            30_000,
            None,
            &[],
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
            30_000,
            None,
            &[],
        );
        let pqtz = find_chunk_payload(&dat, "PQTZ").expect("PQTZ chunk");
        assert_eq!(pqtz.len(), 61 * 8, "120 BPM × 30s = 61 beats × 8 bytes");
    }

    #[test]
    fn dat_pqtz_header_matches_rekordbox_layout() {
        // From a rekordbox export (USB_CUE_RB, "Bash Plate"): 649 beats.
        let rekordbox = "000000000008000000000289";
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            Some(140.0),
            277_820,
            Some(53),
            &[],
        );
        let pos = dat.windows(4).position(|w| w == b"PQTZ").unwrap();
        let header: String = dat[pos + 12..pos + 24]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(&header[..16], &rekordbox[..16]);
        assert!(!has_misplaced_pqtz_header(&dat));
    }

    fn bundle_at(bpm: f64, first_beat_ms: u32) -> (Vec<u8>, Vec<u8>) {
        let waveform = WaveformData::from_peaks(vec![128; 400]);
        (
            build_anlz_dat_file(&waveform, "", Some(bpm), 200_000, Some(first_beat_ms), &[]),
            build_anlz_ext_file(&waveform, "", Some(bpm), 200_000, Some(first_beat_ms), &[]),
        )
    }

    fn u32_at(data: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn pqt2_matches_rekordbox_layout_and_checksum() {
        let (dat, ext) = bundle_at(131.82, 295);
        let beats = pqtz_beats(&dat).unwrap();
        let pos = ext.windows(4).position(|w| w == b"PQT2").unwrap();
        let header = &ext[pos..pos + 56];
        assert_eq!(u32_at(header, 4), 56);
        assert_eq!(u32_at(header, 16), 0x0100_0002);
        let entry = |at: usize| {
            (
                u16::from_be_bytes([header[at], header[at + 1]]),
                u16::from_be_bytes([header[at + 2], header[at + 3]]),
                u32_at(header, at + 4),
            )
        };
        assert_eq!(entry(24), beats[0]);
        assert_eq!(entry(32), *beats.last().unwrap());
        assert_eq!(u32_at(header, 40), beats.len() as u32);
        let sum: u32 = beats
            .iter()
            .map(|(n, t, ms)| u32::from(*n) + u32::from(*t) + ms)
            .sum();
        assert_eq!(u32_at(header, 44), sum);
        assert_eq!(&header[48..56], &[0; 8]);
        assert_eq!(pqt2_checksum_ok(&dat, &ext), Some(true));

        // The body is each beat's microseconds; PQTZ holds the floored ms.
        let body = find_chunk_payload(&ext, "PQT2").unwrap();
        let interval_us = 60_000_000.0 / 131.82;
        for (i, (&(_, _, ms), us)) in beats.iter().zip(body.chunks_exact(2)).enumerate() {
            let exact = 295_000 + (i as f64 * interval_us).round() as u64;
            let us = u64::from(u16::from_be_bytes([us[0], us[1]]));
            assert!(us < 1000);
            assert_eq!(u64::from(ms) * 1000 + us, exact, "beat {i}");
        }
    }

    #[test]
    fn pqt2_body_reproduces_a_rekordbox_100_bpm_grid() {
        // rekordbox: beats at 355.102 ms + n × 600 ms, every body entry 0x66.
        let grid = BeatGrid {
            first_beat_number: 3,
            tempo_x100: 10_000,
            first_us: 355_102,
            interval_us: 600_000.0,
            num_beats: 398,
        };
        let mut chunk = Vec::new();
        append_pqt2_grid(&mut chunk, &grid);
        let body = &chunk[56..];
        assert!(body.chunks_exact(2).all(|e| e == [0x00, 0x66]));
        assert_eq!(u32_at(&chunk, 24 + 4), 355);
        assert_eq!(u32_at(&chunk, 32 + 4), 238_555);
        assert_eq!(&chunk[24..26], &[0, 3]);
    }

    #[test]
    fn misplaced_pqtz_header_is_found_in_the_first_4_kb() {
        let (dat, _) = bundle_at(120.0, 437);
        let pqtz = dat.windows(4).position(|w| w == b"PQTZ").unwrap();
        let mut old = dat.clone();
        old[pqtz + 12..pqtz + 20].copy_from_slice(&PQTZ_HEADER_MISPLACED);
        let probe = PQTZ_PROBE_BYTES as usize;
        assert!(has_misplaced_pqtz_header(&old[..probe]));
        assert!(!has_misplaced_pqtz_header(&dat[..probe]));
        assert!(
            !has_misplaced_pqtz_header(&old[..pqtz + 16]),
            "header cut off"
        );
    }

    #[test]
    fn pre_0_3_7_grids_are_rebuilt_from_their_own_beats() {
        let (dat, ext) = bundle_at(120.0, 437);
        // As a pre-0.3.7 app wrote them: PQTZ value two bytes early, PQT2
        // without checksum and with the old `(i % 4, 0)` body.
        let mut old_dat = dat.clone();
        let pqtz = old_dat.windows(4).position(|w| w == b"PQTZ").unwrap();
        old_dat[pqtz + 12..pqtz + 20].copy_from_slice(&PQTZ_HEADER_MISPLACED);
        let mut old_ext = ext.clone();
        let pqt2 = old_ext.windows(4).position(|w| w == b"PQT2").unwrap();
        old_ext[pqt2 + 44..pqt2 + 48].fill(0);
        let count = u32_at(&old_ext, pqt2 + 40) as usize;
        for i in 0..count {
            let at = pqt2 + 56 + i * 2;
            old_ext[at..at + 2].copy_from_slice(&[(i % 4) as u8, 0]);
        }
        assert!(has_misplaced_pqtz_header(&old_dat));
        assert_eq!(pqt2_checksum_ok(&old_dat, &old_ext), Some(false));

        let grid = beat_grid_from_pqtz(&old_dat).expect("constant-tempo grid");
        assert_eq!(with_beat_grid(&old_dat, &grid), Some(dat));
        assert_eq!(with_beat_grid(&old_ext, &grid), Some(ext));
    }

    #[test]
    fn grid_rebuild_keeps_beats_whose_tempo_field_is_rounded() {
        // 127.996 BPM is stored as 128.00; following the field would drift
        // by over 10 ms by the end, so the beats' own spacing is used.
        let (dat, _) = bundle_at(127.996, 0);
        let grid = beat_grid_from_pqtz(&dat).expect("grid");
        let rebuilt = pqtz_beats(&with_beat_grid(&dat, &grid).unwrap()).unwrap();
        for (old, new) in pqtz_beats(&dat).unwrap().iter().zip(&rebuilt) {
            assert!(old.2.abs_diff(new.2) <= 1, "{old:?} vs {new:?}");
        }
    }

    #[test]
    fn dat_pqtz_beat_entries_have_correct_format() {
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            Some(120.0),
            10_000,
            Some(0),
            &[],
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
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![100; 100]),
            "",
            None,
            30_000,
            None,
            &[],
        );
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
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![100; 100]),
            "",
            None,
            30_000,
            None,
            &[],
        );
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
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            None,
            30_000,
            None,
            &[],
        );
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
            Some(128.0),
            30_000,
            None,
            &[],
        );
        let tags = collect_chunk_tags(&ext);
        assert_eq!(
            tags,
            vec![
                "PPTH", "PWV3", "PCOB", "PCOB", "PCO2", "PCO2", "PQT2", "PWV5", "PWV4",
            ],
            "EXT chunk order must match reference (no PSSI: phrases are not analyzed)"
        );
    }

    #[test]
    fn ext_pwv3_whiteness_uses_frequency_band_bits() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![80; 100]),
            "",
            None,
            10_000,
            None,
            &[],
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
            30_000,
            None,
            &[],
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
            30_000,
            None,
            &[],
        );
        let pwv3 = find_chunk_payload(&ext, "PWV3").expect("PWV3");
        let pwv5 = find_chunk_payload(&ext, "PWV5").expect("PWV5");
        assert_eq!(pwv5.len(), pwv3.len() * 2, "PWV5 = 2 bytes per PWV3 entry");
    }

    #[test]
    fn ext_pwv4_has_1200_entries() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 100]),
            "",
            None,
            30_000,
            None,
            &[],
        );
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
        let ext = build_anlz_ext_file(&waveform, "", None, 30_000, None, &[]);
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
            30_000,
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
        let twoex = build_anlz_2ex_file(&WaveformData::from_peaks(vec![128; 100]), "", 30_000);
        let pwv7 = find_chunk_payload(&twoex, "PWV7").expect("PWV7");
        assert_eq!(pwv7.len(), 4504 * 3);
    }

    #[test]
    fn twoex_pwv6_has_1200_entries() {
        let twoex = build_anlz_2ex_file(&WaveformData::from_peaks(vec![128; 100]), "", 30_000);
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
        let twoex = build_anlz_2ex_file(&waveform, "", 10_000);
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
        let got = estimate_first_beat_ms(&wf, Some(140.0), 180_000);
        assert!(got.abs_diff(52) <= 20, "expected ~52ms, got {got}ms");
    }

    #[test]
    fn estimate_first_beat_handles_mid_band_driven_beats() {
        let wf = pulsed_waveform_for_band(6000, 128.0, 210_000, 120, 1);
        let got = estimate_first_beat_ms(&wf, Some(128.0), 210_000);
        assert!(got.abs_diff(120) <= 25, "expected ~120ms, got {got}ms");
    }

    #[test]
    fn estimate_first_beat_handles_high_band_driven_beats() {
        let wf = pulsed_waveform_for_band(6000, 90.0, 200_000, 165, 2);
        let got = estimate_first_beat_ms(&wf, Some(90.0), 200_000);
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
                60_000,
                None,
                &[],
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
                60_000,
                None,
                &[],
            ),
            "EXT",
        );
    }

    #[test]
    fn twoex_structure_consistent() {
        verify_anlz_structure(
            &build_anlz_2ex_file(&WaveformData::from_peaks(vec![128; 512]), "", 60_000),
            "2EX",
        );
    }

    #[test]
    fn all_files_consistent_with_empty_peaks() {
        verify_anlz_structure(
            &build_anlz_dat_file(&WaveformData::empty(), "", None, 30_000, None, &[]),
            "DAT-empty",
        );
        verify_anlz_structure(
            &build_anlz_ext_file(&WaveformData::empty(), "", None, 30_000, None, &[]),
            "EXT-empty",
        );
        verify_anlz_structure(
            &build_anlz_2ex_file(&WaveformData::empty(), "", 30_000),
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
            30_000,
            None,
            &[],
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
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 10]),
            path,
            None,
            30_000,
            None,
            &[],
        );
        assert_eq!(ppth_path_from_anlz(&dat).as_deref(), Some(path));
    }

    #[test]
    fn ensure_ppth_chunk_inserts_unicode_path_before_existing_chunks() {
        let path = "/Contents/Fixture Ö Artist/Fixture Ä Album/05 - Mitä Fixture.flac";
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 10]),
            "",
            Some(120.0),
            30_000,
            None,
            &[],
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
        let paths = AnlzBundlePaths {
            dat_path: dir.path().join("ANLZ0000.DAT"),
            ext_path: dir.path().join("ANLZ0000.EXT"),
            twoex_path: dir.path().join("ANLZ0000.2EX"),
        };

        write_generated_anlz_bundle(
            &WaveformData::from_peaks(vec![128; 100]),
            &paths,
            "",
            None,
            30_000,
        )
        .unwrap();

        assert!(paths.dat_path.is_file());
        assert!(paths.ext_path.is_file());
        assert!(paths.twoex_path.is_file());
        assert_eq!(&std::fs::read(&paths.dat_path).unwrap()[0..4], b"PMAI");
        assert_eq!(&std::fs::read(&paths.ext_path).unwrap()[0..4], b"PMAI");
        assert_eq!(&std::fs::read(&paths.twoex_path).unwrap()[0..4], b"PMAI");
    }

    #[test]
    fn with_seek_index_fills_pvbr_in_place_and_keeps_other_chunks() {
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "",
            Some(128.0),
            30_000,
            None,
            &[],
        );
        assert_eq!(pvbr_total_samples(&dat), Some(0));
        let mut payload = vec![0u8; PVBR_PAYLOAD_LEN];
        payload[4..8].copy_from_slice(&417u32.to_be_bytes());
        payload[PVBR_PAYLOAD_LEN - 4..].copy_from_slice(&11_520u32.to_be_bytes());

        let patched = with_seek_index(&dat, &SeekIndex::Pvbr(payload.clone())).unwrap();
        assert_eq!(patched.len(), dat.len());
        assert_eq!(pvbr_total_samples(&patched), Some(11_520));
        assert_eq!(
            find_chunk_payload(&patched, "PVBR").unwrap(),
            payload.as_slice()
        );
        assert_eq!(collect_chunk_tags(&patched), collect_chunk_tags(&dat));
        for tag in ["PQTZ", "PWAV", "PWV2"] {
            assert_eq!(
                find_chunk_payload(&patched, tag),
                find_chunk_payload(&dat, tag)
            );
        }
    }

    #[test]
    fn with_seek_index_appends_or_replaces_pvb2_at_the_end() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "",
            Some(128.0),
            30_000,
            None,
            &[],
        );
        assert_eq!(has_pvb2_chunk(&ext), Some(false));
        let chunk = |entries: u32| super::super::anlz_seek::Pvb2Chunk {
            header: [&[0u8; 12][..], &entries.to_be_bytes(), &20u32.to_be_bytes()].concat(),
            payload: vec![0u8; entries as usize * 20],
        };

        let once = with_seek_index(&ext, &SeekIndex::Pvb2(chunk(3))).unwrap();
        assert_eq!(has_pvb2_chunk(&once), Some(true));
        let mut tags = collect_chunk_tags(&ext);
        tags.push("PVB2".to_string());
        assert_eq!(collect_chunk_tags(&once), tags);
        assert_eq!(read_u32_be_at(&once, 8), Some(once.len() as u32));

        let twice = with_seek_index(&once, &SeekIndex::Pvb2(chunk(5))).unwrap();
        assert_eq!(collect_chunk_tags(&twice), tags);
        assert_eq!(find_chunk_payload(&twice, "PVB2").unwrap().len(), 5 * 20);
        assert_eq!(&twice[28..ext.len()], &ext[28..]);
    }

    #[test]
    fn fill_mp3_pvbr_writes_the_index_once_and_only_for_mp3s() {
        let dir = tempdir().unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
        let dat_path = dir.path().join("A.DAT");
        let dat = build_anlz_dat_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "",
            Some(120.0),
            30_000,
            None,
            &[],
        );
        std::fs::write(&dat_path, &dat).unwrap();

        let flac = fixtures.join("formats/track_format_flac.flac");
        assert_eq!(fill_mp3_pvbr(&dat_path, &flac).unwrap(), PvbrFill::NotMp3);
        let mp3 = fixtures.join("embedded/track_embedded.mp3");
        assert_eq!(fill_mp3_pvbr(&dat_path, &mp3).unwrap(), PvbrFill::Filled);
        let filled = std::fs::read(&dat_path).unwrap();
        assert!(pvbr_total_samples(&filled).is_some_and(|total| total > 0));
        assert_eq!(filled.len(), dat.len());
        assert_eq!(
            fill_mp3_pvbr(&dat_path, &mp3).unwrap(),
            PvbrFill::AlreadySet
        );
    }

    #[test]
    fn with_seek_index_refuses_a_damaged_file() {
        let mut dat = build_anlz_dat_file(&WaveformData::empty(), "", None, 30_000, None, &[]);
        let len = dat.len();
        dat.truncate(len - 3);
        assert_eq!(pvbr_total_samples(&dat), None);
        assert!(with_seek_index(&dat, &SeekIndex::Pvbr(vec![0; PVBR_PAYLOAD_LEN])).is_none());
    }

    // --- Cue points ---

    fn sample_cues() -> Vec<AnlzCue> {
        vec![
            AnlzCue {
                position_ms: 1_000,
                hot_cue: 0,
                comment: "Intro".to_string(),
                ..AnlzCue::default()
            },
            AnlzCue {
                position_ms: 64_000,
                hot_cue: 0,
                ..AnlzCue::default()
            },
            AnlzCue {
                position_ms: 2_000,
                hot_cue: 1,
                color_id: 2,
                color_rgb: (0xE1, 0x24, 0x24),
                color_code: 2,
                comment: "Drop".to_string(),
            },
            AnlzCue {
                position_ms: 128_000,
                hot_cue: 3,
                color_id: 5,
                color_rgb: (0x4E, 0xB6, 0x48),
                color_code: 5,
                ..AnlzCue::default()
            },
        ]
    }

    #[test]
    fn empty_cue_chunks_match_previous_placeholder_bytes() {
        // The historical empty placeholders: PCOB header type + memory_count
        // 0xFFFFFFFF, PCO2 header type only.
        let mut pcob = Vec::new();
        append_pcob_chunk(&mut pcob, 1, &[]);
        assert_eq!(&pcob[0..4], b"PCOB");
        assert_eq!(read_u32_be(&pcob, 4), 0x18); // len_header
        assert_eq!(read_u32_be(&pcob, 8), 0x18); // len_tag (no entries)
        assert_eq!(read_u32_be(&pcob, 12), 1); // type
        assert_eq!(read_u32_be(&pcob, 20), 0xFFFF_FFFF); // memory_count

        let mut pco2 = Vec::new();
        append_pco2_chunk(&mut pco2, 0, &[]);
        assert_eq!(&pco2[0..4], b"PCO2");
        assert_eq!(read_u32_be(&pco2, 4), 0x14);
        assert_eq!(read_u32_be(&pco2, 8), 0x14);
        assert_eq!(read_u32_be(&pco2, 12), 0); // type
    }

    #[test]
    fn pcpt_entry_is_56_bytes_with_expected_fields() {
        let mut pcob = Vec::new();
        append_pcob_chunk(&mut pcob, 1, &sample_cues());
        // sample_cues has two hot cues (slots 1 and 3), sorted by slot.
        assert_eq!(read_u32_be(&pcob, 12), 1); // type
        assert_eq!(u16::from_be_bytes([pcob[18], pcob[19]]), 2); // len_cues
        // Hot list: real Rekordbox always writes the 0xFFFFFFFF sentinel here,
        // regardless of entry count.
        assert_eq!(read_u32_be(&pcob, 20), 0xFFFF_FFFF); // memory_count
        let body = &pcob[24..];
        assert_eq!(&body[0..4], b"PCPT");
        assert_eq!(read_u32_be(body, 4), 0x1C); // len_header
        assert_eq!(read_u32_be(body, 8), 0x38); // len_entry
        assert_eq!(read_u32_be(body, 12), 1); // hot_cue slot
        assert_eq!(read_u32_be(body, 16), 0); // status (always 0)
        assert_eq!(read_u32_be(body, 20), 0x0001_0000); // unknown1
        assert_eq!(body[28], 1); // type = point
        assert_eq!(&body[29..32], &[0x00, 0x03, 0xe8]); // reserved constant
        assert_eq!(read_u32_be(body, 32), 2_000); // time
        assert_eq!(read_u32_be(body, 36), 0xFFFF_FFFF); // loop_time
    }

    #[test]
    fn pcob_memory_count_matches_real_rekordbox_semantics() {
        // Memory list, 2 entries (sample_cues has two memory points):
        // real Rekordbox writes `count - 1` when non-empty.
        let mut mem_pcob = Vec::new();
        append_pcob_chunk(&mut mem_pcob, 0, &sample_cues());
        assert_eq!(read_u32_be(&mem_pcob, 20), 1); // memory_count = count - 1

        // Memory list, empty: sentinel, unchanged from before.
        let mut empty_mem_pcob = Vec::new();
        append_pcob_chunk(&mut empty_mem_pcob, 0, &[]);
        assert_eq!(read_u32_be(&empty_mem_pcob, 20), 0xFFFF_FFFF);

        // Hot list, empty: sentinel too.
        let mut empty_hot_pcob = Vec::new();
        append_pcob_chunk(&mut empty_hot_pcob, 1, &[]);
        assert_eq!(read_u32_be(&empty_hot_pcob, 20), 0xFFFF_FFFF);
    }

    #[test]
    fn pcp2_entry_encodes_comment_and_rgb() {
        let mut pco2 = Vec::new();
        append_pco2_chunk(&mut pco2, 1, &sample_cues());
        let body = &pco2[20..];
        assert_eq!(&body[0..4], b"PCP2");
        assert_eq!(read_u32_be(body, 4), 0x10); // len_header
        let len_entry = read_u32_be(body, 8) as usize;
        assert_eq!(len_entry, body.len().min(len_entry));
        assert_eq!(len_entry % 4, 0, "entry length must be 4-byte aligned");
        assert_eq!(read_u32_be(body, 12), 1); // hot_cue
        assert_eq!(body[16], 1); // type
        assert_eq!(&body[17..20], &[0x00, 0x03, 0xe8]); // reserved constant
        assert_eq!(read_u32_be(body, 20), 2_000); // time
        assert_eq!(body[28], 2); // color_id
        assert_eq!(body[29], 1); // reserved constant
        let len_comment = read_u32_be(body, 40) as usize;
        assert_eq!(len_comment, ("Drop".encode_utf16().count() + 1) * 2);
        let comment_units: Vec<u16> = body[44..44 + len_comment - 2]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        assert_eq!(String::from_utf16_lossy(&comment_units), "Drop");
        // color_code + RGB follow the NUL-terminated comment
        let rgb_off = 44 + len_comment;
        assert_eq!(body[rgb_off], 2); // color_code
        assert_eq!(&body[rgb_off + 1..rgb_off + 4], &[0xE1, 0x24, 0x24]);
    }

    #[test]
    fn cues_round_trip_through_ext_file() {
        let ext = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "",
            Some(128.0),
            180_000,
            Some(0),
            &sample_cues(),
        );
        verify_anlz_structure(&ext, "EXT with cues");

        let decoded = read_cues_from_anlz(&ext);
        assert_eq!(decoded.len(), 4);

        let mut memory: Vec<_> = decoded.iter().filter(|c| c.hot_cue == 0).collect();
        memory.sort_by_key(|c| c.position_ms);
        assert_eq!(memory[0].position_ms, 1_000);
        assert_eq!(memory[0].comment, "Intro");
        assert_eq!(memory[1].position_ms, 64_000);

        let hot: Vec<_> = decoded.iter().filter(|c| c.hot_cue != 0).collect();
        assert_eq!(hot.len(), 2);
        let drop = hot.iter().find(|c| c.hot_cue == 1).expect("slot 1");
        assert_eq!(drop.position_ms, 2_000);
        assert_eq!(drop.color_id, 2);
        assert_eq!(drop.comment, "Drop");
    }

    #[test]
    fn apply_analysis_edits_injects_cues_and_shifts_beatgrid_only() {
        let plain = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "/Contents/x.mp3",
            Some(120.0),
            120_000,
            None,
            &[],
        );
        let before_tags = collect_chunk_tags(&plain);

        let edited = apply_analysis_edits_to_anlz(
            &plain,
            &AnlzAnalysisEdits {
                bpm: Some(120.0),
                duration_ms: Some(120_000),
                first_beat_ms: Some(250),
                cues: Some(&sample_cues()),
            },
        );

        // Chunk order and every non-cue / non-beatgrid chunk are unchanged.
        assert_eq!(collect_chunk_tags(&edited), before_tags);
        for tag in ["PPTH", "PWV3", "PWV5", "PWV4"] {
            assert_eq!(
                find_chunk_payload(&plain, tag),
                find_chunk_payload(&edited, tag),
                "{tag} must be byte-identical"
            );
        }
        // PQT2 beat times moved.
        assert_ne!(
            find_chunk_payload(&plain, "PQT2"),
            find_chunk_payload(&edited, "PQT2"),
        );
        // Cues now decode back out.
        assert_eq!(read_cues_from_anlz(&edited).len(), 4);
        assert!(read_cues_from_anlz(&plain).is_empty());
        verify_anlz_structure(&edited, "edited EXT");
    }

    #[test]
    fn no_bpm_writes_no_beat_grid() {
        let waveform = WaveformData::from_peaks(vec![128; 400]);
        let dat = build_anlz_dat_file(&waveform, "/Contents/x.mp3", None, 120_000, None, &[]);
        let ext = build_anlz_ext_file(&waveform, "/Contents/x.mp3", None, 120_000, None, &[]);
        verify_anlz_structure(&dat, "no-bpm DAT");
        verify_anlz_structure(&ext, "no-bpm EXT");
        assert!(find_chunk_payload(&dat, "PQTZ").is_none());
        assert!(find_chunk_payload(&ext, "PQT2").is_none());
        assert_eq!(read_beatgrid_tempo_from_anlz(&dat), None);
        // The waveforms are still there.
        assert!(find_chunk_payload(&dat, "PWAV").is_some());
        assert!(find_chunk_payload(&ext, "PWV5").is_some());
    }

    #[test]
    fn apply_analysis_edits_without_bpm_keeps_the_grid_and_applies_cues() {
        let plain = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "/Contents/x.mp3",
            Some(128.0),
            120_000,
            None,
            &[],
        );
        let edited = apply_analysis_edits_to_anlz(
            &plain,
            &AnlzAnalysisEdits {
                bpm: None,
                duration_ms: Some(120_000),
                first_beat_ms: Some(250),
                cues: Some(&sample_cues()),
            },
        );
        assert_eq!(
            find_chunk_payload(&plain, "PQT2"),
            find_chunk_payload(&edited, "PQT2"),
            "no known tempo: the grid must not be rebuilt"
        );
        assert_eq!(read_beatgrid_tempo_from_anlz(&edited), Some(12_800));
        assert_eq!(read_cues_from_anlz(&edited).len(), 4);
        verify_anlz_structure(&edited, "edited EXT");
    }

    #[test]
    fn apply_analysis_edits_without_duration_keeps_the_grid() {
        let plain = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "/Contents/x.mp3",
            Some(128.0),
            120_000,
            None,
            &[],
        );
        let edited = apply_analysis_edits_to_anlz(
            &plain,
            &AnlzAnalysisEdits {
                bpm: Some(140.0),
                duration_ms: None,
                first_beat_ms: Some(250),
                cues: Some(&sample_cues()),
            },
        );
        assert_eq!(
            find_chunk_payload(&plain, "PQT2"),
            find_chunk_payload(&edited, "PQT2"),
            "no known length: the grid must not be rebuilt"
        );
        assert_eq!(read_cues_from_anlz(&edited).len(), 4);
        verify_anlz_structure(&edited, "edited EXT");
    }

    #[test]
    fn apply_analysis_edits_rebuilds_beatgrid_from_bpm_alone() {
        // Simulates a bundle whose beat grid was baked with a stale tempo:
        // built at 120 BPM with an explicit anchor at 0ms.
        let plain = build_anlz_ext_file(
            &WaveformData::from_peaks(vec![128; 400]),
            "/Contents/x.mp3",
            Some(120.0),
            120_000,
            Some(0),
            &[],
        );
        assert_eq!(read_beatgrid_tempo_from_anlz(&plain), Some(12_000));

        // A bpm-only correction -- no cues, no explicit first_beat_ms, which
        // is exactly the shape of a track re-analyzed to the right tempo
        // with no confident first-beat -- must still rebuild the grid.
        let edited = apply_analysis_edits_to_anlz(
            &plain,
            &AnlzAnalysisEdits {
                bpm: Some(140.0),
                duration_ms: Some(120_000),
                first_beat_ms: None,
                cues: None,
            },
        );

        assert_eq!(read_beatgrid_tempo_from_anlz(&edited), Some(14_000));
        // The anchor already embedded in the bundle (0ms) is preserved,
        // wrapped to the new tempo's beat interval -- not discarded.
        assert_eq!(read_first_beat_from_anlz(&edited), Some(0));
        for tag in ["PPTH", "PWV3", "PWV5", "PWV4"] {
            assert_eq!(
                find_chunk_payload(&plain, tag),
                find_chunk_payload(&edited, tag),
                "{tag} must be byte-identical"
            );
        }
        verify_anlz_structure(&edited, "bpm-only edited EXT");

        // A no-op edit (nothing to change) still returns the input verbatim.
        let untouched = apply_analysis_edits_to_anlz(
            &plain,
            &AnlzAnalysisEdits {
                bpm: None,
                duration_ms: None,
                first_beat_ms: None,
                cues: None,
            },
        );
        assert_eq!(untouched, plain);
    }
}
