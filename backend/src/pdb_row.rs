//! PDB row layouts: where a row's strings sit, and the DeviceSQL strings
//! themselves.
//!
//! The reader, the writers, the detectors and their repairs all find a row's
//! strings through this, so they agree on where each one is and how long it
//! is. The encoders that build rows live in
//! `service::export_helpers::pdb_encoding`.

use crate::utils::{read_u8_at, read_u16_le_at, write_u16_le_at};

// ---------------------------------------------------------------------------
// Track rows (t00)
// ---------------------------------------------------------------------------

/// String slots in a track row.
pub const TRACK_STRING_SLOTS: usize = 21;
/// Row offset of a track row's string offset table: one u16 per slot, each
/// the row offset of that slot's string.
const TRACK_STRING_OFFSETS_AT: usize = 94;

/// What each track row string slot holds, by slot. Slots 14/15 and 17/18
/// are the start and end of the analysis path and the title.
pub const TRACK_STRING_SLOT_LABELS: [&str; TRACK_STRING_SLOTS] = [
    "isrc",
    "lyricist",
    "unknown_string_2",
    "unknown_string_3",
    "unknown_string_4",
    "message",
    "publish_track_info",
    "autoload_hotcues",
    "unknown_string_5",
    "unknown_string_6",
    "date_added",
    "release_date",
    "mix_name",
    "unknown_string_7",
    "analysis_path_start",
    "analysis_path_end",
    "comment",
    "title_start",
    "title_end",
    "filename",
    "track_file_path",
];

/// The row offset of a track row's string `slot`.
pub fn track_string_offset(row: &[u8], slot: usize) -> Option<usize> {
    if slot >= TRACK_STRING_SLOTS {
        return None;
    }
    read_u16_le_at(row, TRACK_STRING_OFFSETS_AT + 2 * slot).map(usize::from)
}

/// Every string slot's row offset in a track row, 0 for one the row is too
/// short to hold.
pub fn track_string_offsets(row: &[u8]) -> [u16; TRACK_STRING_SLOTS] {
    std::array::from_fn(|slot| track_string_offset(row, slot).map_or(0, |offset| offset as u16))
}

/// Set a track row's string `slot` to start at row offset `offset`. False
/// when the row is too short to hold the offset table.
pub fn set_track_string_offset(row: &mut [u8], slot: usize, offset: u16) -> bool {
    slot < TRACK_STRING_SLOTS && write_u16_le_at(row, TRACK_STRING_OFFSETS_AT + 2 * slot, offset)
}

/// The string slots of a track row whose long string
/// ([`long_string_misaligned`]) isn't 4-byte aligned.
pub fn misaligned_track_string_slots(row: &[u8]) -> impl Iterator<Item = usize> + '_ {
    (0..TRACK_STRING_SLOTS).filter(move |&slot| {
        track_string_offset(row, slot).is_some_and(|offset| long_string_misaligned(row, offset))
    })
}

// ---------------------------------------------------------------------------
// Artist (t02) and album (t03) rows
// ---------------------------------------------------------------------------

/// The artist row subtype whose name offset is a u16 at 10 (the "far"
/// variant), used when the name would sit past 255 bytes into the row.
const ARTIST_FAR_SUBTYPE: u16 = 0x64;

/// The row offset of an artist row's name: a u16 at 10 in the far variant
/// (subtype `0x64`), else the byte at 9.
pub fn artist_name_offset(row: &[u8]) -> Option<usize> {
    if read_u16_le_at(row, 0)? == ARTIST_FAR_SUBTYPE {
        read_u16_le_at(row, 10).map(usize::from)
    } else {
        read_u8_at(row, 9).map(usize::from)
    }
}

/// The row offset of an album row's name: the byte at 21, else 22. Read from
/// the row rather than assumed: the writer puts a non-ASCII name at 24 to
/// keep it 4-byte aligned (see `encode_album_row`).
pub fn album_name_offset(row: &[u8]) -> usize {
    read_u8_at(row, 21).map_or(22, usize::from)
}

// ---------------------------------------------------------------------------
// DeviceSQL strings
// ---------------------------------------------------------------------------
//
// A string starts with a kind byte (docs/PDB.md, "Track String Slots"). An
// odd one is short ASCII: the kind is `len * 2 + 3`, followed by `len` bytes.
// `0x40` (ASCII) and `0x90` (UTF-16LE) are long strings: a u16 total length
// (header included) and a pad byte follow, then the text. No writer seen
// (rekordbox, players, Engine DJ) uses any other kind, so another kind byte
// is damage, not a string.

/// Long ASCII kind byte.
const LONG_ASCII: u8 = 0x40;
/// Long UTF-16LE kind byte.
const LONG_UTF16: u8 = 0x90;

/// Whether the string at `offset` is a long one (`0x40` ASCII or `0x90`
/// UTF-16), which has a 4-byte header.
pub fn is_long_string(row: &[u8], offset: usize) -> bool {
    matches!(row.get(offset), Some(&(LONG_ASCII | LONG_UTF16)))
}

/// Whether the string at `offset` is a long one that isn't 4-byte aligned.
/// MIPS-based players read its header as one word, and an unaligned one
/// froze them or gave a communication error while browsing.
pub fn long_string_misaligned(row: &[u8], offset: usize) -> bool {
    is_long_string(row, offset) && !offset.is_multiple_of(4)
}

/// The encoded length in bytes (kind byte and header included) of the
/// string at `offset`, when it is above 0 and fits in the row.
pub fn pdb_string_len(row: &[u8], offset: usize) -> Option<usize> {
    let kind = *row.get(offset)?;
    let len = match kind {
        _ if kind & 1 == 1 => usize::from(kind >> 1),
        LONG_ASCII | LONG_UTF16 => usize::from(read_u16_le_at(row, offset + 1)?),
        _ => return None,
    };
    (len > 0 && offset.checked_add(len)? <= row.len()).then_some(len)
}

/// The string at `offset` without its trailing NULs.
pub fn get_string_from_pdb(row: &[u8], offset: usize) -> Option<String> {
    raw_pdb_string(row, offset).map(|(value, _)| value.trim_end_matches('\0').to_string())
}

/// The string at `offset` with every NUL kept (a crashed rekordbox write can
/// leave NULs at its end too), and its encoded length in bytes. `None` for
/// an empty one.
pub fn raw_pdb_string(row: &[u8], offset: usize) -> Option<(String, usize)> {
    let len = pdb_string_len(row, offset)?;
    let kind = row[offset];
    if kind & 1 == 1 {
        let text = row
            .get(offset + 1..offset + len)
            .filter(|t| !t.is_empty())?;
        return Some((String::from_utf8_lossy(text).into_owned(), len));
    }
    let text = row.get(offset + 4..offset + len)?;
    let value = if kind == LONG_UTF16 {
        let units: Vec<u16> = text
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        text.iter().map(|&b| b as char).collect::<String>()
    };
    Some((value, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ascii_length_counts_the_kind_byte() {
        // "OK": kind 2 * 2 + 3 = 7, then two bytes.
        let row = [7, b'O', b'K', 0xEE];
        assert_eq!(pdb_string_len(&row, 0), Some(3));
        assert_eq!(raw_pdb_string(&row, 0), Some(("OK".to_string(), 3)));
    }

    #[test]
    fn long_strings_and_their_alignment() {
        let mut row = vec![0u8; 4];
        row.extend_from_slice(&[0x90, 8, 0, 0, b'h', 0, b'i', 0]);
        assert_eq!(get_string_from_pdb(&row, 4).as_deref(), Some("hi"));
        assert_eq!(pdb_string_len(&row, 4), Some(8));
        assert!(!long_string_misaligned(&row, 4));
        let mut shifted = vec![0u8];
        shifted.extend_from_slice(&row);
        assert!(long_string_misaligned(&shifted, 5));
    }

    #[test]
    fn other_kinds_are_no_string() {
        // Kind 0x24, as a crashed rekordbox write left in a track's string slot.
        let row = [0x24, 6, 0, 0, b'h', b'i'];
        assert_eq!(pdb_string_len(&row, 0), None);
        assert_eq!(raw_pdb_string(&row, 0), None);
    }

    #[test]
    fn a_string_running_past_the_row_has_no_value() {
        let row = [0x90, 0xFF, 0, 0, b'h', 0];
        assert_eq!(pdb_string_len(&row, 0), None);
        assert_eq!(raw_pdb_string(&row, 0), None);
    }
}
