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

pub(super) fn lowercase_file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
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
                    .filter(|(_, (_, other))| other.as_ref() == Some(fp))
                    .map(|(ui, _)| ui)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let candidates = if by_fingerprint.is_empty() {
            let name = lowercase_file_name(old_path);
            unindexed
                .iter()
                .enumerate()
                .filter(|(_, (path, _))| !name.is_empty() && lowercase_file_name(path) == name)
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
    edb_conn: Option<&mut rusqlite::Connection>,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(usize, usize)> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let mut handle = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings));
    let mismatches: Vec<BpmKeyMismatch> =
        detect_bpm_key_mismatches(usb_root, &parsed, handle.as_mut().map(|h| &*h.conn()), true);

    let mut anlz_fixed = 0usize;
    for m in &mismatches {
        let Some(dat) = m.anlz_dat_path.as_deref() else {
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
    }

    let mut edb_fixed = 0usize;
    let edb_targets: Vec<&BpmKeyMismatch> = mismatches
        .iter()
        .filter(|m| m.edb_bpmx100.is_some() || m.pdb_key_for_edb.is_some())
        .collect();
    if !edb_targets.is_empty() {
        let Some(handle) = handle.as_mut() else {
            return Err(BackendError::Internal(
                "unable to open eDB to align content BPM/key".to_string(),
            ));
        };
        let tx = handle.conn().transaction()?;
        for m in edb_targets {
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
        }
        tx.commit()?;
    }
    Ok((anlz_fixed, edb_fixed))
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
) -> BackendResult<usize> {
    let mut fixed = 0usize;
    for dat in detect_app_beat_grids(usb_root) {
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
        let extension = Path::new(path).extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("mp3") {
            Some(Self::Mp3)
        } else if extension.eq_ignore_ascii_case("flac") {
            Some(Self::Flac)
        } else {
            None
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
    track_path: String,
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

/// Write the missing seek indexes of one format found by
/// [`detect_missing_seek_data`], re-detected so bundles regenerated earlier in
/// the same pass are covered. A file that doesn't parse cleanly is skipped and
/// named in the Event Log with its reason, so it can be reported; nothing is
/// ever approximated. Returns (bundles written, tracks skipped).
pub(super) fn apply_missing_seek_data_repair(
    usb_root: &Path,
    format: SeekFormat,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(usize, usize)> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let mut fixed = 0usize;
    let mut skipped = 0usize;
    for target in detect_missing_seek_data(usb_root, &parsed, format) {
        let (reason, advice) = match seek_index_for_audio(&target.audio_path) {
            Ok(Some(index)) => {
                let bytes = std::fs::read(&target.bundle_path)?;
                match with_seek_index(&bytes, &index) {
                    Some(patched) => {
                        atomic_write_bytes(&target.bundle_path, &patched)?;
                        fixed += 1;
                        continue;
                    }
                    None => ("analysis file could not be parsed", SEEK_SKIP_REPORT_HINT),
                }
            }
            Ok(None) => continue,
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
    }
    Ok((fixed, skipped))
}

impl BackendService {
    pub(super) fn apply_fix_empty_analysis_files(
        &self,
        usb_root: &std::path::Path,
        empty_analysis_paths: &[String],
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
                    map_by_file.insert(
                        canonicalize_playlist_name(&a.to_string_lossy()),
                        target.clone(),
                    );
                    map_by_dir.insert(
                        canonicalize_playlist_name(&analysis_dir.to_string_lossy()),
                        target,
                    );
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
                        .entry(canonicalize_playlist_name(&ra.to_string_lossy()))
                        .or_insert_with(|| target.clone());
                    map_by_dir
                        .entry(canonicalize_playlist_name(&analysis_dir.to_string_lossy()))
                        .or_insert(target);
                }
            }
        }

        let mut fixed = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        let mut writes = 0usize;

        for empty_path in empty_analysis_paths {
            let key_file = canonicalize_playlist_name(empty_path);
            let key_dir = std::path::Path::new(empty_path)
                .parent()
                .map(|p| canonicalize_playlist_name(&p.to_string_lossy()))
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
                    format!("repair skipped (empty analysis): source audio mapping not found for {empty_path}"),
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
                    format!("repair skipped (empty analysis): source audio missing for {empty_path} -> {}", source_audio.display()),
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
                    format!("repair failed (empty analysis): {empty_path}: {err}"),
                ));
                continue;
            }
            // As local analysis does: MP3s get rekordbox's seek index.
            let _ = fill_mp3_pvbr(&bundle_paths.dat_path, &source_audio);
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
        let page_size = bytes
            .get(4..8)
            .and_then(|b| b.try_into().ok())
            .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
            .ok_or_else(|| BackendError::Validation("PDB too small to read page size".into()))?;
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
    /// files and their analysis bundles are already on the stick, so this is
    /// an additive export of a USB-only playlist: tags are read from the
    /// files, BPM/first beat/cues from each file's canonical ANLZ bundle, and
    /// the eDB + PDB writers append the tracks to `UNINDEXED_AUDIO_PLAYLIST_NAME`.
    /// Returns (tracks added, tracks without an analysis bundle).
    pub(super) fn apply_fix_add_unindexed_audio_playlist(
        &self,
        usb_root: &std::path::Path,
        files: &[String],
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize)> {
        let local_conn = self.db.connect()?;
        Self::ensure_track_export_identity_schema(&local_conn)?;
        let app_master_db_id = Self::ensure_local_u32_setting(
            &local_conn,
            SETTING_EXPORT_MASTER_DB_ID,
            "master-db-id",
        )?;

        let mut manifest_tracks = Vec::<ExportManifestTrack>::new();
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

            let (dat_path, ext_path, _) = canonical_analysis_bundle_paths(usb_root, rel);
            let (waveform_path, bpm, first_beat_ms, cues) = if dat_path.is_file() {
                let dat = std::fs::read(&dat_path).unwrap_or_default();
                let cue_bytes = std::fs::read(&ext_path).unwrap_or_else(|_| dat.clone());
                (
                    to_usb_relative_path(usb_root, &dat_path.to_string_lossy()),
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
            manifest_tracks.push(ExportManifestTrack {
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
                file_modified_at: scanned.file_modified_at,
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
                    .map(Self::file_type_from_extension),
                owns_exported_media: false,
                owns_artwork: false,
                owns_waveform: false,
                artwork_path: None,
                waveform_path,
                duration_ms: detect_track_duration_ms(&abs),
                first_beat_ms,
                cues,
            });
        }
        if manifest_tracks.is_empty() {
            return Ok((0, without_analysis));
        }

        let playlist = ExportPlaylistData {
            id: "usb-unindexed-audio".to_string(),
            name: UNINDEXED_AUDIO_PLAYLIST_NAME.to_string(),
            tracks: Vec::new(),
        };
        let manifest = ExportManifest {
            version: 1,
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
        // same name is appended to rather than replaced.
        let mut edb_playlist_id = None;
        let mut edb_sort_order = None;
        let mut handle = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings));
        match handle.as_mut().map(EdbConnHandle::conn) {
            Some(conn) => {
                let written = write_edb_playlist_with_conn(conn, &playlist, &manifest, false)?;
                edb_playlist_id = u32::try_from(written.playlist_id).ok();
                edb_sort_order = u32::try_from(written.sort_order).ok();
            }
            None => warnings.push(logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.unindexed-audio.edb-open-failed",
                "unindexed audio: eDB unavailable, only the PDB was updated",
            )),
        }
        let written = write_pdb(
            usb_root,
            &playlist,
            &manifest,
            false,
            edb_playlist_id,
            edb_sort_order,
        )?;
        warnings.extend(written.writer_warnings.into_iter().map(|message| {
            logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.unindexed-audio.pdb-writer-warning",
                message,
            )
        }));
        Ok((manifest.tracks.len(), without_analysis))
    }

    pub(super) fn apply_fix_remove_missing_audio_references(
        &self,
        usb_root: &std::path::Path,
        missing_track_ids: &HashSet<u32>,
        missing_paths: &[String],
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize, usize)> {
        if missing_track_ids.is_empty() || missing_paths.is_empty() {
            return Ok((0, 0, 0));
        }

        let mut removed_db_content = 0usize;
        let mut removed_db_playlist_links = 0usize;
        let missing_path_set = missing_paths
            .iter()
            .map(|p| normalize_pdb_path_for_edb_lookup(p))
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
                        let normalized = normalize_pdb_path_for_edb_lookup(&path);
                        if missing_path_set.contains(&normalized) {
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

        let removed_pdb_entries = match remove_track_ids_from_pdb_playlist_entries(
            usb_root,
            missing_track_ids,
        ) {
            Ok(removed) => removed,
            Err(err) => {
                warnings.push(logging::log(
                        Level::Error,
                        "usb-repair",
                        "usb.repair.missing-audio.pdb-update-failed",
                        format!("repair skipped (missing audio): unable to update PDB playlist entries ({err})"),
                    ));
                0
            }
        };

        Ok((
            removed_db_content,
            removed_db_playlist_links,
            removed_pdb_entries,
        ))
    }
}
