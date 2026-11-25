//! Strict parity upgrade: collect both USB databases, merge them into the
//! export writers' input, and let the writers write both once.
//!
//! The repair reads and merges; it writes nothing itself. Every row it
//! changes goes through the export writers (`write_edb_playlists_in_tx`,
//! `build_pdb_for_playlists`), which write only what differs.
//!
//! When the USB has only one of the two databases, collecting first stages
//! the missing one as `initialize_usb` would create it, so the merge and
//! the writers rebuild it from the other.

use super::*;
use crate::service::export_helpers::{
    EdbContentFields, canonicalize_track_path_identity, is_placeholder_date,
    read_edb_content_fields,
};

#[derive(Debug, Default, Clone)]
pub(super) struct StrictParityUpgradeApplyResult {
    pub(super) merged_playlists: usize,
    pub(super) edb_playlists_written: usize,
    pub(super) duplicate_entries_removed: usize,
    /// Top-level eDB copies of playlists in folders, removed (see
    /// [`flattened_edb_copies`]).
    pub(super) flattened_copies_removed: usize,
    pub(super) edb_rows_inserted: usize,
    pub(super) edb_rows_updated: usize,
    pub(super) pdb_tracks_inserted: usize,
    pub(super) pdb_tracks_changed: usize,
    pub(super) pdb_playlists_inserted: usize,
    /// The database the collect phase created because the USB lacked it.
    pub(super) rebuilt_database: Option<staging::DbKind>,
}

/// The database to rebuild from the other: the one the stick lacks while
/// the other is there. `None` when it has both or neither.
pub(super) fn missing_database(usb_root: &std::path::Path) -> Option<staging::DbKind> {
    staging::UsbDatabases::present(usb_root)
        .ok()
        .flatten()
        .and_then(|databases| databases.missing())
}

/// Rebuilds the database the stick lacks from the one it has, in `session`:
/// the strict parity repair's rebuild, the one entry point every flow that
/// needs both databases takes. `None` when the stick has both (or neither).
/// The session writes it back on commit.
pub(crate) fn rebuild_missing_database(
    session: &mut staging::UsbWriteSession<'_>,
) -> BackendResult<Option<staging::DbKind>> {
    let Some(missing) = missing_database(session.usb_root()) else {
        return Ok(None);
    };
    // The merge also writes the database the stick has.
    session.will_write(match missing {
        staging::DbKind::Pdb => staging::DbKind::Edb,
        staging::DbKind::Edb => staging::DbKind::Pdb,
    })?;
    let usb_root = session.usb_root().to_path_buf();
    run_strict_parity_upgrade(&usb_root, None, Some(session), true)
        .map(|result| result.rebuilt_database)
}

pub(super) fn edb_track_analysis_key(track: &crate::models::UsbTrack) -> String {
    normalize_analysis_path_for_identity(
        track
            .usb_analysis_path_raw
            .as_deref()
            .or(track.usb_analysis_path.as_deref())
            .unwrap_or_default(),
    )
}

pub(crate) fn playlist_requires_strict_upgrade(detail: &UsbParityPlaylistDetail) -> bool {
    !matches!(detail.status, DiagStatus::Pass)
        || detail.only_in_pdb > 0
        || detail.only_in_edb > 0
        || detail.order_mismatch
        || detail.path_mismatch_tracks > 0
        || detail.dictionary_id_issue_tracks > 0
        || detail.pdb_duplicate_entries > 0
        || detail.pdb_missing_core_metadata > 0
        || detail.artwork_mismatch_tracks > 0
        || detail.rating_colour_mismatch_tracks > 0
        // Notes don't fail parity, but the merge is what makes them agree.
        || detail.field_note_tracks > 0
        || !detail.playlist_in_both
        || !detail.sort_order_match
}

/// The order the repair merges playlists in, by path key
/// ([`PlaylistPath::key`]): the PDB's playlists by sort order, then those
/// only the eDB has, by the eDB's.
///
/// [`PlaylistPath::key`]: crate::pdb_reader::PlaylistPath::key
pub(super) fn strict_repair_ordered_playlist_keys(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
) -> Vec<String> {
    let tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
    let mut pdb_leaves = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.is_folder)
        .collect::<Vec<_>>();
    pdb_leaves.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut edb = edb_playlists
        .map(|playlists| playlists.values().collect::<Vec<_>>())
        .unwrap_or_default();
    edb.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.playlist_id.cmp(&b.playlist_id))
            .then_with(|| a.path.display().cmp(&b.path.display()))
    });
    let mut seen = HashSet::<String>::new();
    pdb_leaves
        .into_iter()
        .map(|leaf| tree.path(leaf).key())
        .chain(edb.into_iter().map(|playlist| playlist.path.key()))
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

/// The eDB's playlists (not folders): `(playlist_id, path key)`.
fn edb_playlist_keys(tx: &rusqlite::Transaction<'_>) -> BackendResult<Vec<(i64, String)>> {
    let rows = crate::edb::read_edb_playlist_tree(tx)?;
    let tree = crate::pdb_reader::PlaylistTree::new(&rows);
    Ok(rows
        .iter()
        .filter(|row| !row.is_folder)
        .map(|row| (i64::from(row.id), tree.path(row).key()))
        .collect())
}

/// Writes a USB playlist's track order to the eDB, in the caller's
/// transaction: each eDB playlist whose path key is in `names` gets its
/// own `playlist_content` rows put in `order` (`order_rows` with the content's
/// path as the key), renumbered from the playlist's lowest `sequenceNo`.
/// Matching by path, not by row id, is what lets one pending order (keyed by
/// media path) order the eDB's rows and the PDB's alike. Returns the rows
/// updated.
pub(crate) fn write_edb_playlist_track_order(
    tx: &rusqlite::Transaction<'_>,
    names: &HashSet<String>,
    order_rows: impl Fn(&mut Vec<(i64, i64, String)>),
) -> BackendResult<usize> {
    if names.is_empty()
        || !table_exists(tx, "playlist")
        || !table_exists(tx, "playlist_content")
        || !table_exists(tx, "content")
    {
        return Ok(0);
    }
    let mut updated = 0usize;
    for (playlist_id, key) in edb_playlist_keys(tx)? {
        if !names.contains(&key) {
            continue;
        }
        // (rowid, sequenceNo, path), in the eDB's current order.
        let mut rows = {
            let mut stmt = tx.prepare(
                "SELECT pc.rowid, COALESCE(pc.sequenceNo, 0), COALESCE(c.path, '') \
                 FROM playlist_content pc LEFT JOIN content c ON c.content_id = pc.content_id \
                 WHERE pc.playlist_id = ?1 ORDER BY pc.sequenceNo, pc.rowid",
            )?;
            stmt.query_map(params![playlist_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let Some(base) = rows.iter().map(|(_, sequence, _)| *sequence).min() else {
            continue;
        };
        order_rows(&mut rows);
        for ((rowid, sequence, _), new_sequence) in rows.iter().zip(base..) {
            if *sequence != new_sequence {
                updated += tx.execute(
                    "UPDATE playlist_content SET sequenceNo = ?1 WHERE rowid = ?2",
                    params![new_sequence, rowid],
                )?;
            }
        }
    }
    Ok(updated)
}

/// What a strict-parity upgrade changed: one line for the repair outcome and the preview card.
pub(super) fn strict_parity_summary(result: &StrictParityUpgradeApplyResult) -> crate::i18n::Text {
    use crate::i18n::tr;
    let summary = tr::repair_strict_parity_summary(
        result.merged_playlists,
        result.edb_rows_inserted,
        result.edb_rows_updated,
        result.pdb_tracks_inserted,
        result.pdb_tracks_changed,
        result.pdb_playlists_inserted,
        result.duplicate_entries_removed,
    );
    let summary = match result.flattened_copies_removed {
        0 => summary,
        count => tr::repair_strict_parity_flattened_copies(count, summary),
    };
    match result.rebuilt_database {
        Some(staging::DbKind::Pdb) => tr::repair_strict_parity_rebuilt_pdb(summary),
        Some(staging::DbKind::Edb) => tr::repair_strict_parity_rebuilt_edb(summary),
        None => summary,
    }
}

/// The eDB's top-level playlists that are flattened copies of PDB playlists
/// in folders, as earlier app versions' repair wrote them (it put every
/// playlist at the top level and merged same-named ones): an eDB playlist at
/// the top level that the PDB doesn't have there, while the PDB has
/// playlists of its name inside folders that the eDB doesn't have. Keyed by
/// path key; the PDB's playlists, in their folders, replace them.
// TODO(1.0.0): only earlier app versions wrote these; remove with the other
// old-writer repairs.
pub(super) fn flattened_edb_copies(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
) -> HashMap<String, String> {
    let Some(edb_playlists) = edb_playlists else {
        return HashMap::new();
    };
    let edb_keys: HashSet<String> = edb_playlists
        .values()
        .map(|playlist| playlist.path.key())
        .collect();
    let tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
    let mut pdb_keys = HashSet::<String>::new();
    let mut names_only_in_pdb_folders = HashSet::<String>::new();
    for leaf in parsed.playlist_tree.iter().filter(|row| !row.is_folder) {
        let path = tree.path(leaf);
        let key = path.key();
        if !path.folders.is_empty() && !edb_keys.contains(&key) {
            names_only_in_pdb_folders.insert(canonicalize_playlist_name(&path.name));
        }
        pdb_keys.insert(key);
    }
    edb_playlists
        .values()
        .filter(|playlist| {
            playlist.path.folders.is_empty()
                && !pdb_keys.contains(&playlist.path.key())
                && names_only_in_pdb_folders
                    .contains(&canonicalize_playlist_name(&playlist.path.name))
        })
        .map(|playlist| (playlist.path.key(), playlist.path.name.clone()))
        .collect()
}

/// The first non-empty of the eDB's and the PDB's text.
fn merged_text(edb: Option<&str>, pdb: Option<&str>) -> Option<String> {
    [edb, pdb]
        .into_iter()
        .flatten()
        .find(|v| !v.trim().is_empty())
        .map(str::to_string)
}

/// The first of the eDB's and the PDB's number that is set (non-zero: 0
/// means "unknown" for these fields on both sides).
fn merged_number(edb: Option<i64>, pdb: Option<i64>) -> Option<i64> {
    [edb, pdb].into_iter().flatten().find(|v| *v > 0)
}

/// One track as the repair hands it to the writers: per field, a non-empty
/// eDB value wins, else the PDB's. A side without a value never blanks the
/// other. Nothing is derived, except two dates: the date added of a track
/// neither side has one for (today, the date the PDB writer would give a new
/// row; carried on the track so both writers store the same date in one
/// pass), and the creation date of a track whose eDB row has none (the audio
/// file's date on the USB, else its date added). A 1970-01-01 placeholder
/// date on either side counts as none ([`is_placeholder_date`]).
pub(super) fn merged_manifest_track(
    usb_root: &std::path::Path,
    edb: Option<&EdbContentFields>,
    pdb: Option<&crate::pdb_reader::PdbTrackRow>,
    parsed: &crate::pdb_reader::ParsedPdb,
    position: usize,
) -> ExportManifestTrack {
    fn pdb_name(map: &HashMap<u32, String>, id: Option<u32>) -> Option<&str> {
        id.filter(|id| *id != 0)
            .and_then(|id| map.get(&id))
            .map(String::as_str)
    }
    let e = |f: fn(&EdbContentFields) -> Option<&str>| edb.and_then(f);
    let en = |f: fn(&EdbContentFields) -> Option<i64>| edb.and_then(f);
    let p_text = |f: fn(&crate::pdb_reader::PdbTrackRow) -> Option<&str>| pdb.and_then(f);
    let p_num = |f: fn(&crate::pdb_reader::PdbTrackRow) -> Option<i64>| pdb.and_then(f);

    let exported_path = merged_text(
        e(|r| r.path.as_deref()),
        p_text(|t| Some(t.track_file_path.as_str())),
    )
    .unwrap_or_default();
    // The PDB's slot 15 holds rekordbox's dateAdded; the app writes the same
    // date to slot 10 too, which is all an older row may have.
    let date_added = merged_text(
        e(|r| r.date_added.as_deref()),
        p_text(|t| t.analyze_date.as_deref())
            .filter(|d| !is_placeholder_date(d))
            .or_else(|| p_text(|t| t.date_added.as_deref()).filter(|d| !is_placeholder_date(d))),
    )
    .or_else(|| Some(chrono::Utc::now().format("%Y-%m-%d").to_string()));
    let date_created = match edb.and_then(|row| row.date_created.clone()) {
        Some(date) => Some(date),
        None => {
            let on_usb = usb_root.join(exported_path.trim_start_matches('/'));
            std::fs::metadata(&on_usb)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| {
                    chrono::DateTime::<chrono::Local>::from(t)
                        .format("%Y-%m-%d")
                        .to_string()
                })
                .or_else(|| date_added.clone())
        }
    };
    // The eDB's rating and colour win whenever it has the track (0 is a
    // rating); the PDB's only fill a track the eDB lacks.
    let rating = match edb {
        Some(row) => row.rating,
        None => pdb.map(|t| i64::from(t.rating)),
    }
    .and_then(|v| u32::try_from(v).ok());
    let color_id = match edb {
        Some(row) => row.color_id,
        None => pdb.map(|t| i64::from(t.color_id)),
    }
    .and_then(|v| u32::try_from(v).ok());
    let to_u32 = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());

    ExportManifestTrack {
        id: match (edb, pdb) {
            (Some(row), _) => format!("edb-{}", row.content_id),
            (None, Some(t)) => format!("pdb-{}", t.id),
            (None, None) => format!("merged-{position}"),
        },
        master_db_id: merged_number(
            en(|r| r.master_db_id),
            p_num(|t| t.master_db_id.map(i64::from)),
        ),
        master_content_id: merged_number(
            en(|r| r.master_content_id),
            p_num(|t| t.master_content_id.map(i64::from)),
        ),
        content_link: merged_number(
            en(|r| r.content_link),
            p_num(|t| t.content_link.map(i64::from)),
        ),
        position,
        track_number: to_u32(merged_number(
            en(|r| r.track_no),
            p_num(|t| Some(i64::from(t.track_number))),
        )),
        title: merged_text(
            e(|r| r.title.as_deref()),
            p_text(|t| Some(t.title.as_str())),
        )
        .unwrap_or_default(),
        artist: merged_text(
            e(|r| r.artist.as_deref()),
            pdb_name(&parsed.artists, pdb.map(|t| t.artist_id)),
        )
        .unwrap_or_default(),
        album: merged_text(
            e(|r| r.album.as_deref()),
            pdb_name(&parsed.albums, pdb.map(|t| t.album_id)),
        ),
        bpm: merged_number(en(|r| r.bpmx100), p_num(|t| Some(i64::from(t.tempo_x100))))
            .map(|v| v as f64 / 100.0),
        key: merged_text(
            e(|r| r.key.as_deref()),
            pdb_name(&parsed.keys, pdb.map(|t| t.key_id)),
        ),
        source_path: exported_path.clone(),
        exported_path,
        file_timestamp_at: None,
        file_size_bytes: merged_number(
            en(|r| r.file_size),
            p_num(|t| t.file_size_bytes.map(i64::from)),
        ),
        sample_rate_hz: to_u32(merged_number(
            en(|r| r.sampling_rate),
            p_num(|t| t.sample_rate_hz.map(i64::from)),
        )),
        bit_depth: merged_number(en(|r| r.bit_depth), p_num(|t| t.bit_depth.map(i64::from)))
            .and_then(|v| u8::try_from(v).ok()),
        bitrate_kbps: to_u32(merged_number(
            en(|r| r.bitrate),
            p_num(|t| t.bitrate_kbps.map(i64::from)),
        )),
        disc_number: to_u32(merged_number(en(|r| r.disc_no), None)),
        subtitle: None,
        comment: merged_text(
            e(|r| r.comment.as_deref()),
            p_text(|t| t.dj_comment.as_deref()),
        ),
        title_for_search: None,
        kuvo_delivery_comment: None,
        dj_play_count: None,
        rating,
        color_id,
        artist_id_lyricist: None,
        artist_id_original_artist: None,
        artist_id_remixer: None,
        artist_id_composer: None,
        genre_id: None,
        genre: merged_text(
            e(|r| r.genre.as_deref()),
            pdb_name(&parsed.genres, pdb.map(|t| t.genre_id)),
        ),
        label_id: None,
        isrc: merged_text(e(|r| r.isrc.as_deref()), p_text(|t| t.isrc.as_deref())),
        release_year: to_u32(merged_number(
            en(|r| r.release_year),
            p_num(|t| t.release_year.map(i64::from)),
        )),
        release_date: merged_text(
            e(|r| r.release_date.as_deref()),
            p_text(|t| t.release_date.as_deref()),
        ),
        recorded_date: None,
        file_type: merged_number(en(|r| r.file_type), p_num(|t| t.file_type.map(i64::from))),
        owns_exported_media: false,
        owns_artwork: false,
        owns_waveform: false,
        artwork_path: merged_text(
            e(|r| r.artwork_path.as_deref()),
            pdb_name(&parsed.artworks, pdb.map(|t| t.artwork_id)),
        ),
        waveform_path: merged_text(
            e(|r| r.analysis_path.as_deref()),
            p_text(|t| Some(t.anlz_path.as_str())),
        ),
        duration_ms: merged_number(
            en(|r| r.length),
            p_num(|t| t.duration_seconds.map(i64::from)),
        )
        .and_then(|secs| u64::try_from(secs).ok())
        .map(|secs| secs * 1000),
        first_beat_ms: None,
        cues: Vec::new(),
        date_added,
        date_created,
        label: merged_text(
            e(|r| r.label.as_deref()),
            pdb_name(&parsed.labels, pdb.map(|t| t.label_id)),
        ),
        remixer: merged_text(
            e(|r| r.remixer.as_deref()),
            pdb_name(&parsed.artists, pdb.map(|t| t.remixer_id)),
        ),
        composer: merged_text(
            e(|r| r.composer.as_deref()),
            pdb_name(&parsed.artists, pdb.map(|t| t.composer_id)),
        ),
        from_library: false,
        pdb_track_id: pdb.map(|t| t.id),
        edb_content_id: edb.map(|row| row.content_id),
    }
}

/// A playlist the repair merged: its PDB identity, its folders and its
/// members in order (the eDB's first, then members only the PDB has).
pub(super) struct MergedPlaylist {
    /// Its own name, without folders.
    name: String,
    /// The PDB playlist row it is written into, when the PDB has it.
    pdb_row_id: Option<u32>,
    playlist_id: u32,
    sort_order: u32,
    /// The folders it sits in, outermost first: the PDB's, or the eDB's
    /// for a playlist only the eDB has.
    folders: Vec<crate::pdb_reader::PdbPlaylistTreeRow>,
    tracks: Vec<ExportManifestTrack>,
}

/// Merge both databases' playlists (all of them, or those whose path key is
/// in `target_keys`), matched by path ([`PlaylistPath::key`]) so same-named
/// playlists in different folders stay apart: membership is the union,
/// PDB-only members are kept, and PDB playlist ids, sort orders and folders
/// are preserved. `edb_tree` is the eDB's playlist tree, for the folders of
/// a playlist only the eDB has.
///
/// [`PlaylistPath::key`]: crate::pdb_reader::PlaylistPath::key
pub(super) fn merge_strict_parity_playlists(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
    edb_tree: &[crate::pdb_reader::PdbPlaylistTreeRow],
    edb_rows: &HashMap<i64, EdbContentFields>,
    target_keys: Option<&HashSet<String>>,
    flattened: &HashMap<String, String>,
) -> Vec<MergedPlaylist> {
    let artist_name = |t: &crate::pdb_reader::PdbTrackRow| {
        parsed
            .artists
            .get(&t.artist_id)
            .map(String::as_str)
            .unwrap_or("")
    };
    let pdb_identity = |t: &crate::pdb_reader::PdbTrackRow| {
        track_identity_key(
            &t.track_file_path,
            &t.title,
            artist_name(t),
            Some(&t.id.to_string()),
        )
    };
    let pdb_track_by_key: HashMap<String, &crate::pdb_reader::PdbTrackRow> =
        parsed.tracks.iter().map(|t| (pdb_identity(t), t)).collect();
    let pdb_track_by_analysis: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
        .tracks
        .iter()
        .filter_map(|t| {
            let key = normalize_analysis_path_for_identity(&t.anlz_path);
            (!key.is_empty()).then_some((key, t))
        })
        .collect();
    // Tracks with an empty path, by title and artist, for an eDB track
    // whose path key finds nothing.
    let pdb_track_by_meta: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
        .tracks
        .iter()
        .filter(|t| t.track_file_path.trim().is_empty())
        .map(|t| (build_meta_key(&t.title, artist_name(t)), t))
        .collect();
    let pdb_track_by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
        parsed.tracks.iter().map(|t| (t.id, t)).collect();
    let edb_row_by_path: HashMap<String, &EdbContentFields> = edb_rows
        .values()
        .filter_map(|row| {
            let key = canonicalize_track_path_identity(row.path.as_deref()?);
            (!key.is_empty()).then_some((key, row))
        })
        .collect();

    struct PdbPlaylistInfo {
        playlist_id: u32,
        sort_order: u32,
        name: String,
        folders: Vec<crate::pdb_reader::PdbPlaylistTreeRow>,
        entries: Vec<crate::pdb_reader::PdbPlaylistEntryRow>,
    }
    let pdb_tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
    let mut pdb_playlists_by_key: HashMap<String, PdbPlaylistInfo> = HashMap::new();
    for leaf in parsed.playlist_tree.iter().filter(|r| !r.is_folder) {
        let mut entries: Vec<_> = parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == leaf.id)
            .cloned()
            .collect();
        entries.sort_by_key(|e| e.entry_index);
        // Several rows of the same name in one folder: prefer better track
        // coverage, then the lower sort order.
        let candidate = PdbPlaylistInfo {
            playlist_id: leaf.id,
            sort_order: leaf.sort_order,
            name: leaf.name.clone(),
            folders: pdb_tree
                .folders(leaf.parent_id)
                .into_iter()
                .cloned()
                .collect(),
            entries,
        };
        let key = pdb_tree.path(leaf).key();
        let replace = pdb_playlists_by_key.get(&key).is_none_or(|existing| {
            candidate.entries.len() > existing.entries.len()
                || (candidate.entries.len() == existing.entries.len()
                    && candidate.sort_order < existing.sort_order)
        });
        if replace {
            pdb_playlists_by_key.insert(key, candidate);
        }
    }
    let edb_by_key: HashMap<String, &ExportDbPlaylist> = edb_playlists
        .map(|playlists| {
            playlists
                .values()
                .map(|playlist| (playlist.path.key(), playlist))
                .collect()
        })
        .unwrap_or_default();
    let edb_tree_index = crate::pdb_reader::PlaylistTree::new(edb_tree);
    let edb_folders_of = |playlist: &ExportDbPlaylist| {
        edb_tree
            .iter()
            .find(|row| !row.is_folder && i64::from(row.id) == playlist.playlist_id)
            .map(|row| {
                edb_tree_index
                    .folders(row.parent_id)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let pdb_tree_ids: HashSet<u32> = parsed.playlist_tree.iter().map(|r| r.id).collect();
    let mut next_playlist_id = pdb_tree_ids.iter().max().copied().unwrap_or(0) + 1;
    let mut next_sort_order = parsed
        .playlist_tree
        .iter()
        .map(|r| r.sort_order)
        .max()
        .unwrap_or(0)
        + 1;
    let has_existing_pdb_order = parsed.playlist_tree.iter().any(|r| !r.is_folder);
    let mut used_playlist_ids = pdb_tree_ids.clone();
    let mut merged = Vec::<MergedPlaylist>::new();

    for key in &strict_repair_ordered_playlist_keys(parsed, edb_playlists) {
        if target_keys.is_some_and(|targets| !targets.contains(key)) {
            continue;
        }
        if flattened.contains_key(key) {
            continue;
        }
        let edb_pl = edb_by_key.get(key).copied();
        let pdb_pl = pdb_playlists_by_key.get(key);
        let (pdb_row_id, playlist_id, sort_order, name, folders) = match (edb_pl, pdb_pl) {
            // PDB ids, sort orders and folders are kept: the PDB order is
            // what players show, and eDB and PDB orders may legitimately
            // differ.
            (_, Some(pdb)) => (
                Some(pdb.playlist_id),
                pdb.playlist_id,
                pdb.sort_order,
                pdb.name.clone(),
                pdb.folders.clone(),
            ),
            (Some(edb), None) => {
                let id = u32::try_from(edb.playlist_id)
                    .ok()
                    .filter(|id| *id != 0 && !used_playlist_ids.contains(id))
                    .unwrap_or_else(|| {
                        while used_playlist_ids.contains(&next_playlist_id) {
                            next_playlist_id += 1;
                        }
                        next_playlist_id
                    });
                used_playlist_ids.insert(id);
                let sort = match u32::try_from(edb.sort_order) {
                    Ok(sort) if !has_existing_pdb_order => sort,
                    _ => {
                        next_sort_order += 1;
                        next_sort_order - 1
                    }
                };
                (None, id, sort, edb.path.name.clone(), edb_folders_of(edb))
            }
            (None, None) => continue,
        };

        let mut tracks = Vec::<ExportManifestTrack>::new();
        let mut seen = HashSet::<String>::new();
        // eDB members first, in the eDB's order.
        for edb_track in edb_pl.map(|pl| pl.tracks.as_slice()).unwrap_or_default() {
            let ident = track_identity_key(
                edb_track.identity_path(),
                &edb_track.title,
                &edb_track.artist,
                Some(&edb_track.id),
            );
            if !seen.insert(ident.clone()) {
                continue;
            }
            // The PDB track: by path identity, then analysis path, then
            // title and artist for a PDB track without a path.
            let pdb_track = pdb_track_by_key
                .get(&ident)
                .copied()
                .or_else(|| {
                    let analysis = edb_track_analysis_key(edb_track);
                    (!analysis.is_empty())
                        .then(|| pdb_track_by_analysis.get(&analysis).copied())
                        .flatten()
                })
                .or_else(|| {
                    pdb_track_by_meta
                        .get(&build_meta_key(&edb_track.title, &edb_track.artist))
                        .copied()
                });
            if let Some(pt) = pdb_track {
                seen.insert(pdb_identity(pt));
            }
            let edb_row = edb_track
                .id
                .parse::<i64>()
                .ok()
                .and_then(|id| edb_rows.get(&id));
            tracks.push(merged_manifest_track(
                usb_root,
                edb_row,
                pdb_track,
                parsed,
                tracks.len() + 1,
            ));
        }
        // Then members only the PDB has: device-side membership is never
        // pruned because one side is sparser.
        for entry in pdb_pl.map(|pl| pl.entries.as_slice()).unwrap_or_default() {
            let Some(pdb_track) = pdb_track_by_id.get(&entry.track_id).copied() else {
                continue;
            };
            // A repeated PDB identity under another legacy track id is the
            // same member.
            if !seen.insert(pdb_identity(pdb_track)) {
                continue;
            }
            let edb_row = edb_row_by_path
                .get(&canonicalize_track_path_identity(
                    &pdb_track.track_file_path,
                ))
                .copied();
            tracks.push(merged_manifest_track(
                usb_root,
                edb_row,
                Some(pdb_track),
                parsed,
                tracks.len() + 1,
            ));
        }

        merged.push(MergedPlaylist {
            name,
            pdb_row_id,
            playlist_id,
            sort_order,
            folders,
            tracks,
        });
    }
    merged
}

impl BackendService {
    /// Collect both databases, merge them, and write the result once
    /// through the export writers (see the module doc).
    pub(super) fn apply_strict_parity_upgrade(
        &self,
        usb_root: &std::path::Path,
        target_playlist_names: Option<&HashSet<String>>,
        session: &mut staging::UsbWriteSession<'_>,
    ) -> BackendResult<StrictParityUpgradeApplyResult> {
        run_strict_parity_upgrade(usb_root, target_playlist_names, Some(session), true)
    }
}

/// The strict parity upgrade, applied (`write`, in `session`) or as a
/// preview: the same collect, merge and writers, with the eDB transaction
/// rolled back and the PDB bytes dropped, so a preview's counts are what an
/// apply writes.
///
/// All or nothing: the eDB is written in one transaction, the PDB built in
/// memory, then the staged PDB written and the transaction committed. Any
/// failure before the commit rolls the eDB back and leaves the staged PDB
/// as it was; a failed commit puts the staged PDB's old bytes back. The
/// session writes both staged databases back to the USB once at its end.
///
/// A database missing from the USB (see [`missing_database`]) is created in
/// the session as `initialize_usb` creates it and every playlist of the
/// other merged into it; that is applied only, never previewed. A failed
/// rebuild drops the created database again.
pub(super) fn run_strict_parity_upgrade(
    usb_root: &std::path::Path,
    target_playlist_names: Option<&HashSet<String>>,
    session: Option<&mut staging::UsbWriteSession<'_>>,
    write: bool,
) -> BackendResult<StrictParityUpgradeApplyResult> {
    let Some(kind) = missing_database(usb_root) else {
        let edb_conn = match session {
            Some(session) => session.edb()?,
            None => None,
        };
        return collect_merge_write(usb_root, target_playlist_names, edb_conn, write);
    };
    let Some(session) = session.filter(|_| write) else {
        return Err(BackendError::Internal(
            "strict parity upgrade: a missing database is rebuilt only when applied".to_string(),
        ));
    };
    let created_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    session.create_database(kind, &created_date)?;
    let result = match session.edb() {
        Ok(edb_conn) => collect_merge_write(usb_root, None, edb_conn, true),
        Err(err) => Err(err),
    };
    match result {
        Ok(result) => Ok(StrictParityUpgradeApplyResult {
            rebuilt_database: Some(kind),
            ..result
        }),
        Err(err) => {
            session.drop_created_database(kind);
            Err(err)
        }
    }
}

fn collect_merge_write(
    usb_root: &std::path::Path,
    target_playlist_names: Option<&HashSet<String>>,
    edb_conn: Option<&mut rusqlite::Connection>,
    write: bool,
) -> BackendResult<StrictParityUpgradeApplyResult> {
    let mut result = StrictParityUpgradeApplyResult::default();
    let pdb_path = staging::required_pdb(usb_root)?;

    // ── Collect ───────────────────────────────────────────────────────
    let parsed = parse_pdb(&pdb_path)?;
    let Some(mut handle) = EdbRw::shared_or_open(edb_conn, usb_root) else {
        return Err(BackendError::Internal(
            "strict parity upgrade: unable to open the eDB".to_string(),
        ));
    };
    let conn = handle.conn();
    // The eDB as this repair has changed it so far: read from the connection, not the cache.
    let edb_playlists = read_edb(
        conn,
        usb_root,
        EdbRead {
            playlists: true,
            resolve_paths: true,
            ..EdbRead::default()
        },
    )
    .map(|library| library.playlists)
    .filter(|playlists| !playlists.is_empty());
    let edb_rows = read_edb_content_fields(conn)?;
    let edb_tree = crate::edb::read_edb_playlist_tree(conn)?;
    // Without a parent column the eDB has no folders to have flattened.
    let flattened = if load_table_columns(conn, "playlist")?
        .iter()
        .any(|c| c == "playlist_id_parent")
    {
        flattened_edb_copies(&parsed, edb_playlists.as_ref())
    } else {
        HashMap::new()
    };

    // ── Merge ─────────────────────────────────────────────────────────
    let merged = merge_strict_parity_playlists(
        usb_root,
        &parsed,
        edb_playlists.as_ref(),
        &edb_tree,
        &edb_rows,
        target_playlist_names,
        &flattened,
    );
    result.merged_playlists = merged.len();
    if merged.is_empty() && flattened.is_empty() {
        return Ok(result);
    }
    let inputs: Vec<(ExportPlaylistData, ExportManifest)> = merged
        .iter()
        .map(|mpl| {
            let playlist = ExportPlaylistData {
                id: format!("usb-pl-{}", mpl.playlist_id),
                name: mpl.name.clone(),
                tracks: Vec::new(),
            };
            let manifest = ExportManifest {
                generated_at: chrono::Utc::now().to_rfc3339(),
                playlist_id: playlist.id.clone(),
                playlist_name: mpl.name.clone(),
                usb_root: usb_root.to_string_lossy().to_string(),
                options: crate::models::ExportToUsbOptions {
                    include_artwork: true,
                    include_analysis: true,
                    prune_stale: false,
                    ..Default::default()
                },
                exported_tracks: mpl.tracks.len(),
                skipped_tracks: 0,
                warnings: Vec::new(),
                tracks: mpl.tracks.clone(),
            };
            (playlist, manifest)
        })
        .collect();
    let identities: Vec<crate::service::export_helpers::EdbPlaylistIdentity> = merged
        .iter()
        .map(|mpl| crate::service::export_helpers::EdbPlaylistIdentity {
            playlist_id: i64::from(mpl.playlist_id),
            sequence_no: i64::from(mpl.sort_order),
            folders: mpl.folders.clone(),
        })
        .collect();
    let edb_writes: Vec<EdbPlaylistWrite<'_>> = inputs
        .iter()
        .zip(&identities)
        .map(|((playlist, manifest), identity)| EdbPlaylistWrite {
            playlist,
            manifest,
            mirror_playlist_entries: true,
            identity: Some(identity),
            placement: PlaylistPlacement::AsGiven,
            folders: None,
        })
        .collect();
    let pdb_writes: Vec<PdbPlaylistWrite<'_>> = merged
        .iter()
        .zip(&inputs)
        .map(|(mpl, (playlist, manifest))| PdbPlaylistWrite {
            playlist,
            manifest,
            mirror_playlist_entries: true,
            row_id: mpl.pdb_row_id,
            playlist_id: Some(mpl.playlist_id),
            sort_order: Some(mpl.sort_order),
            placement: PlaylistPlacement::AsGiven,
            folders: Some(&mpl.folders),
        })
        .collect();

    // ── Write once, all or nothing ────────────────────────────────────
    let tx = conn.transaction()?;
    // The flattened copies go; the PDB's playlists in folders replace them.
    for name in flattened.values() {
        tx.execute(
            "DELETE FROM playlist_content WHERE playlist_id IN (SELECT playlist_id FROM playlist \
             WHERE attribute = 0 AND COALESCE(playlist_id_parent, 0) = 0 AND name = ?1)",
            rusqlite::params![name],
        )?;
        result.flattened_copies_removed += tx.execute(
            "DELETE FROM playlist WHERE attribute = 0 AND COALESCE(playlist_id_parent, 0) = 0 \
             AND name = ?1",
            rusqlite::params![name],
        )?;
    }
    // Every PDB folder in the eDB, empty ones too.
    let pdb_tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
    for folder in parsed.playlist_tree.iter().filter(|row| row.is_folder) {
        let mut path = pdb_tree
            .folders(folder.parent_id)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        path.push(folder.clone());
        crate::edb::ensure_edb_folder_path(&tx, &path, crate::edb::PlaylistPlacement::AsGiven)?;
    }
    let edb_results = write_edb_playlists_in_tx(&tx, &edb_writes)?;
    // The writers take the PDB's sort orders; the eDB numbers each folder
    // 0..n, as rekordbox does. The order stays the PDB's.
    crate::edb::renumber_edb_playlist_siblings(&tx)?;
    let t19_property = t19_property_from_conn(&tx);
    // Every eDB folder in the PDB, empty ones too.
    let edb_tree_index = crate::pdb_reader::PlaylistTree::new(&edb_tree);
    let edb_folder_paths = edb_tree
        .iter()
        .filter(|row| row.is_folder)
        .map(|folder| {
            let mut path = edb_tree_index
                .folders(folder.parent_id)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            path.push(folder.clone());
            path
        })
        .collect::<Vec<_>>();
    let build =
        build_pdb_for_playlists(usb_root, &pdb_writes, t19_property, true, &edb_folder_paths)?;

    result.edb_playlists_written = edb_results.len();
    result.edb_rows_inserted = edb_results.iter().map(|r| r.inserted_content).sum();
    result.edb_rows_updated = edb_results.iter().map(|r| r.updated_content).sum();
    result.pdb_tracks_inserted = build.inserted_tracks;
    result.pdb_tracks_changed = build.changed_tracks;
    result.pdb_playlists_inserted = build.inserted_playlists;
    result.duplicate_entries_removed = build.duplicate_entries_removed;

    if !write {
        // Dropping the transaction rolls the eDB back.
        return Ok(result);
    }
    std::fs::write(&pdb_path, &build.bytes)?;
    if let Err(err) = tx.commit() {
        if let Err(restore_err) = std::fs::write(&pdb_path, &build.before) {
            logging::debug(
                "usb-repair",
                format!(
                    "strict parity upgrade: eDB commit failed ({err}) and the staged PDB could not \
                     be put back ({restore_err}): {}",
                    pdb_path.display()
                ),
            );
        }
        return Err(err.into());
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(result)
}
