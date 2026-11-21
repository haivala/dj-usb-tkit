//! Syncing eDB play history from the PDB.

use super::*;

/// (history_id, sequence_no, name, attribute, history_id_parent)
pub(super) type HistoryRow = (i64, i64, String, i64, i64);
/// (history_id, content_id, sequence_no)
pub(super) type HistoryContentRow = (i64, i64, i64);

/// eDB `content_id` per PDB track id, matched by audio path. rekordbox
/// numbers the two databases on its own, so the same track often has
/// different ids in each.
pub(super) fn edb_content_ids_by_pdb_track(
    parsed: &crate::pdb_reader::ParsedPdb,
    conn: &rusqlite::Connection,
) -> HashMap<u32, i64> {
    let mut by_path = HashMap::<String, i64>::new();
    if let Ok(mut stmt) = conn
        .prepare("SELECT content_id, path FROM content WHERE path IS NOT NULL ORDER BY content_id")
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
    {
        for (content_id, path) in rows.flatten() {
            by_path.entry(audio_path_key(&path)).or_insert(content_id);
        }
    }
    parsed
        .tracks
        .iter()
        .filter_map(|track| {
            let content_id = by_path.get(&audio_path_key(&track.track_file_path))?;
            Some((track.id, *content_id))
        })
        .collect()
}

/// The eDB `history`/`history_content` rows the PDB's play history gives.
/// `content_ids` maps PDB track ids to eDB content ids
/// ([`edb_content_ids_by_pdb_track`]); an entry whose track has no eDB row
/// stays in the PDB only.
pub(super) fn derive_history_sync_payload(
    parsed: &crate::pdb_reader::ParsedPdb,
    content_ids: &HashMap<u32, i64>,
) -> (Vec<HistoryRow>, Vec<HistoryContentRow>) {
    let history_rows = parsed
        .history_playlists
        .iter()
        .filter_map(|h| {
            if !h.name.starts_with("HISTORY ") {
                return None;
            }
            let id = i64::from(h.id);
            if id <= 0 {
                return None;
            }
            Some((id, 0_i64, h.name.clone(), 0_i64, 0_i64))
        })
        .collect::<Vec<_>>();

    let valid_history_ids = history_rows
        .iter()
        .map(|(id, _, _, _, _)| *id)
        .collect::<HashSet<_>>();

    let history_content_rows = parsed
        .history_entries
        .iter()
        .filter_map(|e| {
            let track_id = e.track_id?;
            if track_id == 0 {
                return None;
            }
            let history_id = i64::from(e.playlist_id);
            if !valid_history_ids.contains(&history_id) {
                return None;
            }
            let content_id = *content_ids.get(&track_id)?;
            Some((history_id, content_id, i64::from(e.entry_index)))
        })
        .collect::<Vec<_>>();

    (history_rows, history_content_rows)
}

impl BackendService {
    pub(super) fn apply_fix_sync_edb_history_from_pdb(
        &self,
        usb_root: &std::path::Path,
        parsed: &crate::pdb_reader::ParsedPdb,
        edb_conn: Option<&mut rusqlite::Connection>,
    ) -> BackendResult<(usize, usize)> {
        let Some(mut handle) = EdbRw::shared_or_open(edb_conn, usb_root) else {
            return Ok((0, 0));
        };
        let conn = handle.conn();
        let content_ids = edb_content_ids_by_pdb_track(parsed, conn);
        let (history_rows, history_content_rows) =
            derive_history_sync_payload(parsed, &content_ids);
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer); \
             CREATE TABLE IF NOT EXISTS history_content(history_id integer, content_id integer, sequenceNo integer);",
        )?;
        tx.execute("DELETE FROM history_content", [])?;
        tx.execute("DELETE FROM history", [])?;

        for (history_id, sequence_no, name, attribute, history_id_parent) in &history_rows {
            tx.execute(
                "INSERT INTO history(history_id, sequenceNo, name, attribute, history_id_parent) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![history_id, sequence_no, name, attribute, history_id_parent],
            )?;
        }
        for (history_id, content_id, sequence_no) in &history_content_rows {
            tx.execute(
                "INSERT INTO history_content(history_id, content_id, sequenceNo) VALUES (?1, ?2, ?3)",
                params![history_id, content_id, sequence_no],
            )?;
        }

        tx.commit()?;
        Ok((history_rows.len(), history_content_rows.len()))
    }
}
