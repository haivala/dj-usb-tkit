//! Reading ANLZ analysis files: chunk payloads and the waveform the track
//! tables draw, from the library's or the stick's files.

use std::ops::Range;
use std::path::{Path, PathBuf};

/// Entries in the track tables' small waveform. Each is four values, 0-127:
/// the height, then the low, mid and high band, which the UI turns into
/// shades of the theme's colour.
pub(crate) const ROW_WAVEFORM_ENTRIES: usize = 600;

/// The track tables' small waveform of the bundle at `path`, read from what
/// the bundle holds: its `PWV4` (the `.EXT`'s colour preview, rekordbox's or
/// this app's), else the heights of its `PWV3`, `PWAV` or `PWV2` with no
/// bands. Nothing is drawn from audio here, so the row always shows the
/// file.
pub(crate) fn load_waveform_preview_from_analysis_path(path: &Path) -> Option<Vec<u8>> {
    for candidate in waveform_preview_candidates(path) {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        if let Some(preview) = row_waveform_from_anlz_bytes(&bytes) {
            return Some(preview);
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

/// Extract the raw PWV5 colour-detail waveform payload from a desktop library
/// ANLZ `.EXT` file (no conversion). 2 bytes/entry, BE u16:
/// `R(3) | G(3) | B(3) | Height(5) | _(2)`. Entry count is duration-derived
/// (`ceil(duration_s * 150) + 4`), so this can be tens of thousands of entries.
pub(crate) fn read_pwv5_from_anlz(dat_path: &Path) -> Option<Vec<u8>> {
    read_ext_chunk(dat_path, b"PWV5").filter(|p| p.len() >= 4)
}

/// The raw `tag` chunk payload from the `.EXT` sibling of `dat_path` (or
/// `dat_path` itself when it already is the `.EXT`).
fn read_ext_chunk(dat_path: &Path, tag: &[u8; 4]) -> Option<Vec<u8>> {
    let ext_path = if dat_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ext"))
    {
        dat_path.to_path_buf()
    } else {
        dat_path.with_extension("EXT")
    };
    anlz_chunk_payload(&std::fs::read(&ext_path).ok()?, tag).map(<[u8]>::to_vec)
}

/// [`read_pwv5_from_anlz`] from `.EXT` bytes already read.
pub(crate) fn pwv5_from_ext_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    anlz_chunk_payload(bytes, b"PWV5")
        .filter(|p| p.len() >= 4)
        .map(<[u8]>::to_vec)
}

/// The track tables' small waveform from one ANLZ file's bytes
/// ([`load_waveform_preview_from_analysis_path`]); `None` when it holds no
/// waveform, or only silence.
pub(crate) fn row_waveform_from_anlz_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if let Some(payload) = anlz_chunk_payload(bytes, b"PWV4") {
        let preview = row_waveform_from_pwv4(payload);
        if preview.chunks(4).any(|entry| entry[0] > 0) {
            return Some(preview);
        }
    }
    // One byte per level, the height in its low 5 bits.
    for tag in [b"PWV3", b"PWAV", b"PWV2"] {
        if let Some(payload) = anlz_chunk_payload(bytes, tag) {
            let preview = downsample_waveform_payload(payload, ROW_WAVEFORM_ENTRIES)
                .into_iter()
                .flat_map(|percent| [((u16::from(percent) * 127) / 100) as u8, 0, 0, 0])
                .collect::<Vec<_>>();
            if preview.chunks(4).any(|entry| entry[0] > 0) {
                return Some(preview);
            }
        }
    }
    None
}

/// A `PWV4` payload (six lanes per entry: height, two shading lanes, then
/// low, mid and high) as the track tables' small waveform: at most
/// [`ROW_WAVEFORM_ENTRIES`] entries of height, low, mid, high, each the
/// loudest of the entries it covers. Read only: the file is never changed.
pub(crate) fn row_waveform_from_pwv4(payload: &[u8]) -> Vec<u8> {
    let entries = payload.chunks_exact(6).collect::<Vec<_>>();
    let n = entries.len();
    let count = ROW_WAVEFORM_ENTRIES.min(n);
    let mut out = Vec::with_capacity(count * 4);
    for i in 0..count {
        let start = i * n / count;
        let end = ((i + 1) * n / count).max(start + 1).min(n);
        let loudest = entries[start..end]
            .iter()
            .max_by_key(|entry| entry[0])
            .expect("a window holds at least one entry");
        out.extend_from_slice(&[loudest[0], loudest[3], loudest[4], loudest[5]]);
    }
    out
}

/// An ANLZ chunk's tag, header length and total length from its 12-byte
/// header, when they hold together: a header at least those 12 bytes and a
/// chunk at least its header. Whether the chunk fits in the file is the
/// caller's check.
pub(crate) fn chunk_header(head: &[u8]) -> Option<([u8; 4], usize, usize)> {
    let tag: [u8; 4] = head.get(0..4)?.try_into().ok()?;
    let header_len = u32::from_be_bytes(head.get(4..8)?.try_into().ok()?) as usize;
    let total_len = u32::from_be_bytes(head.get(8..12)?.try_into().ok()?) as usize;
    (header_len >= 12 && total_len >= header_len).then_some((tag, header_len, total_len))
}

/// One chunk of walked ANLZ bytes: its tag, its whole span, and its payload
/// (the bytes after its header), as ranges into those bytes.
pub(crate) struct AnlzChunk {
    pub(crate) tag: [u8; 4],
    pub(crate) range: Range<usize>,
    pub(crate) body: Range<usize>,
}

/// The chunks of ANLZ bytes in file order ([`anlz_chunks`], [`chunk_entries`]).
///
/// The walk stops at the first chunk whose header doesn't hold together or
/// that runs past the end. Every reader treats what comes after as
/// unreadable, so a damaged file reads the same everywhere: the chunks before
/// the damage, and nothing after it. A file whose walk doesn't reach its end
/// is damaged, and its track needs analysis again
/// ([`anlz_file_walks_cleanly`]).
pub(crate) struct AnlzChunks<'a> {
    data: &'a [u8],
    pos: usize,
    stopped: bool,
}

impl AnlzChunks<'_> {
    /// Where the walk stopped: the end of the last chunk it read.
    pub(crate) fn end(&self) -> usize {
        self.pos
    }

    /// Whether the walk got to the end of the bytes. Meaningful once the
    /// iterator is exhausted.
    pub(crate) fn complete(&self) -> bool {
        self.pos == self.data.len()
    }
}

impl Iterator for AnlzChunks<'_> {
    type Item = AnlzChunk;

    fn next(&mut self) -> Option<AnlzChunk> {
        if self.stopped || self.pos == self.data.len() {
            return None;
        }
        let pos = self.pos;
        let header = self
            .data
            .get(pos..pos + 12)
            .and_then(chunk_header)
            .filter(|&(_, _, total_len)| total_len <= self.data.len() - pos);
        let Some((tag, header_len, total_len)) = header else {
            self.stopped = true;
            return None;
        };
        self.pos = pos + total_len;
        Some(AnlzChunk {
            tag,
            range: pos..pos + total_len,
            body: pos + header_len..pos + total_len,
        })
    }
}

/// The chunks of an ANLZ file's bytes, after its 28-byte `PMAI` header. Bytes
/// that aren't an ANLZ file have none and never walk to the end.
pub(crate) fn anlz_chunks(file: &[u8]) -> AnlzChunks<'_> {
    let is_anlz = file.len() >= 28 && file.starts_with(b"PMAI");
    AnlzChunks {
        data: file,
        pos: if is_anlz { 28 } else { 0 },
        stopped: !is_anlz,
    }
}

/// The entries inside a chunk's payload (the `PCPT`/`PCP2` cues of a
/// `PCOB`/`PCO2`), which are laid out like chunks.
pub(crate) fn chunk_entries(body: &[u8]) -> AnlzChunks<'_> {
    AnlzChunks {
        data: body,
        pos: 0,
        stopped: false,
    }
}

/// The payload of the first `tag` chunk that has one, in an ANLZ file's bytes.
pub fn anlz_chunk_payload<'a>(file: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    anlz_chunks(file)
        .find(|chunk| &chunk.tag == tag && !chunk.body.is_empty())
        .map(|chunk| &file[chunk.body])
}

/// Walks the chunk headers of the ANLZ file at `path`, reading through a
/// `buffer_size` buffer and skipping each chunk `keep` turns down by its
/// length. Returns the file's 28-byte header followed by the kept chunks, and
/// whether the walk reached the end of the file (`false` for a file that
/// isn't ANLZ). `None` when the file can't be read.
pub(crate) fn stream_anlz_chunks(
    path: &Path,
    keep: impl Fn(&[u8; 4]) -> bool,
    buffer_size: usize,
) -> Option<(Vec<u8>, bool)> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut file = std::io::BufReader::with_capacity(buffer_size, file);
    let mut out = vec![0u8; 28];
    file.read_exact(&mut out).ok()?;
    if !out.starts_with(b"PMAI") {
        return Some((out, false));
    }
    let mut pos = 28u64;
    while pos < len {
        let mut head = [0u8; 12];
        if len - pos < 12 || file.read_exact(&mut head).is_err() {
            return Some((out, false));
        }
        let Some((tag, _, total_len)) = chunk_header(&head) else {
            return Some((out, false));
        };
        let total_len = total_len as u64;
        if total_len > len - pos {
            return Some((out, false));
        }
        let rest = total_len - 12;
        if keep(&tag) {
            let mut body = vec![0u8; usize::try_from(rest).ok()?];
            if file.read_exact(&mut body).is_err() {
                return Some((out, false));
            }
            out.extend_from_slice(&head);
            out.extend_from_slice(&body);
        } else if file.seek_relative(i64::try_from(rest).ok()?).is_err() {
            return Some((out, false));
        }
        pos += total_len;
    }
    Some((out, true))
}

/// An ANLZ file with no chunks: its 28-byte header, the file length in bytes
/// 8..12. It walks cleanly, for tests that need a bundle but not its content.
#[cfg(test)]
pub(crate) fn empty_anlz_file() -> Vec<u8> {
    let mut file = b"PMAI".to_vec();
    file.extend_from_slice(&28u32.to_be_bytes());
    file.extend_from_slice(&28u32.to_be_bytes());
    file.resize(28, 0);
    file
}

/// Whether the ANLZ file at `path` is there and walks to its end. Reads only
/// the chunk headers, so it is cheap enough to run per listed track.
pub(crate) fn anlz_file_walks_cleanly(path: &Path) -> bool {
    stream_anlz_chunks(path, |_| false, 64).is_some_and(|(_, clean)| clean)
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

        // A .DAT has no PWV4: the heights of its PWAV, with no bands.
        let preview = row_waveform_from_anlz_bytes(&dat_bytes).expect("preview from DAT");
        assert_eq!(preview.len(), ROW_WAVEFORM_ENTRIES * 4);
        assert!(preview.chunks(4).any(|entry| entry[0] > 0));
        assert!(
            preview
                .chunks(4)
                .all(|entry| entry[0] <= 127 && entry[1..] == [0, 0, 0])
        );
    }

    #[test]
    fn the_row_waveform_is_the_ext_pwv4_with_its_bands() {
        use crate::service::anlz::{WaveformData, build_anlz_ext_file, pwv4_payload};

        let peaks: Vec<u8> = (0..2400).map(|i| (i * 37 % 90) as u8 + 5).collect();
        let waveform = WaveformData::from_peaks(peaks);
        let ext = build_anlz_ext_file(&waveform, "", None, 180_000, None, &[]);
        let preview = row_waveform_from_anlz_bytes(&ext).expect("preview from EXT");
        // What the file holds, as analysis shows it before the file is read.
        assert_eq!(preview, row_waveform_from_pwv4(&pwv4_payload(&waveform)));
        assert_eq!(preview.len(), ROW_WAVEFORM_ENTRIES * 4);
        // Each entry is the loudest of its two PWV4 entries, with its bands.
        let pwv4 = anlz_chunk_payload(&ext, b"PWV4").expect("PWV4");
        let first = &pwv4[..12];
        let loudest = if first[6] > first[0] {
            &first[6..]
        } else {
            &first[..6]
        };
        assert_eq!(
            preview[..4],
            [loudest[0], loudest[3], loudest[4], loudest[5]]
        );
        assert!(preview.chunks(4).any(|entry| entry[1..] != [0, 0, 0]));
    }
}
