//! eDB coverage helpers: indexed paths, strict raw coverage, ratings, history rows.

use super::*;

/// Audio paths the eDB `content` table indexes, as written (`path`, or
/// `FolderPath` + `FileNameL` in older schemas). Match them with
/// `audio_path_key`.
pub(crate) fn collect_edb_audio_paths(
    conn: &rusqlite::Connection,
    warnings: &mut Vec<WarningEntry>,
) -> Vec<String> {
    if !table_exists(conn, "content") {
        return Vec::new();
    }
    let Ok(content_columns) = load_table_columns(conn, "content") else {
        warnings.push(logging::log(
            Level::Error,
            "usb-diagnostics",
            "usb.diagnostics.edb-content-column-scan-failed",
            "eDB content column scan failed",
        ));
        return Vec::new();
    };
    let has_column = |name: &str| content_columns.iter().any(|column| column == name);
    let (query_sql, mode) = if has_column("path") {
        (
            r#"
            SELECT DISTINCT COALESCE(c.path, '')
            FROM content c
            "#,
            "path",
        )
    } else if has_column("FolderPath") && has_column("FileNameL") {
        (
            r#"
            SELECT DISTINCT
              COALESCE(c.FolderPath, ''),
              COALESCE(c.FileNameL, '')
            FROM content c
            "#,
            "folder_file",
        )
    } else {
        warnings.push(logging::log(
            Level::Warn,
            "usb-diagnostics",
            "usb.diagnostics.edb-schema-missing-path-columns",
            "eDB content schema missing path columns (expected path or FolderPath/FileNameL)",
        ));
        return Vec::new();
    };
    let mut stmt = match conn.prepare(query_sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            warnings.push(logging::log(
                Level::Error,
                "usb-diagnostics",
                "usb.diagnostics.edb-path-query-prepare-failed",
                format!("eDB playlist path query prepare failed: {err}"),
            ));
            return Vec::new();
        }
    };
    let mut out = Vec::<String>::new();
    let rows = match stmt.query_map([], |row| match mode {
        "path" => Ok((row.get::<_, String>(0).unwrap_or_default(), String::new())),
        _ => Ok((
            row.get::<_, String>(0).unwrap_or_default(),
            row.get::<_, String>(1).unwrap_or_default(),
        )),
    }) {
        Ok(rows) => rows,
        Err(err) => {
            warnings.push(logging::log(
                Level::Error,
                "usb-diagnostics",
                "usb.diagnostics.edb-path-query-failed",
                format!("eDB playlist path query failed: {err}"),
            ));
            return Vec::new();
        }
    };
    for item in rows {
        let Ok((folder, file)) = item else { continue };
        let full = if mode == "path" {
            folder.trim().to_string()
        } else {
            let folder = folder.trim();
            let file = file.trim();
            if folder.is_empty() || file.is_empty() {
                continue;
            }
            format!("{}/{}", folder.trim_end_matches('/'), file)
        };
        if full.is_empty() {
            continue;
        }
        out.push(full);
    }
    out
}

#[derive(Debug, Clone)]
pub(crate) struct StrictRawCoverageParity {
    pub missing_count: usize,
    pub extra_count: usize,
    pub status: DiagStatus,
    pub detail: String,
}

pub(crate) fn evaluate_strict_raw_coverage_parity(
    missing_count: usize,
    extra_count: usize,
    total_indexed: usize,
) -> StrictRawCoverageParity {
    let warn_threshold = std::cmp::max(5, total_indexed / 20);
    let missing_status = if missing_count == 0 {
        DiagStatus::Pass
    } else if missing_count <= warn_threshold {
        DiagStatus::Warn
    } else {
        DiagStatus::Fail
    };
    let extra_status = if extra_count == 0 {
        DiagStatus::Pass
    } else if extra_count <= warn_threshold {
        DiagStatus::Warn
    } else {
        DiagStatus::Fail
    };
    let status = DiagStatus::worst(&missing_status, &extra_status);
    let detail = match (missing_count, extra_count) {
        (0, 0) => format!("all {total_indexed} indexed audio files present, no unindexed files"),
        (m, 0) => format!("{m} of {total_indexed} indexed audio file(s) missing from USB"),
        (0, e) => format!(
            "{e} audio file(s) on USB not referenced in any playlist (total indexed={total_indexed})"
        ),
        (m, e) => {
            format!("{m} of {total_indexed} indexed file(s) missing; {e} unindexed file(s) on USB")
        }
    };
    StrictRawCoverageParity {
        missing_count,
        extra_count,
        status,
        detail,
    }
}

/// Playlist tracks whose star rating or colour differs between the eDB and
/// their PDB row (bytes 89/88), counted once per track.
#[derive(Debug, Default, Clone)]
pub(crate) struct RatingColourParity {
    pub(super) mismatched_tracks: usize,
    pub(super) rating_mismatches: usize,
    pub(super) colour_mismatches: usize,
    /// `normalize_pdb_path_for_edb_lookup` paths of the mismatched tracks.
    pub(super) mismatched_paths: HashSet<String>,
}

pub(super) fn compare_rating_colour(
    conn: &rusqlite::Connection,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: &HashMap<String, ExportDbPlaylist>,
) -> RatingColourParity {
    let pdb_by_path = parsed
        .tracks
        .iter()
        .map(|t| {
            (
                normalize_pdb_path_for_edb_lookup(&t.track_file_path),
                (t.rating, t.color_id),
            )
        })
        .collect::<HashMap<_, _>>();
    let playlist_paths = edb_playlists
        .values()
        .flat_map(|playlist| playlist.tracks.iter())
        .map(|track| normalize_pdb_path_for_edb_lookup(track.identity_path()))
        .collect::<HashSet<_>>();
    let mut out = RatingColourParity::default();
    for (path, edb) in crate::edb::read_rating_colour_by_path(conn) {
        let key = normalize_pdb_path_for_edb_lookup(&path);
        if !playlist_paths.contains(&key) {
            continue;
        }
        let Some(&(pdb_rating, pdb_colour)) = pdb_by_path.get(&key) else {
            continue; // a track missing from the PDB is the membership check's
        };
        let rating_differs = edb.rating != pdb_rating;
        let colour_differs = edb.color_id != pdb_colour;
        out.rating_mismatches += usize::from(rating_differs);
        out.colour_mismatches += usize::from(colour_differs);
        if rating_differs || colour_differs {
            out.mismatched_tracks += 1;
            out.mismatched_paths.insert(key);
        }
    }
    out
}

/// Playlist tracks whose other synced fields (`note_field_differences`)
/// differ between the eDB and their PDB row. Reported as notes only.
#[derive(Debug, Default, Clone)]
pub(crate) struct FieldNoteParity {
    pub(super) mismatched_tracks: usize,
    /// Tracks per differing field.
    pub(super) by_field: std::collections::BTreeMap<&'static str, usize>,
    /// `normalize_pdb_path_for_edb_lookup` paths of the mismatched tracks.
    pub(super) mismatched_paths: HashSet<String>,
}

pub(super) fn compare_note_fields(
    conn: &rusqlite::Connection,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: &HashMap<String, ExportDbPlaylist>,
) -> FieldNoteParity {
    let pdb_by_path = parsed
        .tracks
        .iter()
        .map(|t| (normalize_pdb_path_for_edb_lookup(&t.track_file_path), t))
        .collect::<HashMap<_, _>>();
    let playlist_paths = edb_playlists
        .values()
        .flat_map(|playlist| playlist.tracks.iter())
        .map(|track| normalize_pdb_path_for_edb_lookup(track.identity_path()))
        .collect::<HashSet<_>>();
    let mut out = FieldNoteParity::default();
    let rows = crate::service::export_helpers::read_edb_content_fields(conn).unwrap_or_default();
    for edb in rows.values() {
        let Some(path) = edb.path.as_deref() else {
            continue;
        };
        let key = normalize_pdb_path_for_edb_lookup(path);
        if !playlist_paths.contains(&key) {
            continue;
        }
        let Some(pdb) = pdb_by_path.get(&key) else {
            continue; // a track missing from the PDB is the membership check's
        };
        let fields =
            crate::service::export_helpers::field_table::note_field_differences(edb, pdb, parsed);
        if fields.is_empty() {
            continue;
        }
        for field in fields {
            *out.by_field.entry(field).or_default() += 1;
        }
        if out.mismatched_paths.insert(key) {
            out.mismatched_tracks += 1;
        }
    }
    out
}

/// Count PDB history playlists/entries that look like real rekordbox-recorded
/// sessions, excluding the blank template/seed rows every fresh or
/// never-played export ships in t17/t18 (see `is_named_history_playlist`).
pub(super) fn count_named_history_rows(
    playlists: &[crate::pdb_reader::PdbHistoryPlaylistRow],
    entries: &[crate::pdb_reader::PdbHistoryEntryRow],
) -> (usize, usize) {
    let named_history_ids = playlists
        .iter()
        .filter(|row| is_named_history_playlist(&row.name))
        .map(|row| row.id)
        .collect::<HashSet<_>>();
    let hp = named_history_ids.len();
    let he = entries
        .iter()
        .filter(|row| named_history_ids.contains(&row.playlist_id))
        .count();
    (hp, he)
}
