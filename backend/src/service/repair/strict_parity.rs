//! Strict parity upgrade: collect both USB databases, merge them into the
//! export writers' input, and let the writers write both once.
//!
//! The repair reads and merges; it writes nothing itself. Every row it
//! changes goes through the export writers (`write_edb_playlists_in_tx`,
//! `build_pdb_for_playlists`), which write only what differs.

use super::*;
use crate::service::export_helpers::{
    EdbContentFields, canonicalize_track_path_identity, read_edb_content_fields,
};

#[derive(Debug, Default, Clone)]
pub(super) struct StrictParityUpgradeApplyResult {
    pub(super) merged_playlists: usize,
    pub(super) edb_playlists_written: usize,
    pub(super) duplicate_entries_removed: usize,
    pub(super) edb_rows_inserted: usize,
    pub(super) edb_rows_updated: usize,
    pub(super) pdb_tracks_inserted: usize,
    pub(super) pdb_tracks_changed: usize,
    pub(super) pdb_playlists_inserted: usize,
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
        || !detail.playlist_id_match
        || !detail.sort_order_match
}

pub(super) fn sorted_edb_playlist_names(
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
) -> Vec<String> {
    let Some(edb_playlists) = edb_playlists else {
        return Vec::new();
    };
    let mut names = edb_playlists
        .iter()
        .map(|(name, playlist)| {
            (
                name.clone(),
                playlist.sort_order,
                playlist.playlist_id,
                canonicalize_playlist_name(name),
            )
        })
        .collect::<Vec<_>>();
    names.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.0.cmp(&b.0))
    });
    names.into_iter().map(|(name, _, _, _)| name).collect()
}

pub(super) fn strict_repair_ordered_playlist_names(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
) -> Vec<String> {
    let sorted_edb_names = sorted_edb_playlist_names(edb_playlists);
    let edb_name_set = sorted_edb_names.iter().cloned().collect::<HashSet<_>>();
    let mut edb_names_by_canon = HashMap::<String, Vec<String>>::new();
    for name in &sorted_edb_names {
        edb_names_by_canon
            .entry(canonicalize_playlist_name(name))
            .or_default()
            .push(name.clone());
    }

    let mut pdb_leaves = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder)
        .collect::<Vec<_>>();
    pdb_leaves.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut out = Vec::<String>::new();
    let mut seen_canon = HashSet::<String>::new();
    let mut consumed_edb_names = HashSet::<String>::new();
    for leaf in pdb_leaves {
        let canon = canonicalize_playlist_name(&leaf.name);
        if !seen_canon.insert(canon.clone()) {
            continue;
        }
        let chosen = if edb_name_set.contains(&leaf.name) {
            leaf.name.clone()
        } else {
            edb_names_by_canon
                .get(&canon)
                .and_then(|names| {
                    names
                        .iter()
                        .find(|name| !consumed_edb_names.contains(*name))
                        .cloned()
                })
                .unwrap_or_else(|| leaf.name.clone())
        };
        if edb_name_set.contains(&chosen) {
            consumed_edb_names.insert(chosen.clone());
        }
        out.push(chosen);
    }

    for name in sorted_edb_names {
        let canon = canonicalize_playlist_name(&name);
        if seen_canon.insert(canon) {
            out.push(name);
        }
    }

    out
}

pub(super) fn pdb_page_size_from_bytes(bytes: &[u8]) -> Option<usize> {
    bytes
        .get(4..8)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|value| usize::try_from(value).ok())
}

pub(crate) fn restore_pdb_playlist_sort_orders(
    usb_root: &std::path::Path,
    desired_sort_by_id: &HashMap<u32, u32>,
) -> BackendResult<usize> {
    if desired_sort_by_id.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    if !pdb_path.is_file() {
        return Ok(0);
    }
    let mut bytes = std::fs::read(&pdb_path)?;
    let page_size = pdb_page_size_from_bytes(&bytes).ok_or_else(|| {
        BackendError::Validation(format!(
            "PDB playlist order restore failed: cannot read page size from {}",
            pdb_path.display()
        ))
    })?;
    let patches = desired_sort_by_id
        .iter()
        .map(
            |(id, sort_order)| crate::pdb_writer::PdbPlaylistTreeSortOrderPatch {
                id: *id,
                sort_order: *sort_order,
            },
        )
        .collect::<Vec<_>>();
    let patched = crate::pdb_writer::patch_playlist_tree_sort_orders_in_place(
        &mut bytes, &patches, page_size,
    )?;
    if patched > 0 {
        std::fs::write(&pdb_path, &bytes)?;
    }
    Ok(patched)
}

pub(crate) fn sync_edb_playlist_sort_orders_from_pdb(
    usb_root: &std::path::Path,
    edb_conn: Option<&mut rusqlite::Connection>,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<usize> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let Some(mut handle) = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
    else {
        warnings.push(logging::log(
            Level::Error,
            "usb-repair",
            "usb.repair.strict-parity.order-sync-edb-open-failed",
            "strict parity upgrade: unable to open eDB for playlist order sync",
        ));
        return Ok(0);
    };
    let conn = handle.conn();
    if !table_exists(conn, "playlist") {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut updated = 0usize;
    for leaf in parsed.playlist_tree.iter().filter(|row| !row.row_is_folder) {
        updated += tx.execute(
            "UPDATE playlist SET sequenceNo = ?1 WHERE playlist_id = ?2 AND attribute = 0",
            params![i64::from(leaf.sort_order), i64::from(leaf.id)],
        )?;
    }
    tx.commit()?;
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(updated)
}

/// One line for the repair outcome and the preview card.
pub(super) fn strict_parity_summary(result: &StrictParityUpgradeApplyResult) -> String {
    format!(
        "merged {} playlist(s); eDB: {} row(s) added, {} changed; PDB: {} track(s) added, {} changed, {} playlist(s) added, {} duplicate entry/entries removed",
        result.merged_playlists,
        result.edb_rows_inserted,
        result.edb_rows_updated,
        result.pdb_tracks_inserted,
        result.pdb_tracks_changed,
        result.pdb_playlists_inserted,
        result.duplicate_entries_removed,
    )
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
/// other. Nothing is derived, except the creation date of a track the eDB
/// doesn't have yet (the audio file's date on the USB, else its PDB date
/// added).
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
    let date_added = merged_text(
        e(|r| r.date_added.as_deref()),
        p_text(|t| t.date_added.as_deref()),
    );
    let date_created = match edb {
        Some(row) => row.date_created.clone(),
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

/// A playlist the repair merged: its PDB identity and its members in order
/// (the eDB's first, then members only the PDB has).
pub(super) struct MergedPlaylist {
    name: String,
    /// The PDB playlist row it is written into, when the PDB has it.
    pdb_row_id: Option<u32>,
    playlist_id: u32,
    sort_order: u32,
    tracks: Vec<ExportManifestTrack>,
}

/// Merge both databases' playlists (all of them, or those whose canonical
/// name is in `target_playlist_names`): membership is the union, PDB-only
/// members are kept, and PDB playlist ids and sort orders are preserved.
pub(super) fn merge_strict_parity_playlists(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
    edb_rows: &HashMap<i64, EdbContentFields>,
    target_playlist_names: Option<&HashSet<String>>,
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
        entries: Vec<crate::pdb_reader::PdbPlaylistEntryRow>,
    }
    let mut pdb_playlists_by_exact: HashMap<String, PdbPlaylistInfo> = HashMap::new();
    let mut pdb_canon_to_exact: HashMap<String, Vec<String>> = HashMap::new();
    for leaf in parsed.playlist_tree.iter().filter(|r| !r.row_is_folder) {
        let mut entries: Vec<_> = parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == leaf.id)
            .cloned()
            .collect();
        entries.sort_by_key(|e| e.entry_index);
        // Several rows of the same name: prefer better track coverage, then
        // the lower sort order.
        let candidate = PdbPlaylistInfo {
            playlist_id: leaf.id,
            sort_order: leaf.sort_order,
            entries,
        };
        let replace = pdb_playlists_by_exact
            .get(&leaf.name)
            .is_none_or(|existing| {
                candidate.entries.len() > existing.entries.len()
                    || (candidate.entries.len() == existing.entries.len()
                        && candidate.sort_order < existing.sort_order)
            });
        if replace {
            pdb_playlists_by_exact.insert(leaf.name.clone(), candidate);
        }
        pdb_canon_to_exact
            .entry(canonicalize_playlist_name(&leaf.name))
            .or_default()
            .push(leaf.name.clone());
    }

    let pdb_tree_ids: HashSet<u32> = parsed.playlist_tree.iter().map(|r| r.id).collect();
    let mut next_playlist_id = pdb_tree_ids.iter().max().copied().unwrap_or(0) + 1;
    let mut next_sort_order = parsed
        .playlist_tree
        .iter()
        .map(|r| r.sort_order)
        .max()
        .unwrap_or(0)
        + 1;
    let has_existing_pdb_order = parsed.playlist_tree.iter().any(|r| !r.row_is_folder);
    let mut used_playlist_ids = pdb_tree_ids.clone();
    let mut consumed_pdb_ids: HashSet<u32> = HashSet::new();
    let mut merged = Vec::<MergedPlaylist>::new();

    for playlist_name in &strict_repair_ordered_playlist_names(parsed, edb_playlists) {
        let canon = canonicalize_playlist_name(playlist_name);
        if target_playlist_names.is_some_and(|targets| !targets.contains(&canon)) {
            continue;
        }
        let edb_pl = edb_playlists.and_then(|pls| pls.get(playlist_name));
        // Exact name first, then the canonical name; each PDB playlist is
        // merged once.
        let pdb_pl = pdb_playlists_by_exact
            .get(playlist_name)
            .filter(|p| !consumed_pdb_ids.contains(&p.playlist_id))
            .or_else(|| {
                pdb_canon_to_exact.get(&canon).and_then(|names| {
                    names
                        .iter()
                        .filter_map(|n| pdb_playlists_by_exact.get(n))
                        .find(|p| !consumed_pdb_ids.contains(&p.playlist_id))
                })
            });
        let (pdb_row_id, playlist_id, sort_order) = match (edb_pl, pdb_pl) {
            // PDB ids and sort orders are kept: the PDB order is what players
            // show, and eDB and PDB orders may legitimately differ.
            (_, Some(pdb)) => {
                consumed_pdb_ids.insert(pdb.playlist_id);
                (Some(pdb.playlist_id), pdb.playlist_id, pdb.sort_order)
            }
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
                (None, id, sort)
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
            name: playlist_name.clone(),
            pdb_row_id,
            playlist_id,
            sort_order,
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
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<StrictParityUpgradeApplyResult> {
        run_strict_parity_upgrade(usb_root, target_playlist_names, edb_conn, warnings, true)
    }
}

/// The strict parity upgrade, applied (`write`) or as a preview: the same
/// collect, merge and writers, with the eDB transaction rolled back and the
/// PDB bytes dropped, so a preview's counts are what an apply writes.
///
/// All or nothing: the eDB is written in one transaction, the PDB built in
/// memory, then the staged PDB written and the transaction committed. Any
/// failure before the commit rolls the eDB back and leaves the staged PDB
/// as it was; a failed commit puts the staged PDB's old bytes back. The
/// repair run writes both staged databases back to the USB once at its end.
pub(super) fn run_strict_parity_upgrade(
    usb_root: &std::path::Path,
    target_playlist_names: Option<&HashSet<String>>,
    edb_conn: Option<&mut rusqlite::Connection>,
    warnings: &mut Vec<WarningEntry>,
    write: bool,
) -> BackendResult<StrictParityUpgradeApplyResult> {
    let mut result = StrictParityUpgradeApplyResult::default();
    let pdb_path = usb_staging::stage_pdb(usb_root)?;

    // ── Collect ───────────────────────────────────────────────────────
    let parsed = parse_pdb(&pdb_path)?;
    let Some(mut handle) = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
    else {
        return Err(BackendError::Internal(
            "strict parity upgrade: unable to open the eDB".to_string(),
        ));
    };
    let conn = handle.conn();
    let edb_playlists =
        try_read_playlists_with_metadata_from_edb_with_conn(conn, usb_root, warnings);
    let edb_rows = read_edb_content_fields(conn)?;

    // ── Merge ─────────────────────────────────────────────────────────
    let merged = merge_strict_parity_playlists(
        usb_root,
        &parsed,
        edb_playlists.as_ref(),
        &edb_rows,
        target_playlist_names,
    );
    result.merged_playlists = merged.len();
    if merged.is_empty() {
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
    let edb_writes: Vec<EdbPlaylistWrite<'_>> = merged
        .iter()
        .zip(&inputs)
        .map(|(mpl, (playlist, manifest))| EdbPlaylistWrite {
            playlist,
            manifest,
            mirror_playlist_entries: true,
            identity: Some((i64::from(mpl.playlist_id), i64::from(mpl.sort_order))),
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
            move_to_front: false,
        })
        .collect();

    // ── Write once, all or nothing ────────────────────────────────────
    let tx = conn.transaction()?;
    let edb_results = write_edb_playlists_in_tx(&tx, &edb_writes)?;
    let t19_property = t19_property_from_conn(&tx);
    let build = build_pdb_for_playlists(usb_root, &pdb_writes, t19_property, true)?;

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
        let _ = std::fs::write(&pdb_path, &build.before);
        return Err(err.into());
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(result)
}
