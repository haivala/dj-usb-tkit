//! eDB access, Contents, BPM/key, beat grid, analysis and playlist checks.

use super::*;
use crate::i18n::{Num, tr};

pub(crate) fn diagnose_edb_access(
    edb_path: &std::path::Path,
    conn: Option<&rusqlite::Connection>,
) -> DiagSection {
    let mut checks = Vec::new();

    if !edb_path.exists() {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb(),
            DiagStatus::Warn,
            tr::diagnostics_check_edb_missing(),
        ));
    } else {
        // How it was opened, or why it couldn't be, is in the Event Log (`open_edb`'s warnings).
        let (status, detail) = if conn.is_some() {
            (DiagStatus::Pass, tr::diagnostics_check_edb_opened())
        } else {
            (DiagStatus::Fail, tr::diagnostics_check_edb_unable())
        };
        checks.push(DiagCheck::new(tr::diagnostics_check_edb(), status, detail));
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        key: "",
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
        key: "",
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

/// A database the stick doesn't have is `None`: no count for it, and no
/// agreement check, since there is nothing to compare.
pub(crate) fn diagnose_contents_integrity_db_only(
    pdb_indexed_count: Option<usize>,
    edb_indexed_count: Option<usize>,
    pdb_only_count: usize,
    edb_only_count: usize,
    history_only_count: usize,
) -> DiagSection {
    let mut checks = Vec::new();

    if let Some(count) = pdb_indexed_count {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_paths(),
            DiagStatus::Pass,
            tr::diagnostics_check_paths_count(count),
        ));
    }
    if let Some(count) = edb_indexed_count {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb_paths(),
            DiagStatus::Pass,
            tr::diagnostics_check_paths_count(count),
        ));
    }
    let (Some(pdb_indexed_count), Some(edb_indexed_count)) = (pdb_indexed_count, edb_indexed_count)
    else {
        let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
        return DiagSection {
            key: "",
            title: "Contents Integrity".to_string(),
            status,
            checks,
            counts: Some(DiagCountsSummary {
                contents_count: pdb_indexed_count.unwrap_or(0),
                indexed_count: edb_indexed_count.unwrap_or(0),
                mismatch_count: 0,
            }),
        };
    };

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
        key: "",
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

/// An audio file whose rows disagree on BPM or key, with what every row of
/// it gets. A stick exported from several rekordbox libraries has a row per
/// library for one file, in the PDB and the eDB alike; the newest of them
/// carries the newest data: the eDB row with the highest `content_id` when
/// the eDB has the file, else the PDB row with the highest id. The key is
/// the newest row's (the next newest with one, when it has none). The BPM is
/// the beat-grid tempo of the newest row's analysis file (`.DAT`, else
/// `.EXT`), else the newest row's BPM likewise. Analysis files are never
/// changed.
#[derive(Debug, Clone)]
pub(crate) struct BpmKeyMismatch {
    pub track_path: String,
    /// Tempo (x100) every row gets.
    pub tempo_x100: Option<u32>,
    /// Key every row gets.
    pub key: Option<String>,
    /// PDB rows that differ: (track id, tempo differs, key differs).
    pub pdb_rows: Vec<(u32, bool, bool)>,
    /// eDB rows that differ: (content id, BPM differs, key differs).
    pub edb_rows: Vec<(i64, bool, bool)>,
    /// What each row and the analysis file hold, for the Event Log: data,
    /// not translated.
    pub values: String,
}

/// One eDB `content` row's BPM, key and analysis file.
struct EdbBpmKeyRow {
    content_id: i64,
    bpmx100: Option<i64>,
    key: Option<String>,
    analysis_path: Option<String>,
}

/// The beat-grid tempo (x100) of the bundle at `dat`: the `.DAT`'s, else
/// the `.EXT`'s. Only the beat-grid chunks are read.
fn bundle_grid_tempo(dat: &std::path::Path) -> Option<u32> {
    [dat.to_path_buf(), dat.with_extension("EXT")]
        .iter()
        .find_map(|p| {
            crate::service::anlz::read_anlz_chunks(p, &[*b"PQTZ", *b"PQT2"])
                .and_then(|bytes| crate::service::anlz::read_beatgrid_tempo_from_anlz(&bytes))
        })
        .map(u32::from)
        .filter(|t| *t > 0)
}

/// Every audio file the PDB lists whose PDB rows, eDB rows (when an eDB
/// connection is given) and -- only with `scan_anlz` -- newest analysis
/// file disagree on BPM or key; see [`BpmKeyMismatch`] for what wins.
/// `scan_anlz` reads the analysis files off the stick, so the automatic USB
/// diagnosis (a database-only check) leaves it off and only the repair flow,
/// which scans analysis files anyway, turns it on; without it the BPM is the
/// newest row's. Key names match in any notation (`same_key_any_notation`:
/// `Db` == `C#`, `B` == `1B`): players show their own notation whatever the
/// database holds. An analysis file shared with another track
/// (`analysis_ownership`) isn't this file's to take a BPM from.
pub(crate) fn detect_bpm_key_mismatches(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb: Option<&rusqlite::Connection>,
    scan_anlz: bool,
    app_master_db_id: Option<u32>,
) -> Vec<BpmKeyMismatch> {
    use crate::service::key_notation::same_key_any_notation;
    let mut edb_by_path = HashMap::<String, Vec<EdbBpmKeyRow>>::new();
    if let Some(conn) = edb {
        let key_name = if table_exists(conn, "key") {
            "k.name"
        } else {
            "NULL"
        };
        let key_join = if table_exists(conn, "key") {
            r#"LEFT JOIN "key" k ON k.key_id = c.key_id"#
        } else {
            ""
        };
        let analysis = crate::edb::content_column_or_null(conn, "analysisDataFilePath");
        let sql = format!(
            "SELECT c.content_id, c.path, c.bpmx100, {key_name}, {analysis} FROM content c \
             {key_join} WHERE c.path IS NOT NULL"
        );
        if let Ok(mut stmt) = conn.prepare(&sql)
            && let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    EdbBpmKeyRow {
                        content_id: row.get(0)?,
                        bpmx100: row.get(2)?,
                        key: row.get(3)?,
                        analysis_path: row.get(4)?,
                    },
                ))
            })
        {
            for (path, row) in rows.filter_map(Result::ok) {
                edb_by_path.entry(path).or_default().push(row);
            }
        }
    }

    let non_owners = if scan_anlz {
        scan_analysis_ownership(usb_root, parsed, app_master_db_id, true).non_owner_track_ids()
    } else {
        HashSet::new()
    };
    let key_name = |row: &crate::pdb_reader::PdbTrackRow| {
        parsed
            .keys
            .get(&row.key_id)
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
    };
    let nonblank = |key: &Option<String>| {
        key.as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
    };
    let mut files = std::collections::BTreeMap::<&str, Vec<&crate::pdb_reader::PdbTrackRow>>::new();
    for track in &parsed.tracks {
        files
            .entry(track.track_file_path.as_str())
            .or_default()
            .push(track);
    }
    let mut out = Vec::new();
    for (path, mut pdb_rows) in files {
        // Newest first: eDB rows by content id, then PDB rows by id.
        pdb_rows.sort_by_key(|t| std::cmp::Reverse(t.id));
        let mut edb_rows = edb_by_path
            .get(path)
            .map(|rows| rows.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        edb_rows.sort_by_key(|r| std::cmp::Reverse(r.content_id));

        let newest_bundle = edb_rows
            .first()
            .and_then(|r| r.analysis_path.clone())
            .or_else(|| pdb_rows.first().map(|t| t.anlz_path.clone()))
            .filter(|p| !p.trim().is_empty());
        let grid_tempo = newest_bundle
            .filter(|_| scan_anlz)
            .filter(|bundle| {
                // Not this file's when its PDB rows there are another's.
                !pdb_rows
                    .iter()
                    .any(|t| t.anlz_path == *bundle && non_owners.contains(&t.id))
            })
            .and_then(|bundle| crate::service::usb::root::resolve_usb_side_path(usb_root, &bundle))
            .and_then(|dat| bundle_grid_tempo(&dat));
        let tempo_x100 = grid_tempo.or_else(|| {
            edb_rows
                .iter()
                .filter_map(|r| r.bpmx100.and_then(|v| u32::try_from(v).ok()))
                .chain(pdb_rows.iter().map(|t| t.tempo_x100))
                .find(|t| *t > 0)
        });
        let key = edb_rows
            .iter()
            .find_map(|r| nonblank(&r.key))
            .or_else(|| pdb_rows.iter().find_map(|t| key_name(t)));

        let tempo_differs = |value: Option<u32>| tempo_x100.is_some_and(|t| value != Some(t));
        let key_differs = |value: Option<&str>| {
            key.as_deref()
                .is_some_and(|k| !value.is_some_and(|v| same_key_any_notation(k, v)))
        };
        let pdb_wrong = pdb_rows
            .iter()
            .map(|t| {
                (
                    t.id,
                    tempo_differs((t.tempo_x100 > 0).then_some(t.tempo_x100)),
                    key_differs(key_name(t).as_deref()),
                )
            })
            .filter(|(_, tempo, key)| *tempo || *key)
            .collect::<Vec<_>>();
        let edb_wrong = edb_rows
            .iter()
            .map(|r| {
                (
                    r.content_id,
                    tempo_differs(r.bpmx100.and_then(|v| u32::try_from(v).ok())),
                    key_differs(nonblank(&r.key).as_deref()),
                )
            })
            .filter(|(_, tempo, key)| *tempo || *key)
            .collect::<Vec<_>>();
        if pdb_wrong.is_empty() && edb_wrong.is_empty() {
            continue;
        }
        let bpm =
            |t: Option<u32>| t.map_or("-".to_string(), |t| format!("{:.2}", f64::from(t) / 100.0));
        let mut values = vec![format!(
            "set to {} / {}{}",
            bpm(tempo_x100),
            key.as_deref().unwrap_or("-"),
            if grid_tempo.is_some() {
                " (BPM from the analysis grid)"
            } else {
                ""
            }
        )];
        values.extend(pdb_rows.iter().map(|t| {
            format!(
                "PDB #{} {} / {}",
                t.id,
                bpm((t.tempo_x100 > 0).then_some(t.tempo_x100)),
                key_name(t).as_deref().unwrap_or("-")
            )
        }));
        values.extend(edb_rows.iter().map(|r| {
            format!(
                "eDB #{} {} / {}",
                r.content_id,
                bpm(r.bpmx100.and_then(|v| u32::try_from(v).ok())),
                nonblank(&r.key).as_deref().unwrap_or("-")
            )
        }));
        out.push(BpmKeyMismatch {
            track_path: path.to_string(),
            tempo_x100,
            key,
            pdb_rows: pdb_wrong,
            edb_rows: edb_wrong,
            values: values.join(", "),
        });
    }
    out
}

/// A track whose analysis bundle has a beat grid in the format older app
/// versions wrote (see [`detect_outdated_beat_grids`]).
#[derive(Debug, Clone)]
pub(crate) struct OutdatedBeatGrid {
    pub(crate) track_path: String,
    pub(crate) dat: std::path::PathBuf,
}

/// The tracks whose beat grid is in a format app versions before 0.3.7 wrote
/// and some players (e.g. the XDJ-AZ) reject. The one detector the
/// diagnostics line and the repair both use. Only tracks this app exported
/// can have one, and the stick's export log says which were last exported by
/// such a version (a record without `app_version`), so only those tracks'
/// `.DAT` files are read, and of those only the `PQTZ` chunk. Its misplaced
/// header is the mark: the `.EXT` `PQT2` without its checksum was fixed
/// earlier (0.3.7-rc.3) than the header (0.3.7), so every bundle with the old
/// `PQT2` has the old header too, and the repair rewrites both. When the log
/// can't be read, every track's bundle is checked. rekordbox-made bundles
/// never match. `None` when the stick has no export log: this app never
/// exported to it.
pub(crate) fn detect_outdated_beat_grids(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
) -> Option<Vec<OutdatedBeatGrid>> {
    use crate::service::anlz::{has_misplaced_pqtz_header, read_anlz_chunks};

    // `None`: the log can't be read, so every track is a candidate.
    let candidates = match crate::service::export_log::load_export_log(usb_root) {
        Ok(Some(log)) => {
            Some(crate::service::export_log::fingerprints_last_exported_before_versioning(&log))
        }
        Ok(None) => return None,
        Err(_) => None,
    };
    let mut bundles = parsed
        .tracks
        .iter()
        .filter(|track| {
            candidates.as_ref().is_none_or(|candidates| {
                candidates.contains(&track_identity_key(&track.track_file_path, "", "", None))
            })
        })
        .filter_map(|track| {
            crate::service::usb::root::resolve_usb_side_path_lexical(usb_root, &track.anlz_path)
                .map(|dat| OutdatedBeatGrid {
                    track_path: track.track_file_path.clone(),
                    dat,
                })
        })
        .collect::<Vec<_>>();
    // Folder by folder, not in PDB order: fewer seeks on a cold stick.
    bundles.sort_by(|a, b| a.dat.cmp(&b.dat));
    bundles.retain(|bundle| {
        read_anlz_chunks(&bundle.dat, &[*b"PQTZ"])
            .is_some_and(|dat| has_misplaced_pqtz_header(&dat))
    });
    Some(bundles)
}

/// The diagnostics line for what [`detect_outdated_beat_grids`] found.
pub(super) fn diagnose_beat_grid_format(
    found: Option<&[OutdatedBeatGrid]>,
) -> (DiagStatus, crate::i18n::Text) {
    let Some(found) = found else {
        return (
            DiagStatus::Pass,
            tr::diagnostics_check_beat_grid_not_exported(),
        );
    };
    if found.is_empty() {
        return (DiagStatus::Pass, tr::diagnostics_check_beat_grid_current());
    }
    (
        DiagStatus::Warn,
        tr::diagnostics_check_beat_grid_outdated(found.len()),
    )
}

/// The "Long-track waveforms" check: tracks longer than analysis up to
/// 0.6.6 decoded (about 9 minutes) whose waveform it drew, stretched, and
/// that no repair has replaced since. Found from the export log alone. The
/// tracks themselves go to the Event Log.
pub(super) fn diagnose_long_track_waveforms(
    found: &[&crate::pdb_reader::PdbTrackRow],
) -> (DiagStatus, crate::i18n::Text) {
    if found.is_empty() {
        return (DiagStatus::Pass, tr::diagnostics_check_long_waveforms_ok());
    }
    (
        DiagStatus::Warn,
        tr::diagnostics_check_long_waveforms_stretched(found.len()),
    )
}

/// `edb_content_rows` is the eDB's `content` row count when it could be read;
/// with no tracks there are no analysis refs to miss.
/// The outer `None` of `pdb` / `edb_playlist_tracks` means the stick has no
/// such database: its check is left out rather than failed.
pub(crate) fn diagnose_analysis_integrity(
    pdb: Option<Option<&crate::pdb_reader::ParsedPdb>>,
    edb_playlist_tracks: Option<Option<&HashMap<String, Vec<UsbTrack>>>>,
    edb_content_rows: Option<usize>,
) -> DiagSection {
    let mut checks = Vec::new();

    if let Some(Some(parsed)) = pdb {
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
    } else if pdb.is_some() {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_analysis(),
            DiagStatus::Fail,
            tr::diagnostics_check_no_pdb_data(),
        ));
    }

    if let Some(Some(edb_playlist_tracks)) = edb_playlist_tracks {
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
    } else if edb_playlist_tracks.is_some() && edb_content_rows == Some(0) {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb_analysis(),
            DiagStatus::Pass,
            tr::diagnostics_check_edb_analysis_no_tracks(),
        ));
    } else if edb_playlist_tracks.is_some() {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_edb_analysis(),
            DiagStatus::Warn,
            tr::diagnostics_check_edb_analysis_no_data(),
        ));
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        key: "",
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
    F: crate::progress::ProgressFn,
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
                    key: "",
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
        .filter(|n| !n.is_folder)
        .collect();
    leaves.sort_by_key(|n| n.sort_order);
    // Grouped by path: same-named playlists in one folder are one, in
    // different folders they stay apart (as the eDB's are keyed).
    let tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
    let mut grouped_leaves = Vec::<(String, String, u32, Vec<u32>)>::new();
    let mut grouped_leaf_idx = HashMap::<String, usize>::new();
    for leaf in &leaves {
        let path = tree.path(leaf);
        let canonical = path.key();
        if let Some(existing_idx) = grouped_leaf_idx.get(&canonical).copied() {
            grouped_leaves[existing_idx].3.push(leaf.id);
            continue;
        }
        grouped_leaf_idx.insert(canonical.clone(), grouped_leaves.len());
        grouped_leaves.push((canonical, path.display(), leaf.sort_order, vec![leaf.id]));
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
            &crate::progress::count(
                tr::diagnostics_progress_resolving_playlist(display_name).render(),
                done,
                total,
            ),
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
            Num::fixed(overall_rate * 100.0, 1),
            grouped_leaves.len(),
        ),
    ));
    // The window says how many; the Event Log names each playlist
    // (`playlist_resolution_warnings`).
    if edb_only_playlist_count > 0 {
        checks.push(
            DiagCheck::new(
                tr::diagnostics_check_db_only_playlists(),
                DiagStatus::Fail,
                tr::diagnostics_check_db_only_playlists_detail(edb_only_playlist_count),
            )
            .with_link("event-log"),
        );
    }
    let unresolved = playlist_details
        .iter()
        .filter(|pd| pd.resolution_rate < 1.0 && !is_edb_only_playlist(pd))
        .collect::<Vec<_>>();
    if !unresolved.is_empty() {
        checks.push(
            DiagCheck::new(
                tr::diagnostics_check_unresolved_playlists(),
                DiagStatus::worst_of(&unresolved.iter().map(|pd| &pd.status).collect::<Vec<_>>()),
                tr::diagnostics_check_unresolved_playlists_detail(unresolved.len()),
            )
            .with_link("event-log"),
        );
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
                Num::fixed(pdb_rate * 100.0, 1),
                cross_source_pdb_total,
                Num::fixed(edb_rate * 100.0, 1),
                cross_source_edb_total,
            ),
        ));
    }
    checks.push(DiagCheck::new(
        tr::diagnostics_check_operational(),
        DiagStatus::Pass,
        tr::diagnostics_check_operational_detail(),
    ));

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (
        DiagSection {
            key: "",
            title: "Playlist Resolution".to_string(),
            status,
            checks,
            counts: None,
        },
        playlist_details,
    )
}

/// A playlist only the eDB has: the PDB lists none of its entries.
fn is_edb_only_playlist(pd: &PlaylistDiagEntry) -> bool {
    pd.pdb_entries == 0 && pd.resolution_rate < 1.0
}

/// The Event Log lines naming each playlist the Playlist Resolution checks
/// count: the eDB-only ones and those with entries whose track is missing.
pub(crate) fn playlist_resolution_warnings(details: &[PlaylistDiagEntry]) -> Vec<WarningEntry> {
    details
        .iter()
        .filter(|pd| pd.resolution_rate < 1.0)
        .map(|pd| {
            let text = if is_edb_only_playlist(pd) {
                tr::diagnostics_warning_playlist_edb_only(&pd.name, pd.edb_entries)
            } else {
                tr::diagnostics_warning_playlist_unresolved(
                    &pd.name,
                    pd.resolved_entries,
                    pd.total_entries,
                    Num::fixed(pd.resolution_rate * 100.0, 1),
                )
            };
            logging::log_text(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.playlist-resolution",
                &text,
            )
        })
        .collect()
}
