//! PDB writer.
//!
//! Two ways of producing a USB `export.pdb`:
//!
//! - [`write_pdb`] builds a complete file from an in-memory [`PdbData`] with no
//!   template dependency, following the Deep Symmetry specification.
//! - The additive path ([`try_write_pdb_additive_in_place`] and friends)
//!   edits an existing file in place — appending rows, patching or
//!   tombstoning existing ones — so pages rekordbox or a player already wrote
//!   (menu templates, history, transaction state) are kept as they are.
//!
//! Page footer byte layout lives in [`crate::pdb_page`]; this module decides
//! which per-table conventions to stamp into it.
//!
//! - `model`: `PdbData`, its row types and the standard tables.
//! - `fresh`: row encoding, page building and whole-file assembly.
//! - `inplace`: in-place row deletion and repair passes.
//! - `additive`: the additive in-place writer.
//! - `rows`: page/row primitives shared by the in-place paths.

#[cfg(test)]
use std::path::Path;

use crate::error::{BackendError, BackendResult};
use crate::pdb_page::{self, HEAP_START, footer_size};
use crate::service::export_helpers::pdb_encoding::apply_page_local_index_shift;
use crate::service::export_helpers::{
    PdbLayoutProfile, PdbTrackRowData, encode_album_row, encode_artist_row, encode_artwork_row,
    encode_key_row, encode_pdb_string, encode_playlist_entry_row, encode_playlist_tree_row,
    encode_track_row_with_profile, sanitize_metadata,
};
use crate::utils::{
    collect_chain as collect_chain_pages, packed_page_row_slot_count, page_offset, read_u8_at,
    read_u16_le_at, read_u32_le_at, set_table_ptr_fields, table_ptr_fields, write_u32_le_at,
};

mod additive;
mod fresh;
mod inplace;
mod model;
mod rows;

#[cfg(test)]
mod additive_tests;
#[cfg(test)]
pub(crate) mod writer_tests;

pub(crate) use additive::*;
pub use fresh::*;
pub(crate) use inplace::*;
pub use model::*;
pub use rows::*;

// ── Constants ────────────────────────────────────────────────────────────────

const PAGE_SIZE: usize = 4096;
const NUM_TABLES: u32 = 20;
// `nrs` (the page-local row-slot count at offset 0x18) is a single byte and
// wraps silently past 255. Our own reader tolerates that wraparound (see
// docs/PDB.md "Page Footer Conventions"), but real player parsers are not
// verified to -- the writer must never rely on it. A page must never hold
// more than 255 rows regardless of remaining byte capacity, even for tables
// with small rows (e.g. tt=8 playlist_entries, 12 bytes/row) where a large
// enough playlist could otherwise pack 300+ rows on a single 4096-byte page.
const MAX_ROWS_PER_PAGE: usize = 255;
const TABLE_POINTER_SIZE: usize = 16;
const TABLE_POINTERS_OFFSET: usize = 0x1c;

// Sentinel/index page magic values
const SENTINEL_UNKNOWNA: u16 = 0x1fff;
const SENTINEL_UNKNOWNB: u16 = 0x1fff;
const SENTINEL_MAGIC_03EC: u16 = 0x03ec;
const SENTINEL_MAGIC_03FFFFFF: u32 = 0x03ff_ffff;
const SENTINEL_FIRST_EMPTY: u16 = 0x1fff;
const SENTINEL_EMPTY_ENTRY: u32 = 0x1fff_fff8;
const SENTINEL_TAIL_ZERO_BYTES: usize = 20;

// Page flags
const PAGE_FLAGS_INDEX: u8 = 0x64;
const PAGE_FLAGS_DATA: u8 = 0x24;
const PAGE_FLAGS_DATA_TRACK: u8 = 0x34;
