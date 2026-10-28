//! The eDB↔PDB parity comparison.

use super::*;

/// Report-wide counts summed over the per-playlist parity details.
pub(super) struct ParityTotals {
    pub(super) fail_playlists: usize,
    pub(super) total_only_in_pdb: usize,
    pub(super) total_only_in_edb: usize,
    pub(super) entry_order_mismatches: usize,
    pub(super) sort_order_mismatches: usize,
    pub(super) order_mismatches: usize,
    pub(super) total_duplicate_entries: usize,
    pub(super) total_pdb_missing_core_metadata: usize,
    pub(super) total_edb_missing_core_metadata: usize,
    pub(super) total_artwork_mismatches: usize,
    pub(super) total_path_mismatches: usize,
    pub(super) total_dictionary_id_issues: usize,
    /// Playlist tracks counted once each, however many playlists hold them.
    pub(super) rating_colour_mismatches: usize,
    pub(super) playlist_id_mismatches: usize,
}

impl ParityTotals {
    pub(super) fn from_details(details: &[UsbParityPlaylistDetail]) -> Self {
        let fail_playlists = details
            .iter()
            .filter(|d| matches!(d.status, DiagStatus::Fail))
            .count();
        let total_only_in_pdb: usize = details.iter().map(|d| d.only_in_pdb).sum();
        let total_only_in_edb: usize = details.iter().map(|d| d.only_in_edb).sum();
        let entry_order_mismatches = details.iter().filter(|d| d.order_mismatch).count();
        let sort_order_mismatches = details
            .iter()
            .filter(|d| d.pdb_sort_order.is_some() && d.edb_sort_order.is_some())
            .filter(|d| !d.sort_order_match)
            .count();
        let order_mismatches = entry_order_mismatches + sort_order_mismatches;
        let total_duplicate_entries: usize = details.iter().map(|d| d.pdb_duplicate_entries).sum();
        let total_pdb_missing_core_metadata: usize =
            details.iter().map(|d| d.pdb_missing_core_metadata).sum();
        let total_edb_missing_core_metadata: usize =
            details.iter().map(|d| d.edb_missing_core_metadata).sum();
        let total_artwork_mismatches: usize =
            details.iter().map(|d| d.artwork_mismatch_tracks).sum();
        let total_path_mismatches: usize = details.iter().map(|d| d.path_mismatch_tracks).sum();
        let total_dictionary_id_issues: usize =
            details.iter().map(|d| d.dictionary_id_issue_tracks).sum();
        let rating_colour_mismatches: usize = details
            .iter()
            .map(|d| d.rating_colour_mismatch_tracks)
            .sum();
        let playlist_id_mismatches = details.iter().filter(|d| !d.playlist_id_match).count();
        Self {
            fail_playlists,
            total_only_in_pdb,
            total_only_in_edb,
            entry_order_mismatches,
            sort_order_mismatches,
            order_mismatches,
            total_duplicate_entries,
            total_pdb_missing_core_metadata,
            total_edb_missing_core_metadata,
            total_artwork_mismatches,
            total_path_mismatches,
            total_dictionary_id_issues,
            rating_colour_mismatches,
            playlist_id_mismatches,
        }
    }
}

pub(crate) fn build_usb_parity_comparison(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: &HashMap<String, ExportDbPlaylist>,
    rating_colour: &RatingColourParity,
    field_notes: &FieldNoteParity,
    strict_raw_coverage: Option<StrictRawCoverageParity>,
) -> (
    Vec<DiagCheck>,
    Vec<DiagSummaryRow>,
    Vec<UsbParityPlaylistDetail>,
    DiagStatus,
) {
    #[derive(Clone)]
    struct PdbPlaylistTrackDetail {
        title: String,
        artist: String,
        album: Option<String>,
        key_name: Option<String>,
        track_number: Option<u32>,
        tempo_x100: Option<u32>,
        duration_seconds: Option<u32>,
        analysis_path: String,
        media_path: String,
        artwork_path: Option<String>,
        artist_id: u32,
        album_id: u32,
        key_id: u32,
        artwork_id: u32,
    }

    #[derive(Clone)]
    struct PdbPlaylistInfo {
        playlist_id: u32,
        sort_order: u32,
        track_keys: Vec<String>,
        track_details_by_key: HashMap<String, PdbPlaylistTrackDetail>,
        /// Maps identity_key → meta_key for secondary cross-DB matching.
        meta_by_track_key: HashMap<String, String>,
        /// Maps identity_key → normalized analysis-path key.
        analysis_by_track_key: HashMap<String, String>,
        duplicate_entries: usize,
    }

    #[derive(Clone)]
    struct EdbPlaylistInfo {
        playlist_id: Option<u32>,
        sort_order: Option<u32>,
        track_keys: Vec<String>,
        track_details_by_key: HashMap<String, UsbTrack>,
        /// Maps identity_key → meta_key for secondary cross-DB matching.
        meta_by_track_key: HashMap<String, String>,
        /// Maps identity_key → normalized analysis-path key.
        analysis_by_track_key: HashMap<String, String>,
    }

    fn normalized_optional_str(value: Option<&str>) -> String {
        value
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or_default()
            .to_string()
    }

    fn normalized_required_str(value: &str) -> String {
        value.trim().to_string()
    }

    fn normalized_artist_for_gate(value: &str) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown artist") {
            "Unknown artist".to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn sample_track_label(shared_key: &str, pdb_detail: &PdbPlaylistTrackDetail) -> String {
        let title = pdb_detail.title.trim();
        if !title.is_empty() {
            title.to_string()
        } else {
            shared_key.to_string()
        }
    }

    let track_key_by_id = parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .map(String::as_str)
                .unwrap_or("");
            let key = track_identity_key(
                &t.track_file_path,
                &t.title,
                artist,
                Some(&t.id.to_string()),
            );
            (t.id, key)
        })
        .collect::<HashMap<_, _>>();

    let pdb_track_detail_by_id = parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .cloned()
                .unwrap_or_default();
            let detail = PdbPlaylistTrackDetail {
                title: t.title.clone(),
                artist,
                album: parsed.albums.get(&t.album_id).cloned(),
                key_name: parsed.keys.get(&t.key_id).cloned(),
                track_number: (t.track_number > 0).then_some(t.track_number),
                tempo_x100: (t.tempo_x100 > 0).then_some(t.tempo_x100),
                duration_seconds: t.duration_seconds,
                analysis_path: t.anlz_path.clone(),
                media_path: t.track_file_path.clone(),
                artwork_path: parsed.artworks.get(&t.artwork_id).cloned(),
                artist_id: t.artist_id,
                album_id: t.album_id,
                key_id: t.key_id,
                artwork_id: t.artwork_id,
            };
            (t.id, detail)
        })
        .collect::<HashMap<_, _>>();

    let mut pdb_info_by_playlist = HashMap::<String, PdbPlaylistInfo>::new();
    let mut leaves = parsed
        .playlist_tree
        .iter()
        .filter(|n| !n.row_is_folder)
        .cloned()
        .collect::<Vec<_>>();
    leaves.sort_by_key(|n| n.sort_order);
    for leaf in &leaves {
        let mut rows = parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == leaf.id)
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by_key(|e| e.entry_index);

        let mut track_keys = Vec::<String>::new();
        let mut track_details_by_key = HashMap::<String, PdbPlaylistTrackDetail>::new();
        let mut meta_by_track_key = HashMap::<String, String>::new();
        let mut analysis_by_track_key = HashMap::<String, String>::new();
        for row in &rows {
            let key = track_key_by_id
                .get(&row.track_id)
                .cloned()
                .unwrap_or_else(|| format!("id:{}", row.track_id));
            track_keys.push(key.clone());
            if let Some(detail) = pdb_track_detail_by_id.get(&row.track_id).cloned() {
                let meta = build_meta_key(&detail.title, &detail.artist);
                if meta != "meta:|" {
                    meta_by_track_key.insert(key.clone(), meta);
                }
                let analysis = normalize_analysis_path_for_identity(&detail.analysis_path);
                if !analysis.is_empty() {
                    analysis_by_track_key.insert(key.clone(), analysis);
                }
                track_details_by_key.entry(key).or_insert(detail);
            }
        }

        let duplicate_entries = track_keys
            .len()
            .saturating_sub(unique_preserve_order(&track_keys).len());
        pdb_info_by_playlist.insert(
            canonicalize_playlist_name(&leaf.name),
            PdbPlaylistInfo {
                playlist_id: leaf.id,
                sort_order: leaf.sort_order,
                track_keys,
                track_details_by_key,
                meta_by_track_key,
                analysis_by_track_key,
                duplicate_entries,
            },
        );
    }

    let mut edb_info_by_playlist = HashMap::<String, EdbPlaylistInfo>::new();
    for (name, playlist) in edb_playlists {
        let mut track_keys = Vec::<String>::new();
        let mut track_details_by_key = HashMap::<String, UsbTrack>::new();
        let mut meta_by_track_key = HashMap::<String, String>::new();
        let mut analysis_by_track_key = HashMap::<String, String>::new();
        for track in &playlist.tracks {
            let key = track_identity_key(
                track.identity_path(),
                &track.title,
                &track.artist,
                Some(&track.id),
            );
            let meta = build_meta_key(&track.title, &track.artist);
            if meta != "meta:|" {
                meta_by_track_key.insert(key.clone(), meta);
            }
            let analysis = normalize_analysis_path_for_identity(
                track
                    .usb_analysis_path_raw
                    .as_deref()
                    .or(track.usb_analysis_path.as_deref())
                    .unwrap_or_default(),
            );
            if !analysis.is_empty() {
                analysis_by_track_key.insert(key.clone(), analysis);
            }
            track_keys.push(key.clone());
            track_details_by_key
                .entry(key)
                .or_insert_with(|| track.clone());
        }

        edb_info_by_playlist.insert(
            canonicalize_playlist_name(name),
            EdbPlaylistInfo {
                playlist_id: u32::try_from(playlist.playlist_id).ok(),
                sort_order: u32::try_from(playlist.sort_order).ok(),
                track_keys,
                track_details_by_key,
                meta_by_track_key,
                analysis_by_track_key,
            },
        );
    }

    let mut all_keys = HashSet::<String>::new();
    for key in pdb_info_by_playlist.keys() {
        all_keys.insert(key.clone());
    }
    for key in edb_info_by_playlist.keys() {
        all_keys.insert(key.clone());
    }

    let mut details = Vec::<UsbParityPlaylistDetail>::new();
    for key in all_keys {
        let pdb_name = leaves
            .iter()
            .find(|n| canonicalize_playlist_name(&n.name) == key)
            .map(|n| n.name.clone());
        let edb_name = edb_playlists
            .keys()
            .find(|n| canonicalize_playlist_name(n) == key)
            .cloned();

        let display = pdb_name
            .clone()
            .or(edb_name.clone())
            .unwrap_or_else(|| key.clone());

        let pdb_info = pdb_info_by_playlist.get(&key).cloned();
        let edb_info = edb_info_by_playlist.get(&key).cloned();

        let pdb_list = pdb_info
            .as_ref()
            .map(|info| info.track_keys.clone())
            .unwrap_or_default();
        let edb_list = edb_info
            .as_ref()
            .map(|info| info.track_keys.clone())
            .unwrap_or_default();

        let pdb_unique_list = unique_preserve_order(&pdb_list);
        let edb_unique_list = unique_preserve_order(&edb_list);

        let pdb_set = pdb_unique_list.iter().cloned().collect::<HashSet<_>>();
        let edb_set = edb_unique_list.iter().cloned().collect::<HashSet<_>>();

        // Secondary meta-key cross-matching: resolve path: vs meta: mismatches.
        // When PDB has no path (meta: key) and eDB has a path (path: key) for the
        // same title+artist, count them as matched.
        let pdb_meta_map = pdb_info.as_ref().map(|i| &i.meta_by_track_key);
        let edb_meta_map = edb_info.as_ref().map(|i| &i.meta_by_track_key);
        let pdb_analysis_map = pdb_info.as_ref().map(|i| &i.analysis_by_track_key);
        let edb_analysis_map = edb_info.as_ref().map(|i| &i.analysis_by_track_key);

        // Secondary meta-key cross-matching: when one side has a path: key and the
        // other has a meta: key (empty path), match them if title+artist are the same.
        let mut meta_matched_pdb = HashSet::<String>::new();
        let mut meta_matched_edb = HashSet::<String>::new();
        let mut consumed_meta = HashSet::<String>::new();
        let mut meta_match_pairs = Vec::<(String, String)>::new();
        let mut analysis_matched_pdb = HashSet::<String>::new();
        let mut analysis_matched_edb = HashSet::<String>::new();
        let mut consumed_analysis = HashSet::<String>::new();
        let mut analysis_match_pairs = Vec::<(String, String)>::new();

        // Primary secondary cross-match: analysis path.
        for pdb_key in pdb_set.difference(&edb_set) {
            let Some(pdb_analysis) = pdb_analysis_map.and_then(|m| m.get(pdb_key)).cloned() else {
                continue;
            };
            if consumed_analysis.contains(&pdb_analysis) {
                continue;
            }
            let edb_match = edb_set.difference(&pdb_set).find(|edb_k| {
                if analysis_matched_edb.contains(*edb_k) {
                    return false;
                }
                let edb_analysis = edb_analysis_map.and_then(|m| m.get(*edb_k)).cloned();
                edb_analysis.as_ref() == Some(&pdb_analysis)
            });
            if let Some(edb_key) = edb_match {
                analysis_matched_pdb.insert(pdb_key.clone());
                analysis_matched_edb.insert(edb_key.clone());
                analysis_match_pairs.push((pdb_key.clone(), edb_key.clone()));
                consumed_analysis.insert(pdb_analysis);
            }
        }

        // For each unmatched PDB key, find its meta key and look for an eDB track
        // with the same meta that is also unmatched.
        for pdb_key in pdb_set.difference(&edb_set) {
            if analysis_matched_pdb.contains(pdb_key) {
                continue;
            }
            let pdb_meta = if pdb_key.starts_with("meta:") {
                Some(pdb_key.clone())
            } else {
                pdb_meta_map.and_then(|m| m.get(pdb_key)).cloned()
            };
            if let Some(meta) = pdb_meta {
                if consumed_meta.contains(&meta) {
                    continue;
                }
                // Find an unmatched eDB key with the same meta
                let edb_match = edb_set.difference(&pdb_set).find(|edb_k| {
                    if meta_matched_edb.contains(*edb_k) || analysis_matched_edb.contains(*edb_k) {
                        return false;
                    }
                    let edb_meta = if edb_k.starts_with("meta:") {
                        Some((*edb_k).clone())
                    } else {
                        edb_meta_map.and_then(|m| m.get(*edb_k)).cloned()
                    };
                    edb_meta.as_ref() == Some(&meta)
                });
                if let Some(edb_key) = edb_match {
                    meta_matched_pdb.insert(pdb_key.clone());
                    meta_matched_edb.insert(edb_key.clone());
                    meta_match_pairs.push((pdb_key.clone(), edb_key.clone()));
                    consumed_meta.insert(meta);
                }
            }
        }

        let analysis_extra = analysis_matched_pdb.len();
        let meta_extra = meta_matched_pdb.len();
        let meta_matched: HashSet<String> = meta_matched_pdb
            .iter()
            .chain(meta_matched_edb.iter())
            .cloned()
            .collect();
        let analysis_matched: HashSet<String> = analysis_matched_pdb
            .iter()
            .chain(analysis_matched_edb.iter())
            .cloned()
            .collect();
        let matched_tracks = pdb_set.intersection(&edb_set).count() + analysis_extra + meta_extra;
        let only_in_pdb = pdb_set
            .difference(&edb_set)
            .count()
            .saturating_sub(analysis_extra)
            .saturating_sub(meta_extra);
        let only_in_edb = edb_set
            .difference(&pdb_set)
            .count()
            .saturating_sub(analysis_extra)
            .saturating_sub(meta_extra);
        // Build a canonical-key mapping for meta-matched pairs so order
        // comparison uses the same string on both sides.
        let mut pdb_key_canonical = HashMap::<&String, String>::new();
        let mut edb_key_canonical = HashMap::<&String, String>::new();
        for pdb_key in &analysis_matched_pdb {
            let analysis = pdb_analysis_map
                .and_then(|m| m.get(pdb_key))
                .cloned()
                .unwrap_or_else(|| pdb_key.clone());
            pdb_key_canonical.insert(pdb_key, analysis);
        }
        for edb_key in &analysis_matched_edb {
            let analysis = edb_analysis_map
                .and_then(|m| m.get(edb_key))
                .cloned()
                .unwrap_or_else(|| edb_key.clone());
            edb_key_canonical.insert(edb_key, analysis);
        }
        for pdb_key in &meta_matched_pdb {
            let meta = if pdb_key.starts_with("meta:") {
                pdb_key.clone()
            } else {
                pdb_meta_map
                    .and_then(|m| m.get(pdb_key))
                    .cloned()
                    .unwrap_or_else(|| pdb_key.clone())
            };
            pdb_key_canonical.insert(pdb_key, meta);
        }
        for edb_key in &meta_matched_edb {
            let meta = if edb_key.starts_with("meta:") {
                edb_key.clone()
            } else {
                edb_meta_map
                    .and_then(|m| m.get(edb_key))
                    .cloned()
                    .unwrap_or_else(|| edb_key.clone())
            };
            edb_key_canonical.insert(edb_key, meta);
        }
        let pdb_common_order: Vec<String> = pdb_unique_list
            .iter()
            .filter(|k| {
                edb_set.contains(*k) || meta_matched.contains(*k) || analysis_matched.contains(*k)
            })
            .map(|k| {
                pdb_key_canonical
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| k.clone())
            })
            .collect();
        let edb_common_order: Vec<String> = edb_unique_list
            .iter()
            .filter(|k| {
                pdb_set.contains(*k) || meta_matched.contains(*k) || analysis_matched.contains(*k)
            })
            .map(|k| {
                edb_key_canonical
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| k.clone())
            })
            .collect();
        let order_mismatch = pdb_common_order != edb_common_order;

        let sample_only_in_pdb = pdb_unique_list
            .iter()
            .filter(|item| {
                !edb_set.contains(*item)
                    && !meta_matched.contains(*item)
                    && !analysis_matched.contains(*item)
            })
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        let sample_only_in_edb = edb_unique_list
            .iter()
            .filter(|item| {
                !pdb_set.contains(*item)
                    && !meta_matched.contains(*item)
                    && !analysis_matched.contains(*item)
            })
            .take(3)
            .cloned()
            .collect::<Vec<_>>();

        let mut sample_metadata_mismatches = Vec::<String>::new();
        let mut pdb_missing_core_metadata = 0usize;
        let mut edb_missing_core_metadata = 0usize;
        let mut artwork_mismatch_tracks = 0usize;
        let mut rating_colour_mismatch_tracks = 0usize;
        let mut field_note_tracks = 0usize;
        let mut path_mismatch_tracks = 0usize;
        let mut dictionary_id_issue_tracks = 0usize;

        let shared_pairs = pdb_set
            .intersection(&edb_set)
            .map(|key| (key.clone(), key.clone()))
            .chain(analysis_match_pairs.iter().cloned())
            .chain(meta_match_pairs.iter().cloned())
            .collect::<Vec<_>>();

        for (pdb_key, edb_key) in shared_pairs {
            let pdb_detail = pdb_info
                .as_ref()
                .and_then(|info| info.track_details_by_key.get(&pdb_key));
            let edb_detail = edb_info
                .as_ref()
                .and_then(|info| info.track_details_by_key.get(&edb_key));

            let Some(pdb_detail) = pdb_detail else {
                continue;
            };
            let Some(edb_detail) = edb_detail else {
                continue;
            };

            let pdb_title = normalized_required_str(&pdb_detail.title);
            let edb_title = normalized_required_str(&edb_detail.title);
            let edb_artist_raw = normalized_required_str(&edb_detail.artist);
            let pdb_artist = normalized_artist_for_gate(&pdb_detail.artist);
            let edb_artist = normalized_artist_for_gate(&edb_detail.artist);
            let pdb_album = normalized_optional_str(pdb_detail.album.as_deref());
            let edb_album = normalized_optional_str(edb_detail.album.as_deref());
            let pdb_key = normalized_optional_str(pdb_detail.key_name.as_deref());
            let edb_key = normalized_optional_str(edb_detail.key.as_deref());
            let pdb_analysis = normalized_required_str(&pdb_detail.analysis_path);
            let edb_analysis = normalized_optional_str(
                edb_detail
                    .usb_analysis_path_raw
                    .as_deref()
                    .or(edb_detail.usb_analysis_path.as_deref()),
            );
            let pdb_media = normalized_required_str(&pdb_detail.media_path);
            let edb_media = normalized_required_str(
                edb_detail
                    .usb_media_path
                    .as_deref()
                    .unwrap_or(&edb_detail.file_path),
            );
            let pdb_artwork = normalized_optional_str(pdb_detail.artwork_path.as_deref());
            let edb_artwork = normalized_optional_str(edb_detail.artwork_path.as_deref());

            let edb_bpm_x100 = edb_detail
                .bpm
                .filter(|v| *v > 0.0)
                .map(|v| (v * 100.0).round() as u32);
            let edb_duration_seconds = edb_detail.duration_ms.map(|v| (v / 1000) as u32);

            let pdb_missing_required = pdb_title.is_empty()
                || (pdb_detail.tempo_x100.is_none() && edb_bpm_x100.is_some())
                || (pdb_detail.duration_seconds.is_none() && edb_duration_seconds.is_some())
                || pdb_analysis.is_empty()
                || pdb_media.is_empty()
                || (!edb_album.is_empty() && pdb_album.is_empty())
                || (!edb_key.is_empty() && pdb_key.is_empty());

            if pdb_missing_required {
                pdb_missing_core_metadata += 1;
            }

            let edb_missing_required = edb_title.is_empty()
                || (edb_bpm_x100.is_none() && pdb_detail.tempo_x100.is_some())
                || (edb_duration_seconds.is_none() && pdb_detail.duration_seconds.is_some())
                || edb_analysis.is_empty()
                || edb_media.is_empty()
                || (!pdb_album.is_empty() && edb_album.is_empty())
                || (!pdb_key.is_empty() && edb_key.is_empty());

            if edb_missing_required {
                edb_missing_core_metadata += 1;
            }

            let edb_artist_requires_dict = !edb_artist_raw.is_empty()
                && !edb_artist_raw.eq_ignore_ascii_case("unknown artist");
            let artist_id_unresolved = edb_artist_requires_dict
                && (pdb_detail.artist_id == 0
                    || !parsed.artists.contains_key(&pdb_detail.artist_id));
            let album_id_unresolved = !edb_album.is_empty()
                && (pdb_detail.album_id == 0 || !parsed.albums.contains_key(&pdb_detail.album_id));
            let key_id_unresolved = !edb_key.is_empty()
                && (pdb_detail.key_id == 0 || !parsed.keys.contains_key(&pdb_detail.key_id));
            let artwork_id_unresolved = !edb_artwork.is_empty()
                && (pdb_detail.artwork_id == 0
                    || !parsed.artworks.contains_key(&pdb_detail.artwork_id));
            let has_dictionary_id_issue = artist_id_unresolved
                || album_id_unresolved
                || key_id_unresolved
                || artwork_id_unresolved;
            if has_dictionary_id_issue {
                dictionary_id_issue_tracks += 1;
            }

            let media_path_mismatch = normalize_track_path_for_identity(&pdb_detail.media_path)
                != normalize_track_path_for_identity(
                    edb_detail
                        .usb_media_path
                        .as_deref()
                        .unwrap_or(&edb_detail.file_path),
                );
            let analysis_path_mismatch = normalize_usb_path_for_parity(&pdb_detail.analysis_path)
                != normalize_usb_path_for_parity(
                    edb_detail
                        .usb_analysis_path_raw
                        .as_deref()
                        .or(edb_detail.usb_analysis_path.as_deref())
                        .unwrap_or(""),
                );
            if media_path_mismatch || analysis_path_mismatch {
                path_mismatch_tracks += 1;
            }

            let pdb_has_artwork = !pdb_artwork.is_empty();
            let edb_has_artwork = !edb_artwork.is_empty();
            let artwork_mismatch = pdb_has_artwork != edb_has_artwork;
            if artwork_mismatch {
                artwork_mismatch_tracks += 1;
            }
            if rating_colour
                .mismatched_paths
                .contains(&normalize_pdb_path_for_edb_lookup(
                    edb_detail
                        .usb_media_path
                        .as_deref()
                        .unwrap_or(&edb_detail.file_path),
                ))
            {
                rating_colour_mismatch_tracks += 1;
            }
            if field_notes
                .mismatched_paths
                .contains(&normalize_pdb_path_for_edb_lookup(
                    edb_detail
                        .usb_media_path
                        .as_deref()
                        .unwrap_or(&edb_detail.file_path),
                ))
            {
                field_note_tracks += 1;
            }

            let mut mismatches = Vec::<String>::new();
            if pdb_title != edb_title {
                mismatches.push("title".to_string());
            }
            if pdb_artist != edb_artist {
                mismatches.push("artist".to_string());
            }
            if pdb_album != edb_album {
                mismatches.push("album".to_string());
            }
            if pdb_key != edb_key {
                mismatches.push("key".to_string());
            }
            if pdb_detail.track_number != edb_detail.track_number {
                mismatches.push("trackNumber".to_string());
            }
            if pdb_detail.tempo_x100 != edb_bpm_x100 {
                mismatches.push("tempoX100".to_string());
            }
            if pdb_detail.duration_seconds != edb_duration_seconds {
                mismatches.push("durationSeconds".to_string());
            }
            if analysis_path_mismatch {
                mismatches.push("analysisPath".to_string());
            }
            if media_path_mismatch {
                mismatches.push("mediaPath".to_string());
            }
            if artwork_mismatch {
                if pdb_has_artwork && !edb_has_artwork {
                    mismatches.push("artworkMissingEdb".to_string());
                } else {
                    mismatches.push("artworkMissingPdb".to_string());
                }
            }
            if artist_id_unresolved {
                mismatches.push("artistDictId".to_string());
            }
            if album_id_unresolved {
                mismatches.push("albumDictId".to_string());
            }
            if key_id_unresolved {
                mismatches.push("keyDictId".to_string());
            }
            if artwork_id_unresolved {
                mismatches.push("artworkDictId".to_string());
            }
            if pdb_missing_required {
                let mut pdb_gaps = Vec::<&str>::new();
                if pdb_title.is_empty() {
                    pdb_gaps.push("title");
                }
                if pdb_detail.tempo_x100.is_none() && edb_bpm_x100.is_some() {
                    pdb_gaps.push("bpm");
                }
                if pdb_detail.duration_seconds.is_none() && edb_duration_seconds.is_some() {
                    pdb_gaps.push("duration");
                }
                if pdb_analysis.is_empty() {
                    pdb_gaps.push("analysisPath");
                }
                if pdb_media.is_empty() {
                    pdb_gaps.push("mediaPath");
                }
                if !edb_album.is_empty() && pdb_album.is_empty() {
                    pdb_gaps.push("album");
                }
                if !edb_key.is_empty() && pdb_key.is_empty() {
                    pdb_gaps.push("key");
                }
                mismatches.push(format!("pdbMissing({})", pdb_gaps.join("+")));
            }
            if edb_missing_required {
                let mut edb_gaps = Vec::<&str>::new();
                if edb_title.is_empty() {
                    edb_gaps.push("title");
                }
                if edb_bpm_x100.is_none() && pdb_detail.tempo_x100.is_some() {
                    edb_gaps.push("bpm");
                }
                if edb_duration_seconds.is_none() && pdb_detail.duration_seconds.is_some() {
                    edb_gaps.push("duration");
                }
                if edb_analysis.is_empty() {
                    edb_gaps.push("analysisPath");
                }
                if edb_media.is_empty() {
                    edb_gaps.push("mediaPath");
                }
                if !pdb_album.is_empty() && edb_album.is_empty() {
                    edb_gaps.push("album");
                }
                if !pdb_key.is_empty() && edb_key.is_empty() {
                    edb_gaps.push("key");
                }
                mismatches.push(format!("edbMissing({})", edb_gaps.join("+")));
            }

            if !mismatches.is_empty() {
                let entry = format!(
                    "{} [{}]",
                    sample_track_label(&pdb_key, pdb_detail),
                    mismatches.join(", ")
                );
                // Prioritize gap/dict entries over pure artwork mismatches
                let is_gap = pdb_missing_required
                    || edb_missing_required
                    || artist_id_unresolved
                    || album_id_unresolved
                    || key_id_unresolved
                    || artwork_id_unresolved;
                if is_gap {
                    sample_metadata_mismatches.insert(0, entry);
                } else if sample_metadata_mismatches.len() < 5 {
                    sample_metadata_mismatches.push(entry);
                }
                if sample_metadata_mismatches.len() > 10 {
                    sample_metadata_mismatches.truncate(10);
                }
            }
        }

        let pdb_duplicate_entries = pdb_info
            .as_ref()
            .map(|info| info.duplicate_entries)
            .unwrap_or(0);
        let playlist_id_match = match (pdb_info.as_ref(), edb_info.as_ref()) {
            (Some(pdb), Some(edb)) => edb
                .playlist_id
                .map(|playlist_id| playlist_id == pdb.playlist_id)
                .unwrap_or(false),
            _ => false,
        };
        let sort_order_match = match (pdb_info.as_ref(), edb_info.as_ref()) {
            (Some(pdb), Some(edb)) => edb
                .sort_order
                .map(|sort_order| sort_order == pdb.sort_order)
                .unwrap_or(false),
            _ => false,
        };

        let structural_pass = only_in_pdb == 0
            && only_in_edb == 0
            && !order_mismatch
            && pdb_duplicate_entries == 0
            && path_mismatch_tracks == 0
            && dictionary_id_issue_tracks == 0
            && playlist_id_match
            && sort_order_match;

        let status = if !structural_pass {
            DiagStatus::Fail
        } else if pdb_missing_core_metadata > 0 || edb_missing_core_metadata > 0 {
            DiagStatus::Warn
        } else {
            DiagStatus::Pass
        };

        let mut detail = UsbParityPlaylistDetail {
            name: display,
            pdb_tracks: pdb_unique_list.len(),
            edb_tracks: edb_unique_list.len(),
            matched_tracks,
            only_in_pdb,
            only_in_edb,
            order_mismatch,
            parent_match: None,
            pdb_playlist_id: pdb_info.as_ref().map(|info| info.playlist_id),
            edb_playlist_id: edb_info.as_ref().and_then(|info| info.playlist_id),
            pdb_sort_order: pdb_info.as_ref().map(|info| info.sort_order),
            edb_sort_order: edb_info.as_ref().and_then(|info| info.sort_order),
            pdb_duplicate_entries,
            edb_missing_core_metadata,
            pdb_missing_core_metadata,
            artwork_mismatch_tracks,
            rating_colour_mismatch_tracks,
            field_note_tracks,
            path_mismatch_tracks,
            dictionary_id_issue_tracks,
            playlist_id_match,
            sort_order_match,
            sample_only_in_pdb,
            sample_only_in_edb,
            sample_metadata_mismatches,
            status,
            issue_labels: Vec::new(),
        };
        detail.issue_labels = parity_issue_labels(&detail);
        details.push(detail);
    }
    // Keep strict parity playlist details in playlist order, not alphabetical:
    // 1) prefer PDB sort order when present (device-facing ordering source)
    // 2) fallback to eDB sort order for eDB-only playlists
    // 3) stable tie-breakers
    details.sort_by(|a, b| {
        let a_bucket = if a.pdb_sort_order.is_some() {
            0u8
        } else if a.edb_sort_order.is_some() {
            1u8
        } else {
            2u8
        };
        let b_bucket = if b.pdb_sort_order.is_some() {
            0u8
        } else if b.edb_sort_order.is_some() {
            1u8
        } else {
            2u8
        };
        a_bucket
            .cmp(&b_bucket)
            .then_with(|| {
                a.pdb_sort_order
                    .unwrap_or(u32::MAX)
                    .cmp(&b.pdb_sort_order.unwrap_or(u32::MAX))
            })
            .then_with(|| {
                a.edb_sort_order
                    .unwrap_or(u32::MAX)
                    .cmp(&b.edb_sort_order.unwrap_or(u32::MAX))
            })
            .then_with(|| {
                a.pdb_playlist_id
                    .unwrap_or(u32::MAX)
                    .cmp(&b.pdb_playlist_id.unwrap_or(u32::MAX))
            })
            .then_with(|| {
                a.edb_playlist_id
                    .unwrap_or(u32::MAX)
                    .cmp(&b.edb_playlist_id.unwrap_or(u32::MAX))
            })
            .then_with(|| a.name.cmp(&b.name))
    });

    let ParityTotals {
        fail_playlists,
        total_only_in_pdb,
        total_only_in_edb,
        entry_order_mismatches,
        sort_order_mismatches,
        order_mismatches,
        total_duplicate_entries,
        total_pdb_missing_core_metadata,
        total_edb_missing_core_metadata,
        total_artwork_mismatches,
        total_path_mismatches,
        total_dictionary_id_issues,
        // The report counts these by track (`rating_colour`) instead.
        rating_colour_mismatches: _,
        playlist_id_mismatches,
    } = ParityTotals::from_details(&details);

    let warn_playlists = details
        .iter()
        .filter(|d| matches!(d.status, DiagStatus::Warn))
        .count();
    let mut overall_status = if fail_playlists > 0 {
        DiagStatus::Fail
    } else if warn_playlists > 0 {
        DiagStatus::Warn
    } else {
        DiagStatus::Pass
    };
    if let Some(raw_coverage) = strict_raw_coverage.as_ref() {
        overall_status = DiagStatus::worst(&overall_status, &raw_coverage.status);
    }
    if rating_colour.mismatched_tracks > 0 {
        overall_status = DiagStatus::worst(&overall_status, &DiagStatus::Warn);
    }

    let mut checks = Vec::<DiagCheck>::new();
    checks.push(DiagCheck {
        label: "Overall player parity status".to_string(),
        status: overall_status.clone(),
        detail: format!(
            "playlists checked: {}, failing playlists: {}, playlist id mismatches: {}",
            details.len(),
            fail_playlists,
            playlist_id_mismatches,
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Parity-report section (required)".to_string(),
        status: overall_status.clone(),
        detail: "See parity summary rows for category counts.".to_string(),
        link: None,
    });
    let mut summary_rows = vec![
        DiagSummaryRow {
            label: "Failing playlists".to_string(),
            status: if fail_playlists == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: fail_playlists,
        },
        DiagSummaryRow {
            label: "Membership only-in-PDB".to_string(),
            status: if total_only_in_pdb == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_only_in_pdb,
        },
        DiagSummaryRow {
            label: "Membership only-in-eDB".to_string(),
            status: if total_only_in_edb == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_only_in_edb,
        },
        DiagSummaryRow {
            label: "Order mismatches".to_string(),
            status: if order_mismatches == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: order_mismatches,
        },
        DiagSummaryRow {
            label: "Duplicate PDB entries".to_string(),
            status: if total_duplicate_entries == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_duplicate_entries,
        },
        DiagSummaryRow {
            label: "PDB metadata gaps".to_string(),
            status: if total_pdb_missing_core_metadata == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: total_pdb_missing_core_metadata,
        },
        DiagSummaryRow {
            label: "eDB source gaps".to_string(),
            status: if total_edb_missing_core_metadata == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: total_edb_missing_core_metadata,
        },
        DiagSummaryRow {
            label: "Path mismatches".to_string(),
            status: if total_path_mismatches == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_path_mismatches,
        },
        DiagSummaryRow {
            label: "Artwork presence mismatches".to_string(),
            status: if total_artwork_mismatches == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: total_artwork_mismatches,
        },
        DiagSummaryRow {
            label: "Unresolved PDB dictionary ids".to_string(),
            status: if total_dictionary_id_issues == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_dictionary_id_issues,
        },
        DiagSummaryRow {
            label: "Rating/colour mismatches".to_string(),
            status: if rating_colour.mismatched_tracks == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: rating_colour.mismatched_tracks,
        },
        // A note: other synced fields that differ never fail parity.
        DiagSummaryRow {
            label: if field_notes.by_field.is_empty() {
                "Other track field differences (note)".to_string()
            } else {
                format!(
                    "Other track field differences (note: {})",
                    field_notes
                        .by_field
                        .iter()
                        .map(|(field, n)| format!("{field} {n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            status: DiagStatus::Pass,
            count: field_notes.mismatched_tracks,
        },
    ];
    if let Some(raw_coverage) = strict_raw_coverage.as_ref() {
        summary_rows.push(DiagSummaryRow {
            label: "Indexed audio file presence".to_string(),
            status: raw_coverage.status.clone(),
            count: raw_coverage.missing_count + raw_coverage.extra_count,
        });
    }
    checks.push(DiagCheck {
        label: "Playlist identity parity".to_string(),
        status: if playlist_id_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} playlist(s) have mismatched or unresolved playlist ids between PDB and eDB",
            playlist_id_mismatches
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Playlist membership parity".to_string(),
        status: if total_only_in_pdb == 0 && total_only_in_edb == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "membership deltas: only-in-PDB={}, only-in-eDB={}",
            total_only_in_pdb, total_only_in_edb
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Playlist ordering parity".to_string(),
        status: if order_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "entry-order mismatches={}, playlist-sort-order mismatches={}",
            entry_order_mismatches, sort_order_mismatches
        ),
        link: None,
    });
    if let Some(raw_coverage) = strict_raw_coverage.as_ref() {
        checks.push(DiagCheck {
            label: "Indexed audio file presence".to_string(),
            status: raw_coverage.status.clone(),
            detail: raw_coverage.detail.clone(),
            // Each missing/unindexed path is logged individually.
            link: (raw_coverage.missing_count + raw_coverage.extra_count > 0)
                .then(|| "event-log".to_string()),
        });
    }
    checks.push(DiagCheck {
        label: "Duplicate PDB entries".to_string(),
        status: if total_duplicate_entries == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} duplicate PDB playlist entry/entries detected",
            total_duplicate_entries
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "PDB metadata completeness".to_string(),
        status: if total_pdb_missing_core_metadata == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked PDB track(s) are missing required player metadata",
            total_pdb_missing_core_metadata
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Media and analysis path parity".to_string(),
        status: if total_path_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} playlist-linked track(s) have media or analysis path mismatches",
            total_path_mismatches
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Artwork presence parity".to_string(),
        status: if total_artwork_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked track(s) have artwork in one DB but not the other",
            total_artwork_mismatches
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "PDB dictionary id resolution".to_string(),
        status: if total_dictionary_id_issues == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} playlist-linked track(s) have unresolved required PDB dictionary ids",
            total_dictionary_id_issues
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "eDB source completeness".to_string(),
        status: if total_edb_missing_core_metadata == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked eDB track(s) are missing metadata used by strict parity comparison",
            total_edb_missing_core_metadata
        ),
        link: None,
    });
    // Minor (a rating or colour differs), so it doesn't count toward the overall status.
    let overall = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    checks.push(DiagCheck {
        label: "Rating and colour parity".to_string(),
        status: if rating_colour.mismatched_tracks == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked track(s) have a different rating or colour in the PDB than in \
             the eDB (rating differs: {}, colour differs: {})",
            rating_colour.mismatched_tracks,
            rating_colour.rating_mismatches,
            rating_colour.colour_mismatches
        ),
        link: None,
    });

    (checks, summary_rows, details, overall)
}
