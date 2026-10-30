//! Audio and analysis-file repairs: BPM/key, beat grids, seek data, moved,
//! unindexed and missing audio.

use super::*;

/// A track whose PDB path points at a missing file, paired with the
/// unindexed file on the USB that is the same track (`relink_moved_audio`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MovedAudioRelink {
    pub(super) track_id: u32,
    /// USB-relative path the databases point at now (file missing).
    pub(super) old_path: String,
    /// USB-relative path of the unindexed file (`/Contents/...`).
    pub(super) new_path: String,
}

/// Pair missing references (`(track id, path, fingerprint)`) with unindexed
/// files (`(path, fingerprint)`). A reference's candidates are the files with
/// the same content fingerprint (size + title + artist, as export uses to
/// spot a track already on the USB); without any, the files with the same
/// file name in another folder. Only one-to-one pairs are kept: a reference
/// with several candidates, or a file claimed by several references, stays
/// unmatched, so nothing is ever merged on a guess.
pub(super) fn match_moved_audio(
    missing: &[(u32, String, Option<ContentFingerprint>)],
    unindexed: &[(String, Option<ContentFingerprint>)],
) -> Vec<MovedAudioRelink> {
    let mut proposed = Vec::<(usize, usize)>::new();
    for (mi, (_, old_path, fingerprint)) in missing.iter().enumerate() {
        let by_fingerprint = fingerprint
            .as_ref()
            .map(|fp| {
                unindexed
                    .iter()
                    .enumerate()
                    .filter(|(_, (path, other))| {
                        other.as_ref() == Some(fp) && same_format_extension(old_path, path)
                    })
                    .map(|(ui, _)| ui)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let candidates = if by_fingerprint.is_empty() {
            let name = matching_file_name(old_path);
            unindexed
                .iter()
                .enumerate()
                .filter(|(_, (path, _))| {
                    !name.is_empty()
                        && matching_file_name(path) == name
                        && same_format_extension(old_path, path)
                })
                .map(|(ui, _)| ui)
                .collect::<Vec<_>>()
        } else {
            by_fingerprint
        };
        if let [ui] = candidates[..] {
            proposed.push((mi, ui));
        }
    }
    let mut claims = HashMap::<usize, usize>::new();
    for (_, ui) in &proposed {
        *claims.entry(*ui).or_default() += 1;
    }
    proposed
        .into_iter()
        .filter(|(_, ui)| claims.get(ui) == Some(&1))
        .map(|(mi, ui)| MovedAudioRelink {
            track_id: missing[mi].0,
            old_path: missing[mi].1.clone(),
            new_path: unindexed[ui].0.clone(),
        })
        .collect()
}

/// An unindexed file that is a byte-for-byte copy of an indexed file in the
/// same folder (`remove_duplicate_audio_copies`), e.g. `Track.mp3` next to the
/// exported `Track-1.mp3`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DuplicateAudioCopy {
    /// USB-relative path of the unindexed copy (`/Contents/...`).
    pub(super) copy_path: String,
    /// USB-relative path of the indexed file it duplicates.
    pub(super) original_path: String,
}

/// Folder part of `audio_path_key`, so files compare as the presence scan
/// compares them.
fn folder_match_key(path: &str) -> String {
    let key = audio_path_key(path);
    key.rsplit_once('/')
        .map(|(folder, _)| folder.to_string())
        .unwrap_or_default()
}

/// `audio_path_key`s of the tracks in a history playlist: the PDB's history
/// entries and the eDB's `history_content` rows. Their tracks are never
/// removed by `remove_missing_audio_references`.
pub(super) fn missing_audio_history_keys(
    parsed: &ParsedPdb,
    edb_conn: Option<&rusqlite::Connection>,
) -> HashSet<String> {
    let history_ids = parsed
        .history_entries
        .iter()
        .filter_map(|e| e.track_id)
        .collect::<HashSet<_>>();
    let mut keys = parsed
        .tracks
        .iter()
        .filter(|t| history_ids.contains(&t.id))
        .map(|t| audio_path_key(&t.track_file_path))
        .collect::<HashSet<_>>();
    if let Some(conn) = edb_conn
        && table_exists(conn, "history_content")
        && table_exists(conn, "content")
        && let Ok(mut stmt) = conn.prepare(
            "SELECT c.path FROM history_content hc JOIN content c ON c.content_id = hc.content_id WHERE c.path IS NOT NULL",
        )
        && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
    {
        keys.extend(rows.flatten().map(|p| audio_path_key(&p)));
    }
    keys
}

/// The `unindexed` files that are copies of an `indexed` on-disk file in the
/// same folder: same size, then same bytes. Only same-size files are read.
pub(super) fn find_duplicate_audio_copies(
    usb_root: &Path,
    unindexed: &[String],
    indexed: &[&String],
) -> Vec<DuplicateAudioCopy> {
    let file_size = |rel: &str| {
        resolve_usb_side_path(usb_root, rel)
            .and_then(|abs| std::fs::metadata(&abs).ok().map(|m| (abs, m.len())))
    };
    let mut indexed_by_folder_size =
        HashMap::<(String, u64), Vec<(&str, std::path::PathBuf)>>::new();
    for rel in indexed {
        if let Some((abs, size)) = file_size(rel) {
            indexed_by_folder_size
                .entry((folder_match_key(rel), size))
                .or_default()
                .push((rel.as_str(), abs));
        }
    }
    unindexed
        .iter()
        .filter_map(|copy| {
            let (abs, size) = file_size(copy)?;
            let candidates = indexed_by_folder_size.get(&(folder_match_key(copy), size))?;
            let (original, _) = candidates
                .iter()
                .find(|(_, original_abs)| same_file_bytes(&abs, original_abs))?;
            Some(DuplicateAudioCopy {
                copy_path: copy.clone(),
                original_path: (*original).to_string(),
            })
        })
        .collect()
}

/// Delete unindexed copies of indexed audio files
/// (`remove_duplicate_audio_copies`). Each copy is compared with its original
/// again first, so a file that changed since the scan is kept. Returns the
/// number of files deleted.
pub(super) fn apply_remove_duplicate_audio_copies(
    usb_root: &Path,
    copies: &[DuplicateAudioCopy],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<usize> {
    let mut deleted = 0usize;
    let mut failed = Vec::<String>::new();
    for copy in copies {
        let (Some(copy_abs), Some(original_abs)) = (
            resolve_usb_side_path(usb_root, &copy.copy_path),
            resolve_usb_side_path(usb_root, &copy.original_path),
        ) else {
            failed.push(copy.copy_path.clone());
            continue;
        };
        if !same_file_bytes(&copy_abs, &original_abs) {
            failed.push(copy.copy_path.clone());
            continue;
        }
        match std::fs::remove_file(&copy_abs) {
            Ok(()) => {
                deleted += 1;
                warnings.push(logging::log(
                    Level::Info,
                    "usb-diagnostics",
                    "usb.diagnostics.duplicate-audio-removed",
                    format!("removed duplicate audio file: {}", copy.copy_path),
                ));
            }
            Err(_) => failed.push(copy.copy_path.clone()),
        }
    }
    if failed.is_empty() {
        Ok(deleted)
    } else {
        Err(BackendError::Internal(format!(
            "deleted {deleted}, kept {} that no longer match their original or could not be deleted: {}",
            failed.len(),
            failed.join(", ")
        )))
    }
}

/// Align every track's ANLZ beat grid, eDB `bpmx100` and eDB key to its PDB
/// row (`fix_bpm_key_mismatch`). Mismatches are re-detected against the
/// current staged PDB and the given eDB connection, so earlier fixes in the
/// same apply pass (e.g. strict parity rewriting the databases) are accounted
/// for and one pass leaves nothing behind. The grid is rebuilt exactly as
/// export does it -- `apply_analysis_edits_to_anlz` with the tempo only,
/// reusing the bundle's own first-beat anchor and leaving cues, waveforms and
/// `PSSI` as they are. Returns (ANLZ bundles rewritten, eDB rows updated).
pub(super) fn apply_bpm_key_mismatch_repair(
    usb_root: &std::path::Path,
    app_master_db_id: Option<u32>,
    edb_conn: Option<&mut rusqlite::Connection>,
    warnings: &mut Vec<WarningEntry>,
    on_item: &mut impl FnMut(usize, usize),
) -> BackendResult<(usize, usize)> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let mut handle = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings));
    let mismatches: Vec<BpmKeyMismatch> = detect_bpm_key_mismatches(
        usb_root,
        &parsed,
        handle.as_mut().map(|h| &*h.conn()),
        true,
        app_master_db_id,
    );

    let edb_targets: Vec<&BpmKeyMismatch> = mismatches
        .iter()
        .filter(|m| m.edb_bpmx100.is_some() || m.pdb_key_for_edb.is_some())
        .collect();
    let total = mismatches.len() + edb_targets.len();
    let mut anlz_fixed = 0usize;
    for (index, m) in mismatches.iter().enumerate() {
        let Some(dat) = m.anlz_dat_path.as_deref() else {
            on_item(index + 1, total);
            continue;
        };
        // The grid's beat count comes from the track length: the PDB's, else
        // the audio file's own. Never a made-up one.
        let Some(duration_ms) = m.duration_ms.or_else(|| {
            detect_track_duration_ms(&usb_root.join(m.track_path.trim_start_matches('/')))
        }) else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.bpm-key.duration-unknown",
                format!(
                    "beat grid not rebuilt: track length unknown for {}",
                    m.track_path
                ),
            ));
            on_item(index + 1, total);
            continue;
        };
        let edits = AnlzAnalysisEdits {
            bpm: Some(f64::from(m.pdb_tempo_x100) / 100.0),
            duration_ms: Some(duration_ms),
            first_beat_ms: None,
            cues: None,
        };
        for path in [dat.to_path_buf(), dat.with_extension("EXT")] {
            if let Ok(bytes) = std::fs::read(&path) {
                let rebuilt = apply_analysis_edits_to_anlz(&bytes, &edits);
                if rebuilt != bytes {
                    atomic_write_bytes(&path, &rebuilt)?;
                }
            }
        }
        anlz_fixed += 1;
        on_item(index + 1, total);
    }

    let mut edb_fixed = 0usize;
    if !edb_targets.is_empty() {
        let Some(handle) = handle.as_mut() else {
            return Err(BackendError::Internal(
                "unable to open eDB to align content BPM/key".to_string(),
            ));
        };
        let tx = handle.conn().transaction()?;
        for (index, m) in edb_targets.into_iter().enumerate() {
            let mut changed = false;
            if m.edb_bpmx100.is_some() {
                changed |= tx.execute(
                    "UPDATE content SET bpmx100 = ?1 WHERE path = ?2",
                    params![i64::from(m.pdb_tempo_x100), m.track_path],
                )? > 0;
            }
            if let Some(key) = m.pdb_key_for_edb.as_deref() {
                let key_id = find_key_id_by_name(&tx, Some(key))?;
                changed |= tx.execute(
                    "UPDATE content SET key_id = ?1 WHERE path = ?2",
                    params![key_id, m.track_path],
                )? > 0;
            }
            if changed {
                edb_fixed += 1;
            }
            on_item(mismatches.len() + index + 1, total);
        }
        tx.commit()?;
    }
    Ok((anlz_fixed, edb_fixed))
}

/// Add the seek index rekordbox writes to a bundle the repair just
/// generated: `PVBR` in an MP3's `.DAT` (as local analysis does), `PVB2` in a
/// FLAC's `.EXT` (local analysis leaves that to the USB repair). Without it the
/// seek-data fixes would find the new bundle on the next pass. A file whose
/// index can't be built keeps none; those fixes list it as unsupported.
fn add_seek_index_to_new_bundle(dat: &Path, audio: &Path) {
    match SeekFormat::of(&audio.to_string_lossy()) {
        Some(SeekFormat::Mp3) => {
            let _ = fill_mp3_pvbr(dat, audio);
        }
        Some(SeekFormat::Flac) => {
            let ext = dat.with_extension("EXT");
            if let (Ok(Some(index)), Ok(bytes)) = (seek_index_for_audio(audio), std::fs::read(&ext))
                && let Some(patched) = with_seek_index(&bytes, &index)
            {
                let _ = atomic_write_bytes(&ext, &patched);
            }
        }
        None => {}
    }
}

/// Put `track_path` into a bundle's `PPTH` (`.DAT`, `.EXT`, `.2EX`).
fn stamp_bundle_ppth(dat: &Path, track_path: &str) -> BackendResult<()> {
    for file in [
        dat.to_path_buf(),
        dat.with_extension("EXT"),
        dat.with_extension("2EX"),
    ] {
        let Ok(data) = std::fs::read(&file) else {
            continue;
        };
        let patched = ensure_ppth_chunk(&data, track_path);
        if patched != data {
            atomic_write_bytes(&file, &patched)?;
        }
    }
    Ok(())
}

/// Name each track in its own bundle (`add_analysis_track_path`): the
/// bundles only one track uses whose `PPTH` is missing or stale, re-detected
/// from the staged PDB. Returns the number of bundles stamped.
pub(super) fn apply_add_analysis_track_path(
    usb_root: &Path,
    app_master_db_id: Option<u32>,
) -> BackendResult<usize> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let ownership = scan_analysis_ownership(usb_root, &parsed, app_master_db_id, true);
    for bundle in &ownership.unmarked {
        stamp_bundle_ppth(&bundle.dat, &bundle.track_path)?;
    }
    Ok(ownership.unmarked.len())
}

/// Give every track that shares an analysis bundle its own
/// (`separate_shared_analysis`), re-detected from the staged PDB. The owner
/// (`scan_analysis_ownership`) keeps the bundle and gets its `PPTH`. Each
/// other track gets the next free `ANLZnnnn` in its own hash folder
/// (`analysis_bundle_paths_for_track`), generated from its audio as the
/// empty-analysis fix does, with the PDB tempo; its PDB row and eDB content
/// row then point there. Returns (tracks given their own bundle, bundles kept
/// by an owner); a track whose audio can't be analysed fails the fix after
/// the others are done.
pub(super) fn apply_separate_shared_analysis(
    usb_root: &Path,
    app_master_db_id: Option<u32>,
    edb_conn: Option<&mut rusqlite::Connection>,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(usize, usize)> {
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let page_size = pdb_page_size(&bytes)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes)?;
    let ownership = scan_analysis_ownership(usb_root, &parsed, app_master_db_id, true);
    let row_by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
        parsed.tracks.iter().map(|t| (t.id, t)).collect();

    let mut kept = 0usize;
    let mut mutations = Vec::<crate::pdb_writer::PdbTrackRowMutation>::new();
    // (track path, new USB-relative `.DAT` path) for the eDB.
    let mut moved = Vec::<(String, String)>::new();
    let mut failed = Vec::<String>::new();
    for bundle in &ownership.shared {
        // The owner's PPTH first, so its file is never picked as free below.
        if let Some(owner) = bundle.owner.and_then(|id| row_by_id.get(&id)) {
            stamp_bundle_ppth(&bundle.dat, &owner.track_file_path)?;
            kept += 1;
        }
        for row in bundle
            .track_ids
            .iter()
            .filter(|id| Some(**id) != bundle.owner)
            .filter_map(|id| row_by_id.get(id))
        {
            let audio =
                resolve_usb_side_path(usb_root, &row.track_file_path).filter(|path| path.is_file());
            let duration_ms = row
                .duration_seconds
                .filter(|s| *s > 0)
                .map(|s| u64::from(s) * 1000)
                .or_else(|| audio.as_deref().and_then(detect_track_duration_ms));
            let waveform = audio
                .as_deref()
                .and_then(|audio| {
                    build_waveform_preview_from_audio(
                        audio,
                        crate::service::WAVEFORM_PREVIEW_BINS,
                        2_000_000,
                    )
                    .ok()
                })
                .filter(|w| !w.peaks.is_empty());
            let (Some(audio), Some(duration_ms), Some(waveform)) = (audio, duration_ms, waveform)
            else {
                failed.push(row.track_file_path.clone());
                continue;
            };
            let (dat, ext, twoex) = analysis_bundle_paths_for_track(usb_root, &row.track_file_path);
            write_generated_anlz_bundle(
                &waveform,
                &AnlzBundlePaths {
                    dat_path: dat.clone(),
                    ext_path: ext,
                    twoex_path: twoex,
                },
                &row.track_file_path,
                (row.tempo_x100 > 0).then(|| f64::from(row.tempo_x100) / 100.0),
                duration_ms,
            )?;
            add_seek_index_to_new_bundle(&dat, &audio);
            let Some(rel) = usb_relative_path(usb_root, &dat) else {
                failed.push(row.track_file_path.clone());
                continue;
            };
            let mut data = track_row_data_from_reader_row(row);
            data.anlz_path = rel.clone();
            mutations.push(crate::pdb_writer::PdbTrackRowMutation {
                row: data,
                changed_fields: vec!["anlz_path"],
            });
            warnings.push(logging::log(
                Level::Info,
                "usb-repair",
                "usb.repair.shared-analysis.separated",
                format!("own analysis file for {}: {rel}", row.track_file_path),
            ));
            moved.push((row.track_file_path.clone(), rel));
        }
    }

    if !mutations.is_empty() {
        crate::pdb_writer::mutate_tracks_in_place(
            &mut bytes,
            &mutations,
            PdbLayoutProfile::DEFAULT,
            page_size,
        )?;
        let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&bytes);
        if !mismatches.is_empty() {
            let detail = mismatches
                .iter()
                .take(8)
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(BackendError::Validation(format!(
                "separating shared analysis blocked: page-header convention mismatches ({detail})"
            )));
        }
        std::fs::write(&pdb_path, &bytes)?;
    }

    if !moved.is_empty()
        && let Some(mut handle) =
            EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
        && table_exists(handle.conn(), "content")
    {
        let new_by_key = moved
            .iter()
            .map(|(track, rel)| (audio_path_key(track), rel))
            .collect::<HashMap<_, _>>();
        let tx = handle.conn().transaction()?;
        let targets = {
            let mut stmt =
                tx.prepare("SELECT content_id, path FROM content WHERE path IS NOT NULL")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut targets = Vec::<(i64, String)>::new();
            for row in rows {
                let (content_id, path) = row?;
                if let Some(rel) = new_by_key.get(&audio_path_key(&path)) {
                    targets.push((content_id, (*rel).clone()));
                }
            }
            targets
        };
        for (content_id, rel) in targets {
            tx.execute(
                "UPDATE content SET analysisDataFilePath = ?1 WHERE content_id = ?2",
                params![rel, content_id],
            )?;
        }
        tx.commit()?;
    }

    if failed.is_empty() {
        Ok((moved.len(), kept))
    } else {
        Err(BackendError::Internal(format!(
            "gave {} a new analysis file; could not analyse the audio of: {}",
            counted(moved.len(), "track", "tracks"),
            failed.join(", ")
        )))
    }
}

/// `.DAT` files under `USBANLZ` whose beat grid is in the format the app
/// wrote before 0.3.7 (`fix_beat_grid_header`): a `PQTZ` header with its
/// value two bytes early, or an `.EXT` `PQT2` whose checksum doesn't match the
/// `PQTZ` beats. rekordbox's own bundles never match. The files are found on
/// disk, not through the PDB, so a bundle the database doesn't point at yet is
/// still fixed.
pub(super) fn detect_app_beat_grids(usb_root: &Path) -> Vec<std::path::PathBuf> {
    walkdir::WalkDir::new(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("DAT"))
        })
        .filter(|dat| {
            let Ok(dat_bytes) = std::fs::read(dat) else {
                return false;
            };
            has_misplaced_pqtz_header(&dat_bytes)
                || std::fs::read(dat.with_extension("EXT"))
                    .is_ok_and(|ext| pqt2_checksum_ok(&dat_bytes, &ext) == Some(false))
        })
        .collect()
}

/// Rewrite the grids found by [`detect_app_beat_grids`] in rekordbox's
/// format: `PQTZ` in the `.DAT` and `PQT2` in the `.EXT`, both from the grid
/// the `.DAT` already holds. Re-detected so earlier fixes in the same pass are
/// accounted for. A grid that isn't one constant tempo is left alone with a
/// warning. Returns the number of bundles written.
pub(super) fn apply_beat_grid_repair(
    usb_root: &Path,
    warnings: &mut Vec<WarningEntry>,
    on_item: &mut impl FnMut(usize, usize),
) -> BackendResult<usize> {
    let mut fixed = 0usize;
    let targets = detect_app_beat_grids(usb_root);
    let total = targets.len();
    for (index, dat) in targets.into_iter().enumerate() {
        match upgrade_bundle_beat_grid(&dat)? {
            GridUpgrade::Rewritten => fixed += 1,
            GridUpgrade::AlreadyCurrent => {}
            GridUpgrade::NotRebuildable => warnings.push(logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.beat-grid.skipped",
                format!(
                    "beat grid not rewritten (not a constant-tempo grid): {}",
                    dat.display()
                ),
            )),
        }
        on_item(index + 1, total);
    }
    Ok(fixed)
}

/// The two seek indexes rekordbox writes: `PVBR` in an MP3's `.DAT`, `PVB2`
/// in a FLAC's `.EXT`. Each has its own fix, so the exactly reproduced MP3
/// one (`add_missing_mp3_seek_data`) can be applied without the FLAC one
/// (`add_missing_flac_seek_data`), whose long-track rule matches about 98% of
/// rekordbox's entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SeekFormat {
    Mp3,
    Flac,
}

impl SeekFormat {
    pub(super) fn of(path: &str) -> Option<Self> {
        match crate::audio_format::AudioFormat::of_path_str(path)? {
            crate::audio_format::AudioFormat::Mp3 => Some(Self::Mp3),
            crate::audio_format::AudioFormat::Flac => Some(Self::Flac),
            _ => None,
        }
    }

    pub(super) fn fix_id(self) -> &'static str {
        match self {
            Self::Mp3 => MISSING_MP3_SEEK_DATA_FIX_ID,
            Self::Flac => MISSING_FLAC_SEEK_DATA_FIX_ID,
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Mp3 => "Add Missing MP3 Seek Data",
            Self::Flac => "Add Missing FLAC Seek Data",
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
        }
    }
}

/// A track whose USB bundle lacks the seek index rekordbox writes for its
/// format: an MP3 whose `.DAT` `PVBR` has a zero total, or a FLAC whose `.EXT`
/// has no `PVB2`. rekordbox's own bundles always carry both, so they never
/// match. Only the bundle is read here; the audio is parsed when the fix is
/// applied.
#[derive(Debug, Clone)]
pub(super) struct MissingSeekData {
    pub(super) track_path: String,
    audio_path: std::path::PathBuf,
    /// The `.DAT` for an MP3, the `.EXT` for a FLAC.
    bundle_path: std::path::PathBuf,
}

pub(super) fn detect_missing_seek_data(
    usb_root: &Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    format: SeekFormat,
) -> Vec<MissingSeekData> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for track in &parsed.tracks {
        if SeekFormat::of(&track.track_file_path) != Some(format) {
            continue;
        }
        let Some(dat) = resolve_usb_side_path(usb_root, &track.anlz_path) else {
            continue;
        };
        let Some(audio_path) =
            resolve_usb_side_path(usb_root, &track.track_file_path).filter(|path| path.is_file())
        else {
            continue;
        };
        let bundle_path = match format {
            SeekFormat::Mp3 => dat,
            SeekFormat::Flac => dat.with_extension("EXT"),
        };
        let Ok(bytes) = std::fs::read(&bundle_path) else {
            continue;
        };
        let missing = match format {
            SeekFormat::Mp3 => pvbr_total_samples(&bytes) == Some(0),
            SeekFormat::Flac => has_pvb2_chunk(&bytes) == Some(false),
        };
        if missing && seen.insert(bundle_path.clone()) {
            out.push(MissingSeekData {
                track_path: track.track_file_path.clone(),
                audio_path,
                bundle_path,
            });
        }
    }
    out
}

/// Split [`detect_missing_seek_data`]'s targets into the ones the fix can
/// write an index for and the ones it would skip, each with the skip reason
/// and advice the apply step logs. Builds the index the apply step builds, so
/// a file the fix always skips is never proposed as fixable.
pub(super) fn partition_fixable_seek_data(
    targets: Vec<MissingSeekData>,
) -> (Vec<MissingSeekData>, Vec<(MissingSeekData, String)>) {
    let mut fixable = Vec::new();
    let mut unfixable = Vec::new();
    for target in targets {
        let skip = match seek_index_for_audio(&target.audio_path) {
            Ok(Some(index)) => std::fs::read(&target.bundle_path)
                .ok()
                .and_then(|bytes| with_seek_index(&bytes, &index))
                .is_none()
                .then(|| format!("analysis file could not be parsed; {SEEK_SKIP_REPORT_HINT}")),
            Ok(None) => Some("not an MP3 or FLAC".to_string()),
            Err(skip) => Some(format!("{}; {}", skip.describe(), skip.advice())),
        };
        match skip {
            None => fixable.push(target),
            Some(reason) => unfixable.push((target, reason)),
        }
    }
    (fixable, unfixable)
}

/// Write the missing seek indexes of one format found by
/// [`detect_missing_seek_data`], re-detected so bundles regenerated earlier in
/// the same pass are covered. A file that doesn't parse cleanly is skipped and
/// named in the Event Log with its reason, so it can be reported; nothing is
/// ever approximated. Returns (bundles written, tracks skipped).
pub(super) fn apply_missing_seek_data_repair(
    usb_root: &Path,
    format: SeekFormat,
    warnings: &mut Vec<WarningEntry>,
    on_item: &mut impl FnMut(usize, usize),
) -> BackendResult<(usize, usize)> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let mut fixed = 0usize;
    let mut skipped = 0usize;
    let targets = detect_missing_seek_data(usb_root, &parsed, format);
    let total = targets.len();
    for (item_index, target) in targets.into_iter().enumerate() {
        let (reason, advice) = match seek_index_for_audio(&target.audio_path) {
            Ok(Some(index)) => {
                let bytes = std::fs::read(&target.bundle_path)?;
                match with_seek_index(&bytes, &index) {
                    Some(patched) => {
                        atomic_write_bytes(&target.bundle_path, &patched)?;
                        fixed += 1;
                        on_item(item_index + 1, total);
                        continue;
                    }
                    None => ("analysis file could not be parsed", SEEK_SKIP_REPORT_HINT),
                }
            }
            Ok(None) => {
                on_item(item_index + 1, total);
                continue;
            }
            Err(skip) => (skip.describe(), skip.advice()),
        };
        skipped += 1;
        warnings.push(logging::log(
            Level::Warn,
            "usb-repair",
            "usb.repair.seek-data.skipped",
            format!(
                "{} seek data not added ({reason}): {}; {advice}",
                format.name(),
                target.track_path,
            ),
        ));
        on_item(item_index + 1, total);
    }
    Ok((fixed, skipped))
}

/// Lookup key for an analysis file or folder on the USB: its whole path with
/// `/` separators, lowercased because the stick's filesystem ignores case.
fn analysis_path_key(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

impl BackendService {
    pub(super) fn apply_fix_empty_analysis_files(
        &self,
        usb_root: &std::path::Path,
        empty_analysis_paths: &[std::path::PathBuf],
        edb_conn: Option<&mut rusqlite::Connection>,
        parsed_pdb: Option<&crate::pdb_reader::ParsedPdb>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize, usize, usize)> {
        if empty_analysis_paths.is_empty() {
            return Ok((0, 0, 0, 0));
        }

        #[derive(Clone)]
        struct AnalysisRepairTarget {
            source_audio: std::path::PathBuf,
            analysis_dir: std::path::PathBuf,
            track_path: String,
            // Known-good tempo/duration from the already-parsed PDB row, so a
            // regenerated bundle bakes in the track's real beat grid (with no
            // known tempo it gets no beat grid at all; with no known length the
            // source file's own length is used; see docs/DIAGNOSTICS_REPAIRS.md).
            bpm: Option<f64>,
            duration_ms: Option<u64>,
        }

        let mut map_by_file = std::collections::HashMap::<String, AnalysisRepairTarget>::new();
        let mut map_by_dir = std::collections::HashMap::<String, AnalysisRepairTarget>::new();
        // Reuse the caller's already-parsed PDB when given -- this fix always
        // runs before anything that mutates the PDB, so it's still current.
        // Only stage/parse our own copy if the caller didn't have one.
        let owned_parsed_pdb;
        let parsed_pdb_ref: Option<&crate::pdb_reader::ParsedPdb> = match parsed_pdb {
            Some(parsed) => Some(parsed),
            None => {
                owned_parsed_pdb = usb_staging::stage_pdb(usb_root)
                    .ok()
                    .and_then(|staged_pdb_path| parse_pdb(&staged_pdb_path).ok());
                owned_parsed_pdb.as_ref()
            }
        };
        if let Some(parsed) = parsed_pdb_ref {
            for t in &parsed.tracks {
                if let (Some(a), Some(s)) = (
                    resolve_usb_side_path(usb_root, &t.anlz_path),
                    resolve_usb_side_path(usb_root, &t.track_file_path),
                ) {
                    let analysis_dir = a
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| usb_root.to_path_buf());
                    let target = AnalysisRepairTarget {
                        source_audio: s,
                        analysis_dir: analysis_dir.clone(),
                        track_path: t.track_file_path.clone(),
                        bpm: (t.tempo_x100 > 0).then(|| t.tempo_x100 as f64 / 100.0),
                        duration_ms: t
                            .duration_seconds
                            .filter(|s| *s > 0)
                            .map(|s| s as u64 * 1000),
                    };
                    map_by_file.insert(analysis_path_key(&a), target.clone());
                    map_by_dir.insert(analysis_path_key(&analysis_dir), target);
                }
            }
        }

        if let Some(mut handle) =
            EdbConnHandle::acquire(edb_conn, || open_edb_from_usb_root(usb_root, warnings))
        {
            let conn = handle.conn();
            let mut stmt = conn.prepare(
                "SELECT path, analysisDataFilePath, bpmx100, length FROM content \
                 WHERE analysisDataFilePath IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })?;
            for row in rows {
                let (path, analysis_path, bpmx100, length_seconds) = row?;
                if let (Some(p), Some(a)) = (path.as_deref(), analysis_path.as_deref())
                    && let (Some(ra), Some(rp)) = (
                        resolve_usb_side_path(usb_root, a),
                        resolve_usb_side_path(usb_root, p),
                    )
                {
                    let analysis_dir = ra
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| usb_root.to_path_buf());
                    let target = AnalysisRepairTarget {
                        source_audio: rp,
                        analysis_dir: analysis_dir.clone(),
                        track_path: p.to_string(),
                        bpm: bpmx100.filter(|v| *v > 0).map(|v| v as f64 / 100.0),
                        duration_ms: length_seconds.filter(|v| *v > 0).map(|v| v as u64 * 1000),
                    };
                    map_by_file
                        .entry(analysis_path_key(&ra))
                        .or_insert_with(|| target.clone());
                    map_by_dir
                        .entry(analysis_path_key(&analysis_dir))
                        .or_insert(target);
                }
            }
        }

        let mut fixed = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        let mut writes = 0usize;

        for empty_path in empty_analysis_paths {
            let empty_path_text = sanitize_warning_path(empty_path);
            let key_file = analysis_path_key(empty_path);
            let key_dir = empty_path
                .parent()
                .map(analysis_path_key)
                .unwrap_or_default();
            let target = map_by_file
                .get(&key_file)
                .cloned()
                .or_else(|| map_by_dir.get(&key_dir).cloned());
            let Some(target) = target else {
                skipped += 1;
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.empty-analysis.no-mapping",
                    format!("repair skipped (empty analysis): source audio mapping not found for {empty_path_text}"),
                ));
                continue;
            };
            let source_audio = target.source_audio;
            if !source_audio.is_file() {
                skipped += 1;
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.empty-analysis.source-missing",
                    format!("repair skipped (empty analysis): source audio missing for {empty_path_text} -> {}", source_audio.display()),
                ));
                continue;
            }
            let waveform = build_waveform_preview_from_audio(
                &source_audio,
                crate::service::WAVEFORM_PREVIEW_BINS,
                2_000_000,
            )
            .unwrap_or_else(|_| WaveformData::empty());
            if waveform.peaks.is_empty() {
                failed += 1;
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.empty-analysis.analyze-failed",
                    format!(
                        "repair failed (empty analysis): unable to analyze source audio {}",
                        source_audio.display()
                    ),
                ));
                continue;
            }
            let Some(duration_ms) = target
                .duration_ms
                .or_else(|| detect_track_duration_ms(&source_audio))
            else {
                failed += 1;
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.empty-analysis.duration-unknown",
                    format!(
                        "repair failed (empty analysis): track length unknown for {}",
                        source_audio.display()
                    ),
                ));
                continue;
            };
            let base_dir = target.analysis_dir;
            let bundle_paths = AnlzBundlePaths {
                dat_path: base_dir.join("ANLZ0000.DAT"),
                ext_path: base_dir.join("ANLZ0000.EXT"),
                twoex_path: base_dir.join("ANLZ0000.2EX"),
            };
            if let Err(err) = write_generated_anlz_bundle(
                &waveform,
                &bundle_paths,
                &target.track_path,
                target.bpm,
                duration_ms,
            ) {
                failed += 1;
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.empty-analysis.write-failed",
                    format!("repair failed (empty analysis): {empty_path_text}: {err}"),
                ));
                continue;
            }
            // The seek index rekordbox writes (MP3 PVBR, FLAC PVB2).
            add_seek_index_to_new_bundle(&bundle_paths.dat_path, &source_audio);
            fixed += 1;
            writes += 3;
        }

        Ok((fixed, skipped, failed, writes))
    }

    /// Point tracks whose file is missing at the same track found elsewhere
    /// on the USB (`relink_moved_audio`). The PDB row and eDB content row keep
    /// their ids, so playlists, history, cues and analysis stay attached;
    /// only the path, file name and size change. The analysis bundle stays
    /// where it is, with its embedded path (`PPTH`) updated to the new file.
    /// Returns the number of tracks relinked.
    pub(super) fn apply_fix_relink_moved_audio(
        &self,
        usb_root: &std::path::Path,
        relinks: &[MovedAudioRelink],
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<usize> {
        let file_facts = |rel: &str| {
            let rel = rel.replace('\\', "/");
            let size = resolve_usb_side_path(usb_root, &rel)
                .and_then(|abs| std::fs::metadata(abs).ok())
                .and_then(|m| u32::try_from(m.len()).ok());
            let name = rel.rsplit('/').next().unwrap_or_default().to_string();
            (rel, name, size)
        };

        // ── PDB: re-encode each track row with its new path ──────────
        let pdb_path = usb_staging::stage_pdb(usb_root)?;
        let mut bytes = std::fs::read(&pdb_path)?;
        let page_size = pdb_page_size(&bytes)?;
        let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes)?;
        let row_by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
            parsed.tracks.iter().map(|t| (t.id, t)).collect();
        let mut mutations = Vec::<crate::pdb_writer::PdbTrackRowMutation>::new();
        let mut bundles = Vec::<(String, String)>::new();
        for relink in relinks {
            let Some(row) = row_by_id.get(&relink.track_id) else {
                continue;
            };
            let (rel, name, size) = file_facts(&relink.new_path);
            let mut data = track_row_data_from_reader_row(row);
            data.file_path = rel.clone();
            data.file_name = Some(name);
            data.file_size_bytes = size.or(data.file_size_bytes);
            mutations.push(crate::pdb_writer::PdbTrackRowMutation {
                row: data,
                changed_fields: vec!["file_path", "file_name", "file_size_bytes"],
            });
            if !row.anlz_path.is_empty() {
                bundles.push((row.anlz_path.clone(), rel));
            }
        }
        let relinked = crate::pdb_writer::mutate_tracks_in_place(
            &mut bytes,
            &mutations,
            PdbLayoutProfile::DEFAULT,
            page_size,
        )?;
        let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&bytes);
        if !mismatches.is_empty() {
            let detail = mismatches
                .iter()
                .take(8)
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(BackendError::Validation(format!(
                "Relink blocked: page-header convention mismatches ({detail})"
            )));
        }
        std::fs::write(&pdb_path, &bytes)?;

        // ── ANLZ: the bundle names the file it belongs to ─────────────
        for (anlz_path, rel) in &bundles {
            let Some(dat) = resolve_usb_side_path(usb_root, anlz_path) else {
                continue;
            };
            for file in [
                dat.clone(),
                dat.with_extension("EXT"),
                dat.with_extension("2EX"),
            ] {
                let Ok(data) = std::fs::read(&file) else {
                    continue;
                };
                let patched = ensure_ppth_chunk(&data, rel);
                if patched != data {
                    atomic_write_bytes(&file, &patched)?;
                }
            }
        }

        // ── eDB: same content row, new path ───────────────────────────
        let Some(mut handle) = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
        else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.moved-audio.edb-open-failed",
                "relink moved audio: eDB unavailable, only the PDB was updated",
            ));
            return Ok(relinked);
        };
        let conn = handle.conn();
        if table_exists(conn, "content") {
            let columns = load_table_columns(conn, "content")?;
            let has = |c: &str| columns.iter().any(|col| col == c);
            let new_by_old = relinks
                .iter()
                .map(|r| (normalize_pdb_path_for_edb_lookup(&r.old_path), &r.new_path))
                .collect::<HashMap<_, _>>();
            let tx = conn.transaction()?;
            let targets = {
                let mut stmt =
                    tx.prepare("SELECT content_id, path FROM content WHERE path IS NOT NULL")?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut targets = Vec::<(i64, String)>::new();
                for row in rows {
                    let (content_id, path) = row?;
                    if let Some(new_path) =
                        new_by_old.get(&normalize_pdb_path_for_edb_lookup(&path))
                    {
                        targets.push((content_id, (*new_path).clone()));
                    }
                }
                targets
            };
            for (content_id, new_path) in targets {
                let (rel, name, size) = file_facts(&new_path);
                tx.execute(
                    "UPDATE content SET path = ?1 WHERE content_id = ?2",
                    params![rel, content_id],
                )?;
                if has("fileName") {
                    tx.execute(
                        "UPDATE content SET fileName = ?1 WHERE content_id = ?2",
                        params![name, content_id],
                    )?;
                }
                if has("fileSize")
                    && let Some(size) = size
                {
                    tx.execute(
                        "UPDATE content SET fileSize = ?1 WHERE content_id = ?2",
                        params![i64::from(size), content_id],
                    )?;
                }
            }
            tx.commit()?;
        }
        Ok(relinked)
    }

    /// Index audio files that sit under `Contents/` but are in neither
    /// database (`add_unindexed_audio_playlist`), typically left behind when
    /// the databases were restored from a backup taken before an export. The
    /// files are already on the stick, so this is an additive export of a
    /// USB-only playlist: tags are read from the files, BPM/first beat/cues
    /// from each file's canonical ANLZ bundle (generated from the audio when
    /// there is none), and the eDB + PDB writers append the tracks to
    /// `UNINDEXED_AUDIO_PLAYLIST_NAME`. Returns (tracks added, bundles
    /// generated, tracks left without an analysis bundle).
    pub(super) fn apply_fix_add_unindexed_audio_playlist(
        &self,
        usb_root: &std::path::Path,
        files: &[String],
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize, usize)> {
        let local_conn = self.db.connect()?;
        Self::ensure_track_export_identity_schema(&local_conn)?;
        let app_master_db_id = Self::ensure_local_u32_setting(
            &local_conn,
            SETTING_EXPORT_MASTER_DB_ID,
            "master-db-id",
        )?;

        let mut manifest_tracks = Vec::<ExportManifestTrack>::new();
        let mut analysed = 0usize;
        let mut without_analysis = 0usize;
        for rel in files {
            // The ANLZ path hash and the DB rows both expect `/Contents/...`.
            let rel = &rel.replace('\\', "/");
            let Some(abs) = resolve_usb_side_path(usb_root, rel) else {
                continue;
            };
            let scanned = match scan_audio_file(&abs) {
                Ok(scanned) => scanned,
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.unindexed-audio.read-failed",
                        format!("unindexed audio skipped, unreadable: {rel}: {err}"),
                    ));
                    continue;
                }
            };

            // Its own bundle, or the next free `ANLZnnnn`: never another track's.
            let (dat_path, ext_path, twoex_path) = analysis_bundle_paths_for_track(usb_root, rel);
            let duration_ms = detect_track_duration_ms(&abs);
            // No bundle on the USB: analyse the file as the empty-analysis fix
            // does, so the track has its waveform and analysis path like any
            // exported one.
            if !dat_path.is_file()
                && let Some(duration_ms) = duration_ms
            {
                let waveform = build_waveform_preview_from_audio(
                    &abs,
                    crate::service::WAVEFORM_PREVIEW_BINS,
                    2_000_000,
                )
                .unwrap_or_else(|_| WaveformData::empty());
                let bundle_paths = AnlzBundlePaths {
                    dat_path: dat_path.clone(),
                    ext_path: ext_path.clone(),
                    twoex_path,
                };
                if !waveform.peaks.is_empty()
                    && write_generated_anlz_bundle(&waveform, &bundle_paths, rel, None, duration_ms)
                        .is_ok()
                {
                    add_seek_index_to_new_bundle(&dat_path, &abs);
                    analysed += 1;
                }
            }
            let (waveform_path, bpm, first_beat_ms, cues) = if dat_path.is_file() {
                let dat = std::fs::read(&dat_path).unwrap_or_default();
                let cue_bytes = std::fs::read(&ext_path).unwrap_or_else(|_| dat.clone());
                (
                    usb_relative_path(usb_root, &dat_path),
                    read_beatgrid_tempo_from_anlz(&dat).map(|t| f64::from(t) / 100.0),
                    read_first_beat_from_anlz(&dat),
                    collapse_anlz_cues(&cue_bytes),
                )
            } else {
                without_analysis += 1;
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.unindexed-audio.no-analysis",
                    format!("unindexed audio has no analysis files on the USB: {rel}"),
                ));
                (None, None, None, Vec::new())
            };

            // Identity keyed by the USB path, so re-running the fix on the
            // same file gets the same content id. Our master_db_id makes
            // desktop DJ software read the bundle from the USB, not its cache.
            let identity_key = format!("usb-file:{rel}");
            let (master_db_id, master_content_id, content_link) = Self::resolve_manifest_identity(
                &local_conn,
                &identity_key,
                None,
                true,
                app_master_db_id,
                APP_CONTENT_LINK_ID,
            )?;
            let file_timestamp_at = scanned.file_timestamp();
            manifest_tracks.push(ExportManifestTrack {
                date_added: None,
                date_created: None,
                label: None,
                remixer: None,
                composer: None,
                from_library: false,
                pdb_track_id: None,
                edb_content_id: None,
                id: identity_key,
                master_db_id,
                master_content_id,
                content_link,
                position: manifest_tracks.len() + 1,
                track_number: scanned.track_number,
                title: scanned.title,
                artist: scanned.artist,
                album: scanned.album,
                bpm,
                key: scanned.tonality,
                source_path: rel.clone(),
                exported_path: rel.clone(),
                file_timestamp_at,
                file_size_bytes: scanned.file_size_bytes,
                sample_rate_hz: scanned.sample_rate_hz,
                bit_depth: scanned.bit_depth,
                bitrate_kbps: scanned.bitrate_kbps,
                disc_number: scanned.disc_number,
                subtitle: scanned.subtitle,
                comment: scanned.comment,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: scanned.genre,
                label_id: None,
                isrc: scanned.isrc,
                release_year: scanned.release_year,
                release_date: scanned.release_date,
                recorded_date: scanned.recorded_date,
                file_type: scanned
                    .format_ext
                    .as_deref()
                    .map(crate::audio_format::rekordbox_file_type),
                owns_exported_media: false,
                owns_artwork: false,
                owns_waveform: false,
                artwork_path: None,
                waveform_path,
                duration_ms,
                first_beat_ms,
                cues,
            });
        }
        if manifest_tracks.is_empty() {
            return Ok((0, analysed, without_analysis));
        }

        let playlist = ExportPlaylistData {
            id: "usb-unindexed-audio".to_string(),
            name: UNINDEXED_AUDIO_PLAYLIST_NAME.to_string(),
            tracks: Vec::new(),
        };
        let manifest = ExportManifest {
            generated_at: crate::service::now(),
            playlist_id: playlist.id.clone(),
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

        // Same order as export: eDB first, then the PDB takes its playlist id
        // and sort order. Additive (no mirror), so an existing playlist of the
        // same name is appended to rather than replaced. All or nothing, and
        // only the staged copies: the eDB transaction commits after the
        // staged PDB is written, and the repair run writes both back once.
        let pdb_path = usb_staging::stage_pdb(usb_root)?;
        let mut handle = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings));
        let tx = match handle.as_mut().map(EdbConnHandle::conn) {
            Some(conn) => Some(conn.transaction()?),
            None => {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.unindexed-audio.edb-open-failed",
                    "unindexed audio: eDB unavailable, only the PDB was updated",
                ));
                None
            }
        };
        let (edb_identity, t19_property) = match tx.as_ref() {
            Some(tx) => {
                let written = write_edb_playlists_in_tx(
                    tx,
                    &[EdbPlaylistWrite {
                        playlist: &playlist,
                        manifest: &manifest,
                        mirror_playlist_entries: false,
                        identity: None,
                    }],
                )?;
                let identity = written.first().map(|w| {
                    (
                        u32::try_from(w.playlist_id).ok(),
                        u32::try_from(w.sort_order).ok(),
                    )
                });
                (identity, t19_property_from_conn(tx))
            }
            None => (None, None),
        };
        let (playlist_id, sort_order) = edb_identity.unwrap_or((None, None));
        let build = build_pdb_for_playlists(
            usb_root,
            &[PdbPlaylistWrite {
                playlist: &playlist,
                manifest: &manifest,
                mirror_playlist_entries: false,
                row_id: None,
                playlist_id,
                sort_order,
                move_to_front: true,
            }],
            t19_property,
            false,
        )?;
        std::fs::write(&pdb_path, &build.bytes)?;
        if let Some(tx) = tx
            && let Err(err) = tx.commit()
        {
            if let Err(restore_err) = std::fs::write(&pdb_path, &build.before) {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.unindexed-audio.pdb-restore-failed",
                    format!(
                        "adding unindexed audio: eDB commit failed ({err}) and the staged PDB could \
                         not be put back ({restore_err}): {}",
                        pdb_path.display()
                    ),
                ));
            }
            return Err(err.into());
        }
        Ok((manifest.tracks.len(), analysed, without_analysis))
    }

    /// Remove the tracks whose audio file is gone (`remove_missing_audio_references`):
    /// eDB `content` rows and their `playlist_content` links, PDB playlist
    /// entries, track rows and now-unused artwork rows. `missing_paths` are
    /// the presence scan's missing paths minus history tracks, matched with
    /// `audio_path_key`. Returns (content rows, playlist links, PDB removal).
    pub(super) fn apply_fix_remove_missing_audio_references(
        &self,
        usb_root: &std::path::Path,
        missing_paths: &[String],
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize, PdbTrackRemoval)> {
        if missing_paths.is_empty() {
            return Ok((0, 0, PdbTrackRemoval::default()));
        }

        let mut removed_db_content = 0usize;
        let mut removed_db_playlist_links = 0usize;
        let missing_keys = missing_paths
            .iter()
            .map(|p| audio_path_key(p))
            .collect::<HashSet<_>>();

        if let Some(mut handle) =
            EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
        {
            let conn = handle.conn();
            let tx = conn.transaction()?;
            if table_exists(&tx, "content") {
                let columns = load_table_columns(&tx, "content")?;
                let path_col = if columns.iter().any(|c| c == "path") {
                    Some("path")
                } else if columns.iter().any(|c| c == "filePath") {
                    Some("filePath")
                } else if columns.iter().any(|c| c == "file_path") {
                    Some("file_path")
                } else {
                    None
                };

                if let Some(path_col) = path_col {
                    let mut stmt = tx.prepare(&format!(
                        "SELECT content_id, {path_col} FROM content WHERE {path_col} IS NOT NULL"
                    ))?;
                    let rows = stmt.query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
                    })?;
                    let mut content_ids_to_remove = Vec::<i64>::new();
                    for row in rows {
                        let (content_id, path) = row?;
                        let Some(path) = path else { continue };
                        if missing_keys.contains(&audio_path_key(&path)) {
                            content_ids_to_remove.push(content_id);
                        }
                    }

                    if !content_ids_to_remove.is_empty() {
                        if table_exists(&tx, "playlist_content") {
                            for content_id in &content_ids_to_remove {
                                let changed = tx.execute(
                                    "DELETE FROM playlist_content WHERE content_id = ?1",
                                    rusqlite::params![content_id],
                                )?;
                                removed_db_playlist_links += changed;
                            }
                        }
                        for content_id in &content_ids_to_remove {
                            removed_db_content += tx.execute(
                                "DELETE FROM content WHERE content_id = ?1",
                                rusqlite::params![content_id],
                            )?;
                        }
                    }
                } else {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.missing-audio.no-path-column",
                        "repair skipped (missing audio): eDB content path column not found",
                    ));
                }
            }
            tx.commit()?;
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        } else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.missing-audio.edb-open-failed",
                "repair skipped (missing audio): unable to open eDB read-write",
            ));
        }

        // Track ids from the PDB as it is now: an earlier fix in this pass
        // (strict parity) may have rewritten it.
        let missing_track_ids = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?
            .tracks
            .iter()
            .filter(|t| missing_keys.contains(&audio_path_key(&t.track_file_path)))
            .map(|t| t.id)
            .collect::<HashSet<_>>();
        let removed_pdb = remove_tracks_from_pdb(usb_root, &missing_track_ids).map_err(|err| {
            BackendError::Internal(format!(
                "eDB rows removed, but the PDB could not be updated: {err}"
            ))
        })?;

        Ok((removed_db_content, removed_db_playlist_links, removed_pdb))
    }
}
