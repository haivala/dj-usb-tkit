//! PDB pages: which pages hold rows, the rows on them, the row-slot header
//! fields and the backward-growing row index footer.
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

use crate::utils::{read_u16_le_at, read_u32_le_at, write_u16_le_at};

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
/// Read as one field, not as the single byte at `0x18` (`nrs`), which wraps
/// past 255 slots: 12-byte `t08` rows put well over 255 on a 4096-byte page.
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
    // More live rows than slots is what a crashed rekordbox write leaves
    // (live count 2040 on a 9-slot page); never write it.
    let num_rows = num_rows.min(num_row_offsets);
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

/// Whether the footer for `slot_count` slots sits clear of the page's row
/// heap (`HEAP_START` plus the header's used size). A page where it doesn't
/// has a slot count the rows can't back, and writing that footer would
/// overwrite rows, so the in-place passes leave such a page alone.
pub fn footer_clear_of_heap(page: &[u8], slot_count: usize) -> bool {
    let heap_end = HEAP_START + read_u16_le_at(page, 0x1e).unwrap_or(0) as usize;
    heap_end + footer_size(slot_count) <= page.len()
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
// Transaction fields
// ---------------------------------------------------------------------------
//
// `tranrf` marks the row slots the page's last write touched, deleted rows
// included. Every data page rekordbox writes, in every table, has
// `u5` = the number of those slots and `num_rl` = the lowest one, or the
// placeholder pair `(0x1FFF, 0x1FFF)`. rekordbox 6 crashes or loses its
// playlists on a page where they don't match. See docs/PDB.md "Page Footer
// Conventions".

/// The `u5`/`num_rl` placeholder pair rekordbox leaves on some data pages.
pub const PLACEHOLDER_FIELD: u16 = 0x1FFF;

/// `(u5, num_rl)` the transaction rule gives for per-group `tranrf` words
/// (one per group of `groups`), or `None` when no bit is set.
pub fn transaction_fields(groups: &[FooterGroup], tranrf: &[u16]) -> Option<(u16, u16)> {
    let mut count = 0u32;
    let mut lowest = None;
    for (group, &bits) in groups.iter().zip(tranrf) {
        let bits = bits & group.mask();
        if bits != 0 && lowest.is_none() {
            lowest = Some(group.first_slot + bits.trailing_zeros() as usize);
        }
        count += bits.count_ones();
    }
    lowest.map(|slot| (count as u16, slot as u16))
}

/// The page's `(u5, num_rl)` header fields.
pub fn read_transaction_header(page: &[u8]) -> (u16, u16) {
    (
        read_u16_le_at(page, 0x20).unwrap_or(0),
        read_u16_le_at(page, 0x22).unwrap_or(0),
    )
}

/// Write `(u5, num_rl)` to the page header.
pub fn write_transaction_header(page: &mut [u8], u5: u16, num_rl: u16) {
    write_u16_le_at(page, 0x20, u5);
    write_u16_le_at(page, 0x22, num_rl);
}

/// Whether a data page with `slot_count` row slots follows the transaction
/// rule. A page with no slots has nothing to describe.
pub fn transaction_consistent(page: &[u8], slot_count: usize) -> bool {
    if slot_count == 0 {
        return true;
    }
    let header = read_transaction_header(page);
    if header == (PLACEHOLDER_FIELD, PLACEHOLDER_FIELD) {
        return true;
    }
    let groups: Vec<_> = footer_groups(page.len(), slot_count).collect();
    let tranrf: Vec<u16> = groups.iter().map(|g| g.tranrf(page)).collect();
    transaction_fields(&groups, &tranrf) == Some(header)
}

/// Mark exactly `slots` as the page's last transaction: `tranrf` holds those
/// slots and nothing else, and `(u5, num_rl)` follow from it. `slots` must
/// not be empty and must be below `slot_count`.
pub fn write_transaction(page: &mut [u8], slot_count: usize, slots: &[usize]) {
    let groups: Vec<_> = footer_groups(page.len(), slot_count).collect();
    let mut tranrf = vec![0u16; groups.len()];
    for &slot in slots {
        if let Some(group) = groups.iter().find(|g| g.contains_slot(slot)) {
            tranrf[group.index] |= 1 << (slot - group.first_slot);
        }
    }
    for (group, &bits) in groups.iter().zip(&tranrf) {
        group.set_tranrf(page, bits);
    }
    if let Some((u5, num_rl)) = transaction_fields(&groups, &tranrf) {
        write_transaction_header(page, u5, num_rl);
    }
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------
//
// Which pages hold rows is decided here and nowhere else, so the reader, the
// writers, the detectors and their repairs all look at the same pages.

/// The page flag bit that marks an index page. Data pages have it clear
/// (`0x24`/`0x34` on every page rekordbox writes, `0x10` on one it crashed
/// writing); index pages have it set (`0x64`).
pub const INDEX_PAGE_FLAG: u8 = 0x40;
/// The page flags of a table's active data pages: the ones its index page's
/// B-tree lists. Sealed pages (`0x24`) are reached through `next_page`.
pub const ACTIVE_PAGE_FLAGS: u8 = 0x34;

/// The page size in a PDB file's header, when it isn't 0 and the file holds
/// the header page and at least one more.
pub fn file_page_size(bytes: &[u8]) -> Option<usize> {
    let size = usize::try_from(read_u32_le_at(bytes, 4)?).ok()?;
    (size > 0 && bytes.len() >= size.checked_mul(2)?).then_some(size)
}

/// A page's table type.
pub fn table_type(page: &[u8]) -> Option<u32> {
    read_u32_le_at(page, 8)
}

/// A page's flags byte.
pub fn page_flags(page: &[u8]) -> u8 {
    page.get(0x1b).copied().unwrap_or(0)
}

/// Whether a page was ever written: a blank page has page index 0.
fn written(page: &[u8]) -> bool {
    read_u32_le_at(page, 4).is_some_and(|index| index != 0)
}

/// Whether a page is a written index page ([`INDEX_PAGE_FLAG`] set).
pub fn is_index_page(page: &[u8]) -> bool {
    written(page) && page_flags(page) & INDEX_PAGE_FLAG != 0
}

/// Whether a page holds rows: written, not an index page, and with a row
/// heap (a used size above 0). Every pass that reads rows or decides which
/// pages a table owns goes by this.
pub fn is_data_page(page: &[u8]) -> bool {
    written(page)
        && page_flags(page) & INDEX_PAGE_FLAG == 0
        && read_u16_le_at(page, 0x1e).is_some_and(|used| used > 0)
}

/// Every page of a PDB file after its header page, with its page number.
/// None when the header's page size doesn't fit the file
/// ([`file_page_size`]).
pub fn pages(bytes: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    file_page_size(bytes).into_iter().flat_map(move |size| {
        bytes
            .chunks_exact(size)
            .enumerate()
            .skip(1)
            .map(|(index, page)| (index as u32, page))
    })
}

/// Whether the page at `index` carries that index in its header. Every page
/// written to a PDB does; a page that names another position is torn or
/// garbage, not a page of the file at this one.
fn self_indexed(index: u32, page: &[u8]) -> bool {
    read_u32_le_at(page, 4) == Some(index)
}

/// The pages of [`pages`] that hold rows ([`is_data_page`]) and carry their
/// own position as their page index.
pub fn data_pages(bytes: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    pages(bytes).filter(|&(index, page)| self_indexed(index, page) && is_data_page(page))
}

/// The pages of [`pages`] that are index pages ([`is_index_page`]) and carry
/// their own position as their page index.
pub fn index_pages(bytes: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    pages(bytes).filter(|&(index, page)| self_indexed(index, page) && is_index_page(page))
}

/// The active data pages of table `table_type` ([`ACTIVE_PAGE_FLAGS`]), in
/// page order: what its index page's B-tree lists.
pub fn active_data_pages(bytes: &[u8], table_type: u32) -> Vec<u32> {
    data_pages(bytes)
        .filter(|(_, page)| {
            self::table_type(page) == Some(table_type) && page_flags(page) == ACTIVE_PAGE_FLAGS
        })
        .map(|(index, _)| index)
        .collect()
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// One row slot of a page, as [`row_slots`] reads it. `start..end` is the
/// row's span in the heap, relative to [`HEAP_START`].
#[derive(Debug, Clone, Copy)]
pub struct RowSlot {
    pub start: usize,
    pub end: usize,
    /// Whether the slot's `rowpf` bit is set (the row is live).
    pub present: bool,
    /// The slot's footer entry: its number, group and bit.
    pub footer: FooterSlot,
}

/// The row slots of a page (its whole length): as many as the header's
/// `num_row_offsets` ([`read_slot_counts`]), each running from its heap
/// offset to the next slot's, the last to the end of the used heap. A slot
/// whose span is empty or runs past the used heap is left out.
///
/// Every reader and writer reads rows through this, so a damaged page reads
/// the same everywhere: nothing past the header's slot count and nothing
/// past the heap, however the footer space around it looks.
pub fn row_slots(page: &[u8]) -> Vec<RowSlot> {
    if page.len() < 64 {
        return Vec::new();
    }
    let used = read_u16_le_at(page, 0x1e).unwrap_or(0) as usize;
    let heap_len = used.min(page.len() - HEAP_START);
    if heap_len == 0 {
        return Vec::new();
    }
    let slot_count = read_slot_counts(page).map_or(0, |(slots, _)| slots);
    let footer: Vec<FooterSlot> = footer_slots(page, slot_count).collect();
    footer
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            let start = slot.heap_off;
            let end = footer.get(i + 1).map_or(heap_len, |next| next.heap_off);
            (start < end && end <= heap_len).then_some(RowSlot {
                start,
                end,
                present: slot.present,
                footer: *slot,
            })
        })
        .collect()
}

/// The live rows of a page ([`row_slots`] with their `rowpf` bit set), in
/// slot order.
pub fn present_rows(page: &[u8]) -> Vec<Vec<u8>> {
    row_slots(page)
        .into_iter()
        .filter(|slot| slot.present)
        .map(|slot| page[HEAP_START + slot.start..HEAP_START + slot.end].to_vec())
        .collect()
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

    #[test]
    fn transaction_rule_counts_tranrf_bits_and_takes_the_lowest() {
        // rekordbox: slots 3-6 of 7 in the last transaction.
        let mut page = vec![0u8; 4096];
        write_footer(&mut page, &[0; 7], |_| (0b011_0000, 0b111_1000));
        write_transaction_header(&mut page, 4, 3);
        assert!(transaction_consistent(&page, 7));
        // The old repair's forced (trc, 0) on the same page.
        write_transaction_header(&mut page, 7, 0);
        assert!(!transaction_consistent(&page, 7));
        // The placeholder pair is valid whatever tranrf holds.
        write_transaction_header(&mut page, PLACEHOLDER_FIELD, PLACEHOLDER_FIELD);
        assert!(transaction_consistent(&page, 7));
    }

    #[test]
    fn write_transaction_spans_groups() {
        let mut page = vec![0u8; 4096];
        write_footer(&mut page, &[0; 20], |group| (group.mask(), group.mask()));
        write_transaction(&mut page, 20, &[17, 19]);
        let groups: Vec<_> = footer_groups(4096, 20).collect();
        assert_eq!(groups[0].tranrf(&page), 0);
        assert_eq!(groups[1].tranrf(&page), 0b1010);
        assert_eq!(read_transaction_header(&page), (2, 17));
        assert!(transaction_consistent(&page, 20));
    }

    #[test]
    fn footer_clear_of_heap_needs_room_after_the_rows() {
        // Rows filling all but 36 bytes leave room for 16 slots' footer
        // (36 bytes), not 17 (42 bytes).
        let mut page = vec![0u8; 4096];
        page[0x1e..0x20].copy_from_slice(&(4096 - HEAP_START as u16 - 36).to_le_bytes());
        assert!(footer_clear_of_heap(&page, 16));
        assert!(!footer_clear_of_heap(&page, 17));
    }
}
