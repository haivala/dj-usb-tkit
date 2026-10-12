//! eDB access, Contents, BPM/key, beat grid, analysis and playlist checks.

use super::*;

pub(crate) fn diagnose_edb_access(
    edb_path: &std::path::Path,
    conn: Option<&rusqlite::Connection>,
    edb_warnings: &[WarningEntry],
) -> DiagSection {
    let mut checks = Vec::new();

    if !edb_path.exists() {
        checks.push(DiagCheck {
            label: "eDB".to_string(),
            status: DiagStatus::Warn,
            detail: "File not found".to_string(),
            link: None,
        });
    } else {
        let (status, detail) = if conn.is_some() {
            let key_msg = edb_warnings
                .last()
                .map(|w| w.message.clone())
                .unwrap_or_else(|| "Opened successfully".to_string());
            (DiagStatus::Pass, key_msg)
        } else {
            let msg = edb_warnings
                .last()
                .map(|w| w.message.clone())
                .unwrap_or_else(|| "Unable to open".to_string());
            (DiagStatus::Fail, msg)
        };
        checks.push(DiagCheck {
            label: "eDB".to_string(),
            status,
            detail,
            link: None,
        });
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

    checks.push(DiagCheck {
        label: "Audio files".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{contents_count} audio files found"),
        link: None,
    });

    checks.push(DiagCheck {
        label: "Indexed tracks".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{indexed_count} indexed track path(s) in PDB/eDB"),
        link: None,
    });

    let mismatch = contents_count as i64 - indexed_count as i64;
    let mismatch_status = if mismatch == 0 {
        DiagStatus::Pass
    } else if mismatch < 0 && mismatch.unsigned_abs() > 5 {
        DiagStatus::Fail
    } else {
        DiagStatus::Warn
    };
    checks.push(DiagCheck {
        label: "Count match".to_string(),
        status: mismatch_status,
        detail: if mismatch == 0 {
            "Exact match".to_string()
        } else if mismatch > 0 {
            format!("{mismatch} audio files not playlist-referenced in PDB/eDB")
        } else {
            format!(
                "{} DB entries have no matching audio file",
                mismatch.unsigned_abs()
            )
        },
        link: None,
    });

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

    checks.push(DiagCheck {
        label: "PDB indexed paths".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{pdb_indexed_count} canonical track path(s)"),
        link: None,
    });
    checks.push(DiagCheck {
        label: "eDB indexed paths".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{edb_indexed_count} canonical track path(s)"),
        link: None,
    });

    let disagreement_status = if pdb_only_count == 0 && edb_only_count == 0 {
        DiagStatus::Pass
    } else {
        DiagStatus::Warn
    };
    let history_suffix = if history_only_count > 0 {
        format!("; {history_only_count} history-only track(s)")
    } else {
        String::new()
    };
    let disagreement_detail = if pdb_only_count == 0
        && edb_only_count == 0
        && history_only_count == 0
    {
        "PDB/eDB canonical-path sets agree".to_string()
    } else if pdb_only_count == 0 && edb_only_count == 0 {
        format!("{history_only_count} history-only track(s)")
    } else if pdb_only_count > 0 && edb_only_count > 0 {
        format!(
            "{pdb_only_count} PDB-only path(s); {edb_only_count} eDB-only path(s){history_suffix}"
        )
    } else if pdb_only_count > 0 {
        format!("{pdb_only_count} PDB-only path(s){history_suffix}")
    } else {
        format!("{edb_only_count} eDB-only path(s){history_suffix}")
    };
    let agreement_link = if history_only_count > 0 {
        Some("event-log".to_string())
    } else {
        None
    };
    checks.push(DiagCheck {
        label: "PDB/eDB path agreement".to_string(),
        status: disagreement_status,
        detail: disagreement_detail,
        link: agreement_link,
    });

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

    let mut out = Vec::new();
    for track in &parsed.tracks {
        let pdb_tempo_x100 = track.tempo_x100;
        let edb_row = edb_by_path.get(&track.track_file_path);

        let anlz_dat_path = (scan_anlz && pdb_tempo_x100 > 0)
            .then(|| crate::service::usb_utils::resolve_usb_side_path(usb_root, &track.anlz_path))
            .flatten()
            .filter(|dat| {
                [dat.clone(), dat.with_extension("EXT")].iter().any(|p| {
                    std::fs::read(p)
                        .ok()
                        .and_then(|bytes| {
                            crate::service::anlz::read_beatgrid_tempo_from_anlz(&bytes)
                        })
                        .is_some_and(|t| u32::from(t) != pdb_tempo_x100)
                })
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
) -> DiagCheck {
    use std::io::Read;

    let check = |status, detail: String| DiagCheck {
        label: "Beat grid format".to_string(),
        status,
        detail,
        link: None,
    };
    // `None`: the log can't be read, so every track is a candidate.
    let candidates = match crate::service::export_log::load_export_log(usb_root) {
        Ok(Some(log)) => {
            Some(crate::service::export_log::fingerprints_last_exported_before_versioning(&log))
        }
        Ok(None) => {
            return check(
                DiagStatus::Pass,
                "no tracks on this USB were exported by this app".to_string(),
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
        return check(
            DiagStatus::Pass,
            "every beat grid this app wrote to this USB is in the current format".to_string(),
        );
    }
    check(
        DiagStatus::Warn,
        format!(
            "{} track(s) have a beat grid in an outdated format that some players (e.g. the \
             XDJ-AZ) reject, so Beat Sync is unavailable on them; e.g. {}; re-export them or \
             run the repair (fix_beat_grid_header) to fix",
            outdated.len(),
            outdated
                .iter()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

pub(crate) fn diagnose_analysis_integrity(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
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
        checks.push(DiagCheck {
            label: "PDB analysis refs".to_string(),
            status: if missing == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            detail: format!(
                "{tracks_with_ref}/{total_tracks} track row(s) have analysis paths; {} unique bundle reference(s)",
                path_refs.len()
            ),
            link: None,
        });
    } else {
        checks.push(DiagCheck {
            label: "PDB analysis refs".to_string(),
            status: DiagStatus::Fail,
            detail: "No PDB data available".to_string(),
            link: None,
        });
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
        checks.push(DiagCheck {
            label: "eDB analysis refs".to_string(),
            status: if missing == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            detail: format!(
                "{}/{} playlist-linked content row(s) have analysis paths; {} unique bundle reference(s) from {} playlist entries",
                refs.len(),
                unique_content_rows,
                unique_refs,
                total_playlist_entries
            ),
            link: None,
        });
    } else {
        checks.push(DiagCheck {
            label: "eDB analysis refs".to_string(),
            status: DiagStatus::Warn,
            detail: "No eDB playlist data available".to_string(),
            link: None,
        });
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
            checks.push(DiagCheck {
                label: "Playlist data".to_string(),
                status: DiagStatus::Fail,
                detail: "No PDB data available".to_string(),
                link: None,
            });
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
            checks.push(DiagCheck {
                label: edb_name.clone(),
                status: DiagStatus::Fail,
                detail: format!("present in eDB only ({edb_entries} entries), missing from PDB"),
                link: None,
            });
        }
    }

    let total_entries: usize = playlist_details.iter().map(|p| p.total_entries).sum();
    let total_resolved: usize = playlist_details.iter().map(|p| p.resolved_entries).sum();
    let overall_rate = if total_entries > 0 {
        total_resolved as f64 / total_entries as f64
    } else {
        1.0
    };

    checks.push(DiagCheck {
        label: "Overall resolution".to_string(),
        status: if all_pass {
            DiagStatus::Pass
        } else if any_fail {
            DiagStatus::Fail
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{total_resolved}/{total_entries} entries resolve ({:.1}%) across {} playlists",
            overall_rate * 100.0,
            grouped_leaves.len()
        ),
        link: None,
    });
    if edb_only_playlist_count > 0 {
        checks.push(DiagCheck {
            label: "DB-only playlists".to_string(),
            status: DiagStatus::Fail,
            detail: format!("{edb_only_playlist_count} playlist(s) exist in eDB but not in PDB"),
            link: None,
        });
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
        let status = DiagStatus::Pass;
        let detail = format!(
            "matched {} track keys; PDB {:.1}% ({}/{}), DB {:.1}% ({}/{})",
            cross_source_matches_total,
            pdb_rate * 100.0,
            cross_source_matches_total,
            cross_source_pdb_total,
            edb_rate * 100.0,
            cross_source_matches_total,
            cross_source_edb_total
        );
        checks.push(DiagCheck {
            label: "PDB vs eDB key overlap (informational)".to_string(),
            status,
            detail,
            link: None,
        });
    }
    checks.push(DiagCheck {
        label: "Operational interpretation".to_string(),
        status: DiagStatus::Pass,
        detail: "pass/warn here means operationally usable; strict parity may still fail on the same USB".to_string(),
        link: None,
    });

    // List playlists with issues
    for pd in &playlist_details {
        if pd.resolution_rate < 1.0 {
            checks.push(DiagCheck {
                label: pd.name.clone(),
                status: pd.status.clone(),
                detail: format!(
                    "{}/{} entries ({:.1}%)",
                    pd.resolved_entries,
                    pd.total_entries,
                    pd.resolution_rate * 100.0
                ),
                link: None,
            });
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
