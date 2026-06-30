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

/// Convenience variant that returns only `(first_page, last_page)`.
pub fn table_ptr_first_last(bytes: &[u8], table_type: u32) -> Option<(u32, u32)> {
    let (_, first, last) = table_ptr_fields(bytes, table_type)?;
    Some((first, last))
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
// Tests
// ---------------------------------------------------------------------------

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
    fn table_ptr_first_last_drops_ec() {
        let mut buf = vec![0u8; 0x1c + 16];
        assert!(set_table_ptr_fields(&mut buf, 0, 99, 2, 7));
        assert_eq!(table_ptr_first_last(&buf, 0), Some((2, 7)));
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
