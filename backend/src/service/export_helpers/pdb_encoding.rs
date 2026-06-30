//! PDB string encoding, row builders, and layout profiles.

use super::PdbTrackRowData;
use crate::error::{BackendError, BackendResult};
use crate::metadata::sanitize_metadata;

pub fn encode_pdb_string(value: &str) -> Vec<u8> {
    if !value.is_ascii() {
        return encode_utf16le(value, false);
    }
    let bytes = value.as_bytes();
    if bytes.len() <= 126 {
        return encode_short_ascii(bytes, false);
    }
    encode_long_ascii(bytes)
}

pub fn encode_pdb_track_inline_string(value: &str) -> Vec<u8> {
    if !value.is_ascii() {
        return encode_utf16le(value, false);
    }
    let bytes = value.as_bytes();
    if bytes.len() <= 126 {
        return encode_short_ascii(bytes, false);
    }
    encode_long_ascii(bytes)
}

pub fn encode_pdb_track_range_string(value: &str) -> Vec<u8> {
    if !value.is_ascii() {
        return encode_utf16le(value, false);
    }
    let bytes = value.as_bytes();
    if bytes.len() <= 126 {
        return encode_short_ascii(bytes, false);
    }
    encode_long_ascii(bytes)
}

pub fn encode_pdb_track_path_string(value: &str) -> Vec<u8> {
    // Reference exports: no trailing null on path strings (total_len covers exact char count).
    if !value.is_ascii() {
        return encode_utf16le(value, false);
    }
    let bytes = value.as_bytes();
    if bytes.len() <= 126 {
        return encode_short_ascii(bytes, false);
    }
    encode_long_ascii(bytes)
}

pub fn encode_pdb_track_isrc_slot(value: Option<&str>) -> Vec<u8> {
    encode_pdb_track_inline_string(value.map(str::trim).filter(|v| !v.is_empty()).unwrap_or(""))
}

const PDB_TRACK_UNKNOWN_SLOT_2_DEFAULT: &str = "2";
const PDB_TRACK_UNKNOWN_SLOT_3_DEFAULT: &str = "2";
const PDB_TRACK_UNKNOWN_PRE_OFFSETS_86_89: [u8; 4] = [0x29, 0x00, 0x00, 0x00];
const PDB_TRACK_UNKNOWN_PRE_OFFSETS_92_93: [u8; 2] = [0x03, 0x00];
const PDB_TRACK_UNKNOWN_PRE_OFFSETS_86_89_ALT: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
const PDB_TRACK_UNKNOWN_PRE_OFFSETS_92_93_ALT: [u8; 2] = [0x00, 0x00];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdbLayoutProfile {
    Current,
    Rb6Compatible,
    Rb7Compatible,
}

impl PdbLayoutProfile {
    /// Default export profile — conservative compatibility layout.
    pub const DEFAULT: Self = Self::Rb6Compatible;

    pub fn from_env() -> Self {
        let raw = std::env::var("PDB_LAYOUT_PROFILE").unwrap_or_default();
        let norm = raw.trim().to_ascii_lowercase();
        match norm.as_str() {
            "" => Self::DEFAULT,
            "current" => Self::Current,
            "rb6" | "rb6_compatible" => Self::Rb6Compatible,
            "rb7" | "rb7_compatible" => Self::Rb7Compatible,
            _ => Self::DEFAULT,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Rb6Compatible => "rb6_compatible",
            Self::Rb7Compatible => "rb7_compatible",
        }
    }
}

pub fn encode_playlist_tree_row(
    id: u32,
    parent_id: u32,
    sort_order: u32,
    row_is_folder: bool,
    name: &str,
) -> Vec<u8> {
    let offset = 20usize;
    let name_bytes = encode_pdb_string(&sanitize_metadata(name));
    let mut row = vec![0u8; offset + name_bytes.len()];
    row[0..4].copy_from_slice(&parent_id.to_le_bytes());
    row[8..12].copy_from_slice(&sort_order.to_le_bytes());
    row[12..16].copy_from_slice(&id.to_le_bytes());
    row[16..20].copy_from_slice(&(if row_is_folder { 1u32 } else { 0u32 }).to_le_bytes());
    row[offset..offset + name_bytes.len()].copy_from_slice(&name_bytes);
    row
}

pub fn encode_playlist_entry_row(entry_index: u32, track_id: u32, playlist_id: u32) -> Vec<u8> {
    let mut row = vec![0u8; 12];
    row[0..4].copy_from_slice(&entry_index.to_le_bytes());
    row[4..8].copy_from_slice(&track_id.to_le_bytes());
    row[8..12].copy_from_slice(&playlist_id.to_le_bytes());
    row
}

pub fn encode_artwork_row(id: u32, path: &str) -> Vec<u8> {
    let path_bytes = encode_pdb_string(path);
    let mut row = vec![0u8; 4 + path_bytes.len()];
    row[0..4].copy_from_slice(&id.to_le_bytes());
    row[4..4 + path_bytes.len()].copy_from_slice(&path_bytes);
    row
}

pub fn encode_artist_row(id: u32, name: &str) -> Vec<u8> {
    let name_bytes = encode_pdb_string(&sanitize_metadata(name));
    // Near variant (subtype 0x0060):
    // subtype(2) + index_shift(2) + id(4) + const_3(1) + ofs_name_near(1) + padding(2) + name
    // Reference layout: ofs_name_near = 12 (0x0c), 2 zero bytes at 10-11, name at 12.
    // MIPS-based player hardware freezes when ofs_name_near = 10 and a UTF-16 name is at byte 10 —
    // the player likely hardcodes the name at offset 12 for the near-variant artist row.
    let name_start = 12usize;
    let mut row = vec![0u8; name_start + name_bytes.len()];
    row[0..2].copy_from_slice(&0x0060u16.to_le_bytes()); // subtype = near
    // bytes 2-3: page-local index_shift; assigned by the page writer.
    row[4..8].copy_from_slice(&id.to_le_bytes());
    row[8] = 3; // const 3
    row[9] = name_start as u8; // ofs_name_near = 12 = 0x0c
    // bytes 10-11: zero padding (part of fixed layout)
    row[name_start..name_start + name_bytes.len()].copy_from_slice(&name_bytes);
    row
}

pub(crate) fn apply_page_local_index_shift(table_type: u32, row: &mut [u8], row_slot: usize) {
    if row.len() < 4 || !matches!(table_type, 0 | 2 | 3) {
        return;
    }

    let subtype = u16::from_le_bytes([row[0], row[1]]);
    let has_index_shift = match table_type {
        0 => subtype == 0x0024,
        2 => matches!(subtype, 0x0060 | 0x0064),
        3 => matches!(subtype, 0x0080 | 0x0084),
        _ => false,
    };
    if !has_index_shift {
        return;
    }

    let shift = row_slot.saturating_mul(32).min(u16::MAX as usize) as u16;
    row[2..4].copy_from_slice(&shift.to_le_bytes());
}

pub fn encode_album_row(id: u32, name: &str, artist_id: u32) -> Vec<u8> {
    let name_bytes = encode_pdb_string(&sanitize_metadata(name));
    // Near variant (subtype 0x0080):
    // subtype(2) + index_shift(2) + unknown(4) + artist_id(4) + id(4) + unknown(5) + ofs_name_near(1) + name
    let name_start = 22usize; // 0x16
    let mut row = vec![0u8; name_start + name_bytes.len()];
    row[0..2].copy_from_slice(&0x0080u16.to_le_bytes()); // subtype = near
    // bytes 2-3: page-local index_shift; assigned by the page writer.
    // bytes 4-7: unknown = 0
    row[8..12].copy_from_slice(&artist_id.to_le_bytes());
    row[12..16].copy_from_slice(&id.to_le_bytes());
    // bytes 16-19: unknown = 0
    row[20] = 3; // const 3 at byte 0x14 (matches reference exports)
    row[21] = name_start as u8; // ofs_name_near at byte 0x15
    row[name_start..name_start + name_bytes.len()].copy_from_slice(&name_bytes);
    row
}

pub fn encode_key_row(id: u32, name: &str) -> Vec<u8> {
    let name_bytes = encode_pdb_string(&sanitize_metadata(name));
    // Key row: id(4) + id_duplicate(4) + name
    let name_offset = 8usize;
    let mut row = vec![0u8; name_offset + name_bytes.len()];
    row[0..4].copy_from_slice(&id.to_le_bytes());
    row[4..8].copy_from_slice(&id.to_le_bytes()); // duplicate id
    row[name_offset..name_offset + name_bytes.len()].copy_from_slice(&name_bytes);
    row
}

pub fn encode_track_row_with_profile(
    track: &PdbTrackRowData,
    profile: PdbLayoutProfile,
) -> BackendResult<Vec<u8>> {
    let string_start = 136usize;
    let empty_inline = encode_pdb_track_inline_string("");
    let mut slot_bytes = vec![empty_inline; 21];
    slot_bytes[0] = encode_pdb_track_isrc_slot(track.isrc.as_deref());
    // Keep the reference-documented but still-unmapped slots structurally present
    // in every row. Current reference samples do not give confident semantic
    // values for them yet, so export writes conservative placeholder/default bytes.
    let (slot_2_default, slot_3_default, pre_86_89, pre_92_93) = match profile {
        PdbLayoutProfile::Current => (
            PDB_TRACK_UNKNOWN_SLOT_2_DEFAULT,
            PDB_TRACK_UNKNOWN_SLOT_3_DEFAULT,
            PDB_TRACK_UNKNOWN_PRE_OFFSETS_86_89,
            PDB_TRACK_UNKNOWN_PRE_OFFSETS_92_93,
        ),
        PdbLayoutProfile::Rb6Compatible => (
            PDB_TRACK_UNKNOWN_SLOT_2_DEFAULT,
            PDB_TRACK_UNKNOWN_SLOT_3_DEFAULT,
            PDB_TRACK_UNKNOWN_PRE_OFFSETS_86_89,
            PDB_TRACK_UNKNOWN_PRE_OFFSETS_92_93,
        ),
        PdbLayoutProfile::Rb7Compatible => (
            "",
            "",
            PDB_TRACK_UNKNOWN_PRE_OFFSETS_86_89_ALT,
            PDB_TRACK_UNKNOWN_PRE_OFFSETS_92_93_ALT,
        ),
    };
    slot_bytes[2] = encode_pdb_track_inline_string(slot_2_default);
    slot_bytes[3] = encode_pdb_track_inline_string(slot_3_default);
    slot_bytes[6] = encode_pdb_track_inline_string(if track.publish_track_info_on == Some(true) {
        "ON"
    } else {
        ""
    });
    slot_bytes[7] = encode_pdb_track_inline_string(if track.autoload_hotcues_on == Some(true) {
        "ON"
    } else {
        ""
    });
    slot_bytes[10] = encode_pdb_track_inline_string(track.date_added.as_deref().unwrap_or(""));
    slot_bytes[11] = encode_pdb_track_inline_string(track.release_date.as_deref().unwrap_or(""));
    slot_bytes[14] = encode_pdb_track_range_string(&track.anlz_path);
    slot_bytes[15] = encode_pdb_track_inline_string(track.date_added.as_deref().unwrap_or(""));
    slot_bytes[16] = encode_pdb_track_inline_string(&sanitize_metadata(
        track.dj_comment.as_deref().unwrap_or(""),
    ));
    slot_bytes[17] = encode_pdb_track_range_string(&sanitize_metadata(&track.title));
    slot_bytes[19] = encode_pdb_track_inline_string(&sanitize_metadata(
        track.file_name.as_deref().unwrap_or(&track.file_path),
    ));
    slot_bytes[20] = encode_pdb_track_path_string(&track.file_path);

    let mut row = vec![0u8; string_start];
    let mut offsets = [0u16; 21];
    let mut cursor = string_start;
    for index in 0..21usize {
        // Slot 20 is a UTF-16 path string (0x90 header + u16 total_len = 4-byte read).
        // MIPS-based player hardware requires 4-byte aligned reads; misalignment causes Address
        // Error exception → freeze. Pad to the next 4-byte boundary before this slot.
        if index == 20 {
            let pad = (4 - cursor % 4) % 4;
            row.resize(row.len() + pad, 0u8);
            cursor += pad;
        }
        offsets[index] = cursor as u16;
        row.extend_from_slice(&slot_bytes[index]);
        cursor += slot_bytes[index].len();
    }

    if cursor > u16::MAX as usize {
        return Err(BackendError::Validation(format!(
            "track row too large for PDB encoding (id {})",
            track.id
        )));
    }

    row[0..4].copy_from_slice(
        &track
            .header_flags_u32
            .unwrap_or(0x0000_0024u32)
            .to_le_bytes(),
    );
    row[4..8].copy_from_slice(&track.content_link.unwrap_or(0).to_le_bytes());
    row[8..12].copy_from_slice(&track.sample_rate_hz.unwrap_or(0).to_le_bytes());
    row[16..20].copy_from_slice(&track.file_size_bytes.unwrap_or(0).to_le_bytes());
    row[20..24].copy_from_slice(&track.master_content_id.unwrap_or(0).to_le_bytes());
    row[24..28].copy_from_slice(&track.master_db_id.unwrap_or(0).to_le_bytes());
    row[28..32].copy_from_slice(&track.artwork_id.to_le_bytes());
    row[32..36].copy_from_slice(&track.key_id.to_le_bytes());
    row[48..52].copy_from_slice(&track.bitrate_kbps.unwrap_or(0).to_le_bytes());
    row[52..56].copy_from_slice(&track.track_number.unwrap_or(0).to_le_bytes());
    let tempo_x100 = track
        .bpm
        .map(|v| (v * 100.0).round().max(0.0) as u32)
        .unwrap_or(0);
    row[56..60].copy_from_slice(&tempo_x100.to_le_bytes());
    row[60..64].copy_from_slice(&track.genre_id.to_le_bytes());
    row[64..68].copy_from_slice(&track.album_id.to_le_bytes());
    row[68..72].copy_from_slice(&track.artist_id.to_le_bytes());
    row[72..76].copy_from_slice(&track.id.to_le_bytes());
    row[80..82].copy_from_slice(&track.release_year.unwrap_or(0).to_le_bytes());
    row[82..84].copy_from_slice(&track.bit_depth.unwrap_or(0).to_le_bytes());
    let duration_seconds = track.duration_seconds.unwrap_or(0).min(u16::MAX as u32) as u16;
    row[84..86].copy_from_slice(&duration_seconds.to_le_bytes());
    row[86..90].copy_from_slice(&pre_86_89);
    row[90..92].copy_from_slice(&track.file_type.unwrap_or(0).to_le_bytes());
    row[92..94].copy_from_slice(&pre_92_93);

    for (index, offset) in offsets.iter().enumerate() {
        row[94 + 2 * index..94 + 2 * index + 2].copy_from_slice(&offset.to_le_bytes());
    }
    Ok(row)
}

fn encode_utf16le(value: &str, trailing_nul: bool) -> Vec<u8> {
    let utf16: Vec<u16> = value.encode_utf16().collect();
    let data_bytes = utf16.len() * 2 + if trailing_nul { 2 } else { 0 };
    let total_len = 4 + data_bytes;
    let mut out = Vec::with_capacity(total_len);
    out.push(0x90u8);
    out.extend_from_slice(&(total_len as u16).to_le_bytes());
    out.push(0x00u8);
    for &cu in &utf16 {
        out.extend_from_slice(&cu.to_le_bytes());
    }
    if trailing_nul {
        out.extend_from_slice(&[0x00, 0x00]);
    }
    out
}

fn encode_short_ascii(bytes: &[u8], trailing_nul: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + if trailing_nul { 2 } else { 1 });
    out.push((bytes.len() as u8).saturating_mul(2).saturating_add(3));
    out.extend_from_slice(bytes);
    if trailing_nul {
        out.push(0);
    }
    out
}

fn encode_long_ascii(bytes: &[u8]) -> Vec<u8> {
    let total_len = 4 + bytes.len();
    let mut out = Vec::with_capacity(total_len);
    out.push(0x40);
    out.extend_from_slice(&(total_len as u16).to_le_bytes());
    out.push(0);
    out.extend_from_slice(bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_metadata_passthrough_when_clean() {
        let s = "Artist Name";
        let result = sanitize_metadata(s);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), s);
    }

    #[test]
    fn sanitize_metadata_strips_null_bytes() {
        let s = "Art\0ist";
        assert_eq!(sanitize_metadata(s).as_ref(), "Artist");
    }

    #[test]
    fn sanitize_metadata_truncates_to_255_chars() {
        let long: String = "a".repeat(300);
        let result = sanitize_metadata(&long);
        assert_eq!(result.chars().count(), 255);
    }

    #[test]
    fn sanitize_metadata_truncates_unicode_by_char_not_byte() {
        // 200 × 3-byte UTF-8 char → should be truncated at 255 chars, not 255 bytes
        let long: String = "ä".repeat(300);
        let result = sanitize_metadata(&long);
        assert_eq!(result.chars().count(), 255);
    }

    #[test]
    fn sanitize_metadata_null_and_long_combined() {
        let mut s = "x\0".repeat(200);
        s.push_str("tail");
        let result = sanitize_metadata(&s);
        // After stripping nulls: 204 'x' chars + "tail" = 204+4 = 208, fits in 255
        assert!(!result.contains('\0'));
        assert!(result.chars().count() <= 255);
    }

    #[test]
    fn encode_artist_row_sanitizes_null_in_name() {
        let row = encode_artist_row(1, "Bad\0Name");
        // The encoded string should not contain a null in the name payload
        let encoded_str: String = row[12..]
            .iter()
            .filter(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        assert!(encoded_str.contains("BadName"));
        assert!(!encoded_str.contains('\0'));
    }

    // Helper that builds a minimal track row for alignment tests.
    fn test_track(
        title: &str,
        release_date: Option<&str>,
        file_path: &str,
    ) -> super::super::PdbTrackRowData {
        super::super::PdbTrackRowData {
            header_flags_u32: None,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id: 1,
            artist_id: 1,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            bitrate_kbps: None,
            track_number: None,
            bpm: None,
            release_year: None,
            bit_depth: None,
            duration_seconds: None,
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: release_date.map(str::to_owned),
            dj_comment: None,
            file_name: None,
            publish_track_info_on: None,
            autoload_hotcues_on: None,
            title: title.to_owned(),
            anlz_path: "/PIONEER/USBANLZ/01/000001.DAT".to_owned(),
            file_path: file_path.to_owned(),
        }
    }

    fn slot_20_offset(row: &[u8]) -> usize {
        // Offset table starts at byte 94; each entry is 2 bytes LE; slot 20 is at index 20.
        u16::from_le_bytes([row[134], row[135]]) as usize
    }

    #[test]
    fn track_row_slot_20_is_4byte_aligned() {
        use super::PdbLayoutProfile;

        // Four combinations that vary the cursor position before slot 20:
        // ASCII vs non-ASCII title (slot 17) and with/without release_date (slot 11).
        let cases = [
            ("ASCII title", None, "/Contents/track.mp3"),
            ("ASCII title", Some("2024-01-01"), "/Contents/track.mp3"),
            ("Niño de Elche", None, "/Contents/track.flac"),
            ("Niño de Elche", Some("2024-01-01"), "/Contents/track.flac"),
            // Very long ASCII title to maximize cursor variance
            (&"a".repeat(100)[..], None, "/Contents/long.mp3"),
        ];

        for (title, release_date, path) in cases {
            let track = test_track(title, release_date, path);
            let row = encode_track_row_with_profile(&track, PdbLayoutProfile::Current)
                .expect("encode should succeed");
            let off20 = slot_20_offset(&row);
            assert_eq!(
                off20 % 4,
                0,
                "slot 20 must be at a 4-byte aligned offset (title={title:?}, release_date={release_date:?}): offset was {off20}"
            );
        }
    }

    #[test]
    fn track_row_slot_20_path_no_trailing_null() {
        // Reference exports: UTF-16 path strings do not have a trailing U+0000.
        // The header's total_len field = 4 (header size) + char_count * 2 (code units).
        // With a trailing null it would be 4 + char_count * 2 + 2 instead.
        let path = "/Contents/Ünité/track.flac";
        let encoded = encode_pdb_track_path_string(path);
        // Format: 0x90 [len_lo] [len_hi] 0x00 [utf16le bytes...]
        assert_eq!(encoded[0], 0x90, "UTF-16 string marker");
        let total_len = u16::from_le_bytes([encoded[1], encoded[2]]) as usize;
        let char_count = path.chars().count();
        assert_eq!(
            total_len,
            4 + char_count * 2,
            "total_len must be 4+char_count*2 (no trailing null): got {total_len}, expected {}",
            4 + char_count * 2
        );
        assert_eq!(
            encoded.len(),
            4 + char_count * 2,
            "encoded bytes must be exactly 4-byte header + char_count*2 body bytes"
        );
    }

    #[test]
    fn artist_row_name_offset_is_4byte_aligned() {
        // MIPS-based player hardware freezes when the UTF-16 name header (0x90 [len_lo] [len_hi] 0x00)
        // is at an unaligned address. Reference exports place ofs_name_near = 12 (% 4 == 0).
        let row = encode_artist_row(1, "Ärtti");
        let ofs_name_near = row[9] as usize;
        assert_eq!(ofs_name_near, 12, "ofs_name_near must be 12");
        assert_eq!(ofs_name_near % 4, 0, "ofs_name_near must be 4-byte aligned");
    }

    #[test]
    fn album_row_name_offset_matches_reference_export() {
        // Reference exports have ofs_name_near = 22 for the near-variant album row.
        // NOTE: 22 % 4 == 2 — not 4-byte aligned. Keep the reference shape while
        // still using the normal DeviceSQL UTF-16LE encoding for non-ASCII names.
        let row = encode_album_row(1, "Álbum", 1);
        let ofs_name_near = row[21] as usize;
        assert_eq!(
            ofs_name_near, 22,
            "album row ofs_name_near must be 22 (matches reference export)"
        );
        assert_eq!(
            row[ofs_name_near], 0x90,
            "non-ASCII album must use UTF-16LE"
        );
    }
}
