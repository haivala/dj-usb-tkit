//! Syncing eDB play history from the PDB.

use super::*;

/// (history_id, sequence_no, name, attribute, history_id_parent)
pub(super) type HistoryRow = (i64, i64, String, i64, i64);
/// (history_id, content_id, sequence_no)
pub(super) type HistoryContentRow = (i64, i64, i64);

pub(super) fn derive_history_sync_payload(
    parsed: &crate::pdb_reader::ParsedPdb,
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
            Some((history_id, i64::from(track_id), i64::from(e.entry_index)))
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
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize)> {
        let (history_rows, history_content_rows) = derive_history_sync_payload(parsed);
        let Some(mut handle) = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
        else {
            return Ok((0, 0));
        };
        let conn = handle.conn();
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
