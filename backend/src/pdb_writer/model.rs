//! In-memory PDB content (`PdbData` and its row types) and the standard
//! tables every export carries.

use super::*;

// ── Data Model ───────────────────────────────────────────────────────────────

/// Complete in-memory representation of a PDB file's content.
#[derive(Clone)]
pub struct PdbData {
    pub tracks: Vec<PdbTrackRowData>,
    pub artists: Vec<PdbArtistRow>,
    pub albums: Vec<PdbAlbumRow>,
    pub genres: Vec<PdbDictRow>,
    pub labels: Vec<PdbDictRow>,
    pub keys: Vec<PdbKeyRow>,
    pub colors: Vec<PdbColorRow>,
    pub artwork: Vec<PdbArtworkRow>,
    pub playlist_tree: Vec<PdbPlaylistTreeRow>,
    pub playlist_entries: Vec<PdbPlaylistEntryRow>,
    // Tables 9-12, 14-15: always empty
    // Table 16 (columns): raw pass-through
    pub columns_raw_rows: Vec<Vec<u8>>,
    // History tables
    pub history_playlists: Vec<PdbDictRow>,
    pub history_entries: Vec<PdbHistoryEntryRow>,
    pub history_raw_rows: Vec<Vec<u8>>,
    /// The eDB's `property` row, which the t19 rows mirror (see
    /// `t19_ring_rows`). `None` without an eDB: t19 keeps what it has.
    pub t19_property: Option<T19Property>,
    /// Layout profile for track row encoding
    pub profile: PdbLayoutProfile,
}

#[derive(Clone, Debug)]
pub struct PdbDictRow {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbArtistRow {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbAlbumRow {
    pub id: u32,
    pub name: String,
    pub artist_id: u32,
}

#[derive(Clone, Debug)]
pub struct PdbKeyRow {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbColorRow {
    pub id: u16,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbArtworkRow {
    pub id: u32,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct PdbPlaylistTreeRow {
    pub id: u32,
    pub parent_id: u32,
    pub sort_order: u32,
    pub is_folder: bool,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbPlaylistEntryRow {
    pub entry_index: u32,
    pub track_id: u32,
    pub playlist_id: u32,
}

#[derive(Clone, Debug)]
pub struct PdbHistoryEntryRow {
    pub track_id: u32,
    pub playlist_id: u32,
    pub entry_index: u32,
}

impl PdbData {
    /// Create an empty PDB with no data in any table.
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            artists: Vec::new(),
            albums: Vec::new(),
            genres: Vec::new(),
            labels: Vec::new(),
            keys: Vec::new(),
            colors: Vec::new(),
            artwork: Vec::new(),
            playlist_tree: Vec::new(),
            playlist_entries: Vec::new(),
            columns_raw_rows: Vec::new(),
            history_playlists: Vec::new(),
            history_entries: Vec::new(),
            history_raw_rows: Vec::new(),
            t19_property: None,
            profile: PdbLayoutProfile::DEFAULT,
        }
    }
}

// ── Standard Table Data ──────────────────────────────────────────────────────
//
// Colors and columns tables are required for player compatibility.
// Values extracted from working reference exports.

/// Standard player color definitions (8 rows: Pink, Red, Orange, Yellow, Green, Aqua, Blue, Purple).
pub fn standard_colors() -> Vec<PdbColorRow> {
    vec![
        PdbColorRow {
            id: 1,
            name: "Pink".to_string(),
        },
        PdbColorRow {
            id: 2,
            name: "Red".to_string(),
        },
        PdbColorRow {
            id: 3,
            name: "Orange".to_string(),
        },
        PdbColorRow {
            id: 4,
            name: "Yellow".to_string(),
        },
        PdbColorRow {
            id: 5,
            name: "Green".to_string(),
        },
        PdbColorRow {
            id: 6,
            name: "Aqua".to_string(),
        },
        PdbColorRow {
            id: 7,
            name: "Blue".to_string(),
        },
        PdbColorRow {
            id: 8,
            name: "Purple".to_string(),
        },
    ]
}

/// Full player browse catalog written to PDB t16 on fresh USBs and clean exports.
///
/// Matches the standard 27-row PDB t16 written on initialized USB exports.
/// Older players read all of these rows as available browse categories; eDB
/// `.category.isVisible` / `sequenceNo` then controls what newer players and
/// desktop library software shows. Keeping all 27 rows in PDB preserves the complete catalog
/// regardless of which subset the user has marked visible.
///
/// Order matches the eDB `menuItem_id` order seeded by `initialize_usb` so
/// that `pdb_missing_kinds` stays empty on a fresh USB.
///
/// Rows are encoded via `encode_pdb_t16_row` so they round-trip byte-for-byte
/// against reference t16 rows on the same kind/name (see
/// `encode_pdb_t16_row_roundtrips_*` tests).
pub fn standard_columns_raw() -> Vec<Vec<u8>> {
    use crate::service::export_helpers::encode_pdb_t16_row;
    DEFAULT_PLAYER_MENU_FULL
        .iter()
        .enumerate()
        .map(|(idx, (kind, name))| {
            let id = u16::try_from(idx + 1).expect("default menu fits in u16");
            encode_pdb_t16_row(id, *kind, name)
        })
        .collect()
}

/// All 27 player browse categories, in eDB menuItem_id order. Written to PDB
/// t16 as the full catalog; eDB.category controls visibility.
pub(crate) const DEFAULT_PLAYER_MENU_FULL: &[(u16, &str)] = &[
    (128, "GENRE"),
    (129, "ARTIST"),
    (130, "ALBUM"),
    (131, "TRACK"),
    (133, "BPM"),
    (134, "RATING"),
    (135, "YEAR"),
    (136, "REMIXER"),
    (137, "LABEL"),
    (138, "ORIGINAL ARTIST"),
    (139, "KEY"),
    (141, "CUE"),
    (142, "COLOR"),
    (146, "TIME"),
    (147, "BITRATE"),
    (148, "FILE NAME"),
    (132, "PLAYLIST"),
    (152, "HOT CUE BANK"),
    (149, "HISTORY"),
    (145, "SEARCH"),
    (150, "COMMENTS"),
    (140, "DATE ADDED"),
    (151, "DJ PLAY COUNT"),
    (144, "FOLDER"),
    (161, "DEFAULT"),
    (162, "ALPHABET"),
    (170, "MATCHING"),
];
