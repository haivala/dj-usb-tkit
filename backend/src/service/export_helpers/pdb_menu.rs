//! PDB t16 (columns catalog) and t17 (category snapshot) read/write helpers.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::super::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};
use crate::error::{BackendError, BackendResult};
use crate::pdb_writer::{parse_page_row_slots, rewrite_variable_page_rows_in_place};
use crate::utils::{
    collect_chain as collect_chain_pages, page_offset, read_u16_le_at, table_ptr_fields,
};

fn load_pdb_t16_rows(bytes: &[u8], page_size: usize) -> BackendResult<(Vec<u32>, Vec<Vec<u8>>)> {
    let Some((_ec16, first16, last16)) = table_ptr_fields(bytes, 16) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let chain16 = collect_chain_pages(bytes, page_size, first16, last16).ok_or_else(|| {
        BackendError::Validation("PDB columns patch failed: invalid t16 chain".to_string())
    })?;
    if chain16.len() <= 1 {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut data_pages = Vec::<u32>::new();
    let mut rows = Vec::<Vec<u8>>::new();
    for page_idx in chain16.iter().skip(1).copied() {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
        if used_s == 0 {
            continue;
        }
        let payload_len = used_s.min(page_size.saturating_sub(40));
        let Some(payload) = page.get(40..40 + payload_len) else {
            continue;
        };
        let mut page_rows = Vec::<Vec<u8>>::new();
        for slot in parse_page_row_slots(page, page_size) {
            if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            page_rows.push(payload[slot.start..slot.end].to_vec());
        }
        if !page_rows.is_empty() {
            data_pages.push(page_idx);
            rows.extend(page_rows);
        }
    }
    Ok((data_pages, rows))
}

fn rewrite_pdb_t16_rows(
    bytes: &mut [u8],
    page_size: usize,
    data_pages: &[u32],
    rows: &[Vec<u8>],
) -> BackendResult<()> {
    if data_pages.is_empty() {
        return Ok(());
    }
    let mut cursor = 0usize;
    for (idx, page_idx) in data_pages.iter().copied().enumerate() {
        let pages_left = data_pages.len() - idx;
        let rows_left = rows.len().saturating_sub(cursor);
        let mut page_rows = Vec::<Vec<u8>>::new();
        if rows_left > 0 {
            let target = rows_left.div_ceil(pages_left).max(1);
            for row in rows.iter().skip(cursor).take(target) {
                page_rows.push(row.clone());
            }
            while !rewrite_variable_page_rows_in_place(bytes, page_idx, &page_rows, page_size) {
                if page_rows.is_empty() {
                    break;
                }
                let _ = page_rows.pop();
            }
            if page_rows.is_empty() {
                return Err(BackendError::Validation(format!(
                    "PDB columns patch failed: unable to fit any rows on t16 page {}",
                    page_idx
                )));
            }
        }
        if !rewrite_variable_page_rows_in_place(bytes, page_idx, &page_rows, page_size) {
            return Err(BackendError::Validation(format!(
                "PDB columns patch failed: could not rewrite t16 page {}",
                page_idx
            )));
        }
        cursor += page_rows.len();
    }
    if cursor < rows.len() {
        return Err(BackendError::Validation(
            "PDB columns patch failed: t16 rows exceeded existing page capacity".to_string(),
        ));
    }
    Ok(())
}

fn row_kind_u16(row: &[u8]) -> Option<u16> {
    read_u16_le_at(row, 2)
}

fn row_id_u16(row: &[u8]) -> Option<u16> {
    read_u16_le_at(row, 0)
}

/// Decode a PDB string at `offset` in `row`. Returns the decoded String.
/// Handles the three PDB string variants:
/// - 0x90: long UTF-16LE with 2-byte total length + pad
/// - 0x40: long ASCII with 2-byte total length + pad
/// - odd marker (len*2+3): short ASCII
///
/// If the decoded string is wrapped in `\u{fffa}...\u{fffb}` mojibake markers
/// (used for localizable menu labels), those markers are stripped.
fn decode_pdb_row_string(row: &[u8], offset: usize) -> Option<String> {
    let marker = *row.get(offset)?;
    let raw = match marker {
        0x90 | 0x40 => {
            let total = read_u16_le_at(row, offset + 1)? as usize;
            if total < 4 {
                return None;
            }
            let body_start = offset + 4;
            let body_end = offset + total;
            let body = row.get(body_start..body_end)?;
            if marker == 0x90 {
                if body.len() % 2 != 0 {
                    return None;
                }
                let code_units = body
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>();
                String::from_utf16(&code_units).ok()?
            } else {
                let end = body.iter().position(|b| *b == 0).unwrap_or(body.len());
                String::from_utf8(body[..end].to_vec()).ok()?
            }
        }
        m if m >= 3 && m % 2 == 1 => {
            let len = (m as usize - 3) / 2;
            let body_start = offset + 1;
            let body = row.get(body_start..body_start + len)?;
            String::from_utf8(body.to_vec()).ok()?
        }
        _ => return None,
    };
    Some(
        raw.trim_matches(|c: char| c == '\u{fffa}' || c == '\u{fffb}' || c == '\u{0000}')
            .to_string(),
    )
}

/// Decoded PDB t16 (columns) row.
#[derive(Debug, Clone)]
pub struct PdbT16Row {
    pub id: u16,
    pub kind: u16,
    pub name: String,
}

/// Encode a PDB t16 row with the standard menu-label format:
/// `u16 id | u16 kind | 0x90 long-UTF16LE string wrapped in U+FFFA..U+FFFB`,
/// padded with NUL bytes so the row length is a multiple of 4.
///
/// The name is encoded exactly once: callers pass the clean menu label
/// (e.g. `"GENRE"`), not a pre-wrapped string. Round-trips byte-for-byte
/// against reference t16 rows across the test sample set.
pub fn encode_pdb_t16_row(id: u16, kind: u16, name: &str) -> Vec<u8> {
    let chars: Vec<u16> = name.encode_utf16().collect();
    let body_chars = chars.len() + 2;
    let string_total = 4 + body_chars * 2;
    if string_total > u16::MAX as usize {
        // Menu names this long never occur in practice; clamp defensively.
        return Vec::new();
    }
    let body_bytes = 4 + string_total;
    let padded = (body_bytes + 3) & !3;
    let mut row = Vec::with_capacity(padded);
    row.extend_from_slice(&id.to_le_bytes());
    row.extend_from_slice(&kind.to_le_bytes());
    row.push(0x90);
    row.extend_from_slice(&(string_total as u16).to_le_bytes());
    row.push(0);
    row.extend_from_slice(&0xfffau16.to_le_bytes());
    for code in chars {
        row.extend_from_slice(&code.to_le_bytes());
    }
    row.extend_from_slice(&0xfffbu16.to_le_bytes());
    while row.len() < padded {
        row.push(0);
    }
    row
}

/// Decode raw PDB t16 row bytes from an in-memory PDB image. Returns the row
/// payloads in their on-disk order with header metadata stripped, suitable for
/// re-emission via the writer (which will repack and rewrite page indices).
///
/// Used by the export pipeline to keep an existing stick's t16 byte-stable
/// instead of replacing it with a synthesized default — newer players have
/// rejected sticks whose t16 row count changed even when older players
/// accepted the result.
pub fn load_pdb_t16_rows_from_bytes(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let page_size = 4096usize;
    if bytes.len() < page_size || !bytes.len().is_multiple_of(page_size) {
        return None;
    }
    let (_pages, rows) = load_pdb_t16_rows(bytes, page_size).ok()?;
    if rows.is_empty() { None } else { Some(rows) }
}

/// Load raw PDB t16 row bytes for a USB root. Used by write-path tests to
/// verify byte-for-byte round-trip equality between decode and re-encode.
pub fn load_pdb_t16_raw(usb_root: &Path) -> BackendResult<Vec<Vec<u8>>> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if !pdb_path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&pdb_path)?;
    let page_size = 4096usize;
    if bytes.len() < page_size || bytes.len() % page_size != 0 {
        return Ok(Vec::new());
    }
    let (_pages, rows) = load_pdb_t16_rows(&bytes, page_size)?;
    Ok(rows)
}

/// Load decoded PDB t16 rows for a USB root. PDB is the source older players
/// use for the browse menu.
pub fn load_pdb_t16_decoded(usb_root: &Path) -> BackendResult<Vec<PdbT16Row>> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if !pdb_path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&pdb_path)?;
    let page_size = 4096usize;
    if bytes.len() < page_size || bytes.len() % page_size != 0 {
        return Ok(Vec::new());
    }
    let (_pages, rows) = load_pdb_t16_rows(&bytes, page_size)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(id) = row_id_u16(&row) else { continue };
        let Some(kind) = row_kind_u16(&row) else {
            continue;
        };
        let name = decode_pdb_row_string(&row, 4).unwrap_or_default();
        out.push(PdbT16Row { id, kind, name });
    }
    Ok(out)
}

pub fn inspect_pdb_columns_playlist_order(usb_root: &Path) -> BackendResult<Option<(bool, usize)>> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if !pdb_path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&pdb_path)?;
    let page_size = 4096usize;
    if bytes.len() < page_size || bytes.len() % page_size != 0 {
        return Ok(None);
    }
    let (_pages, rows) = load_pdb_t16_rows(&bytes, page_size)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let playlist_idx = rows
        .iter()
        .position(|r| row_kind_u16(r) == Some(132) || row_id_u16(r) == Some(17));
    let Some(playlist_idx) = playlist_idx else {
        return Ok(None);
    };
    Ok(Some((playlist_idx != 0, rows.len())))
}

pub fn patch_pdb_columns_menu_order_by_kind(
    usb_root: &Path,
    desired_kinds: &[u16],
) -> BackendResult<bool> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let mut bytes = std::fs::read(&pdb_path)?;
    let page_size = 4096usize;
    if bytes.len() < page_size || bytes.len() % page_size != 0 {
        return Err(BackendError::Validation(
            "PDB columns patch failed: invalid page alignment".to_string(),
        ));
    }

    let (data_pages, rows) = load_pdb_t16_rows(&bytes, page_size)?;
    if data_pages.is_empty() || rows.is_empty() {
        return Ok(false);
    }
    let original_rows = rows.clone();
    let mut remaining = rows;
    let mut reordered = Vec::<Vec<u8>>::new();
    let mut desired_left = desired_kinds.to_vec();
    desired_left.dedup();
    for kind in desired_left {
        if let Some(idx) = remaining
            .iter()
            .position(|row| row_kind_u16(row) == Some(kind))
        {
            reordered.push(remaining.remove(idx));
        }
    }
    reordered.extend(remaining);

    for (idx, row) in reordered.iter_mut().enumerate() {
        if row.len() >= 2 {
            let row_id = u16::try_from(idx + 1).unwrap_or(u16::MAX);
            row[0..2].copy_from_slice(&row_id.to_le_bytes());
        }
    }

    if reordered == original_rows {
        return Ok(false);
    }

    rewrite_pdb_t16_rows(&mut bytes, page_size, &data_pages, &reordered)?;
    std::fs::write(&pdb_path, bytes)?;
    Ok(true)
}

pub fn patch_pdb_columns_playlist_first(usb_root: &Path) -> BackendResult<bool> {
    patch_pdb_columns_menu_order_by_kind(usb_root, &[132])
}

/// Rewrite the PDB `columns` table (t16) to exactly match `desired`, which is
/// the ordered list of `(kind, display_name)` pairs that should appear on the
/// player. Adds rows for kinds not currently in PDB, drops rows for kinds not in
/// `desired`, and preserves existing row bytes byte-for-byte when both the
/// kind and display name match.
///
/// `display_name` is the clean menu label (e.g. `"GENRE"`) without the
/// U+FFFA..U+FFFB localization-marker wrapping; the wrapping is applied during
/// encoding. Row ids are renumbered 1..N in the new order.
///
/// Returns `Ok(true)` when PDB bytes changed, `Ok(false)` when the patch was a
/// no-op. Returns an error if the new t16 payload cannot fit in the existing
/// page capacity (we never grow the PDB here; large reflows go through the
/// export writer).
pub fn patch_pdb_columns_menu_set_by_kind(
    usb_root: &Path,
    desired: &[(u16, String)],
) -> BackendResult<bool> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let mut bytes = std::fs::read(&pdb_path)?;
    let page_size = 4096usize;
    if bytes.len() < page_size || bytes.len() % page_size != 0 {
        return Err(BackendError::Validation(
            "PDB columns set patch failed: invalid page alignment".to_string(),
        ));
    }

    let (data_pages, rows) = load_pdb_t16_rows(&bytes, page_size)?;
    if data_pages.is_empty() {
        return Ok(false);
    }
    let original_rows = rows.clone();

    let mut existing_by_kind: HashMap<u16, Vec<u8>> = HashMap::new();
    for row in &rows {
        if let Some(kind) = row_kind_u16(row) {
            existing_by_kind.entry(kind).or_insert_with(|| row.clone());
        }
    }

    let mut seen = HashSet::<u16>::new();
    let mut rebuilt = Vec::<Vec<u8>>::with_capacity(desired.len());
    for (idx, (kind, name)) in desired.iter().enumerate() {
        if !seen.insert(*kind) {
            continue;
        }
        let id = u16::try_from(idx + 1).unwrap_or(u16::MAX);
        let row = if let Some(existing) = existing_by_kind.get(kind) {
            // Reuse existing bytes when the name also matches; otherwise
            // re-encode from scratch with the supplied display name.
            let existing_name = decode_pdb_row_string(existing, 4).unwrap_or_default();
            if existing_name == *name {
                let mut bytes = existing.clone();
                if bytes.len() >= 2 {
                    bytes[0..2].copy_from_slice(&id.to_le_bytes());
                }
                bytes
            } else {
                encode_pdb_t16_row(id, *kind, name)
            }
        } else {
            encode_pdb_t16_row(id, *kind, name)
        };
        if row.is_empty() {
            return Err(BackendError::Validation(format!(
                "PDB columns set patch failed: could not encode kind {kind}"
            )));
        }
        rebuilt.push(row);
    }

    if rebuilt == original_rows {
        return Ok(false);
    }

    rewrite_pdb_t16_rows(&mut bytes, page_size, &data_pages, &rebuilt)?;
    std::fs::write(&pdb_path, bytes)?;
    Ok(true)
}

/// Encode one eDB.category row as an 8-byte PDB t17 entry.
///
/// Format: menuItemId(u16LE) | categoryId(u16LE) | 0x63(u8) | flags(u8) | seqNo(u16LE)
/// flags: 1 = hidden, 2 = MATCHING visible (kind=170), 0 = all other visible items.
pub fn encode_pdb_t17_cat_row(
    menu_item_id: u16,
    category_id: u16,
    is_visible: bool,
    kind: u32,
    seq_no: u16,
) -> [u8; 8] {
    let flags: u8 = if !is_visible {
        1
    } else if kind == 170 {
        2
    } else {
        0
    };
    let mut b = [0u8; 8];
    b[0..2].copy_from_slice(&menu_item_id.to_le_bytes());
    b[2..4].copy_from_slice(&category_id.to_le_bytes());
    b[4] = 0x63;
    b[5] = flags;
    b[6..8].copy_from_slice(&seq_no.to_le_bytes());
    b
}

/// Rewrite PDB t17 data pages with a complete snapshot of eDB.category state.
///
/// Returns true if the PDB file was rewritten (content changed). PDB t17 stores
/// one 8-byte entry per eDB.category row; readers take the current state from this
/// table when rendering the player browse menu category set. The existing page-chain
/// topology is preserved; only the row payloads are replaced.
pub fn patch_pdb_t17_category_snapshot(
    usb_root: &Path,
    encoded_rows: &[[u8; 8]],
) -> BackendResult<bool> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if !pdb_path.is_file() {
        return Ok(false);
    }
    let before = std::fs::read(&pdb_path)?;
    let page_size = 4096usize;
    if before.len() < page_size || before.len() % page_size != 0 {
        return Ok(false);
    }
    let Some((_ec, first, last)) = table_ptr_fields(&before, 17) else {
        return Ok(false);
    };
    let chain = collect_chain_pages(&before, page_size, first, last).ok_or_else(|| {
        BackendError::Validation("PDB t17 category patch failed: invalid chain".to_string())
    })?;
    if chain.len() <= 1 {
        return Ok(false);
    }
    let data_pages: Vec<u32> = chain.iter().skip(1).copied().collect();
    let row_vecs: Vec<Vec<u8>> = encoded_rows.iter().map(|r| r.to_vec()).collect();
    let mut bytes = before.clone();
    rewrite_pdb_t16_rows(&mut bytes, page_size, &data_pages, &row_vecs)?;
    if bytes == before {
        return Ok(false);
    }
    std::fs::write(&pdb_path, bytes)?;
    Ok(true)
}
