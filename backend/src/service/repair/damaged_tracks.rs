//! Track rows a crashed rekordbox write left with NUL bytes in their strings.
//!
//! rekordbox 6 can crash halfway through a write and leave 2-byte NUL runs
//! through a page's row heap. A track row hit this way has a path that names
//! no file. When rekordbox had already given the same audio a second, intact
//! row (it does when it doesn't recognise a row another collection owns), the
//! damaged row is a broken copy of that twin: its playlist and history
//! entries move to the twin and the row goes. The number fields survive the
//! overwrite and the strings don't (characters can also be missing and paths
//! cut short), so the twin is found by file size and duration, plus the
//! rekordbox master ids in the eDB, and only when exactly one row fits.

use super::*;

/// Damaged track rows, and the intact twin each one merges into.
#[derive(Debug, Default, Clone)]
pub(super) struct NulDamagedTracks {
    /// Damaged PDB track id → id of its intact twin.
    pub(super) pdb_twins: HashMap<u32, u32>,
    /// Damaged eDB content_id → content_id of its intact twin.
    pub(super) edb_twins: HashMap<i64, i64>,
    /// Paths of the merged rows, which the missing-audio fixes leave alone.
    pub(super) merged_paths: Vec<String>,
    /// Paths of damaged rows without exactly one twin: nothing to merge
    /// them into, so they stay missing audio.
    pub(super) without_twin: Vec<String>,
}

impl NulDamagedTracks {
    pub(super) fn is_empty(&self) -> bool {
        self.pdb_twins.is_empty() && self.edb_twins.is_empty()
    }

    /// Rows merged across both databases.
    pub(super) fn len(&self) -> usize {
        self.pdb_twins.len() + self.edb_twins.len()
    }
}

/// A NUL inside the string, not just padding at its end.
fn has_nul_run(value: &str) -> bool {
    value.trim_end_matches('\0').contains('\0')
}

/// One track row, from either database, as the twin match sees it.
struct TrackFacts<Id> {
    id: Id,
    path: String,
    damaged: bool,
    missing: bool,
    /// What a twin must share: file size, duration and, in the eDB, the
    /// rekordbox master ids.
    key: Option<(u64, u64, i64, i64)>,
}

/// Damaged rows whose file is missing, each paired with the one intact row
/// whose file exists and whose key matches. Returns the pairs and the
/// damaged rows without exactly one twin.
fn pair_twins<Id: Copy + Eq + std::hash::Hash>(
    rows: &[TrackFacts<Id>],
) -> (HashMap<Id, Id>, Vec<&TrackFacts<Id>>) {
    let mut twins = HashMap::new();
    let mut without_twin = Vec::new();
    for row in rows.iter().filter(|r| r.damaged && r.missing) {
        let mut candidates = rows
            .iter()
            .filter(|t| !t.damaged && !t.missing && t.key.is_some() && t.key == row.key);
        match (candidates.next(), candidates.next()) {
            (Some(twin), None) => {
                twins.insert(row.id, twin.id);
            }
            _ => without_twin.push(row),
        }
    }
    (twins, without_twin)
}

/// Finds the damaged rows in the PDB (`parsed`) and the eDB. `missing_keys`
/// are the `audio_path_key`s of the indexed paths with no file on the USB.
pub(super) fn detect_nul_damaged_tracks(
    parsed: &ParsedPdb,
    edb: Option<&rusqlite::Connection>,
    missing_keys: &HashSet<String>,
) -> NulDamagedTracks {
    let missing = |path: &str| missing_keys.contains(&audio_path_key(path));
    let mut found = NulDamagedTracks::default();

    let pdb_rows: Vec<_> = parsed
        .tracks
        .iter()
        .map(|t| TrackFacts {
            id: t.id,
            path: t.track_file_path.clone(),
            damaged: has_nul_run(&t.track_file_path) || has_nul_run(&t.title),
            missing: missing(&t.track_file_path),
            key: t
                .file_size_bytes
                .filter(|&size| size > 0)
                .zip(t.duration_seconds)
                .map(|(size, duration)| (u64::from(size), u64::from(duration), 0, 0)),
        })
        .collect();
    let (pdb_twins, pdb_without) = pair_twins(&pdb_rows);
    found.merged_paths.extend(
        pdb_rows
            .iter()
            .filter(|r| pdb_twins.contains_key(&r.id))
            .map(|r| r.path.clone()),
    );
    found
        .without_twin
        .extend(pdb_without.iter().map(|r| r.path.clone()));
    found.pdb_twins = pdb_twins;

    let edb_rows: Vec<TrackFacts<i64>> = edb
        .filter(|conn| table_exists(conn, "content"))
        .and_then(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT content_id, path, title, fileSize, length, masterDbId, masterContentId \
                     FROM content WHERE path IS NOT NULL",
                )
                .ok()?;
            let rows = stmt
                .query_map([], |row| {
                    let path: String = row.get(1)?;
                    let title: Option<String> = row.get(2)?;
                    let size: Option<i64> = row.get(3)?;
                    let length: Option<i64> = row.get(4)?;
                    let master_db: Option<i64> = row.get(5)?;
                    let master_content: Option<i64> = row.get(6)?;
                    let key = match (size, length, master_db, master_content) {
                        (Some(size), Some(length), Some(db), Some(content))
                            if size > 0 && db != 0 && content != 0 =>
                        {
                            Some((size as u64, length.max(0) as u64, db, content))
                        }
                        _ => None,
                    };
                    Ok(TrackFacts {
                        id: row.get(0)?,
                        damaged: has_nul_run(&path) || title.as_deref().is_some_and(has_nul_run),
                        missing: missing(&path),
                        path,
                        key,
                    })
                })
                .ok()?;
            Some(rows.flatten().collect())
        })
        .unwrap_or_default();
    let (edb_twins, edb_without) = pair_twins(&edb_rows);
    found.merged_paths.extend(
        edb_rows
            .iter()
            .filter(|r| edb_twins.contains_key(&r.id))
            .map(|r| r.path.clone()),
    );
    found
        .without_twin
        .extend(edb_without.iter().map(|r| r.path.clone()));
    found.edb_twins = edb_twins;

    found.merged_paths.sort();
    found.merged_paths.dedup();
    let merged_keys: HashSet<_> = found
        .merged_paths
        .iter()
        .map(|p| audio_path_key(p))
        .collect();
    found
        .without_twin
        .retain(|p| !merged_keys.contains(&audio_path_key(p)));
    found.without_twin.sort();
    found.without_twin.dedup();
    found
}

/// Rows the merge changed: per database, the entries moved to a twin, the
/// duplicate playlist entries dropped and the damaged rows deleted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct TwinMerge {
    pub(super) edb_moved: usize,
    pub(super) edb_dropped: usize,
    pub(super) edb_deleted: usize,
    pub(super) pdb_moved: usize,
    pub(super) pdb_dropped: usize,
    pub(super) pdb_deleted: usize,
}

impl BackendService {
    /// Merges every damaged row of `found` into its twin, in both
    /// databases: playlist and history entries move to the twin (a playlist
    /// entry is dropped where the playlist already holds the twin; history
    /// is never dropped), then the damaged row is deleted.
    pub(super) fn apply_fix_merge_nul_damaged_tracks(
        &self,
        usb_root: &Path,
        found: &NulDamagedTracks,
        edb_conn: Option<&mut rusqlite::Connection>,
    ) -> BackendResult<TwinMerge> {
        let mut merge = TwinMerge::default();
        let mut t19_property = None;
        if !found.edb_twins.is_empty() {
            let Some(mut handle) = EdbRw::shared_or_open(edb_conn, usb_root) else {
                return Err(BackendError::Internal(
                    "the eDB could not be opened".to_string(),
                ));
            };
            let conn = handle.conn();
            let tx = conn.transaction()?;
            let has_playlists = table_exists(&tx, "playlist_content");
            let has_history = table_exists(&tx, "history_content");
            for (&damaged, &twin) in &found.edb_twins {
                if has_playlists {
                    merge.edb_dropped += tx.execute(
                        "DELETE FROM playlist_content WHERE content_id = ?1 AND playlist_id IN \
                         (SELECT playlist_id FROM playlist_content WHERE content_id = ?2)",
                        params![damaged, twin],
                    )?;
                    merge.edb_moved += tx.execute(
                        "UPDATE playlist_content SET content_id = ?2 WHERE content_id = ?1",
                        params![damaged, twin],
                    )?;
                }
                if has_history {
                    merge.edb_moved += tx.execute(
                        "UPDATE history_content SET content_id = ?2 WHERE content_id = ?1",
                        params![damaged, twin],
                    )?;
                }
                merge.edb_deleted += tx.execute(
                    "DELETE FROM content WHERE content_id = ?1",
                    params![damaged],
                )?;
            }
            if merge.edb_deleted > 0 {
                sync_property_content_count(&tx)?;
            }
            t19_property = t19_property_from_conn(&tx);
            tx.commit()?;
        }

        if !found.pdb_twins.is_empty() {
            let pdb_path = usb_staging::required_pdb(usb_root)?;
            let mut bytes = std::fs::read(&pdb_path)?;
            (merge.pdb_moved, merge.pdb_dropped) =
                crate::pdb_writer::merge_track_entries_inplace(&mut bytes, &found.pdb_twins);
            std::fs::write(&pdb_path, &bytes)?;
            let damaged: HashSet<u32> = found.pdb_twins.keys().copied().collect();
            let removal = remove_tracks_from_pdb(usb_root, &damaged)?;
            merge.pdb_dropped += removal.playlist_entries;
            merge.pdb_deleted = removal.track_rows;
            // Fewer tracks: tt=19 follows the new count, as an export's
            // write does (a no-op on a page rekordbox or a player wrote).
            if removal.track_rows > 0 {
                let mut bytes = std::fs::read(&pdb_path)?;
                if apply_pdb_t19_template_state_repair(&mut bytes, t19_property.as_ref())? {
                    std::fs::write(&pdb_path, &bytes)?;
                }
            }
        }
        Ok(merge)
    }
}
