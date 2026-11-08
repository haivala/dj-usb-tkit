//! eDB access, Contents, BPM/key, beat grid, analysis and playlist checks.

use super::*;
use crate::i18n::tr;

pub(crate) fn diagnose_edb_access(
    edb_path: &std::path::Path,
    conn: Option<&rusqlite::Connection>,
    edb_warnings: &[WarningEntry],
) -> DiagSection {
    let mut checks = Vec::new();

    if !edb_path.exists() {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb(),
            DiagStatus::Warn,
            tr::diagnostics_check_edb_missing(),
        ));
    } else {
        let (status, detail) = if conn.is_some() {
            let key_msg = edb_warnings
                .last()
                .map(|w| w.message.clone())
                .unwrap_or_else(|| tr::diagnostics_check_edb_opened().render());
            (DiagStatus::Pass, key_msg)
        } else {
            let msg = edb_warnings
                .last()
                .map(|w| w.message.clone())
                .unwrap_or_else(|| tr::diagnostics_check_edb_unable().render());
            (DiagStatus::Fail, msg)
        };
        checks.push(DiagCheck::new(tr::diagnostics_check_edb(), status, detail));
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Database Access".to_string(),
        status,
        checks,
        counts: None,
    }
}

#[cfg(test)]
pub(crate) fn diagnose_contents_integrity(
    contents_count: usize,
    indexed_count: usize,
) -> DiagSection {
    let mut checks = Vec::new();

    // Test-only, so its text stays English.
    checks.push(DiagCheck::named(
        "Audio files".to_string(),
        DiagStatus::Pass,
        format!("{contents_count} audio files found"),
    ));

    checks.push(DiagCheck::named(
        "Indexed tracks".to_string(),
        DiagStatus::Pass,
        format!(
            "{} in PDB/eDB",
            counted(indexed_count, "indexed track path", "indexed track paths")
        ),
    ));

    let mismatch = contents_count as i64 - indexed_count as i64;
    let mismatch_status = if mismatch == 0 {
        DiagStatus::Pass
    } else if mismatch < 0 && mismatch.unsigned_abs() > 5 {
        DiagStatus::Fail
    } else {
        DiagStatus::Warn
    };
    checks.push(DiagCheck::named(
        "Count match".to_string(),
        mismatch_status,
        if mismatch == 0 {
            "Exact match".to_string()
        } else if mismatch > 0 {
            format!("{mismatch} audio files not playlist-referenced in PDB/eDB")
        } else {
            format!(
                "{} DB entries have no matching audio file",
                mismatch.unsigned_abs()
            )
        },
    ));

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Contents Integrity".to_string(),
        status,
        checks,
        counts: Some(DiagCountsSummary {
            contents_count,
            indexed_count,
            mismatch_count: mismatch,
        }),
    }
}

pub(crate) fn diagnose_contents_integrity_db_only(
    pdb_indexed_count: usize,
    edb_indexed_count: usize,
    pdb_only_count: usize,
    edb_only_count: usize,
    history_only_count: usize,
) -> DiagSection {
    let mut checks = Vec::new();

    checks.push(DiagCheck::new(
        tr::diagnostics_check_pdb_paths(),
        DiagStatus::Pass,
        tr::diagnostics_check_paths_count(pdb_indexed_count),
    ));
    checks.push(DiagCheck::new(
        tr::diagnostics_check_edb_paths(),
        DiagStatus::Pass,
        tr::diagnostics_check_paths_count(edb_indexed_count),
    ));

    let disagreement_status = if pdb_only_count == 0 && edb_only_count == 0 {
        DiagStatus::Pass
    } else {
        DiagStatus::Warn
    };
    let parts: Vec<crate::i18n::Text> = [
        (
            pdb_only_count,
            tr::diagnostics_check_path_agreement_pdb_only as fn(usize) -> _,
        ),
        (
            edb_only_count,
            tr::diagnostics_check_path_agreement_edb_only,
        ),
        (
            history_only_count,
            tr::diagnostics_check_path_agreement_history_only,
        ),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, text)| text(count))
    .collect();
    let disagreement_detail = if parts.is_empty() {
        tr::diagnostics_check_path_agreement_agree().render()
    } else {
        super::pdb_integrity::join_rendered(&parts, "; ")
    };
    let mut agreement = DiagCheck::new(
        tr::diagnostics_check_path_agreement(),
        disagreement_status,
        disagreement_detail,
    );
    if history_only_count > 0 {
        agreement = agreement.with_link("event-log");
    }
    checks.push(agreement);

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Contents Integrity".to_string(),
        status,
        checks,
        counts: Some(DiagCountsSummary {
            contents_count: pdb_indexed_count,
            indexed_count: edb_indexed_count,
            mismatch_count: pdb_only_count as i64 - edb_only_count as i64,
        }),
    }
}

/// A track whose BPM or key disagrees between its PDB row and its ANLZ beat
/// grid / eDB `content` row. The PDB is the reference: it is what older CDJs
/// read and what `fix_empty_analysis_files` already trusts.
#[derive(Debug, Clone)]
pub(crate) struct BpmKeyMismatch {
    pub track_path: String,
    pub pdb_tempo_x100: u32,
    pub duration_ms: Option<u64>,
    /// Absolute `.DAT` path, set when the DAT or EXT beat-grid tempo differs.
    pub anlz_dat_path: Option<std::path::PathBuf>,
    /// The `.DAT` and `.EXT` beat-grid tempos (x100) read, when there is a grid.
    pub anlz_tempos_x100: [Option<u16>; 2],
    /// Current eDB `bpmx100`, set when it differs.
    pub edb_bpmx100: Option<i64>,
    /// The PDB key name, set when the eDB key differs (or is missing).
    pub pdb_key_for_edb: Option<String>,
}

/// Compare each PDB track's tempo and key against, when an eDB connection is
/// given, `content.bpmx100` and the `key` row `content.key_id` points at, and
/// -- only with `scan_anlz` -- its ANLZ beat grid (`.DAT` and `.EXT`, tempo
/// only; ANLZ has no key). `scan_anlz` reads two files per track off the
/// stick, so the automatic USB diagnosis (a database-only check) leaves it
/// off and only the repair flow, which scans analysis files anyway, turns it
/// on. Key names match by export's rule (`key_names_match`: `Db` == `C#`).
/// Tracks without a PDB tempo/key, without an eDB row, or with a
/// missing/unparseable bundle are skipped for that field.
pub(crate) fn detect_bpm_key_mismatches(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb: Option<&rusqlite::Connection>,
    scan_anlz: bool,
    app_master_db_id: Option<u32>,
) -> Vec<BpmKeyMismatch> {
    type EdbBpmKey = (Option<i64>, Option<String>);
    let edb_by_path: HashMap<String, EdbBpmKey> = edb
        .and_then(|conn| {
            let with_key = table_exists(conn, "key");
            let sql = if with_key {
                r#"SELECT c.path, c.bpmx100, k.name FROM content c
                   LEFT JOIN "key" k ON k.key_id = c.key_id WHERE c.path IS NOT NULL"#
            } else {
                "SELECT path, bpmx100, NULL FROM content WHERE path IS NOT NULL"
            };
            let mut stmt = conn.prepare(sql).ok()?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        (
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ),
                    ))
                })
                .ok()?;
            Some(rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();

    // A shared bundle is only its owner's to align: rebuilding it for
    // another track breaks it for the owner, and the next scan flags the
    // owner. `separate_shared_analysis` gives the other tracks their own
    // bundle, analysed with their own BPM, before this fix runs.
    let shared_bundle_tracks = if scan_anlz {
        scan_analysis_ownership(usb_root, parsed, app_master_db_id, true).non_owner_track_ids()
    } else {
        HashSet::new()
    };
    let mut out = Vec::new();
    for track in &parsed.tracks {
        let pdb_tempo_x100 = track.tempo_x100;
        let edb_row = edb_by_path.get(&track.track_file_path);

        let dat = (scan_anlz && pdb_tempo_x100 > 0 && !shared_bundle_tracks.contains(&track.id))
            .then(|| crate::service::usb_utils::resolve_usb_side_path(usb_root, &track.anlz_path))
            .flatten();
        let anlz_tempos_x100 = dat.as_ref().map_or([None, None], |dat| {
            [dat.clone(), dat.with_extension("EXT")].map(|p| {
                std::fs::read(p)
                    .ok()
                    .and_then(|bytes| crate::service::anlz::read_beatgrid_tempo_from_anlz(&bytes))
            })
        });
        let anlz_dat_path = dat.filter(|_| {
            anlz_tempos_x100
                .iter()
                .flatten()
                .any(|t| u32::from(*t) != pdb_tempo_x100)
        });
        let edb_bpmx100 = edb_row
            .and_then(|(bpm, _)| *bpm)
            .filter(|v| pdb_tempo_x100 > 0 && *v != i64::from(pdb_tempo_x100));
        let pdb_key_for_edb = parsed
            .keys
            .get(&track.key_id)
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .filter(|pdb_key| {
                edb_row.is_some_and(|(_, edb_key)| {
                    !edb_key.as_deref().is_some_and(|edb_key| {
                        crate::service::export_helpers::key_names_match(pdb_key, edb_key)
                    })
                })
            })
            .map(str::to_string);

        if anlz_dat_path.is_some() || edb_bpmx100.is_some() || pdb_key_for_edb.is_some() {
            out.push(BpmKeyMismatch {
                track_path: track.track_file_path.clone(),
                pdb_tempo_x100,
                duration_ms: track
                    .duration_seconds
                    .filter(|s| *s > 0)
                    .map(|s| u64::from(s) * 1000),
                anlz_dat_path,
                anlz_tempos_x100,
                edb_bpmx100,
                pdb_key_for_edb,
            });
        }
    }
    out
}

/// Older app versions wrote beat grids in a format some players (e.g. the
/// XDJ-AZ) reject. Only tracks this app exported can have one, and the stick's
/// export log says which were last exported by such a version (a record
/// without `app_version`), so only those tracks' `.DAT` files are read, and
/// only their first few KB. When the log can't be read, every track's
/// `.DAT` is checked instead. rekordbox-made bundles never match.
pub(super) fn diagnose_beat_grid_format(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
) -> (DiagStatus, crate::i18n::Text) {
    use std::io::Read;

    let check = |status, detail: crate::i18n::Text| (status, detail);
    // `None`: the log can't be read, so every track is a candidate.
    let candidates = match crate::service::export_log::load_export_log(usb_root) {
        Ok(Some(log)) => {
            Some(crate::service::export_log::fingerprints_last_exported_before_versioning(&log))
        }
        Ok(None) => {
            return check(
                DiagStatus::Pass,
                tr::diagnostics_check_beat_grid_not_exported(),
            );
        }
        Err(_) => None,
    };
    let mut outdated = Vec::new();
    for track in &parsed.tracks {
        if candidates.as_ref().is_some_and(|candidates| {
            !candidates.contains(&track_identity_key(&track.track_file_path, "", "", None))
        }) {
            continue;
        }
        let Some(dat) =
            crate::service::usb_utils::resolve_usb_side_path(usb_root, &track.anlz_path)
        else {
            continue;
        };
        let mut prefix = Vec::new();
        let read = std::fs::File::open(&dat).and_then(|file| {
            file.take(crate::service::anlz::PQTZ_PROBE_BYTES)
                .read_to_end(&mut prefix)
        });
        if read.is_ok() && crate::service::anlz::has_misplaced_pqtz_header(&prefix) {
            outdated.push(track.track_file_path.as_str());
        }
    }
    if outdated.is_empty() {
        return check(DiagStatus::Pass, tr::diagnostics_check_beat_grid_current());
    }
    check(
        DiagStatus::Warn,
        tr::diagnostics_check_beat_grid_outdated(
            outdated.len(),
            outdated
                .iter()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .join(", "),
        ),
    )
}

/// `edb_content_rows` is the eDB's `content` row count when it could be read;
/// with no tracks there are no analysis refs to miss.
pub(crate) fn diagnose_analysis_integrity(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
    edb_content_rows: Option<usize>,
) -> DiagSection {
    let mut checks = Vec::new();

    if let Some(parsed) = parsed {
        let total_tracks = parsed.tracks.len();
        let mut path_refs = HashSet::<String>::new();
        let mut tracks_with_ref = 0usize;
        for track in &parsed.tracks {
            let path = normalize_analysis_path_for_identity(&track.anlz_path);
            if path.is_empty() {
                continue;
            }
            tracks_with_ref += 1;
            path_refs.insert(path);
        }
        let missing = total_tracks.saturating_sub(tracks_with_ref);
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_analysis(),
            if missing == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            tr::diagnostics_check_pdb_analysis_detail(
                tracks_with_ref,
                total_tracks,
                path_refs.len(),
            ),
        ));
    } else {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_analysis(),
            DiagStatus::Fail,
            tr::diagnostics_check_no_pdb_data(),
        ));
    }

    if let Some(edb_playlist_tracks) = edb_playlist_tracks {
        let mut analysis_by_content_id = HashMap::<String, String>::new();
        let mut total_playlist_entries = 0usize;
        for tracks in edb_playlist_tracks.values() {
            total_playlist_entries += tracks.len();
            for track in tracks {
                let key = if track.id.trim().is_empty() {
                    track_identity_key(track.identity_path(), &track.title, &track.artist, None)
                } else {
                    track.id.clone()
                };
                analysis_by_content_id.entry(key).or_insert_with(|| {
                    normalize_analysis_path_for_identity(
                        track
                            .usb_analysis_path_raw
                            .as_deref()
                            .or(track.usb_analysis_path.as_deref())
                            .unwrap_or_default(),
                    )
                });
            }
        }
        let unique_content_rows = analysis_by_content_id.len();
        let refs = analysis_by_content_id
            .values()
            .filter(|path| !path.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let unique_refs = refs.iter().cloned().collect::<HashSet<_>>().len();
        let missing = unique_content_rows.saturating_sub(refs.len());
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb_analysis(),
            if missing == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            tr::diagnostics_check_edb_analysis_detail(
                refs.len(),
                unique_content_rows,
                unique_refs,
                total_playlist_entries,
            ),
        ));
    } else if edb_content_rows == Some(0) {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb_analysis(),
            DiagStatus::Pass,
            tr::diagnostics_check_edb_analysis_no_tracks(),
        ));
    } else {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb_analysis(),
            DiagStatus::Warn,
            tr::diagnostics_check_edb_analysis_no_data(),
        ));
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Analysis Files".to_string(),
        status,
        checks,
        counts: None,
    }
}

#[cfg(test)]
pub(crate) fn diagnose_playlist_resolution(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
) -> (DiagSection, Vec<PlaylistDiagEntry>) {
    diagnose_playlist_resolution_with_db(parsed, None)
}

#[cfg(test)]
pub(crate) fn diagnose_playlist_resolution_with_db(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
) -> (DiagSection, Vec<PlaylistDiagEntry>) {
    diagnose_playlist_resolution_with_edb_internal(parsed, edb_playlist_tracks, |_, _, _| {})
}

pub(crate) fn diagnose_playlist_resolution_with_edb_internal<F>(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
    mut on_playlist_progress: F,
) -> (DiagSection, Vec<PlaylistDiagEntry>)
where
    F: FnMut(usize, usize, &str),
{
    let mut checks = Vec::new();
    let mut playlist_details = Vec::new();

    let parsed = match parsed {
        Some(p) => p,
        None => {
            checks.push(DiagCheck::new(
                tr::diagnostics_check_playlist_data(),
                DiagStatus::Fail,
                tr::diagnostics_check_no_pdb_data(),
            ));
            return (
                DiagSection {
                    title: "Playlist Resolution".to_string(),
                    status: DiagStatus::Fail,
                    checks,
                    counts: None,
                },
                playlist_details,
            );
        }
    };

    let track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    let pdb_track_key_by_id = parsed
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
    let mut entries_by_playlist: HashMap<u32, Vec<&crate::pdb_reader::PdbPlaylistEntryRow>> =
        HashMap::new();
    for entry in &parsed.playlist_entries {
        entries_by_playlist
            .entry(entry.playlist_id)
            .or_default()
            .push(entry);
    }

    let mut leaves: Vec<&crate::pdb_reader::PdbPlaylistTreeRow> = parsed
        .playlist_tree
        .iter()
        .filter(|n| !n.row_is_folder)
        .collect();
    leaves.sort_by_key(|n| n.sort_order);
    let mut grouped_leaves = Vec::<(String, String, u32, Vec<u32>)>::new();
    let mut grouped_leaf_idx = HashMap::<String, usize>::new();
    for leaf in &leaves {
        let canonical = canonicalize_playlist_name(&leaf.name);
        if let Some(existing_idx) = grouped_leaf_idx.get(&canonical).copied() {
            grouped_leaves[existing_idx].3.push(leaf.id);
            continue;
        }
        grouped_leaf_idx.insert(canonical.clone(), grouped_leaves.len());
        grouped_leaves.push((canonical, leaf.name.clone(), leaf.sort_order, vec![leaf.id]));
    }
    grouped_leaves.sort_by_key(|(_, _, sort_order, _)| *sort_order);
    let pdb_leaf_name_keys = grouped_leaves
        .iter()
        .map(|(canonical, _, _, _)| canonical.clone())
        .collect::<HashSet<_>>();
    let edb_playlist_tracks_canonical = edb_playlist_tracks.map(|m| {
        m.iter()
            .map(|(name, tracks)| (canonicalize_playlist_name(name), tracks))
            .collect::<HashMap<_, _>>()
    });

    let mut all_pass = true;
    let mut any_fail = false;
    let mut cross_source_matches_total = 0usize;
    let mut cross_source_pdb_total = 0usize;
    let mut cross_source_edb_total = 0usize;

    for (idx, (_canonical_name, display_name, _sort_order, playlist_ids)) in
        grouped_leaves.iter().enumerate()
    {
        let done = idx + 1;
        let total = grouped_leaves.len();
        on_playlist_progress(
            done,
            total,
            &format!("Resolving playlist {done}/{total}: {display_name}"),
        );
        let mut unique_track_ids = HashSet::<u32>::new();
        let mut pdb_ids = HashSet::<String>::new();
        for playlist_id in playlist_ids {
            if let Some(entries) = entries_by_playlist.get(playlist_id) {
                for entry in entries {
                    unique_track_ids.insert(entry.track_id);
                    let identity = pdb_track_key_by_id
                        .get(&entry.track_id)
                        .cloned()
                        .unwrap_or_else(|| format!("id:{}", entry.track_id));
                    pdb_ids.insert(identity);
                }
            }
        }
        let total = unique_track_ids.len();
        let resolved = unique_track_ids
            .iter()
            .filter(|track_id| track_ids.contains(track_id))
            .count();
        let edb_tracks = edb_playlist_tracks
            .and_then(|m| {
                m.get(display_name).or_else(|| {
                    let key = canonicalize_playlist_name(display_name);
                    edb_playlist_tracks_canonical
                        .as_ref()
                        .and_then(|cm| cm.get(&key).copied())
                })
            })
            .cloned()
            .unwrap_or_default();
        let edb_ids = edb_tracks
            .iter()
            .map(|t| track_identity_key(t.identity_path(), &t.title, &t.artist, Some(&t.id)))
            .collect::<HashSet<String>>();
        let matched_entries = pdb_ids.intersection(&edb_ids).count();
        let pdb_entries = pdb_ids.len();
        let edb_entries = edb_ids.len();
        let pdb_match_rate = if pdb_entries > 0 {
            matched_entries as f64 / pdb_entries as f64
        } else {
            1.0
        };
        let edb_match_rate = if edb_entries > 0 {
            matched_entries as f64 / edb_entries as f64
        } else {
            1.0
        };
        cross_source_matches_total += matched_entries;
        cross_source_pdb_total += pdb_entries;
        cross_source_edb_total += edb_entries;
        let rate = if total > 0 {
            resolved as f64 / total as f64
        } else {
            1.0
        };
        let status = if rate >= 1.0 {
            DiagStatus::Pass
        } else if rate >= 0.8 {
            DiagStatus::Warn
        } else {
            DiagStatus::Fail
        };
        if !matches!(status, DiagStatus::Pass) {
            all_pass = false;
        }
        if matches!(status, DiagStatus::Fail) {
            any_fail = true;
        }
        playlist_details.push(PlaylistDiagEntry {
            name: display_name.clone(),
            total_entries: total,
            resolved_entries: resolved,
            resolution_rate: (rate * 1000.0).round() / 1000.0,
            status,
            pdb_entries,
            edb_entries,
            matched_entries,
            pdb_match_rate: (pdb_match_rate * 1000.0).round() / 1000.0,
            edb_match_rate: (edb_match_rate * 1000.0).round() / 1000.0,
        });
    }

    let mut edb_only_playlist_count = 0usize;
    if let Some(edb_map) = edb_playlist_tracks {
        for (edb_name, edb_tracks) in edb_map {
            let key = canonicalize_playlist_name(edb_name);
            if pdb_leaf_name_keys.contains(&key) {
                continue;
            }
            edb_only_playlist_count += 1;
            all_pass = false;
            any_fail = true;
            let edb_ids = edb_tracks
                .iter()
                .map(|t| track_identity_key(t.identity_path(), &t.title, &t.artist, Some(&t.id)))
                .collect::<HashSet<String>>();
            let edb_entries = edb_ids.len();
            cross_source_edb_total += edb_entries;
            playlist_details.push(PlaylistDiagEntry {
                name: edb_name.clone(),
                total_entries: edb_entries,
                resolved_entries: 0,
                resolution_rate: 0.0,
                status: DiagStatus::Fail,
                pdb_entries: 0,
                edb_entries,
                matched_entries: 0,
                pdb_match_rate: 0.0,
                edb_match_rate: 0.0,
            });
            checks.push(DiagCheck::named(
                edb_name.clone(),
                DiagStatus::Fail,
                tr::diagnostics_check_playlist_edb_only(edb_entries),
            ));
        }
    }

    let total_entries: usize = playlist_details.iter().map(|p| p.total_entries).sum();
    let total_resolved: usize = playlist_details.iter().map(|p| p.resolved_entries).sum();
    let overall_rate = if total_entries > 0 {
        total_resolved as f64 / total_entries as f64
    } else {
        1.0
    };

    checks.push(DiagCheck::new(
        tr::diagnostics_check_overall_resolution(),
        if all_pass {
            DiagStatus::Pass
        } else if any_fail {
            DiagStatus::Fail
        } else {
            DiagStatus::Warn
        },
        tr::diagnostics_check_overall_resolution_detail(
            total_resolved,
            total_entries,
            format!("{:.1}", overall_rate * 100.0),
            grouped_leaves.len(),
        ),
    ));
    if edb_only_playlist_count > 0 {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_db_only_playlists(),
            DiagStatus::Fail,
            tr::diagnostics_check_db_only_playlists_detail(edb_only_playlist_count),
        ));
    }
    if edb_playlist_tracks.is_some() {
        let pdb_rate = if cross_source_pdb_total > 0 {
            cross_source_matches_total as f64 / cross_source_pdb_total as f64
        } else {
            1.0
        };
        let edb_rate = if cross_source_edb_total > 0 {
            cross_source_matches_total as f64 / cross_source_edb_total as f64
        } else {
            1.0
        };
        checks.push(DiagCheck::new(
            tr::diagnostics_check_key_overlap(),
            DiagStatus::Pass,
            tr::diagnostics_check_key_overlap_detail(
                cross_source_matches_total,
                format!("{:.1}", pdb_rate * 100.0),
                cross_source_pdb_total,
                format!("{:.1}", edb_rate * 100.0),
                cross_source_edb_total,
            ),
        ));
    }
    checks.push(DiagCheck::new(
        tr::diagnostics_check_operational(),
        DiagStatus::Pass,
        tr::diagnostics_check_operational_detail(),
    ));

    // List playlists with issues
    for pd in &playlist_details {
        if pd.resolution_rate < 1.0 {
            checks.push(DiagCheck::named(
                pd.name.clone(),
                pd.status.clone(),
                tr::diagnostics_check_playlist_resolution(
                    pd.resolved_entries,
                    pd.total_entries,
                    format!("{:.1}", pd.resolution_rate * 100.0),
                ),
            ));
        }
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (
        DiagSection {
            title: "Playlist Resolution".to_string(),
            status,
            checks,
            counts: None,
        },
        playlist_details,
    )
}
