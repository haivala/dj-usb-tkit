//! Shared binary/PDB byte-level helpers.
//!
//! These were previously duplicated across diagnostics, pdb_patch, export_helpers,
//! usb_utils, and several dev binaries. Canonical implementations live here.

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Byte reads
// ---------------------------------------------------------------------------

pub fn read_u8_at(bytes: &[u8], offset: usize) -> Option<u8> {
    bytes.get(offset).copied()
}

pub fn read_u16_le_at(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset + 2)
        .and_then(|s| <[u8; 2]>::try_from(s).ok())
        .map(u16::from_le_bytes)
}

pub fn read_u32_le_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map(u32::from_le_bytes)
}

/// Row-slot count packed into a PDB data page header at `0x18..0x1b`: bits
/// 0-12 are `num_row_offsets`, bits 13-23 are `num_rows`. Reading this as one
/// 24-bit field — instead of the single byte at `0x18` (`nrs`), which wraps
/// past 255 rows — is correct up to 8191 rows/page and needs no fallback.
/// `t08` playlist_entries rows are 12 bytes, so byte capacity alone lets a
/// single 4096-byte page hold well over 255 of them; see docs/PDB.md "Page
/// Footer Conventions".
pub fn packed_page_row_slot_count(page: &[u8]) -> Option<usize> {
    let packed = u32::from(*page.get(0x18)?)
        | (u32::from(*page.get(0x19)?) << 8)
        | (u32::from(*page.get(0x1a)?) << 16);
    usize::try_from(packed & 0x1fff).ok()
}

// ---------------------------------------------------------------------------
// Byte writes
// ---------------------------------------------------------------------------

pub fn write_u8_at(bytes: &mut [u8], offset: usize, value: u8) -> bool {
    if let Some(dst) = bytes.get_mut(offset) {
        *dst = value;
        true
    } else {
        false
    }
}

pub fn write_u16_le_at(bytes: &mut [u8], offset: usize, value: u16) -> bool {
    if let Some(dst) = bytes.get_mut(offset..offset + 2) {
        dst.copy_from_slice(&value.to_le_bytes());
        true
    } else {
        false
    }
}

pub fn write_u32_le_at(bytes: &mut [u8], offset: usize, value: u32) -> bool {
    if let Some(dst) = bytes.get_mut(offset..offset + 4) {
        dst.copy_from_slice(&value.to_le_bytes());
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// PDB page helpers
// ---------------------------------------------------------------------------

/// Byte offset of `page_index` inside a PDB blob with the given `page_size`.
pub fn page_offset(page_index: u32, page_size: usize) -> Option<usize> {
    (page_index as usize).checked_mul(page_size)
}

/// Read the PDB header table-pointer triple `(entry_count, first_page, last_page)`
/// for the given `table_type`.
pub fn table_ptr_fields(bytes: &[u8], table_type: u32) -> Option<(u32, u32, u32)> {
    let off = 0x1cusize + table_type as usize * 16;
    let ec = read_u32_le_at(bytes, off + 4)?;
    let first = read_u32_le_at(bytes, off + 8)?;
    let last = read_u32_le_at(bytes, off + 12)?;
    Some((ec, first, last))
}

/// Write the PDB header table-pointer triple for the given `table_type`.
pub fn set_table_ptr_fields(
    bytes: &mut [u8],
    table_type: u32,
    ec: u32,
    first: u32,
    last: u32,
) -> bool {
    let off = 0x1cusize + table_type as usize * 16;
    write_u32_le_at(bytes, off + 4, ec)
        && write_u32_le_at(bytes, off + 8, first)
        && write_u32_le_at(bytes, off + 12, last)
}

/// Walk a PDB page chain from `first` to `last` (strict).
///
/// Returns `None` on cycle, out-of-bounds, or broken next-pointer.
pub fn collect_chain(bytes: &[u8], page_size: usize, first: u32, last: u32) -> Option<Vec<u32>> {
    let total_pages = bytes.len() / page_size;
    if first == 0 || first as usize >= total_pages || last as usize >= total_pages {
        return None;
    }
    let mut out = Vec::<u32>::new();
    let mut seen = HashSet::<u32>::new();
    let mut current = first;
    for _ in 0..=total_pages {
        if !seen.insert(current) {
            return None;
        }
        out.push(current);
        if current == last {
            return Some(out);
        }
        let off = page_offset(current, page_size)?;
        let next = read_u32_le_at(bytes, off + 0x0c)?;
        if next == 0 || next as usize >= total_pages {
            return None;
        }
        current = next;
    }
    None
}

/// Walk a PDB page chain from `first` to `last` (lenient).
///
/// Returns whatever pages were collected before the first error or cycle,
/// never fails.
pub fn collect_chain_lenient(bytes: &[u8], page_size: usize, first: u32, last: u32) -> Vec<u32> {
    let max_page = match (bytes.len() / page_size).checked_sub(1) {
        Some(m) => m as u32,
        None => return Vec::new(),
    };
    if first == 0 || first > max_page || last > max_page {
        return Vec::new();
    }
    let mut out = Vec::<u32>::new();
    let mut seen = HashSet::<u32>::new();
    let mut cur = first;
    for _ in 0..=max_page {
        if !seen.insert(cur) {
            break;
        }
        out.push(cur);
        if cur == last {
            break;
        }
        let Some(off) = page_offset(cur, page_size) else {
            break;
        };
        let Some(next) = read_u32_le_at(bytes, off + 0x0c) else {
            break;
        };
        if next == 0 || next > max_page {
            break;
        }
        cur = next;
    }
    out
}

// ---------------------------------------------------------------------------
// Counted text
// ---------------------------------------------------------------------------

/// `count` followed by the singular or plural phrase ("1 track" / "3 tracks"),
/// English's plural rule. Only for English-only text: log-file lines
/// (`logging::debug`), a repair's file-format detail, `detected_issues`,
/// `Internal` errors. A text a user reads is a catalog key, whose plural forms
/// a translation picks.
pub fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The status line's warning note (" | (2 warnings)"), empty without warnings.
pub fn warning_suffix(warning_count: usize) -> String {
    if warning_count == 0 {
        String::new()
    } else {
        format!(" | ({})", crate::i18n::tr::usb_warning_count(warning_count))
    }
}

const KIB: f64 = 1024.0;
const MIB: f64 = KIB * KIB;
const GIB: f64 = MIB * KIB;

/// A size as the track tables and headings show it: whole KB and MB, GB with one decimal
/// ("2 KB", "412 MB", "1.2 GB"). Binary units, like file managers and `df` show a USB
/// stick's size.
pub fn byte_size(bytes: u64) -> crate::i18n::Text {
    use crate::i18n::{Num, tr};
    let n = bytes as f64;
    if n >= GIB {
        tr::format_size_gb(Num::fixed(n / GIB, 1))
    } else if n >= MIB {
        tr::format_size_mb(Num::fixed((n / MIB).round(), 0))
    } else {
        tr::format_size_kb(Num::fixed((n / KIB).round(), 0))
    }
}

/// A size with one decimal from KB up, and bytes below that ("512 B", "2.5 KB", "3.0 GB"):
/// for sizes that are small or close together.
pub fn byte_size_precise(bytes: u64) -> crate::i18n::Text {
    use crate::i18n::{Num, tr};
    let n = bytes as f64;
    if n >= GIB {
        tr::format_size_gb(Num::fixed(n / GIB, 1))
    } else if n >= MIB {
        tr::format_size_mb(Num::fixed(n / MIB, 1))
    } else if n >= KIB {
        tr::format_size_kb(Num::fixed(n / KIB, 1))
    } else {
        tr::format_size_b(bytes)
    }
}

/// "30 tracks · 412 MB" above a USB track table. The size is left out when no
/// track in the list has a known size.
pub fn track_list_summary(track_count: usize, size_bytes: u64) -> String {
    use crate::i18n::tr;
    if size_bytes > 0 {
        tr::track_list_summary_size(track_count, byte_size(size_bytes))
    } else {
        tr::track_list_summary(track_count)
    }
    .render()
}

/// A length as `m:ss`, or `h:mm:ss` from an hour ("3:45", "1:02:03"), the same in every
/// language. Rounded down to the second.
pub fn duration_text(ms: u64) -> String {
    let seconds = ms / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A track list's footer: "Total time: 1:02:03", with "(2 without length)" when some
/// tracks have none.
pub fn track_list_total_time(total_ms: u64, unknown_count: usize) -> String {
    use crate::i18n::tr;
    let duration = duration_text(total_ms);
    if unknown_count == 0 {
        tr::track_list_total_time(duration)
    } else {
        tr::track_list_total_time_unknown(duration, unknown_count)
    }
    .render()
}

/// A stored RFC 3339 time in the computer's time zone, as "2026-04-07 13:00": the same in
/// every language, like the Backups page's times. Text that isn't such a time comes back
/// as it is.
pub fn local_date_time(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339.trim())
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.to_string())
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

/// Stop a spawned console-subsystem process (bundled `node`, used to run the
/// essentia analysis runner) from flashing a console window when launched
/// from the GUI app on Windows.
pub fn suppress_console_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod list_text_tests {
    use super::{
        byte_size, byte_size_precise, duration_text, local_date_time, track_list_summary,
        track_list_total_time, warning_suffix,
    };

    #[test]
    fn warning_suffix_is_empty_without_warnings_and_counts_otherwise() {
        assert_eq!(warning_suffix(0), "");
        assert_eq!(warning_suffix(1), " | (1 warning)");
        assert_eq!(warning_suffix(2), " | (2 warnings)");
    }

    #[test]
    fn byte_size_uses_kb_mb_and_gb_like_the_ui() {
        let size = |bytes| byte_size(bytes).render();
        assert_eq!(size(0), "0 KB");
        assert_eq!(size(1536), "2 KB");
        assert_eq!(size(2560), "3 KB");
        assert_eq!(size(412 * 1024 * 1024), "412 MB");
        assert_eq!(size(1288490189), "1.2 GB");
        assert_eq!(size(27 * 1024 * 1024 * 1024), "27.0 GB");
    }

    #[test]
    fn byte_size_precise_has_a_decimal_from_kb_up() {
        let size = |bytes| byte_size_precise(bytes).render();
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1024), "1.0 KB");
        assert_eq!(size(2560), "2.5 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn durations_are_minutes_and_seconds_with_hours_from_an_hour() {
        assert_eq!(duration_text(0), "0:00");
        assert_eq!(duration_text(65_999), "1:05");
        assert_eq!(duration_text(3_661_000), "1:01:01");
        assert_eq!(track_list_total_time(180_000, 0), "Total time: 3:00");
        assert_eq!(
            track_list_total_time(180_000, 2),
            "Total time: 3:00 (2 without length)"
        );
    }

    #[test]
    fn local_date_time_shows_minutes_and_keeps_text_it_cant_read() {
        let shown = local_date_time("2026-04-07T10:00:00Z");
        assert_eq!(shown.len(), "2026-04-07 10:00".len(), "{shown}");
        assert!(shown.starts_with("2026-04-0"), "{shown}");
        assert_eq!(local_date_time("yesterday"), "yesterday");
    }

    #[test]
    fn track_list_summary_leaves_out_an_unknown_size() {
        assert_eq!(track_list_summary(1, 0), "1 track");
        assert_eq!(track_list_summary(30, 0), "30 tracks");
        assert_eq!(
            track_list_summary(30, 412 * 1024 * 1024),
            "30 tracks · 412 MB"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_u8_in_bounds() {
        assert_eq!(read_u8_at(&[0xAB, 0xCD], 0), Some(0xAB));
        assert_eq!(read_u8_at(&[0xAB, 0xCD], 1), Some(0xCD));
    }

    #[test]
    fn read_u8_out_of_bounds() {
        assert_eq!(read_u8_at(&[0xAB], 1), None);
        assert_eq!(read_u8_at(&[], 0), None);
    }

    #[test]
    fn read_u16_le() {
        // 0x0201 little-endian
        assert_eq!(read_u16_le_at(&[0x01, 0x02], 0), Some(0x0201));
        assert_eq!(read_u16_le_at(&[0xFF, 0x01, 0x02], 1), Some(0x0201));
    }

    #[test]
    fn read_u16_le_out_of_bounds() {
        assert_eq!(read_u16_le_at(&[0x01], 0), None);
        assert_eq!(read_u16_le_at(&[0x01, 0x02], 2), None);
    }

    #[test]
    fn read_u32_le() {
        assert_eq!(
            read_u32_le_at(&[0x01, 0x02, 0x03, 0x04], 0),
            Some(0x04030201)
        );
    }

    #[test]
    fn read_u32_le_out_of_bounds() {
        assert_eq!(read_u32_le_at(&[0x01, 0x02, 0x03], 0), None);
    }

    #[test]
    fn write_u8_in_bounds() {
        let mut buf = [0u8; 2];
        assert!(write_u8_at(&mut buf, 1, 0xAB));
        assert_eq!(buf, [0x00, 0xAB]);
    }

    #[test]
    fn write_u8_out_of_bounds() {
        let mut buf = [0u8; 1];
        assert!(!write_u8_at(&mut buf, 1, 0xAB));
    }

    #[test]
    fn write_u16_le() {
        let mut buf = [0u8; 4];
        assert!(write_u16_le_at(&mut buf, 1, 0x0201));
        assert_eq!(buf, [0x00, 0x01, 0x02, 0x00]);
    }

    #[test]
    fn write_u16_le_out_of_bounds() {
        let mut buf = [0u8; 1];
        assert!(!write_u16_le_at(&mut buf, 0, 0x0201));
    }

    #[test]
    fn write_u32_le() {
        let mut buf = [0u8; 4];
        assert!(write_u32_le_at(&mut buf, 0, 0x04030201));
        assert_eq!(buf, [0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn write_u32_le_out_of_bounds() {
        let mut buf = [0u8; 3];
        assert!(!write_u32_le_at(&mut buf, 0, 1));
    }

    #[test]
    fn page_offset_basic() {
        assert_eq!(page_offset(0, 4096), Some(0));
        assert_eq!(page_offset(1, 4096), Some(4096));
        assert_eq!(page_offset(2, 4096), Some(8192));
    }

    #[test]
    fn table_ptr_fields_roundtrip() {
        // Build a minimal PDB header: table_type 0 starts at offset 0x1c
        let mut buf = vec![0u8; 0x1c + 16];
        let ec: u32 = 5;
        let first: u32 = 1;
        let last: u32 = 3;
        assert!(set_table_ptr_fields(&mut buf, 0, ec, first, last));
        assert_eq!(table_ptr_fields(&buf, 0), Some((5, 1, 3)));
    }

    #[test]
    fn collect_chain_single_page() {
        let page_size = 32usize;
        // 3 pages: page 0 = header, page 1 = only data page (first==last)
        let buf = vec![0u8; page_size * 3];
        // page 1 next-pointer at offset 0x0c within the page — doesn't matter
        let result = collect_chain(&buf, page_size, 1, 1);
        assert_eq!(result, Some(vec![1]));
    }

    #[test]
    fn collect_chain_two_pages() {
        let page_size = 32usize;
        let mut buf = vec![0u8; page_size * 4];
        // page 1 -> page 2: write next-pointer at page1_offset + 0x0c
        let p1_off = page_size; // page 1
        buf[p1_off + 0x0c..p1_off + 0x10].copy_from_slice(&2u32.to_le_bytes());
        let result = collect_chain(&buf, page_size, 1, 2);
        assert_eq!(result, Some(vec![1, 2]));
    }

    #[test]
    fn collect_chain_rejects_cycle() {
        let page_size = 32usize;
        let mut buf = vec![0u8; page_size * 4];
        // page 1 -> page 2 -> page 1 (cycle), last=3 so never reached
        let p1 = page_size;
        buf[p1 + 0x0c..p1 + 0x10].copy_from_slice(&2u32.to_le_bytes());
        let p2 = page_size * 2;
        buf[p2 + 0x0c..p2 + 0x10].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(collect_chain(&buf, page_size, 1, 3), None);
    }

    #[test]
    fn collect_chain_rejects_page_zero_start() {
        let buf = vec![0u8; 4096 * 4];
        assert_eq!(collect_chain(&buf, 4096, 0, 2), None);
    }

    #[test]
    fn collect_chain_lenient_returns_partial() {
        let page_size = 32usize;
        let buf = vec![0u8; page_size * 4];
        // page 1 -> broken (next=0)
        let result = collect_chain_lenient(&buf, page_size, 1, 3);
        // should get page 1 at minimum
        assert_eq!(result, vec![1]);
    }
}
