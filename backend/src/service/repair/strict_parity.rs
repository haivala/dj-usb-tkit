//! Strict parity upgrade: re-merges eDB and PDB into a full re-export.

use super::*;

#[derive(Debug, Default, Clone)]
pub(super) struct StrictParityUpgradeApplyResult {
    pub(super) merged_playlists: usize,
    pub(super) edb_playlists_written: usize,
    pub(super) failed_playlists: usize,
    pub(super) artwork_patch_incomplete: bool,
    pub(super) duplicate_entries_removed: usize,
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
        || !detail.playlist_id_match
        || !detail.sort_order_match
}

pub(super) fn strict_raw_coverage_issue_from_parity_checks(checks: &[DiagCheck]) -> Option<String> {
    let raw_check = checks
        .iter()
        .find(|check| check.label == "USB audio directory raw coverage parity (strict)")?;
    if matches!(raw_check.status, DiagStatus::Pass) {
        return None;
    }
    Some(format!(
        "strict raw-count parity drift: {}",
        raw_check.detail
    ))
}

/// Represents a merged track produced during collect-merge-write.
/// Holds PDB track row data + eDB metadata for eDB write-back.
#[derive(Debug, Clone)]
pub(super) struct MergedTrack {
    pdb_track_id: u32,
    pdb_row: PdbTrackRowData,
    /// Canonical media path for eDB (e.g. "/Contents/Artist/Album/file.mp3").
    edb_media_path: String,
    /// eDB fields for write-back.
    title: String,
    artist: String,
    album: Option<String>,
    key: Option<String>,
    track_number: Option<u32>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    artwork_path: Option<String>,
    analysis_path: String,
    /// PDB technical fields preserved for eDB write-back.
    sample_rate_hz: Option<u32>,
    bitrate_kbps: Option<u32>,
    bit_depth: Option<u32>,
    file_size_bytes: Option<u32>,
    file_type: Option<u32>,
    isrc: Option<String>,
    release_year: Option<u32>,
    release_date: Option<String>,
}

/// Represents a merged playlist with its identity and ordered track list.
#[derive(Debug, Clone)]
pub(super) struct MergedPlaylist {
    name: String,
    playlist_id: u32,
    sort_order: u32,
    tracks: Vec<MergedTrack>,
    /// True if eDB needs to be updated for this playlist.
    edb_needs_write: bool,
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

pub(super) fn build_manifest_for_merged_playlist(
    usb_root: &std::path::Path,
    playlist: &MergedPlaylist,
) -> (ExportPlaylistData, ExportManifest) {
    let playlist_data = ExportPlaylistData {
        id: format!("usb-pl-{}", playlist.playlist_id),
        name: playlist.name.clone(),
        tracks: Vec::new(),
    };
    let manifest_tracks: Vec<ExportManifestTrack> = playlist
        .tracks
        .iter()
        .enumerate()
        .map(|(idx, mt)| ExportManifestTrack {
            id: format!("merged-{}", mt.pdb_track_id),
            master_db_id: mt.pdb_row.master_db_id.map(i64::from),
            master_content_id: mt.pdb_row.master_content_id.map(i64::from),
            content_link: mt.pdb_row.content_link.map(i64::from),
            position: idx + 1,
            track_number: mt.track_number,
            title: mt.title.clone(),
            artist: mt.artist.clone(),
            album: mt.album.clone(),
            bpm: mt.bpm,
            key: mt.key.clone(),
            source_path: mt.edb_media_path.clone(),
            exported_path: mt.edb_media_path.clone(),
            file_modified_at: None,
            file_size_bytes: mt.file_size_bytes.map(i64::from),
            sample_rate_hz: mt.sample_rate_hz,
            bit_depth: mt.bit_depth.map(|v| v as u8),
            bitrate_kbps: mt.bitrate_kbps,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: mt.pdb_row.rating.map(u32::from),
            color_id: mt.pdb_row.color_id.map(u32::from),
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: mt.isrc.clone(),
            release_year: mt.release_year,
            release_date: mt.release_date.clone(),
            recorded_date: None,
            file_type: mt.file_type.map(i64::from),
            owns_exported_media: false,
            owns_artwork: false,
            owns_waveform: false,
            artwork_path: mt.artwork_path.clone(),
            waveform_path: (!mt.analysis_path.is_empty()).then(|| mt.analysis_path.clone()),
            duration_ms: mt.duration_ms,
            first_beat_ms: None,
            cues: Vec::new(),
        })
        .collect();
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: playlist_data.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: manifest_tracks.len(),
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: manifest_tracks,
    };
    (playlist_data, manifest)
}

/// Merged track metadata fields shared by [`build_merged_pdb_track_row`] call sites.
pub(super) struct MergedTrackFields<'a> {
    title: &'a str,
    artist: &'a str,
    album: &'a Option<String>,
    key: &'a Option<String>,
    track_number: Option<u32>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    media_path: &'a str,
    analysis_path: &'a str,
    /// The eDB's rating and colour for this track, when it has a row there.
    edb_rating_colour: Option<crate::edb::EdbRatingColour>,
}

/// Build a PDB track row from merged metadata.
/// When an existing PDB track is available, preserve non-core linkage fields.
pub(super) fn build_merged_pdb_track_row(
    track_id: u32,
    existing_pdb: Option<&crate::pdb_reader::PdbTrackRow>,
    fields: MergedTrackFields,
    parsed: &crate::pdb_reader::ParsedPdb,
) -> PdbTrackRowData {
    let MergedTrackFields {
        title,
        artist,
        album,
        key,
        track_number,
        bpm,
        duration_ms,
        media_path,
        analysis_path,
        edb_rating_colour,
    } = fields;
    // Find or create dictionary IDs for artist/album/key
    let artist_id = if !artist.is_empty() {
        let canon = canonicalize_playlist_name(artist);
        parsed
            .artists
            .iter()
            .find(|(_, v)| canonicalize_playlist_name(v) == canon)
            .map(|(id, _)| *id)
            .or_else(|| existing_pdb.map(|t| t.artist_id))
            .unwrap_or(0)
    } else {
        existing_pdb.map(|t| t.artist_id).unwrap_or(0)
    };
    let album_id = if let Some(alb) = album {
        let canon = canonicalize_playlist_name(alb);
        parsed
            .albums
            .iter()
            .find(|(_, v)| canonicalize_playlist_name(v) == canon)
            .map(|(id, _)| *id)
            .or_else(|| existing_pdb.map(|t| t.album_id))
            .unwrap_or(0)
    } else {
        existing_pdb.map(|t| t.album_id).unwrap_or(0)
    };
    let key_id = if let Some(k) = key {
        parsed
            .keys
            .iter()
            .find(|(_, v)| v.trim().eq_ignore_ascii_case(k.trim()))
            .map(|(id, _)| *id)
            .or_else(|| existing_pdb.map(|t| t.key_id))
            .unwrap_or(0)
    } else {
        existing_pdb.map(|t| t.key_id).unwrap_or(0)
    };

    PdbTrackRowData {
        header_flags_u32: None,
        id: track_id,
        title: title.to_string(),
        file_path: media_path.to_string(),
        anlz_path: analysis_path.to_string(),
        artist_id,
        album_id,
        key_id,
        genre_id: existing_pdb.map(|t| t.genre_id).unwrap_or(0),
        artwork_id: existing_pdb.map(|t| t.artwork_id).unwrap_or(0),
        track_number,
        bpm,
        duration_seconds: duration_ms.map(|ms| (ms / 1000) as u32),
        content_link: existing_pdb.and_then(|t| t.content_link),
        sample_rate_hz: existing_pdb.and_then(|t| t.sample_rate_hz),
        file_size_bytes: existing_pdb.and_then(|t| t.file_size_bytes),
        master_content_id: existing_pdb.and_then(|t| t.master_content_id),
        master_db_id: existing_pdb.and_then(|t| t.master_db_id),
        bitrate_kbps: existing_pdb.and_then(|t| t.bitrate_kbps),
        release_year: existing_pdb.and_then(|t| t.release_year),
        bit_depth: existing_pdb.and_then(|t| t.bit_depth),
        file_type: existing_pdb.and_then(|t| t.file_type),
        isrc: existing_pdb.and_then(|t| t.isrc.clone()),
        date_added: existing_pdb.and_then(|t| t.date_added.clone()),
        release_date: existing_pdb.and_then(|t| t.release_date.clone()),
        dj_comment: existing_pdb.and_then(|t| t.dj_comment.clone()),
        file_name: existing_pdb.and_then(|t| t.file_name.clone()),
        publish_track_info_on: existing_pdb.and_then(|t| {
            t.publish_track_info
                .as_deref()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
        }),
        autoload_hotcues_on: existing_pdb.and_then(|t| {
            t.autoload_hotcues
                .as_deref()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
        }),
        color_id: edb_rating_colour
            .map(|v| v.color_id)
            .or_else(|| existing_pdb.map(|t| t.color_id)),
        rating: edb_rating_colour
            .map(|v| v.rating)
            .or_else(|| existing_pdb.map(|t| t.rating)),
    }
}

pub(super) fn resolved_strict_artwork_path(
    usb_root: &std::path::Path,
    pdb_track: Option<&crate::pdb_reader::PdbTrackRow>,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_artwork_path: Option<&str>,
) -> Option<String> {
    edb_artwork_path
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            crate::service::export_helpers::to_usb_relative_path(usb_root, path)
                .unwrap_or_else(|| path.to_string())
        })
        .or_else(|| {
            pdb_track
                .and_then(|track| parsed.artworks.get(&track.artwork_id))
                .filter(|path| !path.trim().is_empty())
                .cloned()
        })
}

impl BackendService {
    /// Collect-merge-write: parse both DBs once, merge all playlists, write once.
    pub(super) fn apply_strict_parity_upgrade(
        &self,
        usb_root: &std::path::Path,
        target_playlist_names: Option<&HashSet<String>>,
        mut edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<StrictParityUpgradeApplyResult> {
        let mut result = StrictParityUpgradeApplyResult::default();
        let pdb_path = usb_staging::stage_pdb(usb_root)?;

        // ── Phase 1: Collect ──────────────────────────────────────────
        let parsed = parse_pdb(&pdb_path)?;
        let edb_playlists = match edb_conn.as_deref() {
            Some(conn) => {
                try_read_playlists_with_metadata_from_edb_with_conn(conn, usb_root, warnings)
            }
            None => try_read_playlists_with_metadata_from_edb(usb_root, warnings),
        };
        // Each track's rating and colour as the eDB has them; the merged PDB
        // rows take these over their own (see `build_merged_pdb_track_row`).
        let owned_edb_conn = if edb_conn.is_some() {
            None
        } else {
            open_edb_from_usb_root(usb_root, warnings)
        };
        let edb_rating_colour_by_path: HashMap<String, crate::edb::EdbRatingColour> = edb_conn
            .as_deref()
            .or(owned_edb_conn.as_ref())
            .map(crate::edb::read_rating_colour_by_path)
            .unwrap_or_default()
            .into_iter()
            .map(|(path, value)| (normalize_pdb_path_for_edb_lookup(&path), value))
            .collect();
        drop(owned_edb_conn);
        let edb_rating_colour = |media_path: &str| {
            edb_rating_colour_by_path
                .get(&normalize_pdb_path_for_edb_lookup(media_path))
                .copied()
        };

        // Build PDB track index: identity_key → PdbTrackRow
        let pdb_track_by_key: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
            .tracks
            .iter()
            .map(|t| {
                let artist = parsed
                    .artists
                    .get(&t.artist_id)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let key = track_identity_key(
                    &t.track_file_path,
                    &t.title,
                    artist,
                    Some(&t.id.to_string()),
                );
                (key, t)
            })
            .collect();

        let pdb_track_by_analysis: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
            .tracks
            .iter()
            .filter_map(|t| {
                let key = normalize_analysis_path_for_identity(&t.anlz_path);
                if key.is_empty() { None } else { Some((key, t)) }
            })
            .collect();

        // Build eDB track index: identity_key → UsbTrack
        let mut edb_track_by_key: HashMap<String, &crate::models::UsbTrack> = HashMap::new();
        if let Some(ref edb_pls) = edb_playlists {
            for pl in edb_pls.values() {
                for track in &pl.tracks {
                    let key = track_identity_key(
                        track.identity_path(),
                        &track.title,
                        &track.artist,
                        Some(&track.id),
                    );
                    edb_track_by_key.entry(key).or_insert(track);
                }
            }
        }

        // Secondary PDB index: meta_key → PdbTrackRow for tracks with empty path.
        // Used for cross-matching when eDB has a path: key but PDB has a meta: key.
        let pdb_track_by_meta: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
            .tracks
            .iter()
            .filter(|t| t.track_file_path.trim().is_empty())
            .map(|t| {
                let artist = parsed
                    .artists
                    .get(&t.artist_id)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let meta = build_meta_key(&t.title, artist);
                (meta, t)
            })
            .collect();

        // Build PDB playlist structure indexed by exact name + canonical fallback
        let pdb_track_by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
            parsed.tracks.iter().map(|t| (t.id, t)).collect();
        struct PdbPlaylistInfo {
            playlist_id: u32,
            sort_order: u32,
            entries: Vec<crate::pdb_reader::PdbPlaylistEntryRow>,
        }
        let mut pdb_playlists_by_exact: HashMap<String, PdbPlaylistInfo> = HashMap::new();
        let mut pdb_canon_to_exact: HashMap<String, Vec<String>> = HashMap::new();
        for leaf in parsed.playlist_tree.iter().filter(|r| !r.row_is_folder) {
            let canon = canonicalize_playlist_name(&leaf.name);
            let mut entries: Vec<_> = parsed
                .playlist_entries
                .iter()
                .filter(|e| e.playlist_id == leaf.id)
                .cloned()
                .collect();
            entries.sort_by_key(|e| e.entry_index);
            // Defensive deterministic selection if multiple rows resolve to the
            // same name during parse: prefer better track coverage; on tie,
            // prefer lower sort order.
            let candidate = PdbPlaylistInfo {
                playlist_id: leaf.id,
                sort_order: leaf.sort_order,
                entries,
            };
            match pdb_playlists_by_exact.get(&leaf.name) {
                None => {
                    pdb_playlists_by_exact.insert(leaf.name.clone(), candidate);
                }
                Some(existing) => {
                    let replace = candidate.entries.len() > existing.entries.len()
                        || (candidate.entries.len() == existing.entries.len()
                            && candidate.sort_order < existing.sort_order);
                    if replace {
                        pdb_playlists_by_exact.insert(leaf.name.clone(), candidate);
                    }
                }
            }
            pdb_canon_to_exact
                .entry(canon)
                .or_default()
                .push(leaf.name.clone());
        }

        // ── Phase 2: Merge playlists ──────────────────────────────────
        let all_playlist_names =
            strict_repair_ordered_playlist_names(&parsed, edb_playlists.as_ref());

        let mut next_track_id = parsed.tracks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let mut next_playlist_id = parsed.playlist_tree.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let mut next_sort_order = parsed
            .playlist_tree
            .iter()
            .map(|r| r.sort_order)
            .max()
            .unwrap_or(0)
            + 1;

        let mut merged_playlists: Vec<MergedPlaylist> = Vec::new();
        let has_existing_pdb_order = parsed.playlist_tree.iter().any(|r| !r.row_is_folder);
        // Track all assigned PDB track IDs to avoid collisions
        let mut assigned_track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
        // Track consumed PDB playlists to prevent double-matching on canonical collisions
        let mut consumed_pdb_ids: HashSet<u32> = HashSet::new();
        // Reserve folder IDs so merged leaf IDs never collide with folder rows.
        let folder_playlist_ids: HashSet<u32> = parsed
            .playlist_tree
            .iter()
            .filter(|r| r.row_is_folder)
            .map(|r| r.id)
            .collect();
        // Track assigned merged leaf playlist IDs.
        let mut assigned_playlist_ids: HashSet<u32> = HashSet::new();
        let allocate_unique_playlist_id = |candidate: u32,
                                           folder_playlist_ids: &HashSet<u32>,
                                           assigned_playlist_ids: &HashSet<u32>,
                                           next_playlist_id: &mut u32|
         -> u32 {
            if !folder_playlist_ids.contains(&candidate)
                && !assigned_playlist_ids.contains(&candidate)
            {
                return candidate;
            }
            while folder_playlist_ids.contains(next_playlist_id)
                || assigned_playlist_ids.contains(next_playlist_id)
            {
                *next_playlist_id += 1;
            }
            let id = *next_playlist_id;
            *next_playlist_id += 1;
            id
        };

        for playlist_name in &all_playlist_names {
            if let Some(targets) = target_playlist_names {
                let canon_name = canonicalize_playlist_name(playlist_name);
                if !targets.contains(&canon_name) {
                    continue;
                }
            }
            let canon = canonicalize_playlist_name(playlist_name);
            let edb_pl = edb_playlists
                .as_ref()
                .and_then(|pls| pls.get(playlist_name));
            // Two-tier PDB lookup: exact name first, canonical fallback (skip consumed)
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
            if let Some(pdb) = pdb_pl {
                consumed_pdb_ids.insert(pdb.playlist_id);
            }

            // Determine playlist identity, avoiding ID collisions
            let (pl_id, pl_sort) = match (edb_pl, pdb_pl) {
                (Some(_edb), Some(pdb)) => {
                    // Preserve existing PDB playlist IDs and sort order for matched
                    // playlists. PDB sort_order controls player display order and must
                    // not be overwritten with eDB values — eDB and PDB sort orders can
                    // legitimately differ without affecting player acceptance.
                    let pdb_id = pdb.playlist_id;
                    (
                        allocate_unique_playlist_id(
                            pdb_id,
                            &folder_playlist_ids,
                            &assigned_playlist_ids,
                            &mut next_playlist_id,
                        ),
                        pdb.sort_order,
                    )
                }
                (Some(edb), None) => {
                    let candidate = u32::try_from(edb.playlist_id).unwrap_or_else(|_| {
                        let id = next_playlist_id;
                        next_playlist_id += 1;
                        id
                    });
                    let sort = if has_existing_pdb_order {
                        let s = next_sort_order;
                        next_sort_order += 1;
                        s
                    } else {
                        u32::try_from(edb.sort_order).unwrap_or_else(|_| {
                            let s = next_sort_order;
                            next_sort_order += 1;
                            s
                        })
                    };
                    (
                        allocate_unique_playlist_id(
                            candidate,
                            &folder_playlist_ids,
                            &assigned_playlist_ids,
                            &mut next_playlist_id,
                        ),
                        sort,
                    )
                }
                (None, Some(pdb)) => {
                    let id = allocate_unique_playlist_id(
                        pdb.playlist_id,
                        &folder_playlist_ids,
                        &assigned_playlist_ids,
                        &mut next_playlist_id,
                    );
                    (id, pdb.sort_order)
                }
                (None, None) => continue,
            };
            assigned_playlist_ids.insert(pl_id);

            // Build merged track list
            let mut merged_tracks: Vec<MergedTrack> = Vec::new();
            let mut seen_identity_keys = HashSet::<String>::new();

            // eDB tracks first (preferred order)
            if let Some(edb) = edb_pl {
                for edb_track in &edb.tracks {
                    let ident = track_identity_key(
                        edb_track.identity_path(),
                        &edb_track.title,
                        &edb_track.artist,
                        Some(&edb_track.id),
                    );
                    if !seen_identity_keys.insert(ident.clone()) {
                        continue; // skip duplicates
                    }

                    // Find matching PDB track (identity path first, then analysis path,
                    // then metadata fallback).
                    let pdb_track = pdb_track_by_key
                        .get(&ident)
                        .copied()
                        .or_else(|| {
                            let analysis = edb_track_analysis_key(edb_track);
                            if analysis.is_empty() {
                                None
                            } else {
                                pdb_track_by_analysis.get(&analysis).copied()
                            }
                        })
                        .or_else(|| {
                            let meta = build_meta_key(&edb_track.title, &edb_track.artist);
                            pdb_track_by_meta.get(&meta).copied()
                        });
                    // If matched via meta fallback, mark the PDB track's identity key
                    // as seen so it won't be re-added in the PDB-only pass.
                    if let Some(pt) = pdb_track
                        && !pdb_track_by_key.contains_key(&ident)
                    {
                        let pt_artist = parsed
                            .artists
                            .get(&pt.artist_id)
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        let pt_ident = track_identity_key(
                            &pt.track_file_path,
                            &pt.title,
                            pt_artist,
                            Some(&pt.id.to_string()),
                        );
                        seen_identity_keys.insert(pt_ident);
                    }
                    let track_id = if let Some(pt) = pdb_track {
                        pt.id
                    } else {
                        // Assign new PDB track ID for eDB-only track
                        while assigned_track_ids.contains(&next_track_id) {
                            next_track_id += 1;
                        }
                        let id = next_track_id;
                        next_track_id += 1;
                        assigned_track_ids.insert(id);
                        id
                    };

                    let media_path = edb_track
                        .usb_media_path
                        .clone()
                        .unwrap_or_else(|| edb_track.file_path.clone());
                    let media_path = if !media_path.is_empty() {
                        media_path
                    } else {
                        pdb_track
                            .map(|t| normalize_pdb_path_for_edb_lookup(&t.track_file_path))
                            .unwrap_or_default()
                    };
                    let analysis_path = edb_track
                        .usb_analysis_path_raw
                        .clone()
                        .or_else(|| edb_track.usb_analysis_path.clone())
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            pdb_track
                                .map(|t| t.anlz_path.clone())
                                .filter(|s| !s.is_empty())
                        })
                        .unwrap_or_default();

                    // Merge metadata: prefer eDB non-empty, fall back to PDB
                    let title = if !edb_track.title.is_empty() {
                        edb_track.title.clone()
                    } else {
                        pdb_track.map(|t| t.title.clone()).unwrap_or_default()
                    };
                    let artist = if !edb_track.artist.is_empty() {
                        edb_track.artist.clone()
                    } else {
                        pdb_track
                            .and_then(|t| parsed.artists.get(&t.artist_id))
                            .cloned()
                            .unwrap_or_default()
                    };
                    let album = edb_track.album.clone().or_else(|| {
                        pdb_track
                            .and_then(|t| parsed.albums.get(&t.album_id))
                            .cloned()
                    });
                    let key = edb_track
                        .key
                        .clone()
                        .or_else(|| pdb_track.and_then(|t| parsed.keys.get(&t.key_id)).cloned());
                    let track_number = edb_track.track_number.or_else(|| {
                        pdb_track.and_then(|t| (t.track_number > 0).then_some(t.track_number))
                    });
                    let bpm = edb_track.bpm.or_else(|| {
                        pdb_track
                            .and_then(|t| (t.tempo_x100 > 0).then_some(t.tempo_x100 as f64 / 100.0))
                    });
                    let duration_ms = edb_track.duration_ms.or_else(|| {
                        pdb_track.and_then(|t| t.duration_seconds.map(|d| u64::from(d) * 1000))
                    });

                    // Build PDB track row from merged data
                    let pdb_row = build_merged_pdb_track_row(
                        track_id,
                        pdb_track,
                        MergedTrackFields {
                            title: &title,
                            artist: &artist,
                            album: &album,
                            key: &key,
                            track_number,
                            bpm,
                            duration_ms,
                            media_path: &media_path,
                            analysis_path: &analysis_path,
                            edb_rating_colour: edb_rating_colour(&media_path),
                        },
                        &parsed,
                    );

                    let artwork_path = resolved_strict_artwork_path(
                        usb_root,
                        pdb_track,
                        &parsed,
                        edb_track.artwork_path.as_deref(),
                    );

                    merged_tracks.push(MergedTrack {
                        pdb_track_id: track_id,
                        pdb_row,
                        edb_media_path: media_path,
                        title,
                        artist,
                        album,
                        key,
                        track_number,
                        bpm,
                        duration_ms,
                        artwork_path,
                        analysis_path,
                        sample_rate_hz: pdb_track.and_then(|t| t.sample_rate_hz),
                        bitrate_kbps: pdb_track.and_then(|t| t.bitrate_kbps),
                        bit_depth: pdb_track.and_then(|t| t.bit_depth.map(u32::from)),
                        file_size_bytes: pdb_track.and_then(|t| t.file_size_bytes),
                        file_type: pdb_track.and_then(|t| t.file_type.map(u32::from)),
                        isrc: pdb_track.and_then(|t| t.isrc.clone()),
                        release_year: pdb_track.and_then(|t| t.release_year.map(u32::from)),
                        release_date: pdb_track.and_then(|t| t.release_date.clone()),
                    });
                }
            }

            // Preserve existing playlist membership from PDB in strict repair.
            // Device-side membership must not be pruned automatically just because
            // one DB side is sparser; strict repair should repair linkage/metadata
            // without silently removing members.
            let preserve_pdb_membership = true;
            if preserve_pdb_membership {
                if let Some(pdb) = pdb_pl {
                    for entry in &pdb.entries {
                        let Some(pdb_track) = pdb_track_by_id.get(&entry.track_id) else {
                            continue;
                        };
                        let artist_name = parsed
                            .artists
                            .get(&pdb_track.artist_id)
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        let ident = track_identity_key(
                            &pdb_track.track_file_path,
                            &pdb_track.title,
                            artist_name,
                            Some(&pdb_track.id.to_string()),
                        );
                        if !seen_identity_keys.insert(ident) {
                            // Strict parity repair rebuilds a canonical playlist membership
                            // list, so repeated PDB identities must not be reintroduced
                            // just because they use a different legacy track id.
                            continue;
                        }

                        let album = parsed.albums.get(&pdb_track.album_id).cloned();
                        let key = parsed.keys.get(&pdb_track.key_id).cloned();
                        let media_path =
                            normalize_pdb_path_for_edb_lookup(&pdb_track.track_file_path);
                        let bpm_val = (pdb_track.tempo_x100 > 0)
                            .then_some(pdb_track.tempo_x100 as f64 / 100.0);
                        let track_number_val =
                            (pdb_track.track_number > 0).then_some(pdb_track.track_number);
                        let duration_ms_val =
                            pdb_track.duration_seconds.map(|d| u64::from(d) * 1000);

                        let pdb_row = build_merged_pdb_track_row(
                            pdb_track.id,
                            Some(pdb_track),
                            MergedTrackFields {
                                title: &pdb_track.title,
                                artist: artist_name,
                                album: &album,
                                key: &key,
                                track_number: track_number_val,
                                bpm: bpm_val,
                                duration_ms: duration_ms_val,
                                media_path: &pdb_track.track_file_path,
                                analysis_path: &pdb_track.anlz_path,
                                edb_rating_colour: edb_rating_colour(&pdb_track.track_file_path),
                            },
                            &parsed,
                        );

                        merged_tracks.push(MergedTrack {
                            pdb_track_id: pdb_track.id,
                            pdb_row,
                            edb_media_path: media_path,
                            title: pdb_track.title.clone(),
                            artist: artist_name.to_string(),
                            album,
                            key,
                            track_number: track_number_val,
                            bpm: bpm_val,
                            duration_ms: duration_ms_val,
                            artwork_path: parsed.artworks.get(&pdb_track.artwork_id).cloned(),
                            analysis_path: pdb_track.anlz_path.clone(),
                            sample_rate_hz: pdb_track.sample_rate_hz,
                            bitrate_kbps: pdb_track.bitrate_kbps,
                            bit_depth: pdb_track.bit_depth.map(u32::from),
                            file_size_bytes: pdb_track.file_size_bytes,
                            file_type: pdb_track.file_type.map(u32::from),
                            isrc: pdb_track.isrc.clone(),
                            release_year: pdb_track.release_year.map(u32::from),
                            release_date: pdb_track.release_date.clone(),
                        });
                    }
                }
            } else if pdb_playlists_by_exact.is_empty() && all_playlist_names.len() == 1 {
                // Fallback for synthetic/no-playlist-mapping snapshots: keep
                // unmatched PDB tracks so strict merge remains additive.
                for pdb_track in &parsed.tracks {
                    let artist_name = parsed
                        .artists
                        .get(&pdb_track.artist_id)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let ident = track_identity_key(
                        &pdb_track.track_file_path,
                        &pdb_track.title,
                        artist_name,
                        Some(&pdb_track.id.to_string()),
                    );
                    if !seen_identity_keys.insert(ident) {
                        continue;
                    }
                    let album = parsed.albums.get(&pdb_track.album_id).cloned();
                    let key = parsed.keys.get(&pdb_track.key_id).cloned();
                    let media_path = normalize_pdb_path_for_edb_lookup(&pdb_track.track_file_path);
                    let bpm_val =
                        (pdb_track.tempo_x100 > 0).then_some(pdb_track.tempo_x100 as f64 / 100.0);
                    let track_number_val =
                        (pdb_track.track_number > 0).then_some(pdb_track.track_number);
                    let duration_ms_val = pdb_track.duration_seconds.map(|d| u64::from(d) * 1000);
                    let pdb_row = build_merged_pdb_track_row(
                        pdb_track.id,
                        Some(pdb_track),
                        MergedTrackFields {
                            title: &pdb_track.title,
                            artist: artist_name,
                            album: &album,
                            key: &key,
                            track_number: track_number_val,
                            bpm: bpm_val,
                            duration_ms: duration_ms_val,
                            media_path: &pdb_track.track_file_path,
                            analysis_path: &pdb_track.anlz_path,
                            edb_rating_colour: edb_rating_colour(&pdb_track.track_file_path),
                        },
                        &parsed,
                    );
                    merged_tracks.push(MergedTrack {
                        pdb_track_id: pdb_track.id,
                        pdb_row,
                        edb_media_path: media_path,
                        title: pdb_track.title.clone(),
                        artist: artist_name.to_string(),
                        album,
                        key,
                        track_number: track_number_val,
                        bpm: bpm_val,
                        duration_ms: duration_ms_val,
                        artwork_path: parsed.artworks.get(&pdb_track.artwork_id).cloned(),
                        analysis_path: pdb_track.anlz_path.clone(),
                        sample_rate_hz: pdb_track.sample_rate_hz,
                        bitrate_kbps: pdb_track.bitrate_kbps,
                        bit_depth: pdb_track.bit_depth.map(u32::from),
                        file_size_bytes: pdb_track.file_size_bytes,
                        file_type: pdb_track.file_type.map(u32::from),
                        isrc: pdb_track.isrc.clone(),
                        release_year: pdb_track.release_year.map(u32::from),
                        release_date: pdb_track.release_date.clone(),
                    });
                }
            }

            // Always write eDB: metadata may differ even when track counts match.
            // write_edb_playlist is idempotent, so this is safe.
            let edb_needs_write = true;

            merged_playlists.push(MergedPlaylist {
                name: playlist_name.clone(),
                playlist_id: pl_id,
                sort_order: pl_sort,
                tracks: merged_tracks,
                edb_needs_write,
            });
        }

        result.merged_playlists = merged_playlists.len();

        // ── Phase 3: Rewrite PDB using the current export writer ──────
        let mut desired_pdb_sort_by_id: HashMap<u32, u32> = parsed
            .playlist_tree
            .iter()
            .map(|row| (row.id, row.sort_order))
            .collect();
        for mpl in &merged_playlists {
            desired_pdb_sort_by_id.insert(mpl.playlist_id, mpl.sort_order);
        }
        for mpl in &merged_playlists {
            let (playlist_data, manifest) = build_manifest_for_merged_playlist(usb_root, mpl);
            if let Err(err) = write_pdb(
                usb_root,
                &playlist_data,
                &manifest,
                true,
                Some(mpl.playlist_id),
                Some(mpl.sort_order),
            ) {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.strict-parity.pdb-rewrite-failed",
                    format!(
                        "strict parity upgrade: PDB rewrite failed for '{}': {err}",
                        mpl.name
                    ),
                ));
                result.failed_playlists += 1;
            }
        }
        if result.merged_playlists > 0
            && let Err(err) = restore_pdb_playlist_sort_orders(usb_root, &desired_pdb_sort_by_id)
        {
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.strict-parity.pdb-order-restore-failed",
                format!("strict parity upgrade: PDB playlist order restore failed: {err}"),
            ));
            result.failed_playlists += 1;
        }

        // ── Phase 3b: Remove stale duplicate playlist_entries rows ──────
        // A playlist's tail can be left behind on a shared tt=8 page from an
        // earlier point in the USB's history when the playlist later grew and
        // its continuation relocated to a fresh page (see docs/PDB.md). The
        // rewrite above only manages rows within each merged playlist's
        // current live boundary, so pre-existing stale copies elsewhere on
        // shared pages survive it. Sweep the whole PDB for any
        // (playlist_id, track_id) pair with more than one active row and
        // tombstone all but the earliest.
        if result.merged_playlists > 0 {
            let pdb_path = usb_staging::stage_pdb(usb_root)?;
            match std::fs::read(&pdb_path) {
                Ok(mut pdb_bytes) => {
                    let removed = crate::pdb_writer::remove_duplicate_playlist_entries_inplace(
                        &mut pdb_bytes,
                    );
                    if removed > 0 {
                        if let Err(err) = std::fs::write(&pdb_path, &pdb_bytes) {
                            warnings.push(logging::log(
                                Level::Error,
                                "usb-repair",
                                "usb.repair.strict-parity.dedup-write-failed",
                                format!(
                                    "strict parity upgrade: failed to write deduplicated PDB: {err}"
                                ),
                            ));
                        } else {
                            result.duplicate_entries_removed = removed;
                        }
                    }
                }
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Error,
                        "usb-repair",
                        "usb.repair.strict-parity.dedup-read-failed",
                        format!(
                            "strict parity upgrade: unable to read PDB for duplicate cleanup: {err}"
                        ),
                    ));
                }
            }
        }

        // Re-read written PDB identities so eDB uses the exact playlist id/sort
        // that ended up in the device-facing PDB after rewrite.
        let pdb_identity_by_name: HashMap<String, (u32, u32)> = {
            let pdb_path = usb_staging::stage_pdb(usb_root)?;
            if pdb_path.is_file() {
                if let Ok(parsed_after_write) = parse_pdb(&pdb_path) {
                    let mut map = HashMap::<String, (u32, u32, usize)>::new();
                    for leaf in parsed_after_write
                        .playlist_tree
                        .iter()
                        .filter(|r| !r.row_is_folder)
                    {
                        let entry_count = parsed_after_write
                            .playlist_entries
                            .iter()
                            .filter(|e| e.playlist_id == leaf.id)
                            .count();
                        match map.get(&leaf.name) {
                            None => {
                                map.insert(
                                    leaf.name.clone(),
                                    (leaf.id, leaf.sort_order, entry_count),
                                );
                            }
                            Some((_, existing_sort, existing_count)) => {
                                let replace = entry_count > *existing_count
                                    || (entry_count == *existing_count
                                        && leaf.sort_order < *existing_sort);
                                if replace {
                                    map.insert(
                                        leaf.name.clone(),
                                        (leaf.id, leaf.sort_order, entry_count),
                                    );
                                }
                            }
                        }
                    }
                    map.into_iter()
                        .map(|(name, (id, sort, _))| (name, (id, sort)))
                        .collect()
                } else {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.strict-parity.identity-sync-parse-failed",
                        "strict parity upgrade: unable to parse rewritten PDB for playlist identity sync; falling back to merged ids",
                    ));
                    HashMap::new()
                }
            } else {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.strict-parity.identity-sync-pdb-missing",
                    "strict parity upgrade: rewritten PDB missing during playlist identity sync; falling back to merged ids",
                ));
                HashMap::new()
            }
        };

        // ── Phase 4: Write eDB ────────────────────────────────────────
        // Open (or reuse the caller-supplied) eDB connection once for the
        // whole write phase below, instead of once per playlist -- each
        // open re-runs SQLCipher key negotiation.
        let mut handle =
            EdbConnHandle::acquire(edb_conn.take(), || open_edb_rw(usb_root, warnings));
        for mpl in &merged_playlists {
            let (playlist_data, manifest) = build_manifest_for_merged_playlist(usb_root, mpl);
            if !mpl.edb_needs_write {
                continue;
            }
            let Some(conn) = handle.as_mut().map(EdbConnHandle::conn) else {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.strict-parity.edb-open-failed",
                    format!(
                        "strict parity upgrade: unable to open eDB for playlist '{}'",
                        mpl.name
                    ),
                ));
                result.failed_playlists += 1;
                continue;
            };
            // Use the re-read PDB id (handles collision remapping). The eDB
            // writer may move this playlist to the front, so all eDB sequenceNos
            // are synced from the final PDB sort order after the write loop.
            let target_playlist_id = pdb_identity_by_name
                .get(&mpl.name)
                .map(|(id, _)| *id)
                .unwrap_or(mpl.playlist_id);
            let tx = conn.transaction()?;
            replace_export_playlist_row_with_identity(
                &tx,
                &playlist_data,
                i64::from(target_playlist_id),
                i64::from(mpl.sort_order),
            )?;
            tx.commit()?;
            match write_edb_playlist_with_conn(conn, &playlist_data, &manifest, true) {
                Ok(_) => result.edb_playlists_written += 1,
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Error,
                        "usb-repair",
                        "usb.repair.strict-parity.edb-write-failed",
                        format!(
                            "strict parity upgrade: eDB write failed for '{}': {err}",
                            mpl.name
                        ),
                    ));
                    result.failed_playlists += 1;
                }
            }
        }
        if result.merged_playlists > 0
            && let Err(err) = sync_edb_playlist_sort_orders_from_pdb(
                usb_root,
                handle.as_mut().map(EdbConnHandle::conn),
                warnings,
            )
        {
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.strict-parity.edb-order-sync-failed",
                format!("strict parity upgrade: eDB playlist order sync failed: {err}"),
            ));
            result.failed_playlists += 1;
        }

        Ok(result)
    }
}
