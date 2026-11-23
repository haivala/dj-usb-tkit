//! Playlist-tree (t07) rows a crashed rekordbox write damaged.
//!
//! The same crash that leaves NUL runs in track rows (`damaged_tracks`) hits
//! the playlist tree two ways:
//!
//! - *A half-written row:* rekordbox was adding a playlist and the crash left
//!   its row live with the id of another live playlist. Of two live rows with
//!   one id, the one the page's last write touched (its `tranrf` bit) is the
//!   half-written newer row and goes; the earlier row stays. When the USB has
//!   an eDB, it must have a playlist whose name the earlier row's name fits,
//!   or nothing is decided and the Event Log says so. The two rows share one
//!   list of entries, so the half-written playlist's own tracks can't be told
//!   apart: strict parity adds it back from the eDB when the eDB has it.
//! - *Damaged names:* NUL runs in a live name, each standing for up to as many
//!   characters. The name comes back from the first source that has exactly
//!   one name that fits: the eDB's playlists, the backup PDBs' (newest
//!   first), the tree's deleted rows (rekordbox leaves old copies of every row
//!   it rewrites), the app's playlists last exported to this USB. With none,
//!   the NULs are dropped and the Event Log says so.
//!
//! Playlists are matched by name only, as everywhere between the PDB and the
//! eDB: the two number their playlists on their own. When a damaged name
//! fits more than one name, the names intact live playlists have and those
//! already given to other damaged rows don't count; rows are resolved in
//! passes, so such a row gets the name left once another row has taken the
//! other. A name that alone fits is taken even when another playlist has it,
//! and a row all of whose fitting names are taken gets the first of them:
//! either way it is a copy, an empty duplicate the empty-duplicate repair
//! removes, or a second playlist with that name.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::pdb_writer::PlaylistTreeSlot;

use super::*;

/// A live row the repair renames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlaylistNameRepair {
    pub(super) page: usize,
    pub(super) slot: usize,
    pub(super) id: u32,
    pub(super) parent_id: u32,
    pub(super) sort_order: u32,
    pub(super) is_folder: bool,
    pub(super) old: String,
    pub(super) new: String,
    /// Whether a source gave the name back; false when the NULs were dropped.
    pub(super) restored: bool,
}

/// What the repair does to the playlist tree.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct PlaylistTreeRepairs {
    /// Half-written rows to remove: `(page, slot, name)`.
    pub(super) half_written: Vec<(usize, usize, String)>,
    pub(super) renames: Vec<PlaylistNameRepair>,
    /// Ids two or more live rows share that the repair can't decide, with
    /// their names.
    pub(super) undecided: Vec<(u32, Vec<String>)>,
}

impl PlaylistTreeRepairs {
    /// Whether there is anything to write.
    pub(super) fn is_empty(&self) -> bool {
        self.half_written.is_empty() && self.renames.is_empty()
    }

    /// Rows changed.
    pub(super) fn len(&self) -> usize {
        self.half_written.len() + self.renames.len()
    }
}

/// What the repair knows besides the PDB itself.
#[derive(Debug, Default, Clone)]
pub(super) struct PlaylistNameSources {
    /// The eDB's playlist names, when the USB has an eDB.
    pub(super) edb: Option<Vec<String>>,
    /// Live, undamaged names in each backup PDB, newest first.
    pub(super) backups: Vec<Vec<String>>,
    /// The names of the app's playlists last exported to this USB.
    pub(super) library: Vec<String>,
}

impl PlaylistNameSources {
    pub(super) fn read(
        usb_root: &Path,
        edb: Option<&rusqlite::Connection>,
        library: Vec<String>,
    ) -> Self {
        let edb = edb.map(|conn| {
            if !table_exists(conn, "playlist") {
                return Vec::new();
            }
            conn.prepare("SELECT name FROM playlist WHERE name IS NOT NULL")
                .and_then(|mut stmt| {
                    let names = stmt.query_map([], |row| row.get::<_, String>(0))?;
                    Ok(names.flatten().collect())
                })
                .unwrap_or_default()
        });
        Self {
            edb,
            backups: Vec::new(),
            library,
        }
        .with_backups(usb_root)
    }

    fn with_backups(mut self, usb_root: &Path) -> Self {
        self.backups = crate::service::usb_backups::backup_pdb_files(usb_root)
            .iter()
            .filter_map(|path| std::fs::read(path).ok())
            .map(|bytes| {
                crate::pdb_writer::playlist_tree_slots(&bytes)
                    .into_iter()
                    .filter(|s| s.live && !s.name.contains('\0'))
                    .map(|s| s.name)
                    .collect()
            })
            .collect();
        self
    }
}

impl BackendService {
    /// The names of the app's playlists last exported to `usb_root`.
    pub(super) fn playlist_names_exported_to(&self, usb_root: &Path) -> Vec<String> {
        let Ok(conn) = self.db.connect() else {
            return Vec::new();
        };
        conn.prepare("SELECT name FROM playlists WHERE last_exported_usb_root = ?1")
            .and_then(|mut stmt| {
                let names = stmt.query_map([usb_root.to_string_lossy()], |row| row.get(0))?;
                Ok(names.flatten().collect())
            })
            .unwrap_or_default()
    }
}

/// What the repair does to the PDB `bytes`, by the rules in the module docs.
pub(super) fn detect_damaged_playlist_tree(
    bytes: &[u8],
    sources: &PlaylistNameSources,
) -> PlaylistTreeRepairs {
    let slots = crate::pdb_writer::playlist_tree_slots(bytes);
    let mut repairs = PlaylistTreeRepairs::default();

    let mut by_id: BTreeMap<u32, Vec<&PlaylistTreeSlot>> = BTreeMap::new();
    for slot in slots.iter().filter(|s| s.live) {
        by_id.entry(slot.id).or_default().push(slot);
    }
    for (&id, rows) in by_id.iter().filter(|(_, rows)| rows.len() > 1) {
        let (newer, earlier): (Vec<&PlaylistTreeSlot>, Vec<&PlaylistTreeSlot>) =
            rows.iter().copied().partition(|s| s.in_last_write);
        let agreed = match (newer.as_slice(), earlier.as_slice()) {
            ([newer], [earlier]) => match &sources.edb {
                Some(names) => names
                    .iter()
                    .any(|name| name_fits(&earlier.name, name))
                    .then_some(newer),
                None => Some(newer),
            },
            _ => None,
        };
        match agreed {
            Some(newer) => {
                repairs
                    .half_written
                    .push((newer.page, newer.slot, newer.name.replace('\0', "")))
            }
            None => repairs.undecided.push((
                id,
                rows.iter()
                    .map(|s| s.name.replace('\0', "\u{fffd}"))
                    .collect(),
            )),
        }
    }

    let half_written = |s: &PlaylistTreeSlot| {
        repairs
            .half_written
            .iter()
            .any(|&(page, at, _)| (page, at) == (s.page, s.slot))
    };
    let live: Vec<&PlaylistTreeSlot> = slots
        .iter()
        .filter(|s| s.live && !half_written(s))
        .collect();
    let deleted: Vec<&str> = slots
        .iter()
        .filter(|s| !s.live && !s.name.is_empty() && !s.name.contains('\0'))
        .map(|s| s.name.as_str())
        .collect();
    let mut pools: Vec<Vec<&str>> = Vec::new();
    pools.extend(
        sources
            .edb
            .iter()
            .map(|names| names.iter().map(String::as_str).collect()),
    );
    pools.extend(
        sources
            .backups
            .iter()
            .map(|names| names.iter().map(String::as_str).collect()),
    );
    pools.push(deleted);
    pools.push(sources.library.iter().map(String::as_str).collect());

    // Names intact live playlists have, then the ones damaged rows take.
    let mut taken: HashSet<String> = live
        .iter()
        .filter(|s| !s.name.contains('\0'))
        .map(|s| canonicalize_playlist_name(&s.name))
        .collect();
    let damaged: Vec<&PlaylistTreeSlot> = live
        .iter()
        .copied()
        .filter(|s| s.name.contains('\0'))
        .collect();
    let mut restored: HashMap<(usize, usize), String> = HashMap::new();
    loop {
        let mut progress = false;
        for slot in &damaged {
            if restored.contains_key(&(slot.page, slot.slot)) {
                continue;
            }
            for pool in &pools {
                let mut fitting: Vec<&str> = pool
                    .iter()
                    .copied()
                    .filter(|n| damaged_name_fits(&slot.name, n))
                    .collect();
                fitting.sort_unstable();
                fitting.dedup();
                if fitting.len() > 1 {
                    // More than one fits: the names other playlists have
                    // don't count.
                    fitting.retain(|n| !taken.contains(&canonicalize_playlist_name(n)));
                }
                match fitting.as_slice() {
                    [] if !pool.iter().any(|n| damaged_name_fits(&slot.name, n)) => continue,
                    [one] => {
                        taken.insert(canonicalize_playlist_name(one));
                        restored.insert((slot.page, slot.slot), one.to_string());
                        progress = true;
                    }
                    // Still more than one: maybe another row takes one of them.
                    _ => {}
                }
                break;
            }
        }
        if !progress {
            break;
        }
    }
    // A row every fitting name of which other playlists have is a copy of
    // one of them: it gets that name (the first, alphabetically).
    for slot in &damaged {
        if restored.contains_key(&(slot.page, slot.slot)) {
            continue;
        }
        let copy_of = pools.iter().find_map(|pool| {
            pool.iter()
                .copied()
                .filter(|n| damaged_name_fits(&slot.name, n))
                .min()
        });
        if let Some(name) = copy_of {
            restored.insert((slot.page, slot.slot), name.to_string());
        }
    }
    for slot in damaged {
        let restored = restored.remove(&(slot.page, slot.slot));
        repairs.renames.push(PlaylistNameRepair {
            page: slot.page,
            slot: slot.slot,
            id: slot.id,
            parent_id: slot.parent_id,
            sort_order: slot.sort_order,
            is_folder: slot.is_folder,
            old: slot.name.clone(),
            restored: restored.is_some(),
            new: restored.unwrap_or_else(|| slot.name.replace('\0', "")),
        });
    }
    repairs
}

/// Whether `candidate` is the playlist name `name` stands for: the name it
/// fits when damaged, the same name (as playlists are matched) when not.
fn name_fits(name: &str, candidate: &str) -> bool {
    if name.contains('\0') {
        damaged_name_fits(name, candidate)
    } else {
        canonicalize_playlist_name(name) == canonicalize_playlist_name(candidate)
    }
}

/// The live playlist tree of the PDB `bytes` as it is once `repairs` are
/// written.
pub(super) fn repaired_playlist_tree(
    bytes: &[u8],
    repairs: &PlaylistTreeRepairs,
) -> Vec<crate::pdb_reader::PdbPlaylistTreeRow> {
    crate::pdb_writer::playlist_tree_slots(bytes)
        .into_iter()
        .filter(|s| {
            s.live
                && !repairs
                    .half_written
                    .iter()
                    .any(|&(page, slot, _)| (page, slot) == (s.page, s.slot))
        })
        .map(|s| {
            let renamed = repairs
                .renames
                .iter()
                .find(|r| (r.page, r.slot) == (s.page, s.slot));
            crate::pdb_reader::PdbPlaylistTreeRow {
                id: s.id,
                parent_id: s.parent_id,
                sort_order: s.sort_order,
                row_is_folder: s.is_folder,
                name: renamed.map_or_else(|| s.name.replace('\0', ""), |r| r.new.clone()),
            }
        })
        .collect()
}

/// Writes `repairs` into the PDB `bytes`: tombstones the half-written rows
/// and renames the damaged ones, in place when the new name encodes to the
/// old length, else as a new row with the same id, parent and sort order in
/// place of the old one. Returns the rows changed.
pub(super) fn apply_damaged_playlist_tree(
    bytes: &mut Vec<u8>,
    repairs: &PlaylistTreeRepairs,
) -> BackendResult<usize> {
    let mut removed: Vec<(usize, usize)> = repairs
        .half_written
        .iter()
        .map(|&(page, slot, _)| (page, slot))
        .collect();
    let mut appended = Vec::new();
    let mut renamed = 0;
    for rename in &repairs.renames {
        if crate::pdb_writer::rename_playlist_tree_slot_inplace(
            bytes,
            (rename.page, rename.slot),
            &rename.new,
        ) {
            renamed += 1;
        } else {
            removed.push((rename.page, rename.slot));
            appended.push(rename);
        }
    }
    let tombstoned = crate::pdb_writer::remove_playlist_tree_slots_inplace(bytes, &removed);
    if !appended.is_empty() {
        let page_size = crate::utils::read_u32_le_at(bytes, 4)
            .map(|v| v as usize)
            .ok_or_else(|| BackendError::Internal("PDB header has no page size".into()))?;
        let rows: Vec<(u32, u32, u32, bool, &str)> = appended
            .iter()
            .map(|r| (r.id, r.parent_id, r.sort_order, r.is_folder, r.new.as_str()))
            .collect();
        crate::pdb_writer::append_playlist_tree_in_place(bytes, &rows, page_size)?;
    }
    crate::pdb_writer::rebuild_sentinel_btrees_for_tables(bytes, &[7]);
    Ok(renamed + tombstoned)
}
