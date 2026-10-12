//! PDB data-page row-slot header fields and the backward-growing row index
//! footer.
//!
//! Footer layout, per group of up to 16 row slots, group 0 ending at the very
//! last byte of the page:
//!
//! ```text
//! [ofs_{n-1}] ... [ofs_1] [ofs_0] [rowpf] [tranrf]
//! ```
//!
//! Every group except the last one walked is full, so group `g` ends at
//! `page_len - g * 36`. Bit `j` of a group's `rowpf`/`tranrf` belongs to row
//! slot `16 * g + j`, and `ofs_j` is that slot's heap offset relative to the
//! 40-byte page header. See docs/PDB.md "Row Footer" and "Page Footer
//! Conventions" for what each table expects in `rowpf`/`tranrf`.
//!
//! This is the only place that knows the byte layout; callers that decide
//! *which* bits to set (per-table conventions, tombstoning) live in
//! `pdb_writer` and `service::repair`.

use crate::utils::{read_u16_le_at, write_u16_le_at};

/// Start of the row heap: 32-byte common header + 8-byte data-page header.
pub const HEAP_START: usize = 0x28;
/// Row slots per footer group.
pub const GROUP_SLOTS: usize = 16;
/// Footer bytes taken by one full group: `rowpf` + `tranrf` + 16 offsets.
const GROUP_BYTES: usize = 4 + GROUP_SLOTS * 2;

// ---------------------------------------------------------------------------
// Header row-slot counts
// ---------------------------------------------------------------------------

/// `(num_row_offsets, num_rows)` from the packed 24-bit header field at
/// `0x18..0x1b` (bits 0-12 and 13-23). `num_row_offsets` is the page's row
/// slot count including tombstoned slots; `num_rows` counts live rows only.
pub fn read_slot_counts(page: &[u8]) -> Option<(usize, usize)> {
    let packed = u32::from(*page.get(0x18)?)
        | (u32::from(*page.get(0x19)?) << 8)
        | (u32::from(*page.get(0x1a)?) << 16);
    Some((
        (packed & 0x1fff) as usize,
        ((packed >> 13) & 0x7ff) as usize,
    ))
}

/// Write the packed `(num_row_offsets, num_rows)` header field at `0x18..0x1b`.
/// Values are masked to their 13/11-bit widths.
pub fn write_slot_counts(page: &mut [u8], num_row_offsets: usize, num_rows: usize) {
    let packed = (num_row_offsets as u32 & 0x1fff) | ((num_rows as u32 & 0x7ff) << 13);
    page[0x18] = (packed & 0xff) as u8;
    page[0x19] = ((packed >> 8) & 0xff) as u8;
    page[0x1a] = ((packed >> 16) & 0xff) as u8;
}

// ---------------------------------------------------------------------------
// Footer geometry
// ---------------------------------------------------------------------------

/// Bytes of footer needed for `slot_count` row slots.
pub fn footer_size(slot_count: usize) -> usize {
    slot_count.div_ceil(GROUP_SLOTS) * 4 + slot_count * 2
}

/// Whether the whole footer for `slot_count` slots fits in a `page_len` page.
pub fn footer_fits(page_len: usize, slot_count: usize) -> bool {
    footer_size(slot_count) <= page_len
}

/// One footer group's position within its page. Offsets are relative to the
/// page start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FooterGroup {
    /// Group number; group 0 sits at the page's end.
    pub index: usize,
    /// Row slot held by bit 0 of this group.
    pub first_slot: usize,
    /// Slots in this group (16, except possibly the last group).
    pub len: usize,
    /// Byte offset of this group's `rowpf` word; `tranrf` follows it.
    pub rowpf_off: usize,
}

impl FooterGroup {
    pub fn tranrf_off(&self) -> usize {
        self.rowpf_off + 2
    }

    /// Byte offset of the heap offset for the group's `j`-th slot.
    pub fn offset_pos(&self, j: usize) -> usize {
        self.rowpf_off - 2 * (j + 1)
    }

    /// Bitmask covering every slot in this group.
    pub fn mask(&self) -> u16 {
        ((1u32 << self.len) - 1) as u16
    }

    pub fn contains_slot(&self, slot: usize) -> bool {
        (self.first_slot..self.first_slot + self.len).contains(&slot)
    }

    pub fn rowpf(&self, page: &[u8]) -> u16 {
        read_u16_le_at(page, self.rowpf_off).unwrap_or(0)
    }

    pub fn tranrf(&self, page: &[u8]) -> u16 {
        read_u16_le_at(page, self.tranrf_off()).unwrap_or(0)
    }

    pub fn set_rowpf(&self, page: &mut [u8], bits: u16) {
        write_u16_le_at(page, self.rowpf_off, bits);
    }

    pub fn set_tranrf(&self, page: &mut [u8], bits: u16) {
        write_u16_le_at(page, self.tranrf_off(), bits);
    }

    /// Heap offset of the group's `j`-th slot.
    pub fn row_offset(&self, page: &[u8], j: usize) -> usize {
        read_u16_le_at(page, self.offset_pos(j)).unwrap_or(0) as usize
    }
}

/// Footer groups for `slot_count` row slots on a `page_len`-byte page, in
/// slot order. Stops before the first group whose footer bytes would not fit
/// inside the page, so a corrupt slot count never yields out-of-page offsets;
/// use [`footer_fits`] when a truncated footer must be treated as invalid.
pub fn footer_groups(page_len: usize, slot_count: usize) -> impl Iterator<Item = FooterGroup> {
    (0..slot_count.div_ceil(GROUP_SLOTS))
        .map(move |index| {
            let first_slot = index * GROUP_SLOTS;
            let len = (slot_count - first_slot).min(GROUP_SLOTS);
            let end = page_len.checked_sub(index * GROUP_BYTES)?;
            if end < 4 + len * 2 {
                return None;
            }
            Some(FooterGroup {
                index,
                first_slot,
                len,
                rowpf_off: end - 4,
            })
        })
        .map_while(|group| group)
}

/// One row slot as recorded in the footer.
#[derive(Clone, Copy, Debug)]
pub struct FooterSlot {
    /// Page-local row slot number.
    pub index: usize,
    pub group: FooterGroup,
    /// This slot's bit within its group's `rowpf`/`tranrf`.
    pub bit: u16,
    /// Heap offset of the row, relative to the end of the 40-byte header.
    pub heap_off: usize,
    /// Whether the slot's `rowpf` bit is set (row is live).
    pub present: bool,
}

/// Every row slot of the groups yielded by [`footer_groups`] for a page whose
/// length is `page.len()`.
pub fn footer_slots(page: &[u8], slot_count: usize) -> impl Iterator<Item = FooterSlot> + '_ {
    footer_groups(page.len(), slot_count).flat_map(move |group| {
        let rowpf = group.rowpf(page);
        (0..group.len).map(move |j| {
            let bit = 1u16 << j;
            FooterSlot {
                index: group.first_slot + j,
                group,
                bit,
                heap_off: group.row_offset(page, j),
                present: rowpf & bit != 0,
            }
        })
    })
}

// ---------------------------------------------------------------------------
// Footer writes
// ---------------------------------------------------------------------------

/// Write a complete footer for `row_offsets` at the end of `page`, taking each
/// group's `(rowpf, tranrf)` from `bits`.
pub fn write_footer(
    page: &mut [u8],
    row_offsets: &[u16],
    bits: impl Fn(&FooterGroup) -> (u16, u16),
) {
    for group in footer_groups(page.len(), row_offsets.len()) {
        let (rowpf, tranrf) = bits(&group);
        group.set_rowpf(page, rowpf);
        group.set_tranrf(page, tranrf);
        for j in 0..group.len {
            write_u16_le_at(page, group.offset_pos(j), row_offsets[group.first_slot + j]);
        }
    }
}

/// Write a footer with every slot live: `rowpf == tranrf == all bits` in each
/// group. This is the `(trc, 0)`-convention shape; other tables restamp
/// `tranrf` afterwards.
pub fn write_footer_all_present(page: &mut [u8], row_offsets: &[u16]) {
    write_footer(page, row_offsets, |group| (group.mask(), group.mask()));
}

/// Set every group's `tranrf` so that only `slot`'s bit is set, in the group
/// holding it; all other groups get 0. This is the `u5 == 1` convention, with
/// `slot` being the page's last row slot (`num_rl`).
pub fn write_single_slot_tranrf(page: &mut [u8], slot_count: usize, slot: usize) {
    for group in footer_groups(page.len(), slot_count) {
        let tranrf = if group.contains_slot(slot) {
            1u16 << (slot - group.first_slot)
        } else {
            0
        };
        group.set_tranrf(page, tranrf);
    }
}

// ---------------------------------------------------------------------------
// Tolerant footer scan
// ---------------------------------------------------------------------------

/// Row-slot offsets and per-group `rowpf` words, as read by
/// [`scan_footer_tolerant`].
#[derive(Debug, Default)]
pub struct ScannedFooter {
    pub offsets: Vec<usize>,
    pub rowpf: Vec<u16>,
}

impl ScannedFooter {
    pub fn is_present(&self, slot: usize) -> bool {
        self.rowpf
            .get(slot / GROUP_SLOTS)
            .is_some_and(|bits| (bits >> (slot % GROUP_SLOTS)) & 1 == 1)
    }

    /// `(start, end)` of `slot`'s row within the payload: up to the next
    /// slot's offset, or `payload_len` for the last slot. Not clamped.
    pub fn row_bounds(&self, slot: usize, payload_len: usize) -> (usize, usize) {
        let start = self.offsets[slot];
        let end = self.offsets.get(slot + 1).copied().unwrap_or(payload_len);
        (start, end)
    }
}

/// Read the footer of a page whose payload ends at `payload_end` (absolute
/// within the page) for parsing an arbitrary, possibly foreign-written page.
///
/// Phase 1 trusts the header's packed slot count, reading as far as the page
/// allows. Phase 2: when the index space between `payload_end` and the page
/// end has room for up to 512 more slots than the header claims (one `nrs`
/// wrap), the scan continues with strict validation — footer bytes must not
/// overlap the payload, offsets must be within the payload and
/// non-decreasing — and stops at the first slot that fails.
pub fn scan_footer_tolerant(page: &[u8], len_page: usize, payload_end: usize) -> ScannedFooter {
    let payload_len = payload_end.saturating_sub(HEAP_START);
    let n_header = crate::utils::packed_page_row_slot_count(page).unwrap_or(0);
    let index_space = len_page.saturating_sub(payload_end);
    let full_groups = index_space / GROUP_BYTES;
    let leftover = index_space % GROUP_BYTES;
    let partial_rows = if leftover >= 6 { (leftover - 4) / 2 } else { 0 };
    let n_max = full_groups * GROUP_SLOTS + partial_rows;

    let mut out = ScannedFooter::default();
    if n_header == 0 && n_max == 0 {
        return out;
    }

    let mut m = len_page;
    // Phase 1: trust the page header count.
    for i in 0..n_header {
        if i % GROUP_SLOTS == 0 {
            if m < 4 {
                break;
            }
            m -= 4;
            let Some(bits) = read_u16_le_at(page, m) else {
                break;
            };
            out.rowpf.push(bits);
        }
        if m < 2 {
            break;
        }
        m -= 2;
        let Some(off) = read_u16_le_at(page, m) else {
            break;
        };
        out.offsets.push(off as usize);
    }

    // Phase 2: extend past a header count that looks wrapped.
    let gap = n_max.saturating_sub(n_header);
    if gap > 0 && gap <= 512 && out.offsets.len() == n_header {
        let mut prev_off = out.offsets.last().copied();
        for i in n_header..n_max {
            if i % GROUP_SLOTS == 0 {
                if m < 4 || m.saturating_sub(4) < payload_end {
                    break;
                }
                m -= 4;
                let Some(bits) = read_u16_le_at(page, m) else {
                    break;
                };
                out.rowpf.push(bits);
            }
            if m < 2 || m.saturating_sub(2) < payload_end {
                break;
            }
            m -= 2;
            let Some(off) = read_u16_le_at(page, m) else {
                break;
            };
            let off = off as usize;
            if off > payload_len {
                break;
            }
            if let Some(prev) = prev_off
                && off < prev
            {
                break;
            }
            prev_off = Some(off);
            out.offsets.push(off);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_counts_round_trip() {
        let mut page = vec![0u8; 64];
        write_slot_counts(&mut page, 300, 299);
        assert_eq!(read_slot_counts(&page), Some((300, 299)));
        assert_eq!(page[0x18], (300 & 0xff) as u8);
    }

    #[test]
    fn footer_size_matches_group_layout() {
        assert_eq!(footer_size(0), 0);
        assert_eq!(footer_size(1), 6);
        assert_eq!(footer_size(16), 36);
        assert_eq!(footer_size(17), 42);
    }

    #[test]
    fn groups_are_laid_out_backwards_from_page_end() {
        let groups: Vec<_> = footer_groups(4096, 20).collect();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rowpf_off, 4092);
        assert_eq!(groups[0].offset_pos(0), 4090);
        assert_eq!(groups[0].offset_pos(15), 4060);
        assert_eq!(groups[1].rowpf_off, 4092 - 36);
        assert_eq!(groups[1].len, 4);
        assert_eq!(groups[1].mask(), 0b1111);
        assert_eq!(groups[0].mask(), u16::MAX);
    }

    #[test]
    fn groups_stop_before_overflowing_the_page() {
        // 64 bytes holds one full group (36) but not a second full one.
        assert_eq!(footer_groups(64, 32).count(), 1);
        assert!(!footer_fits(64, 32));
        assert!(footer_fits(64, 16));
    }

    #[test]
    fn write_then_read_footer_round_trips() {
        let mut page = vec![0u8; 4096];
        let offsets: Vec<u16> = (0..20).map(|i| i * 8).collect();
        write_footer_all_present(&mut page, &offsets);
        let slots: Vec<_> = footer_slots(&page, 20).collect();
        assert_eq!(slots.len(), 20);
        assert!(slots.iter().all(|s| s.present));
        assert_eq!(
            slots.iter().map(|s| s.heap_off as u16).collect::<Vec<_>>(),
            offsets
        );

        write_single_slot_tranrf(&mut page, 20, 19);
        let groups: Vec<_> = footer_groups(4096, 20).collect();
        assert_eq!(groups[0].tranrf(&page), 0);
        assert_eq!(groups[1].tranrf(&page), 1 << 3);
    }
}
