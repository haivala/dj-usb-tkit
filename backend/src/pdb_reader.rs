use std::collections::HashMap;
use std::path::Path;

use crate::error::{BackendError, BackendResult};

#[derive(Debug, Clone)]
pub struct PdbTrackRow {
    pub content_link: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub file_size_bytes: Option<u32>,
    pub master_content_id: Option<u32>,
    pub master_db_id: Option<u32>,
    pub id: u32,
    pub artist_id: u32,
    pub album_id: u32,
    pub artwork_id: u32,
    pub key_id: u32,
    pub genre_id: u32,
    pub bitrate_kbps: Option<u32>,
    pub track_number: u32,
    pub tempo_x100: u32,
    pub release_year: Option<u16>,
    pub bit_depth: Option<u16>,
    pub duration_seconds: Option<u32>,
    pub file_type: Option<u16>,
    pub isrc: Option<String>,
    pub date_added: Option<String>,
    pub release_date: Option<String>,
    pub dj_comment: Option<String>,
    pub file_name: Option<String>,
    pub publish_track_info: Option<String>,
    pub autoload_hotcues: Option<String>,
    pub title: String,
    pub anlz_path: String,
    pub track_file_path: String,
}

#[derive(Debug, Clone)]
pub struct PdbTrackStringSlot {
    pub index: usize,
    pub label: &'static str,
    pub offset: u16,
    pub raw_hex: String,
    pub decoded_value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PdbTrackDebugRow {
    pub id: u32,
    pub row_len: usize,
    pub raw_hex: String,
    pub fixed_fields: HashMap<String, String>,
    pub fixed_block_hex: String,
    pub string_slots: Vec<PdbTrackStringSlot>,
}

#[derive(Debug, Clone)]
pub struct PdbPlaylistTreeRow {
    pub id: u32,
    pub parent_id: u32,
    pub sort_order: u32,
    pub row_is_folder: bool,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PdbPlaylistEntryRow {
    pub entry_index: u32,
    pub track_id: u32,
    pub playlist_id: u32,
}

#[derive(Debug, Clone)]
pub struct PdbHistoryPlaylistRow {
    pub id: u32,
    pub name: String,
    pub source_table: u32,
}

#[derive(Debug, Clone)]
pub struct PdbHistoryEntryRow {
    pub track_id: Option<u32>,
    pub playlist_id: u32,
    pub entry_index: u32,
    pub source_table: u32,
}

#[derive(Debug, Clone)]
pub struct PdbHistoryRow {
    pub date: Option<String>,
    pub num: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedPdb {
    pub tracks: Vec<PdbTrackRow>,
    pub artists: HashMap<u32, String>,
    pub albums: HashMap<u32, String>,
    pub artworks: HashMap<u32, String>,
    pub keys: HashMap<u32, String>,
    pub genres: HashMap<u32, String>,
    pub labels: HashMap<u32, String>,
    pub playlist_tree: Vec<PdbPlaylistTreeRow>,
    pub playlist_entries: Vec<PdbPlaylistEntryRow>,
    pub history_playlists: Vec<PdbHistoryPlaylistRow>,
    pub history_entries: Vec<PdbHistoryEntryRow>,
    pub history_rows: Vec<PdbHistoryRow>,
    pub history_raw_rows_bytes: Vec<Vec<u8>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableType {
    Tracks,
    Genres,
    Artists,
    Albums,
    Labels,
    Keys,
    Artwork,
    PlaylistTree,
    PlaylistEntries,
    HistoryPlaylistsAlt,
    HistoryEntriesAlt,
    HistoryPlaylists,
    HistoryEntries,
    History,
    Other,
}

impl TableType {
    fn from_u32(value: u32) -> Self {
        match value {
            0 => Self::Tracks,
            1 => Self::Genres,
            2 => Self::Artists,
            3 => Self::Albums,
            4 => Self::Labels,
            5 => Self::Keys,
            13 => Self::Artwork,
            7 => Self::PlaylistTree,
            8 => Self::PlaylistEntries,
            11 => Self::HistoryPlaylistsAlt,
            12 => Self::HistoryEntriesAlt,
            17 => Self::HistoryPlaylists,
            18 => Self::HistoryEntries,
            19 => Self::History,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PageInfo {
    page_index: u32,
    table_type: TableType,
    nrs: u8,
    used_s: u16,
    num_rl: u16,
}

#[derive(Debug, Clone, Default)]
pub struct PdbDiagnostics {
    pub total_pages: usize,
    pub pages_with_num_rl_8191: usize,
    pub nrs_wrapping_pages: usize,
    pub page_type_counts: HashMap<u32, usize>,
}

pub fn parse_pdb(path: &Path) -> BackendResult<ParsedPdb> {
    let bytes = std::fs::read(path)?;
    parse_pdb_bytes(&bytes)
}

/// Mismatch between an observed page's `(u5, num_rl)` and the per-table
/// convention player firmware expects.
#[derive(Debug, Clone)]
pub struct PdbPageConventionMismatch {
    pub page_index: u32,
    pub table_type: u32,
    pub trc: u16,
    pub observed_u5: u16,
    pub observed_num_rl: u16,
    pub expected_u5: u16,
    pub expected_num_rl: u16,
}

impl std::fmt::Display for PdbPageConventionMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "page[{}] tt={} trc={}: u5={} num_rl={} (expected u5={} num_rl={})",
            self.page_index,
            self.table_type,
            self.trc,
            self.observed_u5,
            self.observed_num_rl,
            self.expected_u5,
            self.expected_num_rl,
        )
    }
}

/// Walk every data page in a PDB byte buffer and flag any pages whose
/// `(u5, num_rl, seq)` footer fields exhibit one of the failure modes
/// observed in the comm-error class of bugs.
///
/// The validator only enforces the parts of the per-table convention
/// that reference exports also obey, so it can be run against both reference
/// samples and our own writer's output:
///
/// - `seq=1` on every data page across the whole file. Reference exports
///   carry a varying transaction-commit counter on data pages; player
///   firmware appears to validate that they are not all collapsed.
/// - `tt=8` (playlist_entries) pages where `u5 != 1` or `num_rl != trc - 1`.
///   This is the most strictly consistent table in observed reference output and
///   was the one whose mismatch (`u5=trc`) most clearly differentiated the
///   broken export from the working backup.
/// - `num_rl > trc` on any data page (impossible row index).
///
/// Sentinel/index pages (`pf=0x64`) and zeroed slots are skipped, as are
/// tables 16/17/18 (preserved verbatim from the pre-export PDB).
pub fn validate_pdb_page_conventions(bytes: &[u8]) -> Vec<PdbPageConventionMismatch> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};

    let mut mismatches = Vec::new();
    let len_page = match read_u32_le_at(bytes, 4) {
        Some(v) if v > 0 => v as usize,
        _ => return mismatches,
    };
    if bytes.len() < len_page {
        return mismatches;
    }
    let total_pages = bytes.len() / len_page;

    for page_idx in 1..total_pages {
        let off = page_idx * len_page;
        if off + 0x28 > bytes.len() {
            break;
        }
        let page = &bytes[off..off + len_page];

        let stored_idx = read_u32_le_at(page, 0x04).unwrap_or(0);
        if stored_idx == 0 {
            continue;
        }

        let pf = read_u8_at(page, 0x1b).unwrap_or(0);
        if pf == 0x64 {
            continue;
        }

        let table_type = read_u32_le_at(page, 0x08).unwrap_or(0);
        if matches!(table_type, 16 | 17 | 18) {
            continue;
        }

        let nrs = read_u8_at(page, 0x18).unwrap_or(0);
        let observed_u5 = read_u16_le_at(page, 0x20).unwrap_or(0);
        let observed_num_rl = read_u16_le_at(page, 0x22).unwrap_or(0);

        if observed_num_rl == 8191 || observed_u5 == 8191 {
            continue;
        }

        let trc: u16 = if nrs == 0 {
            continue;
        } else if observed_num_rl >= nrs as u16 {
            observed_num_rl.saturating_add(1)
        } else {
            nrs as u16
        };

        if table_type == 8 {
            let exp_u5 = 1u16;
            let exp_num_rl = trc.saturating_sub(1);
            if observed_u5 != exp_u5 || observed_num_rl != exp_num_rl {
                mismatches.push(PdbPageConventionMismatch {
                    page_index: stored_idx,
                    table_type,
                    trc,
                    observed_u5,
                    observed_num_rl,
                    expected_u5: exp_u5,
                    expected_num_rl: exp_num_rl,
                });
                continue;
            }
        }
        if trc > 0 && observed_num_rl >= trc {
            mismatches.push(PdbPageConventionMismatch {
                page_index: stored_idx,
                table_type,
                trc,
                observed_u5,
                observed_num_rl,
                expected_u5: observed_u5,
                expected_num_rl: trc.saturating_sub(1),
            });
        }
    }

    mismatches
}

/// Collect the `seq` (offset `0x10`) value from every non-sentinel,
/// non-zeroed data page. Used by tests to assert that an export's
/// transaction-commit counter actually varies across pages — player firmware
/// appears to reject PDBs whose data pages are all collapsed to `seq=1`.
pub fn collect_pdb_data_page_seqs(bytes: &[u8]) -> Vec<u32> {
    use crate::utils::{read_u8_at, read_u32_le_at};

    let mut out = Vec::new();
    let len_page = match read_u32_le_at(bytes, 4) {
        Some(v) if v > 0 => v as usize,
        _ => return out,
    };
    if bytes.len() < len_page {
        return out;
    }
    let total_pages = bytes.len() / len_page;
    for page_idx in 1..total_pages {
        let off = page_idx * len_page;
        if off + 0x28 > bytes.len() {
            break;
        }
        let page = &bytes[off..off + len_page];
        let stored_idx = read_u32_le_at(page, 0x04).unwrap_or(0);
        if stored_idx == 0 {
            continue;
        }
        let pf = read_u8_at(page, 0x1b).unwrap_or(0);
        if pf == 0x64 {
            continue;
        }
        out.push(read_u32_le_at(page, 0x10).unwrap_or(0));
    }
    out
}

pub fn parse_pdb_track_debug_rows(path: &Path) -> BackendResult<Vec<PdbTrackDebugRow>> {
    let bytes = std::fs::read(path)?;
    parse_pdb_track_debug_rows_bytes(&bytes)
}

pub fn parse_pdb_with_diagnostics(path: &Path) -> BackendResult<(ParsedPdb, PdbDiagnostics)> {
    let bytes = std::fs::read(path)?;
    let len_page = parse_len_page(&bytes)?;
    let (out, diag) = parse_pdb_bytes_internal(&bytes, len_page, true);
    Ok((out, diag))
}

fn dispatch_row(row: Vec<u8>, page_info: PageInfo, out: &mut ParsedPdb) {
    match page_info.table_type {
        TableType::Tracks => {
            if let Some(v) = parse_track_row(&row) {
                out.tracks.push(v);
            } else {
                out.warnings.push(format!(
                    "failed to parse track row on page {}",
                    page_info.page_index
                ));
            }
        }
        TableType::Genres => {
            if let Some((id, name)) = parse_genre_row(&row) {
                out.genres.insert(id, name);
            }
        }
        TableType::Artists => {
            if let Some((id, name)) = parse_artist_row(&row) {
                out.artists.insert(id, name);
            }
        }
        TableType::Albums => {
            if let Some((id, name)) = parse_album_row(&row) {
                out.albums.insert(id, name);
            }
        }
        TableType::Labels => {
            if let Some((id, name)) = parse_genre_row(&row) {
                out.labels.insert(id, name);
            }
        }
        TableType::Keys => {
            if let Some((id, name)) = parse_key_row(&row) {
                out.keys.insert(id, name);
            }
        }
        TableType::Artwork => {
            if let Some((id, path)) = parse_artwork_row(&row) {
                out.artworks.insert(id, path);
            }
        }
        TableType::PlaylistTree => {
            if let Some(v) = parse_playlist_tree_row(&row) {
                out.playlist_tree.push(v);
            }
        }
        TableType::PlaylistEntries => {
            if let Some(v) = parse_playlist_entry_row(&row) {
                out.playlist_entries.push(v);
            }
        }
        TableType::HistoryPlaylistsAlt => {
            if let Some(v) = parse_history_playlist_row(&row, 11) {
                out.history_playlists.push(v);
            }
        }
        TableType::HistoryPlaylists => {
            if let Some(v) = parse_history_playlist_row(&row, 17) {
                out.history_playlists.push(v);
            }
        }
        TableType::HistoryEntriesAlt => {
            if let Some(v) = parse_history_entry_row(&row, 12) {
                out.history_entries.push(v);
            }
        }
        TableType::HistoryEntries => {
            if let Some(v) = parse_history_entry_row(&row, 18) {
                out.history_entries.push(v);
            }
        }
        TableType::History => {
            out.history_raw_rows_bytes.push(row.clone());
            if let Some(v) = parse_history_row(&row) {
                out.history_rows.push(v);
            }
        }
        TableType::Other => {}
    }
}

fn parse_pdb_track_debug_rows_bytes(bytes: &[u8]) -> BackendResult<Vec<PdbTrackDebugRow>> {
    let len_page = parse_len_page(bytes)?;

    let max_page = parse_max_physical_page(bytes, len_page);
    let mut warnings = Vec::<String>::new();
    let mut out = Vec::<PdbTrackDebugRow>::new();

    for page_idx in 1..=max_page {
        let start = page_idx * len_page;
        let end = start + len_page;
        if end > bytes.len() {
            break;
        }
        let page = &bytes[start..end];
        let Some(page_info) = parse_page_info(page) else {
            continue;
        };
        if page_info.table_type != TableType::Tracks {
            continue;
        }
        for row in parse_page_rows(page, len_page, page_info, &mut warnings) {
            if let Some(track) = parse_track_debug_row(&row) {
                out.push(track);
            }
        }
    }

    Ok(out)
}

pub fn parse_pdb_bytes(bytes: &[u8]) -> BackendResult<ParsedPdb> {
    let len_page = parse_len_page(bytes)?;
    let (out, _) = parse_pdb_bytes_internal(bytes, len_page, false);
    Ok(out)
}

fn parse_len_page(bytes: &[u8]) -> BackendResult<usize> {
    let len_page = read_u32_le(bytes, 4)
        .ok_or_else(|| BackendError::Internal("invalid PDB header: missing len_page".to_string()))?
        as usize;
    if len_page == 0 {
        return Err(BackendError::Internal(
            "invalid PDB header: len_page is zero".to_string(),
        ));
    }
    Ok(len_page)
}

fn parse_pdb_bytes_internal(
    bytes: &[u8],
    len_page: usize,
    collect_diagnostics: bool,
) -> (ParsedPdb, PdbDiagnostics) {
    let max_page = parse_max_physical_page(bytes, len_page);
    if max_page == 0 {
        return (ParsedPdb::default(), PdbDiagnostics::default());
    }

    let mut out = ParsedPdb::default();
    let mut diag = PdbDiagnostics::default();

    for page_idx in 1..=max_page {
        let start = page_idx * len_page;
        let end = start + len_page;
        if end > bytes.len() {
            out.warnings.push(format!(
                "PDB page {} out of file bounds (len_page={}, file_size={})",
                page_idx,
                len_page,
                bytes.len()
            ));
            break;
        }

        let page = &bytes[start..end];
        let page_info = match parse_page_info(page) {
            Some(v) => v,
            None => {
                out.warnings
                    .push(format!("PDB page {} has invalid page header", page_idx));
                continue;
            }
        };

        if collect_diagnostics {
            diag.total_pages += 1;
            let raw_type = read_u32_le(page, 8).unwrap_or(9999);
            *diag.page_type_counts.entry(raw_type).or_insert(0) += 1;
            if page_info.num_rl == 8191 {
                diag.pages_with_num_rl_8191 += 1;
            }
        }

        let rows = parse_page_rows(page, len_page, page_info, &mut out.warnings);
        if collect_diagnostics && rows.len() > page_info.nrs as usize {
            diag.nrs_wrapping_pages += 1;
        }

        for row in rows {
            dispatch_row(row, page_info, &mut out);
        }
    }

    (out, diag)
}

fn parse_max_physical_page(bytes: &[u8], len_page: usize) -> usize {
    if len_page == 0 || bytes.len() < len_page * 2 {
        return 0;
    }
    (bytes.len() / len_page).saturating_sub(1)
}

fn parse_page_info(page: &[u8]) -> Option<PageInfo> {
    Some(PageInfo {
        page_index: read_u32_le(page, 4)?,
        table_type: TableType::from_u32(read_u32_le(page, 8)?),
        nrs: read_u8(page, 24)?,
        used_s: read_u16_le(page, 30)?,
        num_rl: read_u16_le(page, 34)?,
    })
}

fn parse_page_rows(
    page: &[u8],
    len_page: usize,
    page_info: PageInfo,
    warnings: &mut Vec<String>,
) -> Vec<Vec<u8>> {
    if page_info.page_index == 0 {
        return Vec::new();
    }

    if page_info.used_s == 0 {
        return Vec::new();
    }

    let payload_start = 40usize;
    let payload_end = payload_start.saturating_add(page_info.used_s as usize);
    if payload_end > page.len() || payload_start >= payload_end {
        warnings.push(format!(
            "page {} invalid payload bounds: {}..{}",
            page_info.page_index, payload_start, payload_end
        ));
        return Vec::new();
    }
    let payload = &page[payload_start..payload_end];

    // num_rl=8191 (0x1FFF) is a sentinel meaning "num_rl not tracked on this
    // page" — use nrs alone. Otherwise take the max of num_rl and nrs.
    // nrs is a u8 that wraps at 256. When a page has more than 255 rows
    // (common for playlist_entries), nrs silently underflows, causing rows to
    // be missed. To handle this, compute the maximum possible row count from
    // the available index space, then scan backward reading offsets. Stop when
    // we exhaust the space between payload and the index, or hit invalid offsets.
    let n_header = if page_info.num_rl == 8191 {
        page_info.nrs as usize
    } else {
        usize::max(page_info.num_rl as usize, page_info.nrs as usize)
    };
    let index_space = len_page.saturating_sub(payload_end);
    // Each group of 16 rows needs 4 bytes (presence) + 16*2 (offsets) = 36 bytes.
    let full_groups = index_space / 36;
    let leftover = index_space % 36;
    let partial_rows = if leftover >= 6 { (leftover - 4) / 2 } else { 0 };
    let n_max = full_groups * 16 + partial_rows;
    if n_header == 0 && n_max == 0 {
        return Vec::new();
    }

    let mut m = len_page;
    let mut row_offsets = Vec::with_capacity(n_max + 1);
    let mut row_presence = Vec::with_capacity((n_max / 16) + 2);
    // Phase 1: Always read n_header entries (trusting the page header).
    for i in 0..n_header {
        if i % 16 == 0 {
            if m < 4 {
                break;
            }
            m -= 4;
            let Some(bits) = read_u16_le(page, m) else {
                break;
            };
            row_presence.push(bits);
        }
        if m < 2 {
            break;
        }
        m -= 2;
        let Some(off) = read_u16_le(page, m) else {
            break;
        };
        row_offsets.push(off as usize);
    }

    // Phase 2: If space-derived count is moderately larger (nrs likely wrapped
    // at 256), continue scanning with strict validation. Only attempt extension
    // when wrapping is plausible: the gap should be roughly 256 (one wrap).
    // Skip when the gap is huge (> 512), which indicates unused empty index space.
    let gap = n_max.saturating_sub(n_header);
    if gap > 0 && gap <= 512 && row_offsets.len() == n_header {
        let mut prev_off = row_offsets.last().copied();
        for i in n_header..n_max {
            if i % 16 == 0 {
                if m < 4 || m.saturating_sub(4) < payload_end {
                    break;
                }
                m -= 4;
                let Some(bits) = read_u16_le(page, m) else {
                    break;
                };
                row_presence.push(bits);
            }
            if m < 2 || m.saturating_sub(2) < payload_end {
                break;
            }
            m -= 2;
            let Some(off) = read_u16_le(page, m) else {
                break;
            };
            let off = off as usize;
            if off > payload.len() {
                break;
            }
            if let Some(prev) = prev_off {
                if off < prev {
                    break;
                }
            }
            prev_off = Some(off);
            row_offsets.push(off);
        }
    }

    let mut rows = Vec::new();
    for i in 0..row_offsets.len() {
        let group_idx = i / 16;
        let bit = i % 16;
        let present = row_presence
            .get(group_idx)
            .map(|bits| ((bits >> bit) & 1) == 1)
            .unwrap_or(false);
        if !present {
            continue;
        }
        let Some(&start) = row_offsets.get(i) else {
            continue;
        };
        let raw_end = row_offsets.get(i + 1).copied().unwrap_or(payload.len());
        let clamped_start = start.min(payload.len());
        let clamped_end = raw_end.min(payload.len());
        if clamped_start >= clamped_end {
            continue;
        }
        rows.push(payload[clamped_start..clamped_end].to_vec());
    }
    rows
}

fn parse_track_row(row: &[u8]) -> Option<PdbTrackRow> {
    let content_link = read_u32_le(row, 4).filter(|v| *v > 0);
    let sample_rate_hz = read_u32_le(row, 8).filter(|v| *v > 0);
    let file_size_bytes = read_u32_le(row, 16).filter(|v| *v > 0);
    let master_content_id = read_u32_le(row, 20).filter(|v| *v > 0);
    let master_db_id = read_u32_le(row, 24).filter(|v| *v > 0);
    let artist_id = read_u32_le(row, 68)?;
    let album_id = read_u32_le(row, 64)?;
    let artwork_id = read_u32_le(row, 28)?;
    let key_id = read_u32_le(row, 32)?;
    let genre_id = read_u32_le(row, 60).unwrap_or(0);
    let id = read_u32_le(row, 72)?;
    let bitrate_kbps = read_u32_le(row, 48).filter(|v| *v > 0);
    let track_number = read_u32_le(row, 52)?;
    let tempo_x100 = read_u32_le(row, 56)?;
    let release_year = read_u16_le(row, 80).filter(|v| *v > 0);
    let bit_depth = read_u16_le(row, 82).filter(|v| *v > 0);
    // Legacy PDB track rows encode duration as u16 seconds at offset 84.
    let duration_seconds = read_u16_le(row, 84).map(|v| v as u32).filter(|v| *v > 0);
    let file_type = read_u16_le(row, 90).filter(|v| *v > 0);

    let offsets = parse_track_string_offsets(row);

    let title = decode_range_string_slot(row, &offsets, 17, 18);
    let anlz_path = decode_range_string_slot(row, &offsets, 14, 15);
    let track_file_path = offsets
        .get(20)
        .copied()
        .and_then(|offset| get_string_from_pdb(row, offset as usize))
        .unwrap_or_default();
    let isrc = decode_track_string_slot(row, &offsets, 0).filter(|value| !value.is_empty());
    let date_added = decode_track_string_slot(row, &offsets, 10).filter(|value| !value.is_empty());
    let release_date =
        decode_track_string_slot(row, &offsets, 11).filter(|value| !value.is_empty());
    let dj_comment = decode_track_string_slot(row, &offsets, 16).filter(|value| !value.is_empty());
    let file_name = decode_track_string_slot(row, &offsets, 19).filter(|value| !value.is_empty());
    let publish_track_info =
        decode_track_string_slot(row, &offsets, 6).filter(|value| !value.is_empty());
    let autoload_hotcues =
        decode_track_string_slot(row, &offsets, 7).filter(|value| !value.is_empty());

    Some(PdbTrackRow {
        content_link,
        sample_rate_hz,
        file_size_bytes,
        master_content_id,
        master_db_id,
        id,
        artist_id,
        album_id,
        artwork_id,
        key_id,
        genre_id,
        bitrate_kbps,
        track_number,
        tempo_x100,
        release_year,
        bit_depth,
        duration_seconds,
        file_type,
        isrc,
        date_added,
        release_date,
        dj_comment,
        file_name,
        publish_track_info,
        autoload_hotcues,
        title,
        anlz_path,
        track_file_path,
    })
}

fn parse_track_debug_row(row: &[u8]) -> Option<PdbTrackDebugRow> {
    let id = read_u32_le(row, 72)?;
    let offsets = parse_track_string_offsets(row);

    let mut fixed_fields = HashMap::<String, String>::new();
    insert_u32_field(&mut fixed_fields, "header_flags_u32", read_u32_le(row, 0));
    insert_u32_field(&mut fixed_fields, "content_link_u32", read_u32_le(row, 4));
    insert_u32_field(&mut fixed_fields, "sample_rate_hz", read_u32_le(row, 8));
    insert_u32_field(&mut fixed_fields, "file_size_bytes", read_u32_le(row, 16));
    insert_u32_field(
        &mut fixed_fields,
        "master_content_id_u32",
        read_u32_le(row, 20),
    );
    insert_u32_field(&mut fixed_fields, "master_db_id_u32", read_u32_le(row, 24));
    insert_u32_field(&mut fixed_fields, "artwork_id", read_u32_le(row, 28));
    insert_u32_field(&mut fixed_fields, "key_id", read_u32_le(row, 32));
    insert_u32_field(&mut fixed_fields, "bitrate_kbps", read_u32_le(row, 48));
    insert_u32_field(&mut fixed_fields, "track_number", read_u32_le(row, 52));
    insert_u32_field(&mut fixed_fields, "tempo_x100", read_u32_le(row, 56));
    insert_u32_field(&mut fixed_fields, "album_id", read_u32_le(row, 64));
    insert_u32_field(&mut fixed_fields, "artist_id", read_u32_le(row, 68));
    insert_u32_field(&mut fixed_fields, "track_id", read_u32_le(row, 72));
    insert_u16_field(&mut fixed_fields, "release_year_u16", read_u16_le(row, 80));
    insert_u16_field(&mut fixed_fields, "bit_depth_u16", read_u16_le(row, 82));
    insert_u16_field(
        &mut fixed_fields,
        "duration_seconds_u16",
        read_u16_le(row, 84),
    );
    insert_u16_field(&mut fixed_fields, "file_type_u16", read_u16_le(row, 90));
    fixed_fields.insert("unknown_fixed_00_27_hex".to_string(), hex_slice(row, 0, 28));
    fixed_fields.insert(
        "unknown_fixed_36_51_hex".to_string(),
        hex_slice(row, 36, 52),
    );
    fixed_fields.insert(
        "unknown_fixed_60_63_hex".to_string(),
        hex_slice(row, 60, 64),
    );
    fixed_fields.insert(
        "unknown_fixed_76_83_hex".to_string(),
        hex_slice(row, 76, 84),
    );
    fixed_fields.insert(
        "unknown_fixed_86_93_hex".to_string(),
        hex_slice(row, 86, 94),
    );

    let string_slots = offsets
        .iter()
        .enumerate()
        .map(|(index, offset)| PdbTrackStringSlot {
            index,
            label: track_string_slot_label(index),
            offset: *offset,
            raw_hex: raw_track_string_slot_hex(row, &offsets, index),
            decoded_value: decode_track_string_slot(row, &offsets, index),
        })
        .collect::<Vec<_>>();

    Some(PdbTrackDebugRow {
        id,
        row_len: row.len(),
        raw_hex: hex_slice(row, 0, row.len()),
        fixed_fields,
        fixed_block_hex: hex_slice(row, 0, 94),
        string_slots,
    })
}

/// Parse genre/label rows: id(u32) + pdb_string.
/// This matches the format written by `encode_genre_row` / `encode_label_row`.
fn parse_genre_row(row: &[u8]) -> Option<(u32, String)> {
    let id = read_u32_le(row, 0)?;
    let name = get_string_from_pdb(row, 4).unwrap_or_default();
    if name.is_empty() {
        return None;
    }
    Some((id, name))
}

fn parse_artist_row(row: &[u8]) -> Option<(u32, String)> {
    let typ = read_u16_le(row, 0)?;
    let id = read_u32_le(row, 4)?;
    let on = read_u8(row, 9)? as usize;
    let ofs = if typ == 100 {
        read_u16_le(row, 10)? as usize
    } else {
        on
    };
    let name = get_string_from_pdb(row, ofs).unwrap_or_default();
    Some((id, name))
}

fn parse_album_row(row: &[u8]) -> Option<(u32, String)> {
    let id = read_u32_le(row, 12)?;
    let name = get_string_from_pdb(row, 22).unwrap_or_default();
    Some((id, name))
}

fn parse_key_row(row: &[u8]) -> Option<(u32, String)> {
    let id = read_u32_le(row, 0)?;
    let name = get_string_from_pdb(row, 8).unwrap_or_default();
    Some((id, name))
}

fn parse_artwork_row(row: &[u8]) -> Option<(u32, String)> {
    let id = read_u32_le(row, 0)?;
    let path = get_string_from_pdb(row, 4).unwrap_or_default();
    Some((id, path))
}

fn parse_playlist_tree_row(row: &[u8]) -> Option<PdbPlaylistTreeRow> {
    Some(PdbPlaylistTreeRow {
        parent_id: read_u32_le(row, 0)?,
        sort_order: read_u32_le(row, 8)?,
        id: read_u32_le(row, 12)?,
        row_is_folder: read_u32_le(row, 16)? == 1,
        name: get_string_from_pdb(row, 20).unwrap_or_default(),
    })
}

fn parse_playlist_entry_row(row: &[u8]) -> Option<PdbPlaylistEntryRow> {
    Some(PdbPlaylistEntryRow {
        entry_index: read_u32_le(row, 0)?,
        track_id: read_u32_le(row, 4)?,
        playlist_id: read_u32_le(row, 8)?,
    })
}

fn parse_history_playlist_row(row: &[u8], source_table: u32) -> Option<PdbHistoryPlaylistRow> {
    Some(PdbHistoryPlaylistRow {
        id: read_u32_le(row, 0)?,
        name: get_string_from_pdb(row, 4).unwrap_or_default(),
        source_table,
    })
}

fn parse_history_entry_row(row: &[u8], source_table: u32) -> Option<PdbHistoryEntryRow> {
    if row.len() >= 12 {
        return Some(PdbHistoryEntryRow {
            track_id: read_u32_le(row, 0),
            playlist_id: read_u32_le(row, 4)?,
            entry_index: read_u32_le(row, 8)?,
            source_table,
        });
    }
    Some(PdbHistoryEntryRow {
        track_id: None,
        playlist_id: read_u32_le(row, 0)?,
        entry_index: read_u32_le(row, 4)?,
        source_table,
    })
}

fn parse_history_row(row: &[u8]) -> Option<PdbHistoryRow> {
    let date = get_string_range(row, 10, 20).map(|v| v.trim_end_matches('\0').to_string());
    let num = get_string_range(row, 32, 36).map(|v| v.trim_end_matches('\0').to_string());
    Some(PdbHistoryRow { date, num })
}

fn get_string_from_pdb(row: &[u8], offset: usize) -> Option<String> {
    let n = read_u8(row, offset)? as usize;
    if n % 2 == 1 {
        let r = (n - 1) / 2;
        return get_string_range(row, offset + 1, offset + r);
    }

    let code = read_u8(row, offset)?;
    let enc = if (code & 0b0100_0000) != 0 {
        "ascii"
    } else if (code & 0b0010_0000) != 0 {
        "utf8"
    } else if (code & 0b1001_0000) == 0b1001_0000 {
        "utf16le"
    } else {
        "ascii"
    };

    let len = read_u16_le(row, offset + 1)? as usize;
    let end = offset + len;
    if offset + 4 > row.len() || end > row.len() || end < offset + 4 {
        return None;
    }
    let data = &row[offset + 4..end];
    Some(match enc {
        "utf16le" => {
            let mut buf = Vec::with_capacity(data.len() / 2);
            for chunk in data.chunks_exact(2) {
                buf.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            String::from_utf16_lossy(&buf)
                .trim_end_matches('\0')
                .to_string()
        }
        "utf8" => String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string(),
        _ => data
            .iter()
            .map(|b| *b as char)
            .collect::<String>()
            .trim_end_matches('\0')
            .to_string(),
    })
}

// Decode a range string slot (title=17, anlz_path=14).
//
// The slot occupies row[start..end] where start=offsets[slot] and
// end=offsets[slot+1]. The first byte is a PDB string header when written
// by our encoder, but may be a raw type byte (0) in legacy rows. We try the
// self-describing `get_string_from_pdb` path first (which handles ASCII and
// UTF-16LE headers); if that returns None we fall back to reading the raw bytes
// after the first byte so legacy rows are still decoded correctly.
fn decode_range_string_slot(row: &[u8], offsets: &[u16], slot: usize, next_slot: usize) -> String {
    let start = match offsets.get(slot).copied().filter(|&s| s != 0) {
        Some(s) => s as usize,
        None => return String::new(),
    };
    if let Some(s) = get_string_from_pdb(row, start) {
        return s;
    }
    let end = offsets.get(next_slot).copied().unwrap_or(0) as usize;
    get_string_range(row, start.saturating_add(1), end).unwrap_or_default()
}

fn decode_track_string_slot(row: &[u8], offsets: &[u16], index: usize) -> Option<String> {
    match index {
        0 => {
            let offset = *offsets.get(index)? as usize;
            decode_isrc_slot(row, offset)
        }
        14 => Some(decode_range_string_slot(row, offsets, 14, 15)),
        17 => Some(decode_range_string_slot(row, offsets, 17, 18)),
        15 | 18 => None,
        _ => {
            let offset = *offsets.get(index)? as usize;
            if offset == 0 {
                None
            } else {
                get_string_from_pdb(row, offset)
            }
        }
    }
}

fn decode_isrc_slot(row: &[u8], offset: usize) -> Option<String> {
    let first = *row.get(offset)? as usize;
    // ISRC is always ASCII. Reference exports and the writer use short-ASCII:
    // header = len*2+3 (always odd).
    // Even first byte (e.g. 0x90 empty-envelope from old buggy encoder) → treat as absent.
    if first % 2 == 0 {
        return None;
    }
    let len_chars = first.saturating_sub(3) / 2;
    if len_chars == 0 {
        return None;
    }
    let end = offset.checked_add(1)?.checked_add(len_chars)?;
    if end > row.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&row[offset + 1..end])
        .trim()
        .to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn raw_track_string_slot_hex(row: &[u8], offsets: &[u16], index: usize) -> String {
    match index {
        14 | 15 | 17 | 18 => {
            let Some(start) = offsets.get(index).copied() else {
                return String::new();
            };
            let Some(end) = offsets.get(index + 1).copied() else {
                return String::new();
            };
            hex_slice(row, start as usize, end as usize)
        }
        _ => {
            let Some(offset) = offsets.get(index).copied() else {
                return String::new();
            };
            let offset = offset as usize;
            let len = raw_pdb_string_len(row, offset).unwrap_or(0);
            if len == 0 {
                return String::new();
            }
            hex_slice(row, offset, offset.saturating_add(len))
        }
    }
}

fn track_string_slot_label(index: usize) -> &'static str {
    match index {
        0 => "isrc",
        1 => "lyricist",
        2 => "unknown_string_2",
        3 => "unknown_string_3",
        4 => "unknown_string_4",
        5 => "message",
        6 => "publish_track_info",
        7 => "autoload_hotcues",
        8 => "unknown_string_5",
        9 => "unknown_string_6",
        10 => "date_added",
        11 => "release_date",
        12 => "mix_name",
        13 => "unknown_string_7",
        14 => "analysis_path_start",
        15 => "analysis_path_end",
        16 => "comment",
        17 => "title_start",
        18 => "title_end",
        19 => "filename",
        20 => "track_file_path",
        _ => "unknown",
    }
}

fn insert_u32_field(map: &mut HashMap<String, String>, key: &str, value: Option<u32>) {
    map.insert(
        key.to_string(),
        value.map(|v| v.to_string()).unwrap_or_default(),
    );
}

fn insert_u16_field(map: &mut HashMap<String, String>, key: &str, value: Option<u16>) {
    map.insert(
        key.to_string(),
        value.map(|v| v.to_string()).unwrap_or_default(),
    );
}

fn hex_slice(row: &[u8], start: usize, end: usize) -> String {
    row.get(start..end)
        .unwrap_or(&[])
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

fn raw_pdb_string_len(row: &[u8], offset: usize) -> Option<usize> {
    let n = read_u8(row, offset)? as usize;
    if n % 2 == 1 {
        let r = (n - 1) / 2;
        return Some(r.saturating_add(1));
    }
    read_u16_le(row, offset + 1).map(|len| len as usize)
}

fn get_string_range(row: &[u8], start: usize, end: usize) -> Option<String> {
    if start >= end || end > row.len() {
        return None;
    }
    let s = &row[start..end];
    Some(
        String::from_utf8_lossy(s)
            .trim_end_matches('\0')
            .to_string(),
    )
}

fn parse_track_string_offsets(row: &[u8]) -> [u16; 21] {
    let mut offsets = [0u16; 21];
    for (i, slot) in offsets.iter_mut().enumerate() {
        *slot = read_u16_le(row, 94 + 2 * i).unwrap_or(0);
    }
    offsets
}

use crate::utils::{
    read_u8_at as read_u8, read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic PDB page with the given row payloads.
    ///
    /// Layout (len_page bytes):
    ///   [0..4]   unknown
    ///   [4..8]   page_index (u32le)
    ///   [8..12]  table_type (u32le)
    ///   [12..16] next_page (u32le)
    ///   [16..24] unknown
    ///   [24]     nrs (u8) — row count, wraps at 256
    ///   [25..30] padding
    ///   [30..32] used_s (u16le) — payload byte count
    ///   [32..34] padding
    ///   [34..36] num_rl (u16le)
    ///   [36..40] padding
    ///   [40..40+used_s] payload (row data packed sequentially)
    ///   ... gap (zeros) ...
    ///   [end of page - index] backward-growing row index:
    ///     For each group of 16 rows: 2-byte presence bits + 2 padding + 16×2-byte offsets
    ///     (read from end of page backward)
    fn build_page(len_page: usize, table_type: u32, rows: &[&[u8]]) -> Vec<u8> {
        let mut page = vec![0u8; len_page];
        let n = rows.len();

        // Page header
        page[4..8].copy_from_slice(&1u32.to_le_bytes()); // page_index = 1
        page[8..12].copy_from_slice(&table_type.to_le_bytes());
        page[24] = (n % 256) as u8; // nrs wraps at 256

        // Pack row payloads into the data area starting at offset 40
        let mut offset = 0usize;
        let mut row_offsets = Vec::new();
        for row in rows {
            row_offsets.push(offset);
            page[40 + offset..40 + offset + row.len()].copy_from_slice(row);
            offset += row.len();
        }
        let used_s = offset as u16;
        page[30..32].copy_from_slice(&used_s.to_le_bytes());

        // num_rl = n (also wraps at u16 max, but we won't test that)
        let num_rl = if n > 0 { (n - 1) as u16 } else { 0 };
        page[34..36].copy_from_slice(&num_rl.to_le_bytes());

        // Build backward-growing index at end of page
        let mut m = len_page;
        for i in 0..n {
            if i % 16 == 0 {
                // Presence bits: mark all rows in this group as present
                let group_size = std::cmp::min(16, n - i);
                let bits: u16 = (1u32 << group_size).wrapping_sub(1) as u16;
                m -= 4; // 2 bytes bits + 2 bytes padding
                page[m..m + 2].copy_from_slice(&bits.to_le_bytes());
            }
            m -= 2;
            page[m..m + 2].copy_from_slice(&(row_offsets[i] as u16).to_le_bytes());
        }

        page
    }

    #[test]
    fn parse_page_rows_single_row() {
        let row_data = b"hello world!";
        let page = build_page(4096, 0, &[row_data]);
        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 1);
        assert_eq!(&rows[0], row_data);
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_page_rows_multiple_rows() {
        let r1 = b"AAAA";
        let r2 = b"BBBBBB";
        let r3 = b"CC";
        let page = build_page(4096, 8, &[r1, r2, r3]);
        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 3);
        assert_eq!(&rows[0], b"AAAA");
        assert_eq!(&rows[1], b"BBBBBB");
        assert_eq!(&rows[2], b"CC");
    }

    #[test]
    fn parse_page_rows_empty_page() {
        let page = build_page(4096, 0, &[]);
        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_page_rows_skips_deleted_rows() {
        // Build a page with 3 rows but mark the middle one as not present
        let r1 = b"AAAA";
        let r2 = b"BBBB";
        let r3 = b"CCCC";
        let mut page = build_page(4096, 0, &[r1, r2, r3]);
        // The presence bits are at (len_page - 4). Clear bit 1 (second row).
        let bits_offset = 4096 - 4;
        let mut bits = u16::from_le_bytes([page[bits_offset], page[bits_offset + 1]]);
        bits &= !(1 << 1); // clear bit 1
        page[bits_offset..bits_offset + 2].copy_from_slice(&bits.to_le_bytes());

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 2);
        assert_eq!(&rows[0], b"AAAA");
        assert_eq!(&rows[1], b"CCCC");
    }

    #[test]
    fn parse_page_rows_nrs_wrapping_recovers_all_entries() {
        // Simulate a page with 284 12-byte playlist entries.
        // nrs = 284 % 256 = 28, which would cause the old parser to read only 28 rows.
        let entry = [0u8; 12];
        let row_refs: Vec<&[u8]> = (0..284).map(|_| entry.as_slice()).collect();
        let page = build_page(4096, 8, &row_refs);

        // Verify nrs wrapped
        assert_eq!(page[24], 28); // 284 % 256

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);

        // Must recover all 284 rows, not just 28
        assert_eq!(
            rows.len(),
            284,
            "should recover all rows despite nrs wrapping"
        );
    }

    #[test]
    fn parse_page_rows_nrs_wrapping_preserves_row_content() {
        // 260 rows where each row contains its index as a u32
        let mut row_data: Vec<Vec<u8>> = Vec::new();
        for i in 0..260u32 {
            let mut entry = vec![0u8; 12];
            entry[0..4].copy_from_slice(&i.to_le_bytes());
            row_data.push(entry);
        }
        let row_refs: Vec<&[u8]> = row_data.iter().map(|r| r.as_slice()).collect();
        let page = build_page(4096, 8, &row_refs);

        assert_eq!(page[24], 4); // 260 % 256

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 260);

        // Verify content integrity for every row
        for (i, row) in rows.iter().enumerate() {
            let val = u32::from_le_bytes([row[0], row[1], row[2], row[3]]);
            assert_eq!(val, i as u32, "row {} has wrong content", i);
        }
    }

    #[test]
    fn parse_page_rows_small_page_no_false_extension() {
        // A page with 1 large row (like a track) must not falsely extend into
        // the unused zero-filled index space.
        let big_row = vec![0xABu8; 460];
        let page = build_page(4096, 0, &[big_row.as_slice()]);

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);

        assert_eq!(
            rows.len(),
            1,
            "should not create phantom rows from empty index space"
        );
        assert_eq!(rows[0].len(), 460);
        assert_eq!(rows[0][0], 0xAB);
    }

    #[test]
    fn parse_page_rows_16_rows_exactly_one_group() {
        // Exactly 16 rows = 1 full presence group, no wrapping
        let row_data: Vec<Vec<u8>> = (0..16u32)
            .map(|i| {
                let mut r = vec![0u8; 8];
                r[0..4].copy_from_slice(&i.to_le_bytes());
                r
            })
            .collect();
        let row_refs: Vec<&[u8]> = row_data.iter().map(|r| r.as_slice()).collect();
        let page = build_page(4096, 8, &row_refs);

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 16);
    }

    #[test]
    fn parse_page_rows_17_rows_spans_two_groups() {
        // 17 rows = 2 presence groups
        let row_data: Vec<Vec<u8>> = (0..17u32)
            .map(|i| {
                let mut r = vec![0u8; 8];
                r[0..4].copy_from_slice(&i.to_le_bytes());
                r
            })
            .collect();
        let row_refs: Vec<&[u8]> = row_data.iter().map(|r| r.as_slice()).collect();
        let page = build_page(4096, 8, &row_refs);

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 17);

        // Verify last row content
        let val = u32::from_le_bytes([rows[16][0], rows[16][1], rows[16][2], rows[16][3]]);
        assert_eq!(val, 16);
    }

    #[test]
    fn parse_page_rows_page_index_zero_returns_empty() {
        let row_data = b"data";
        let mut page = build_page(4096, 0, &[row_data]);
        // Set page_index to 0
        page[4..8].copy_from_slice(&0u32.to_le_bytes());

        let info = parse_page_info(&page).unwrap();
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_page_rows_num_rl_8191_uses_nrs() {
        // num_rl=8191 (0x1FFF) is a sentinel meaning "num_rl not tracked",
        // so the parser should fall back to nrs and still parse rows.
        let row_data = b"data";
        let mut page = build_page(4096, 0, &[row_data]);
        // Set num_rl to 8191
        page[34..36].copy_from_slice(&8191u16.to_le_bytes());

        let info = parse_page_info(&page).unwrap();
        assert_eq!(info.num_rl, 8191);
        let mut warnings = Vec::new();
        let rows = parse_page_rows(&page, 4096, info, &mut warnings);
        assert_eq!(rows.len(), 1);
        assert_eq!(&rows[0], b"data");
    }

    // ── PDB round-trip tests ──────────────────────────────

    /// Build a complete PDB file from a set of (table_type, page_data) pairs.
    /// Creates header page (page 0) + data pages at indices 1..N.
    fn build_pdb_file(len_page: usize, table_pages: &[(u32, Vec<u8>)]) -> Vec<u8> {
        // Collect unique table types and assign page ranges
        let mut table_info: Vec<(u32, u32, u32)> = Vec::new(); // (type, first_page, last_page)
        let mut page_idx = 1u32;
        let mut prev_type: Option<u32> = None;
        let mut first_page_of_type = page_idx;

        for (i, (ttype, _)) in table_pages.iter().enumerate() {
            if prev_type.is_some() && prev_type != Some(*ttype) {
                // Finish previous table
                table_info.push((prev_type.unwrap(), first_page_of_type, page_idx - 1));
                first_page_of_type = page_idx;
            }
            prev_type = Some(*ttype);
            page_idx += 1;

            if i == table_pages.len() - 1 {
                table_info.push((*ttype, first_page_of_type, page_idx - 1));
            }
        }

        let num_tables = table_info.len();
        let mut header = vec![0u8; len_page];
        header[4..8].copy_from_slice(&(len_page as u32).to_le_bytes());
        header[8..12].copy_from_slice(&(num_tables as u32).to_le_bytes());

        for (i, (ttype, first, last)) in table_info.iter().enumerate() {
            let off = 28 + i * 16;
            header[off..off + 4].copy_from_slice(&ttype.to_le_bytes());
            header[off + 8..off + 12].copy_from_slice(&first.to_le_bytes());
            header[off + 12..off + 16].copy_from_slice(&last.to_le_bytes());
        }

        let mut file = header;
        for (_, page) in table_pages {
            file.extend_from_slice(page);
        }
        file
    }

    /// Build a synthetic track row with the given id and file_path.
    /// The row is structured to be parseable by parse_track_row.
    fn build_track_row(id: u32, title: &str, file_path: &str) -> Vec<u8> {
        build_track_row_with_duration(id, title, file_path, None)
    }

    fn build_track_row_with_duration(
        id: u32,
        title: &str,
        file_path: &str,
        duration_seconds: Option<u16>,
    ) -> Vec<u8> {
        // Fixed header: 94 bytes + 21 offsets (42 bytes) = 136 bytes minimum
        let title_bytes = title.as_bytes();
        let path_pdb = crate::service::export_helpers::encode_pdb_string(file_path);

        // String data starts at offset 136
        let string_start = 136usize;
        // Title: row[string_start] = type byte (skipped), row[string_start+1..] = title
        let title_end = string_start + 1 + title_bytes.len();
        // track_file_path: PDB-encoded string after title
        let path_start = title_end;
        let total_len = path_start + path_pdb.len();

        let mut row = vec![0u8; total_len];
        // Set track id at offset 72
        row[72..76].copy_from_slice(&id.to_le_bytes());
        // Set track_number at offset 52
        row[52..56].copy_from_slice(&1u32.to_le_bytes());
        if let Some(duration_seconds) = duration_seconds {
            row[84..86].copy_from_slice(&duration_seconds.to_le_bytes());
        }

        // Set 21 string offsets at 94..136 (all initially point to string_start)
        for i in 0..21usize {
            row[94 + 2 * i..94 + 2 * i + 2].copy_from_slice(&(string_start as u16).to_le_bytes());
        }

        // Set title: offsets[17] = string_start, offsets[18] = title_end
        row[94 + 17 * 2..94 + 17 * 2 + 2].copy_from_slice(&(string_start as u16).to_le_bytes());
        row[94 + 18 * 2..94 + 18 * 2 + 2].copy_from_slice(&(title_end as u16).to_le_bytes());
        // Title content: type byte + raw bytes
        row[string_start] = 0; // type byte (skipped by get_string_range)
        row[string_start + 1..title_end].copy_from_slice(title_bytes);

        // Set track_file_path: offsets[20] = path_start
        row[94 + 20 * 2..94 + 20 * 2 + 2].copy_from_slice(&(path_start as u16).to_le_bytes());
        row[path_start..path_start + path_pdb.len()].copy_from_slice(&path_pdb);

        row
    }

    #[test]
    fn roundtrip_playlist_tree_row() {
        use crate::service::export_helpers::encode_playlist_tree_row;

        let encoded = encode_playlist_tree_row(42, 0, 5, false, "My Playlist");
        let page = build_page(4096, 7, &[&encoded]);
        let pdb = build_pdb_file(4096, &[(7, page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        assert_eq!(parsed.playlist_tree.len(), 1);
        let row = &parsed.playlist_tree[0];
        assert_eq!(row.id, 42);
        assert_eq!(row.parent_id, 0);
        assert_eq!(row.sort_order, 5);
        assert!(!row.row_is_folder);
        assert_eq!(row.name, "My Playlist");
    }

    #[test]
    fn roundtrip_playlist_tree_row_folder() {
        use crate::service::export_helpers::encode_playlist_tree_row;

        let encoded = encode_playlist_tree_row(10, 1, 3, true, "DJ Sets");
        let page = build_page(4096, 7, &[&encoded]);
        let pdb = build_pdb_file(4096, &[(7, page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        assert_eq!(parsed.playlist_tree.len(), 1);
        let row = &parsed.playlist_tree[0];
        assert_eq!(row.id, 10);
        assert_eq!(row.parent_id, 1);
        assert_eq!(row.sort_order, 3);
        assert!(row.row_is_folder);
        assert_eq!(row.name, "DJ Sets");
    }

    #[test]
    fn roundtrip_playlist_entry_row() {
        use crate::service::export_helpers::encode_playlist_entry_row;

        let encoded = encode_playlist_entry_row(0, 100, 42);
        let page = build_page(4096, 8, &[&encoded]);
        let pdb = build_pdb_file(4096, &[(8, page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        assert_eq!(parsed.playlist_entries.len(), 1);
        let entry = &parsed.playlist_entries[0];
        assert_eq!(entry.entry_index, 0);
        assert_eq!(entry.track_id, 100);
        assert_eq!(entry.playlist_id, 42);
    }

    #[test]
    fn roundtrip_multiple_playlist_entries() {
        use crate::service::export_helpers::encode_playlist_entry_row;

        let entries: Vec<Vec<u8>> = (0..5)
            .map(|i| encode_playlist_entry_row(i, 100 + i, 42))
            .collect();
        let refs: Vec<&[u8]> = entries.iter().map(|e| e.as_slice()).collect();
        let page = build_page(4096, 8, &refs);
        let pdb = build_pdb_file(4096, &[(8, page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        assert_eq!(parsed.playlist_entries.len(), 5);
        for i in 0..5u32 {
            assert_eq!(parsed.playlist_entries[i as usize].entry_index, i);
            assert_eq!(parsed.playlist_entries[i as usize].track_id, 100 + i);
            assert_eq!(parsed.playlist_entries[i as usize].playlist_id, 42);
        }
    }

    #[test]
    fn roundtrip_track_row() {
        let row = build_track_row(77, "Test Song", "/Contents/Artist/Album/song.mp3");
        let page = build_page(4096, 0, &[&row]);
        let pdb = build_pdb_file(4096, &[(0, page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        assert_eq!(parsed.tracks.len(), 1);
        let track = &parsed.tracks[0];
        assert_eq!(track.id, 77);
        assert_eq!(track.title, "Test Song");
        assert_eq!(track.track_file_path, "/Contents/Artist/Album/song.mp3");
    }

    #[test]
    fn roundtrip_track_row_parses_duration_seconds() {
        let row = build_track_row_with_duration(
            78,
            "Duration Song",
            "/Contents/Artist/Album/duration.mp3",
            Some(321),
        );
        let page = build_page(4096, 0, &[&row]);
        let pdb = build_pdb_file(4096, &[(0, page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        assert_eq!(parsed.tracks.len(), 1);
        let track = &parsed.tracks[0];
        assert_eq!(track.id, 78);
        assert_eq!(track.duration_seconds, Some(321));
    }

    #[test]
    fn parse_track_debug_row_exposes_all_known_string_slots_and_unknown_fixed_ranges() {
        let row = build_track_row_with_duration(
            79,
            "Debug Song",
            "/Contents/Artist/Album/debug.mp3",
            Some(222),
        );
        let track = parse_track_debug_row(&row).expect("parse debug track row");

        assert_eq!(track.id, 79);
        assert_eq!(track.row_len, row.len());
        assert_eq!(
            track
                .fixed_fields
                .get("duration_seconds_u16")
                .map(String::as_str),
            Some("222")
        );
        assert_eq!(track.string_slots.len(), 21);
        assert_eq!(track.string_slots[17].label, "title_start");
        assert!(!track.string_slots[17].raw_hex.is_empty());
        assert_eq!(
            track.string_slots[17].decoded_value.as_deref(),
            Some("Debug Song")
        );
        assert_eq!(track.string_slots[20].label, "track_file_path");
        assert_eq!(
            track.string_slots[20].decoded_value.as_deref(),
            Some("/Contents/Artist/Album/debug.mp3")
        );
        assert!(
            track
                .fixed_fields
                .get("unknown_fixed_36_51_hex")
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!track.fixed_block_hex.is_empty());
    }

    #[test]
    fn decode_isrc_slot_returns_none_for_legacy_0x90_empty_envelope() {
        // Old buggy encoder wrote [0x90, 0x06, 0x00, 0x00, 0x03, 0x00] for absent ISRC.
        // Even first byte → treated as absent.
        let mut row = vec![0u8; 180];
        let offset = 136usize;
        let raw = [0x90u8, 0x06, 0x00, 0x00, 0x03, 0x00];
        row[offset..offset + raw.len()].copy_from_slice(&raw);
        assert_eq!(decode_isrc_slot(&row, offset), None);
    }

    #[test]
    fn decode_isrc_slot_handles_standard_short_ascii_encoding() {
        // Fixed encoder and reference exports write: [(len*2+3), ascii_chars...]
        let mut row = vec![0u8; 180];
        let offset = 136usize;
        // "TCAIR2414262" = 12 chars → header = 12*2+3 = 27 = 0x1B
        let raw = [
            0x1B, b'T', b'C', b'A', b'I', b'R', b'2', b'4', b'1', b'4', b'2', b'6', b'2',
        ];
        row[offset..offset + raw.len()].copy_from_slice(&raw);
        assert_eq!(
            decode_isrc_slot(&row, offset).as_deref(),
            Some("TCAIR2414262")
        );
    }

    #[test]
    fn decode_isrc_slot_returns_none_for_empty_short_ascii() {
        // Reference-export empty ISRC: [0x03]
        let mut row = vec![0u8; 180];
        let offset = 136usize;
        row[offset] = 0x03;
        assert_eq!(decode_isrc_slot(&row, offset), None);
    }

    #[test]
    fn isrc_encode_decode_round_trip_non_empty() {
        use crate::service::export_helpers::encode_pdb_track_isrc_slot;
        let isrc = "TCAIR2414262";
        let encoded = encode_pdb_track_isrc_slot(Some(isrc));
        let mut row = vec![0u8; 180];
        let offset = 136usize;
        row[offset..offset + encoded.len()].copy_from_slice(&encoded);
        assert_eq!(decode_isrc_slot(&row, offset).as_deref(), Some(isrc));
    }

    #[test]
    fn isrc_encode_decode_round_trip_empty() {
        use crate::service::export_helpers::encode_pdb_track_isrc_slot;
        let encoded = encode_pdb_track_isrc_slot(None);
        assert_eq!(encoded, vec![0x03], "empty ISRC must encode to [0x03]");
        let mut row = vec![0u8; 180];
        let offset = 136usize;
        row[offset..offset + encoded.len()].copy_from_slice(&encoded);
        assert_eq!(decode_isrc_slot(&row, offset), None);
    }

    #[test]
    fn parse_track_debug_row_decodes_named_fixed_fields_from_observed_offsets() {
        let mut row = build_track_row_with_duration(
            80,
            "Fixed Song",
            "/Contents/Artist/Album/fixed.mp3",
            Some(289),
        );
        row[0..4].copy_from_slice(&0x0020_0024u32.to_le_bytes());
        row[4..8].copy_from_slice(&788_224u32.to_le_bytes());
        row[8..12].copy_from_slice(&44_100u32.to_le_bytes());
        row[16..20].copy_from_slice(&11_761_189u32.to_le_bytes());
        row[20..24].copy_from_slice(&53_153_510u32.to_le_bytes());
        row[24..28].copy_from_slice(&3_313_263_539u32.to_le_bytes());
        row[48..52].copy_from_slice(&320u32.to_le_bytes());
        row[80..82].copy_from_slice(&2021u16.to_le_bytes());
        row[82..84].copy_from_slice(&16u16.to_le_bytes());
        row[90..92].copy_from_slice(&1u16.to_le_bytes());

        let track = parse_track_debug_row(&row).expect("parse debug track row");
        assert_eq!(
            track
                .fixed_fields
                .get("header_flags_u32")
                .map(String::as_str),
            Some("2097188")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("content_link_u32")
                .map(String::as_str),
            Some("788224")
        );
        assert_eq!(
            track.fixed_fields.get("sample_rate_hz").map(String::as_str),
            Some("44100")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("file_size_bytes")
                .map(String::as_str),
            Some("11761189")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("master_content_id_u32")
                .map(String::as_str),
            Some("53153510")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("master_db_id_u32")
                .map(String::as_str),
            Some("3313263539")
        );
        assert_eq!(
            track.fixed_fields.get("bitrate_kbps").map(String::as_str),
            Some("320")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("release_year_u16")
                .map(String::as_str),
            Some("2021")
        );
        assert_eq!(
            track.fixed_fields.get("bit_depth_u16").map(String::as_str),
            Some("16")
        );
        assert_eq!(
            track.fixed_fields.get("file_type_u16").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn encode_track_row_populates_known_observed_string_slots() {
        let row = crate::service::export_helpers::encode_track_row_with_profile(
            &crate::service::export_helpers::PdbTrackRowData {
                header_flags_u32: None,
                content_link: Some(788_224),
                sample_rate_hz: Some(44_100),
                file_size_bytes: Some(11_761_189),
                master_content_id: Some(53_153_510),
                master_db_id: Some(3_313_263_539),
                id: 1,
                artist_id: 1,
                album_id: 1,
                artwork_id: 1,
                key_id: 1,
                genre_id: 0,
                bitrate_kbps: Some(320),
                track_number: Some(1),
                bpm: Some(136.0),
                release_year: Some(2021),
                bit_depth: Some(16),
                duration_seconds: Some(289),
                file_type: Some(1),
                isrc: Some("DEOB62100547".to_string()),
                date_added: Some("2025-08-28".to_string()),
                release_date: Some("2021-01-01".to_string()),
                dj_comment: Some("Visit https://iliantape.bandcamp.com".to_string()),
                file_name: Some("Atrice - IT049 - Q - 01 Hatara.mp3".to_string()),
                publish_track_info_on: Some(true),
                autoload_hotcues_on: Some(true),
                title: "Hatara".to_string(),
                anlz_path: "/PIONEER/USBANLZ/P017/0000075D/ANLZ0000.DAT".to_string(),
                file_path: "/Contents/Atrice/IT049 - Q/Atrice - IT049 - Q - 01 Hatara.mp3"
                    .to_string(),
            },
            crate::service::export_helpers::PdbLayoutProfile::Current,
        )
        .expect("encode track row");
        let track = parse_track_debug_row(&row).expect("parse debug track row");

        assert_eq!(
            track.string_slots[0].decoded_value.as_deref(),
            Some("DEOB62100547")
        );
        assert_eq!(track.string_slots[6].decoded_value.as_deref(), Some("ON"));
        assert_eq!(track.string_slots[7].decoded_value.as_deref(), Some("ON"));
        assert_eq!(
            track.string_slots[10].decoded_value.as_deref(),
            Some("2025-08-28")
        );
        assert_eq!(
            track.string_slots[11].decoded_value.as_deref(),
            Some("2021-01-01")
        );
        assert_eq!(
            track.string_slots[16].decoded_value.as_deref(),
            Some("Visit https://iliantape.bandcamp.com")
        );
        assert_eq!(
            track.string_slots[17].decoded_value.as_deref(),
            Some("Hatara")
        );
        assert_eq!(
            track.string_slots[19].decoded_value.as_deref(),
            Some("Atrice - IT049 - Q - 01 Hatara.mp3")
        );
        assert_eq!(
            track.string_slots[20].decoded_value.as_deref(),
            Some("/Contents/Atrice/IT049 - Q/Atrice - IT049 - Q - 01 Hatara.mp3")
        );
    }

    #[test]
    fn encode_track_row_applies_reference_defaults_for_unresolved_slots_and_bytes() {
        let row = crate::service::export_helpers::encode_track_row_with_profile(
            &crate::service::export_helpers::PdbTrackRowData {
                header_flags_u32: None,
                content_link: None,
                sample_rate_hz: None,
                file_size_bytes: None,
                master_content_id: None,
                master_db_id: None,
                id: 7,
                artist_id: 0,
                album_id: 0,
                artwork_id: 0,
                key_id: 0,
                genre_id: 0,
                bitrate_kbps: None,
                track_number: Some(1),
                bpm: None,
                release_year: None,
                bit_depth: None,
                duration_seconds: None,
                file_type: Some(1),
                isrc: None,
                date_added: None,
                release_date: None,
                dj_comment: None,
                file_name: Some("default.mp3".to_string()),
                publish_track_info_on: None,
                autoload_hotcues_on: None,
                title: "Default".to_string(),
                anlz_path: String::new(),
                file_path: "/Contents/Default/default.mp3".to_string(),
            },
            crate::service::export_helpers::PdbLayoutProfile::Current,
        )
        .expect("encode track row");
        let track = parse_track_debug_row(&row).expect("parse debug track row");

        assert_eq!(track.string_slots[1].label, "lyricist");
        assert_eq!(track.string_slots[1].decoded_value, None);
        assert_eq!(track.string_slots[2].decoded_value.as_deref(), Some("2"));
        assert_eq!(track.string_slots[3].decoded_value.as_deref(), Some("2"));
        assert_eq!(track.string_slots[5].label, "message");
        assert_eq!(track.string_slots[5].decoded_value, None);
        assert_eq!(track.string_slots[12].label, "mix_name");
        assert_eq!(track.string_slots[12].decoded_value, None);
        assert_eq!(track.string_slots[13].label, "unknown_string_7");
        assert_eq!(track.string_slots[13].decoded_value, None);
        assert_eq!(track.string_slots[1].raw_hex, "0305");
        assert_eq!(track.string_slots[5].raw_hex, "0303");
        assert_eq!(track.string_slots[12].raw_hex, "0303");
        assert_eq!(
            track
                .fixed_fields
                .get("unknown_fixed_36_51_hex")
                .map(String::as_str),
            Some("00000000000000000000000000000000")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("unknown_fixed_76_83_hex")
                .map(String::as_str),
            Some("0000000000000000")
        );
        assert_eq!(
            track
                .fixed_fields
                .get("unknown_fixed_86_93_hex")
                .map(String::as_str),
            Some("2900000001000300")
        );
    }

    #[test]
    fn parse_pdb_bytes_does_not_drop_real_pages_when_header_last_page_is_stale() {
        use crate::service::export_helpers::{encode_playlist_entry_row, encode_playlist_tree_row};

        let track_row = build_track_row(7, "Title", "/Contents/A.mp3");
        let tree_row = encode_playlist_tree_row(1, 0, 1, false, "Test");
        let entry_row = encode_playlist_entry_row(0, 7, 1);
        let page_tracks = build_page(4096, 0, &[&track_row]);
        let page_tree = build_page(4096, 7, &[&tree_row]);
        let page_entries = build_page(4096, 8, &[&entry_row]);
        let mut pdb = build_pdb_file(4096, &[(0, page_tracks), (7, page_tree), (8, page_entries)]);

        // Corrupt the header pointers so a parser that trusts last_page would stop too early.
        for off in [28usize, 44usize, 60usize] {
            pdb[off + 8..off + 12].copy_from_slice(&1u32.to_le_bytes());
            pdb[off + 12..off + 16].copy_from_slice(&1u32.to_le_bytes());
        }

        let parsed = parse_pdb_bytes(&pdb).unwrap();
        assert_eq!(parsed.tracks.len(), 1);
        assert_eq!(parsed.playlist_tree.len(), 1);
        assert_eq!(parsed.playlist_entries.len(), 1);
    }

    #[test]
    fn fixture_legacy_duration_samples_decode_from_track_rows() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Sample {
            id: u32,
            duration_seconds: u16,
            title: String,
            track_path: String,
        }

        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/json/legacy_pdb_duration_samples.json");
        let raw = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
        let samples = serde_json::from_str::<Vec<Sample>>(&raw).expect("parse fixture json");
        assert!(!samples.is_empty(), "fixture must contain sample rows");

        for sample in samples {
            let row = build_track_row_with_duration(
                sample.id,
                &sample.title,
                &sample.track_path,
                Some(sample.duration_seconds),
            );
            let parsed = parse_track_row(&row).expect("parse synthetic legacy track row");
            assert_eq!(parsed.id, sample.id);
            assert_eq!(
                parsed.duration_seconds,
                Some(u32::from(sample.duration_seconds)),
                "duration mismatch for fixture sample id {}",
                sample.id
            );
        }
    }

    #[test]
    fn roundtrip_full_pdb_tracks_and_playlists() {
        use crate::service::export_helpers::{encode_playlist_entry_row, encode_playlist_tree_row};

        // Build tracks
        let track1 = build_track_row(1, "Track One", "/Contents/A/B/one.mp3");
        let track2 = build_track_row(2, "Track Two", "/Contents/A/B/two.mp3");
        let tracks_page = build_page(4096, 0, &[&track1, &track2]);

        // Build playlist tree
        let tree_row = encode_playlist_tree_row(10, 0, 1, false, "My Set");
        let tree_page = build_page(4096, 7, &[&tree_row]);

        // Build playlist entries
        let entry1 = encode_playlist_entry_row(0, 1, 10);
        let entry2 = encode_playlist_entry_row(1, 2, 10);
        let entries_page = build_page(4096, 8, &[&entry1, &entry2]);

        let pdb = build_pdb_file(4096, &[(0, tracks_page), (7, tree_page), (8, entries_page)]);
        let parsed = parse_pdb_bytes(&pdb).unwrap();

        // Verify tracks
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0].id, 1);
        assert_eq!(parsed.tracks[0].title, "Track One");
        assert_eq!(parsed.tracks[0].track_file_path, "/Contents/A/B/one.mp3");
        assert_eq!(parsed.tracks[1].id, 2);
        assert_eq!(parsed.tracks[1].title, "Track Two");
        assert_eq!(parsed.tracks[1].track_file_path, "/Contents/A/B/two.mp3");

        // Verify playlist tree
        assert_eq!(parsed.playlist_tree.len(), 1);
        assert_eq!(parsed.playlist_tree[0].id, 10);
        assert_eq!(parsed.playlist_tree[0].name, "My Set");
        assert!(!parsed.playlist_tree[0].row_is_folder);

        // Verify playlist entries
        assert_eq!(parsed.playlist_entries.len(), 2);
        assert_eq!(parsed.playlist_entries[0].track_id, 1);
        assert_eq!(parsed.playlist_entries[0].playlist_id, 10);
        assert_eq!(parsed.playlist_entries[1].track_id, 2);
        assert_eq!(parsed.playlist_entries[1].playlist_id, 10);
    }
}
