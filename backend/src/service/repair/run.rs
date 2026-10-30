//! The repair run: detect issues, propose fixes, apply the selected ones.

use super::*;

/// A supported, non-destructive proposal that writes one file -- the shape
/// most fixes share. Fixes that differ override fields with struct-update
/// syntax; `always_applied` and `bookkeeping` are filled in once every
/// proposal is collected.
/// The distinct row ids of `table_type` among `slots`, sorted.
fn misaligned_row_ids(slots: &[MisalignedStringSlot], table_type: u32) -> Vec<u32> {
    let mut ids: Vec<u32> = slots
        .iter()
        .filter(|m| m.table_type == table_type)
        .map(|m| m.id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    ids.sort_unstable();
    ids
}

fn fix_proposal(id: &str, title: &str, description: impl Into<String>) -> RepairFixProposal {
    RepairFixProposal {
        id: id.to_string(),
        title: title.to_string(),
        description: description.into(),
        supported: true,
        destructive: false,
        estimated_writes: 1,
        estimated_deletes: 0,
        always_applied: false,
        bookkeeping: false,
        link: None,
    }
}

/// What the detect phase found, handed to the apply phase.
struct DetectedRepairs {
    empty_analysis_paths: Vec<std::path::PathBuf>,
    parsed_pdb: Option<ParsedPdb>,
    bpm_key_mismatches: Vec<BpmKeyMismatch>,
    analysis_ownership: AnalysisOwnership,
    app_master_db_id: Option<u32>,
    app_beat_grids: Vec<std::path::PathBuf>,
    missing_seek_data: Vec<(SeekFormat, Vec<MissingSeekData>)>,
    pdb_wrong_history_shape_pages: Vec<WrongShapeHistoryPage>,
    pdb_ec_conflicts: Vec<EcDataPageConflict>,
    pdb_truncated_chains: Vec<TruncatedTableChain>,
    pdb_torn_growth: PdbTornGrowthReport,
    pdb_t19_template_state: bool,
    pdb_misaligned_track_ids: Vec<u32>,
    pdb_misaligned_album_ids: Vec<u32>,
    pdb_misaligned_artist_ids: Vec<u32>,
    pdb_header_compatibility_repair: Option<PdbHeaderCompatibilityRepair>,
    missing_audio_paths: Vec<String>,
    unindexed_audio_files: Vec<String>,
    duplicate_audio_copies: Vec<DuplicateAudioCopy>,
    moved_audio_relinks: Vec<MovedAudioRelink>,
    sync_edb_history_supported: bool,
    sync_edb_history_needed: bool,
    strict_upgrade_targets: HashSet<String>,
    pdb_duplicate_entry_count: usize,
    engine_library_fix_needed: bool,
}

/// Progress through the selected repair fixes. Each fix owns a segment of the
/// apply phase; multi-file fixes can report completed items within theirs.
struct ApplyProgress<'a> {
    emit: &'a mut dyn FnMut(usize, usize, &str),
    total_fixes: usize,
    started_fixes: usize,
    start: usize,
    end: usize,
    title: String,
}

impl<'a> ApplyProgress<'a> {
    fn new(emit: &'a mut dyn FnMut(usize, usize, &str), total_fixes: usize) -> Self {
        Self {
            emit,
            total_fixes: total_fixes.max(1),
            started_fixes: 0,
            start: 70,
            end: 70,
            title: String::new(),
        }
    }

    fn start(&mut self, title: &str) {
        let index = self.started_fixes.min(self.total_fixes - 1);
        self.start = 70 + index * 24 / self.total_fixes;
        self.end = 70 + (index + 1) * 24 / self.total_fixes;
        self.started_fixes += 1;
        self.title = title.to_string();
        (self.emit)(self.start, 100, &format!("USB: Applying {title}"));
    }

    fn item_done(&mut self, completed: usize, total: usize) {
        if total == 0 {
            return;
        }
        let completed = completed.min(total);
        let percent = self.start + (self.end - self.start) * completed / total;
        (self.emit)(
            percent,
            100,
            &format!("USB: Applying {} ({completed}/{total})", self.title),
        );
    }

    fn finish(&mut self) {
        (self.emit)(self.end, 100, &format!("USB: Finished {}", self.title));
    }

    /// A step outside the per-fix window (before the first fix, after the last).
    fn report(&mut self, percent: usize, label: &str) {
        (self.emit)(percent, 100, label);
    }
}

#[cfg(test)]
mod progress_tests {
    use super::ApplyProgress;

    #[test]
    fn five_fixes_each_receive_a_segment_and_item_progress_stays_inside_it() {
        let mut events = Vec::new();
        let mut record = |value, _, label: &str| events.push((value, label.to_string()));
        let mut progress = ApplyProgress::new(&mut record, 5);
        for index in 0..5 {
            progress.start(&format!("Fix {index}"));
            if index == 2 {
                progress.item_done(1, 2);
            }
            progress.finish();
        }
        assert_eq!(events[0].0, 70);
        assert_eq!(events[2].0, 74);
        assert_eq!(events[4].0, 79);
        assert_eq!(events[5], (81, "USB: Applying Fix 2 (1/2)".to_string()));
        assert_eq!(events[6].0, 84);
        assert_eq!(events[7].0, 84);
        assert_eq!(events.last().unwrap().0, 94);
    }
}

/// One line per fix: applied, skipped or failed.
#[derive(Default)]
pub(super) struct FixOutcomes {
    pub(super) applied: Vec<String>,
    pub(super) skipped: Vec<String>,
    pub(super) failed: Vec<String>,
}

impl FixOutcomes {
    /// Report the fix that is about to run, then record its outcome. Skipped
    /// fixes do not consume a progress step.
    fn record_with_progress<E: std::fmt::Display>(
        &mut self,
        progress: &mut ApplyProgress<'_>,
        title: &str,
        selected: bool,
        found: bool,
        apply: impl FnOnce() -> Result<String, E>,
    ) {
        if selected && found {
            progress.start(title);
        }
        self.record(title, selected, found, apply);
        if selected && found {
            progress.finish();
        }
    }

    /// Records one fix of the common shape: "not selected" when it was found
    /// but not picked, "nothing to apply" when picked but not found, otherwise
    /// the result of `apply`, whose `Ok` text follows "<title>: ".
    pub(super) fn record<E: std::fmt::Display>(
        &mut self,
        title: &str,
        selected: bool,
        found: bool,
        apply: impl FnOnce() -> Result<String, E>,
    ) {
        match (selected, found) {
            (true, false) => self.skipped.push(format!("{title}: nothing to apply")),
            (true, true) => match apply() {
                Ok(detail) => self.applied.push(format!("{title}: {detail}")),
                Err(err) => self.failed.push(format!("{title} failed: {err}")),
            },
            (false, true) => self.skipped.push(format!("{title}: not selected")),
            (false, false) => {}
        }
    }

    /// The databases could not be saved to the USB, so nothing applied
    /// reached it: every applied fix is reported as failed.
    fn not_saved(&mut self, err: &BackendError) {
        for fix in self.applied.drain(..) {
            self.failed.push(format!("{fix} (not saved to the USB)"));
        }
        self.failed.push(format!(
            "Saving the repaired databases to the USB failed, the USB's databases are unchanged: {err}"
        ));
    }
}

/// The staged PDB a repair run works on, read once. The page-level fixes
/// patch `bytes` in memory; a fix that reads or writes the staged file itself
/// goes through [`StagedPdb::on_file`]. The bytes reach the staged file only
/// when such a fix needs them and once at the end, before write-back.
pub(super) struct StagedPdb {
    path: std::path::PathBuf,
    pub(super) bytes: Vec<u8>,
    on_disk: Vec<u8>,
    /// What the USB held when the run started, for undoing a write-back.
    original: Vec<u8>,
}

impl StagedPdb {
    pub(super) fn load(usb_root: &Path) -> BackendResult<Self> {
        let path = usb_staging::stage_pdb(usb_root)?;
        let bytes = if path.is_file() {
            std::fs::read(&path)?
        } else {
            Vec::new()
        };
        Ok(Self {
            path,
            on_disk: bytes.clone(),
            original: bytes.clone(),
            bytes,
        })
    }

    /// Runs a page-level fix on a copy, keeping its changes only when it
    /// succeeds, so a fix that fails halfway leaves the PDB as it was.
    pub(super) fn patch<T>(
        &mut self,
        fix: impl FnOnce(&mut Vec<u8>) -> BackendResult<T>,
    ) -> BackendResult<T> {
        let mut work = self.bytes.clone();
        let result = fix(&mut work)?;
        self.bytes = work;
        Ok(result)
    }

    /// Runs a fix that reads or writes the staged PDB file: the current
    /// bytes are saved there first and read back afterwards.
    fn on_file<T>(&mut self, fix: impl FnOnce() -> BackendResult<T>) -> BackendResult<T> {
        self.save()?;
        let result = fix();
        self.reload()?;
        result
    }

    fn save(&mut self) -> BackendResult<()> {
        if self.bytes != self.on_disk {
            std::fs::write(&self.path, &self.bytes)?;
            self.on_disk.clone_from(&self.bytes);
        }
        Ok(())
    }

    fn reload(&mut self) -> BackendResult<()> {
        if self.path.is_file() {
            self.bytes = std::fs::read(&self.path)?;
            self.on_disk.clone_from(&self.bytes);
        }
        Ok(())
    }
}

/// Saves both staged databases to the USB, or neither, after putting the
/// PDB's in-memory bytes into its staged file. Before writing, both
/// USB copies are checked for outside changes. When the eDB write fails after
/// the PDB was written, the PDB the USB held before is put back. On failure
/// the staged copies are staged again from the USB, so they match the drive.
pub(super) fn write_back_both_or_neither(
    usb_root: &Path,
    pdb: &mut StagedPdb,
) -> BackendResult<()> {
    let restage_both = || {
        let _ = usb_staging::restage_from_usb(usb_root, usb_staging::DbKind::Pdb);
        let _ = usb_staging::restage_from_usb(usb_root, usb_staging::DbKind::Edb);
    };
    let pdb_written = pdb
        .save()
        .and_then(|()| usb_staging::check_write_back(usb_root, usb_staging::DbKind::Pdb))
        .and_then(|()| usb_staging::check_write_back(usb_root, usb_staging::DbKind::Edb))
        .and_then(|()| usb_staging::write_back_if_changed(usb_root, usb_staging::DbKind::Pdb))
        .inspect_err(|_| restage_both())?;
    if let Err(err) = usb_staging::write_back_if_changed(usb_root, usb_staging::DbKind::Edb) {
        if pdb_written
            && let Err(undo_err) = usb_staging::commit_and_write_back(
                usb_root,
                usb_staging::DbKind::Pdb,
                &pdb.original,
            )
        {
            restage_both();
            return Err(BackendError::Internal(format!(
                "{err}; putting the previous PDB back also failed ({undo_err}), so the USB has the \
                 repaired PDB with the previous eDB -- restore the \"Before repair\" backup"
            )));
        }
        restage_both();
        return Err(err);
    }
    Ok(())
}

impl BackendService {
    pub fn repair_usb_diagnostics(
        &self,
        req: RepairUsbDiagnosticsRequest,
    ) -> BackendResult<RepairUsbDiagnosticsData> {
        self.repair_usb_diagnostics_with_progress(req, |_, _, _| {})
    }

    pub fn repair_usb_diagnostics_with_progress<F>(
        &self,
        req: RepairUsbDiagnosticsRequest,
        mut on_progress: F,
    ) -> BackendResult<RepairUsbDiagnosticsData>
    where
        F: FnMut(usize, usize, &str),
    {
        let start = std::time::Instant::now();
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;

        // Open the eDB and stage the PDB exactly once for this whole repair run,
        // then hand the same connection/path to the diagnostics and parity
        // baselines below (and to this function's own eDB reads further down)
        // instead of each independently re-staging/re-opening from the USB
        // device -- see `run_usb_diagnostics_with_progress_impl`. The ~15 PDB
        // detectors further down read this same staged copy too.
        let mut edb_open_warnings = Vec::<WarningEntry>::new();
        let edb_conn = open_edb_from_usb_root(&usb_root, &mut edb_open_warnings);
        let staged_pdb_path = usb_staging::stage_pdb(&usb_root)?;

        on_progress(5, 100, "USB: Running diagnostics baseline");
        let (diagnostics, parsed_pdb_from_diagnostics) = self
            .run_usb_diagnostics_with_progress_impl(
                RunUsbDiagnosticsRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                },
                edb_conn.as_ref(),
                Some(&staged_pdb_path),
                |c, t, m| {
                    let pct = 5 + ((c * 45) / t.max(1)).min(45);
                    on_progress(pct, 100, m);
                },
            )?;

        on_progress(55, 100, "USB: Building parity guidance");
        let parity = self.run_usb_parity_report_with_progress_impl(
            RunUsbParityReportRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            },
            edb_conn.as_ref(),
            Some(&staged_pdb_path),
            parsed_pdb_from_diagnostics.as_ref(),
            |c, t, m| {
                let pct = 55 + ((c * 10) / t.max(1)).min(10);
                on_progress(pct, 100, m);
            },
        );

        on_progress(68, 100, "USB: Collecting repair opportunities");
        let mut detected_issues = Vec::<String>::new();
        let mut proposed_fixes = Vec::<RepairFixProposal>::new();
        let mut unsupported_items = Vec::<RepairUnsupportedItem>::new();
        let mut warnings = diagnostics.warnings.clone();
        warnings.append(&mut edb_open_warnings);
        let mut estimated_file_writes = 0usize;
        let mut estimated_file_deletes = 0usize;
        let mut missing_audio_paths = Vec::<String>::new();
        // The same files as on-disk USB-relative paths (`/Contents/...`), not
        // match keys -- what the unindexed-audio fix indexes.
        let mut unindexed_audio_files = Vec::<String>::new();
        let mut duplicate_audio_copies = Vec::<DuplicateAudioCopy>::new();
        let mut moved_audio_relinks = Vec::<MovedAudioRelink>::new();
        let mut sync_edb_history_supported = false;
        let mut sync_edb_history_needed = false;

        let anlz_scan = scan_anlz(&usb_root);
        warnings.extend(anlz_scan.warnings);
        let empty_analysis_paths = anlz_scan.empty_files;
        let malformed_analysis_entries = anlz_scan.empty_malformed_entries;
        if !empty_analysis_paths.is_empty() {
            detected_issues.push(format!(
                "{} detected",
                counted(
                    empty_analysis_paths.len(),
                    "empty USB analysis file",
                    "empty USB analysis files"
                )
            ));
            let writes = empty_analysis_paths.len() * 3;
            estimated_file_writes += writes;
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: writes,
                ..fix_proposal(
                    EMPTY_ANALYSIS_FILES_FIX_ID,
                    "Fix Empty Analysis Files",
                    "Regenerate missing/empty DAT/EXT/2EX bundles when source audio is resolvable.",
                )
            });
        }
        if !malformed_analysis_entries.is_empty() {
            detected_issues.push(format!(
                "{} detected under USBANLZ",
                counted(
                    malformed_analysis_entries.len(),
                    "malformed entry",
                    "malformed entries"
                )
            ));
            unsupported_items.push(RepairUnsupportedItem {
                issue: counted(
                    malformed_analysis_entries.len(),
                    "malformed USBANLZ entry",
                    "malformed USBANLZ entries",
                ),
                reason: "These indicate USB analysis directory corruption or stray files. Inspect Event Log entries tagged 'analysis entry malformed' and re-export affected tracks (or rebuild USB analysis data).".to_string(),
                link: Some("event-log".to_string()),
            });
        }

        // Reuse the PDB the diagnostics baseline above already parsed instead
        // of parsing it a third time (once for diagnostics, once for parity,
        // and this one) -- only falls back to a fresh parse if diagnostics'
        // own parse failed.
        let parsed_pdb = parsed_pdb_from_diagnostics.or_else(|| parse_pdb(&staged_pdb_path).ok());
        // This app's export id: the owner of a bundle it shares with another
        // track (`scan_analysis_ownership`).
        let app_master_db_id = self
            .db
            .connect()
            .ok()
            .and_then(|conn| {
                Self::ensure_local_u32_setting(&conn, SETTING_EXPORT_MASTER_DB_ID, "master-db-id")
                    .ok()
            })
            .and_then(|id| u32::try_from(id).ok());
        let bpm_key_mismatches: Vec<BpmKeyMismatch> = parsed_pdb
            .as_ref()
            .map(|parsed| {
                detect_bpm_key_mismatches(
                    &usb_root,
                    parsed,
                    edb_conn.as_ref(),
                    true,
                    app_master_db_id,
                )
            })
            .unwrap_or_default();
        if !bpm_key_mismatches.is_empty() {
            detected_issues.push(format!(
                "{} with ANLZ beat-grid tempo or eDB BPM/key differing from the PDB",
                counted(bpm_key_mismatches.len(), "track", "tracks")
            ));
            let anlz_writes = bpm_key_mismatches
                .iter()
                .filter(|m| m.anlz_dat_path.is_some())
                .count()
                * 2;
            let edb_writes = bpm_key_mismatches
                .iter()
                .filter(|m| m.edb_bpmx100.is_some() || m.pdb_key_for_edb.is_some())
                .count();
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: anlz_writes + edb_writes,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    BPM_KEY_MISMATCH_FIX_ID,
                    "Fix BPM/Key Mismatch",
                    "Rebuild the ANLZ beat grid and set the eDB BPM and key from each \
                     track's PDB row, so every player reads the same BPM and key. \
                     Waveforms, cues and the first beat are kept. See Event Log for the \
                     tracks.",
                )
            });
            for m in &bpm_key_mismatches {
                let tempo = |t: Option<u16>| {
                    t.map_or("-".to_string(), |t| format!("{:.2}", f64::from(t) / 100.0))
                };
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.bpm-key-mismatch",
                    format!(
                        "BPM/key mismatch: {} (PDB {:.2}, DAT grid {}, EXT grid {}, eDB {}{})",
                        m.track_path,
                        f64::from(m.pdb_tempo_x100) / 100.0,
                        tempo(m.anlz_tempos_x100[0]),
                        tempo(m.anlz_tempos_x100[1]),
                        m.edb_bpmx100
                            .map_or("same".to_string(), |v| format!("{:.2}", v as f64 / 100.0)),
                        m.pdb_key_for_edb
                            .as_deref()
                            .map_or(String::new(), |k| format!(", eDB key differs from {k}")),
                    ),
                ));
            }
        }
        // Analysis bundles that aren't one track's own (see `analysis_ownership`).
        let analysis_ownership = parsed_pdb
            .as_ref()
            .map(|parsed| scan_analysis_ownership(&usb_root, parsed, app_master_db_id, true))
            .unwrap_or_default();
        let bundle_rel = |dat: &std::path::Path| {
            usb_relative_path(&usb_root, dat).unwrap_or_else(|| dat.to_string_lossy().to_string())
        };
        if let Some(parsed) = parsed_pdb.as_ref()
            && !analysis_ownership.shared.is_empty()
        {
            let path_of = |id: &u32| {
                parsed
                    .tracks
                    .iter()
                    .find(|t| t.id == *id)
                    .map_or_else(String::new, |t| t.track_file_path.clone())
            };
            let movers = analysis_ownership
                .shared
                .iter()
                .map(|b| b.track_ids.len() - usize::from(b.owner.is_some()))
                .sum::<usize>();
            detected_issues.push(format!(
                "{} shared by tracks that need their own ({})",
                counted(
                    analysis_ownership.shared.len(),
                    "analysis file",
                    "analysis files"
                ),
                counted(movers, "track", "tracks")
            ));
            proposed_fixes.push(RepairFixProposal {
                destructive: true,
                estimated_writes: movers * 3,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    SHARED_ANALYSIS_FIX_ID,
                    "Give Tracks Their Own Analysis Files",
                    format!(
                        "Tracks using another track's analysis file: {movers}. Players can \
                         show that track's waveform and beat grid instead. Each gets its own \
                         file in its folder (ANLZ0001 and on, as rekordbox names them), analysed \
                         from its audio with its BPM; the file's own track keeps it. Old cues \
                         stay with the other track. See Event Log for the list."
                    ),
                )
            });
            for bundle in &analysis_ownership.shared {
                let tracks = bundle
                    .track_ids
                    .iter()
                    .map(path_of)
                    .collect::<Vec<_>>()
                    .join(", ");
                let owner = bundle
                    .owner
                    .as_ref()
                    .map_or_else(|| "no track (each gets its own)".to_string(), path_of);
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.shared-analysis",
                    format!(
                        "shared analysis file: {} used by {tracks}; kept by {owner}",
                        bundle_rel(&bundle.dat)
                    ),
                ));
            }
        }
        if !analysis_ownership.unmarked.is_empty() {
            let count = analysis_ownership.unmarked.len();
            detected_issues.push(format!(
                "{} without the track path players match by",
                counted(count, "analysis file", "analysis files")
            ));
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: count * 3,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    ANALYSIS_TRACK_PATH_FIX_ID,
                    "Add Track Path to Analysis Files",
                    format!(
                        "Analysis files not naming their track (PPTH): {count}. Players use \
                         that path to find a track's file when two tracks hash to the same \
                         folder. Writes the track path into each; nothing else changes. See \
                         Event Log for the list."
                    ),
                )
            });
            for bundle in &analysis_ownership.unmarked {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.analysis-without-track-path",
                    format!(
                        "analysis file without track path: {} ({})",
                        bundle_rel(&bundle.dat),
                        bundle.track_path
                    ),
                ));
            }
        }
        let app_beat_grids = detect_app_beat_grids(&usb_root);
        if !app_beat_grids.is_empty() {
            detected_issues.push(format!(
                "{} with a beat grid in the pre-0.3.7 format",
                counted(app_beat_grids.len(), "analysis bundle", "analysis bundles")
            ));
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: app_beat_grids.len(),
                ..fix_proposal(
                    BEAT_GRID_HEADER_FIX_ID,
                    "Fix Beat Grid",
                    "Rewrite beat grids from app versions before 0.3.7 in rekordbox's \
                     format: the .DAT header value two bytes off, and the .EXT extended \
                     grid without its checksum and sub-millisecond beat times. Some \
                     players (e.g. the XDJ-AZ) refuse Beat Sync on those. Beat positions \
                     stay within 1 ms; cues and waveforms are kept.",
                )
            });
        }
        let missing_seek_data: Vec<(SeekFormat, Vec<MissingSeekData>)> =
            [SeekFormat::Mp3, SeekFormat::Flac]
                .into_iter()
                .map(|format| {
                    let missing = parsed_pdb
                        .as_ref()
                        .map(|parsed| detect_missing_seek_data(&usb_root, parsed, format))
                        .unwrap_or_default();
                    // Preview checks whether each seek index can be built so it
                    // only offers fixable files. An explicit apply selection
                    // checks the audio again in apply_missing_seek_data_repair;
                    // parsing every MP3/FLAC here first can take minutes on a
                    // USB stick and doubles the reads for the selected fix.
                    // Keep the old all-fixes API behavior when no IDs were
                    // selected, since it derives the selection from proposals.
                    let (fixable, unfixable) = if req.apply && !req.selected_fix_ids.is_empty() {
                        (missing, Vec::new())
                    } else {
                        partition_fixable_seek_data(missing)
                    };
                    if !unfixable.is_empty() {
                        unsupported_items.push(RepairUnsupportedItem {
                            issue: format!(
                                "{} {} {} without seek data that can't be added",
                                unfixable.len(),
                                format.name(),
                                plural(unfixable.len(), "track", "tracks")
                            ),
                            reason: "The seek index can't be built from these files (Event Log \
                                     names each one and why). A damaged file needs an intact \
                                     copy."
                                .to_string(),
                            link: Some("event-log".to_string()),
                        });
                        for (target, reason) in &unfixable {
                            warnings.push(logging::log(
                                Level::Warn,
                                "usb-diagnostics",
                                "usb.diagnostics.seek-data-unfixable",
                                format!(
                                    "{} seek data can't be added ({reason}): {}",
                                    format.name(),
                                    target.track_path
                                ),
                            ));
                        }
                    }
                    (format, fixable)
                })
                .collect();
        for (format, missing) in &missing_seek_data {
            if missing.is_empty() {
                continue;
            }
            detected_issues.push(format!(
                "{} {} {} without the seek data rekordbox writes",
                missing.len(),
                format.name(),
                plural(missing.len(), "track", "tracks")
            ));
            let description = match format {
                SeekFormat::Mp3 => {
                    "Read each MP3 on the USB and write the seek index rekordbox writes for it \
                     (PVBR in the .DAT), byte for byte as rekordbox does. Beat grids, cues and \
                     waveforms are kept. Files whose index can't be reproduced exactly are \
                     skipped and named in the Event Log."
                }
                SeekFormat::Flac => {
                    "Read each FLAC on the USB and write a seek index (PVB2 in the .EXT). For \
                     long tracks about 98% of its entries match rekordbox's; the rest are one \
                     frame later. Beat grids, cues and waveforms are kept. Files that don't parse \
                     cleanly are skipped and named in the Event Log."
                }
            };
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: missing.len(),
                ..fix_proposal(format.fix_id(), format.title(), description.to_string())
            });
        }
        // Every PDB detector below reads the same staged bytes.
        let staged_pdb_bytes = std::fs::read(&staged_pdb_path).unwrap_or_default();
        let pdb_sentinel_u5_pages = detect_pdb_sentinel_u5_on_data_pages(&staged_pdb_bytes);
        let pdb_wrong_flags_pages = detect_pdb_wrong_page_flags(&staged_pdb_bytes);
        let pdb_zero_tranrf_pages = detect_pdb_zero_tranrf_all_tables(&staged_pdb_bytes);
        let pdb_wrong_history_shape_pages = detect_pdb_wrong_history_page_shape(&staged_pdb_bytes);
        let pdb_tombstoned_playlist_ids =
            detect_pdb_tombstoned_playlist_tree_ids(&staged_pdb_bytes);
        let pdb_wrong_track_u5_pages = detect_pdb_wrong_track_u5(&staged_pdb_bytes);
        let pdb_t00_multipage_active_pages =
            detect_pdb_t00_multipage_active_pages(&staged_pdb_bytes);
        let pdb_stale_sentinel_btree_pages = detect_pdb_stale_sentinel_btree(&staged_pdb_bytes);
        let pdb_wrong_playlist_tree_shape_pages =
            detect_pdb_wrong_playlist_tree_shape(&staged_pdb_bytes);
        let pdb_ec_conflicts = detect_pdb_ec_data_page_conflicts(&staged_pdb_bytes);
        let pdb_truncated_chains = detect_pdb_truncated_table_chains(&staged_pdb_bytes);
        let pdb_torn_growth = detect_pdb_torn_growth_pages(&staged_pdb_bytes);
        let pdb_t19_template_state = detect_pdb_t19_template_state(
            &staged_pdb_bytes,
            parsed_pdb.as_ref().map_or(0, |p| p.tracks.len()),
            crate::service::export_helpers::edb_t19_property(&usb_root).as_ref(),
        );
        let pdb_misaligned_slots = detect_pdb_string_misalignment(&staged_pdb_bytes);
        let pdb_misaligned_track_ids = misaligned_row_ids(&pdb_misaligned_slots, 0);
        let pdb_misaligned_album_ids = misaligned_row_ids(&pdb_misaligned_slots, 3);
        let pdb_misaligned_artist_ids = misaligned_row_ids(&pdb_misaligned_slots, 2);
        // rekordbox transaction bookkeeping (see `fix_is_bookkeeping`): no
        // detected issue and no write of its own -- it rides along with the
        // PDB write of whatever repair runs.
        if !pdb_zero_tranrf_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_ZERO_TRANRF_FIX_ID,
                "Repair Data Pages With Zero tranrf",
                format!(
                    "Set tranrf = rowpf on {} where tranrf is zero but active rows exist.",
                    counted(pdb_zero_tranrf_pages.len(), "data page", "data pages")
                ),
            ));
        }
        if !pdb_wrong_flags_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_PAGE_FLAGS_FIX_ID,
                "Repair PDB Data Page Flags",
                format!(
                    "Patch page_flags byte (0x1b) on {} to the per-table value \
                     (tracks/history=0x34, all others=0x24).",
                    counted(pdb_wrong_flags_pages.len(), "data page", "data pages")
                ),
            ));
        }
        if !pdb_sentinel_u5_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_SENTINEL_U5_FIX_ID,
                "Repair PDB Data Pages With Sentinel u5",
                format!(
                    "Patch u5/num_rl header fields on {} from sentinel 0x1FFF to the \
                     per-table value.",
                    counted(pdb_sentinel_u5_pages.len(), "data page", "data pages")
                ),
            ));
        }
        if !pdb_wrong_history_shape_pages.is_empty() {
            detected_issues.push(format!(
                "{} with wrong u5/num_rl shape (may be rejected as corrupted)",
                counted(
                    pdb_wrong_history_shape_pages.len(),
                    "PDB history/columns page",
                    "PDB history/columns pages"
                )
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_HISTORY_SHAPE_FIX_ID,
                "Repair PDB History/Columns Page Footer Shape",
                format!(
                    "Patch u5/num_rl footer fields on {} from the wrong \
                     (1, nrs-1) shape to the correct (nrs, 0) convention.",
                    counted(
                        pdb_wrong_history_shape_pages.len(),
                        "tt=16/17/18 data page",
                        "tt=16/17/18 data pages"
                    )
                ),
            ));
        }
        if !pdb_wrong_track_u5_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_TRACK_U5_FIX_ID,
                "Repair PDB Track Page Footer Shape",
                format!(
                    "Normalise u5/num_rl footer fields on {} to (1, nrs-1), \
                     the format written by the current writer. (2, 0) is left untouched on pages \
                     that already have it.",
                    counted(
                        pdb_wrong_track_u5_pages.len(),
                        "tt=0 track page",
                        "tt=0 track pages"
                    )
                ),
            ));
        }
        if !pdb_t00_multipage_active_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_T00_MULTIPAGE_ACTIVE_FIX_ID,
                "Repair Track Page Flag (Multi-Page Chain)",
                format!(
                    "Set flags=0x24 (SEAL) and u5/num_rl=(1, nrs-1) on {} \
                     marked ACTV (0x34) in a multi-page chain, the shape the writer produces.",
                    counted(
                        pdb_t00_multipage_active_pages.len(),
                        "tt=0 track page",
                        "tt=0 track pages"
                    )
                ),
            ));
        }
        if !pdb_tombstoned_playlist_ids.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID,
                "Repair Tombstoned Row IDs",
                format!(
                    "Zero out the id field in {} (tracks + playlist_tree) whose \
                     id is also used by an active row.",
                    counted(
                        pdb_tombstoned_playlist_ids.len(),
                        "tombstoned slot",
                        "tombstoned slots"
                    )
                ),
            ));
        }
        if !pdb_stale_sentinel_btree_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_STALE_SENTINEL_BTREE_FIX_ID,
                "Repair Stale Sentinel B-Tree Index",
                format!(
                    "Reset the B-tree index area in {} to the empty state \
                     (num_entries=0, first_empty=0x1FFF), as fresh exports write it.",
                    counted(
                        pdb_stale_sentinel_btree_pages.len(),
                        "sentinel page",
                        "sentinel pages"
                    )
                ),
            ));
        }
        if !pdb_wrong_playlist_tree_shape_pages.is_empty() {
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID,
                "Repair PDB Playlist-Tree Page Footer Shape",
                format!(
                    "Set u5=nrs and num_rl=0 on {}, the shape \
                     the writer produces for this table.",
                    counted(
                        pdb_wrong_playlist_tree_shape_pages.len(),
                        "tt=7 (playlist_tree) data page",
                        "tt=7 (playlist_tree) data pages"
                    )
                ),
            ));
        }
        if !pdb_ec_conflicts.is_empty() {
            detected_issues.push(format!(
                "{} with empty_candidate pointing to another table's data page \
                 (rejected as corrupted — aliased write pointer)",
                counted(pdb_ec_conflicts.len(), "table", "tables")
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_EC_CONFLICT_FIX_ID,
                "Repair Table Write-Pointer Conflict",
                format!(
                    "Tables whose empty_candidate pointer targets a page already used as data \
                     by a different table: {}. The DJ software validates this invariant \
                     and reports 'database corrupted' when it finds a conflict. The repair \
                     reassigns each conflicting empty_candidate to a free page beyond the \
                     physical file and updates next_unused_page in the header.",
                    pdb_ec_conflicts.len()
                ),
            ));
        }
        if !pdb_truncated_chains.is_empty() {
            detected_issues.push(format!(
                "{} with a declared last page beyond the physical end of the file \
                 (interrupted export left growth pointers ahead of the actual written data)",
                counted(pdb_truncated_chains.len(), "table", "tables")
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_TRUNCATED_TABLE_CHAIN_FIX_ID,
                "Repair Truncated Table Chain",
                format!(
                    "Tables declaring a last page that does not physically exist in the file: \
                     {} (an additive-growth write was interrupted after the table pointer was \
                     advanced but before the new pages were flushed to disk). The repair points \
                     the table's last/empty_candidate fields back at the real last written page; \
                     no page content is changed. This is a structural prerequisite and always \
                     runs before other repairs, including the strict-parity upgrade.",
                    pdb_truncated_chains.len()
                ),
            ));
        }
        if !pdb_torn_growth.is_empty() {
            detected_issues.push(format!(
                "PDB shows signs of a torn additive-growth write (mid-export USB disconnect): \
                 {} with non-zero garbage instead of a blank reusable page{}",
                counted(
                    pdb_torn_growth.garbage_ec_pages.len(),
                    "empty_candidate page",
                    "empty_candidate pages"
                ),
                if pdb_torn_growth.truncate_to_pages.is_some() {
                    ", plus a never-populated file tail beyond next_unused_page"
                } else {
                    ""
                }
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_TORN_GROWTH_PAGES_FIX_ID,
                "Repair Torn Growth Pages (Interrupted Export)",
                format!(
                    "The export was interrupted (e.g. USB disconnected) while rekordbox was \
                     growing the PDB: table empty_candidate pages earmarked as the next slot \
                     to write ({}) never received data, leaving non-zero garbage \
                     instead of the blank page the writer requires before reuse{}. This zeroes \
                     each such page{} and recomputes seqdb. No table chain, page index, or \
                     transaction history is touched.",
                    pdb_torn_growth.garbage_ec_pages.len(),
                    if pdb_torn_growth.truncate_to_pages.is_some() {
                        " and left the file extended past next_unused_page with unpopulated content"
                    } else {
                        ""
                    },
                    if pdb_torn_growth.truncate_to_pages.is_some() {
                        " and truncates the never-populated tail"
                    } else {
                        ""
                    },
                ) + " This is a structural prerequisite and always runs before other repairs, \
                     including the strict-parity upgrade: parity's additive writes can grow the \
                     file, and this repair's tail-truncation boundary would otherwise go stale \
                     and cut off tracks/playlists parity had just written.",
            ));
        }
        if pdb_t19_template_state {
            detected_issues.push(
                "PDB history page (tt=19) does not match the USB's tracks or name \
                 (older players show 0 or 1 tracks)"
                    .to_string(),
            );
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_T19_TEMPLATE_STATE_FIX_ID,
                "Activate PDB History Page",
                "The tt=19 page still has the single sealed row of a never-exported USB, the \
                 fixed two rows an older version wrote, or no USB name although the USB has \
                 one, so older players count 0 or 1 tracks. Rewrite it \
                 as the ring of one row per track rekordbox keeps, with the USB's name, keeping \
                 the template row, and mark the page active. No other page is changed."
                    .to_string(),
            ));
        }
        if !pdb_misaligned_track_ids.is_empty() {
            detected_issues.push(format!(
                "{} with a misaligned UTF-16 or long ASCII string slot (freezes/comm-errors CDJ hardware while browsing or opening the track list)",
                counted(pdb_misaligned_track_ids.len(), "track row", "track rows")
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_TRACK_STRING_ALIGNMENT_FIX_ID,
                "Repair Track String Alignment",
                format!(
                    "Re-encode {} whose title/filename/comment/other UTF-16 or long \
                     ASCII string slot doesn't start at a 4-byte aligned offset. MIPS-based CDJ \
                     hardware requires 4-byte aligned reads for that 4-byte string header; a \
                     misaligned slot freezes/comm-errors the player while browsing, or as soon \
                     as the track list opens.",
                    counted(pdb_misaligned_track_ids.len(), "track row", "track rows")
                ),
            ));
        }
        if !pdb_misaligned_album_ids.is_empty() {
            detected_issues.push(format!(
                "{} with a misaligned UTF-16 name slot (freezes/comm-errors CDJ hardware while browsing)",
                counted(pdb_misaligned_album_ids.len(), "album row", "album rows")
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_ALBUM_STRING_ALIGNMENT_FIX_ID,
                "Repair Album String Alignment",
                format!(
                    "Re-encode {} whose UTF-16 name slot doesn't start at a 4-byte \
                     aligned offset. Same class of issue as the track string alignment repair.",
                    counted(pdb_misaligned_album_ids.len(), "album row", "album rows")
                ),
            ));
        }
        if !pdb_misaligned_artist_ids.is_empty() {
            detected_issues.push(format!(
                "{} with the name at a different offset than rekordbox uses \
                 (Engine DJ players show the artist's tracks without an artist; a misaligned \
                 UTF-16 name also freezes CDJ hardware while browsing)",
                counted(pdb_misaligned_artist_ids.len(), "artist row", "artist rows")
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_ARTIST_NAME_LAYOUT_FIX_ID,
                "Repair Artist Name Layout",
                format!(
                    "Re-encode {} the way rekordbox writes artist rows: a plain ASCII \
                     name at offset 10, a UTF-16 name at the 4-byte aligned offset 12. Older \
                     versions of this app put every artist name at 12, which an Engine DJ \
                     player's own import of export.pdb reads as no artist.",
                    counted(pdb_misaligned_artist_ids.len(), "artist row", "artist rows")
                ),
            ));
        }
        let pdb_header_compatibility_repair =
            detect_pdb_header_compatibility_repair(&staged_pdb_bytes);
        if let Some(repair) = pdb_header_compatibility_repair.as_ref() {
            detected_issues.push(format!(
                "PDB header compatibility field 0x10..0x14 is {}",
                repair.current_value
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_HEADER_COMPATIBILITY_FIX_ID,
                "Repair PDB Header Compatibility Field",
                format!(
                    "Patch only bytes 0x10..0x14 from {} to the built-in compatibility value {}.",
                    repair.current_value, repair.target_value,
                ),
            ));
        }
        // Missing and unindexed audio, by the same rule the parity report's
        // "Indexed audio file presence" check uses (`scan_audio_presence`), so
        // every file it lists gets one of the fixes below and one pass of them
        // leaves nothing it still lists.
        let edb_audio_paths = edb_conn
            .as_ref()
            .map(|conn| collect_edb_audio_paths(conn, &mut warnings))
            .unwrap_or_default();
        if let Some(parsed) = parsed_pdb.as_ref() {
            let presence = scan_audio_presence(&usb_root, parsed, &edb_audio_paths);
            if presence.indexed_on_usb.is_empty() && presence.unindexed.is_empty() {
                // No audio under Contents/ at all: a databases-only snapshot,
                // where "missing" can't be told apart from "never copied".
                warnings.push(logging::log(
                    Level::Info,
                    "usb-diagnostics",
                    "usb.diagnostics.missing-audio-scan-skipped",
                    "missing-audio scan skipped: Contents directory is absent or empty (DB-only snapshot)",
                ));
                if !presence.missing.is_empty() {
                    unsupported_items.push(RepairUnsupportedItem {
                        issue: format!(
                            "{} missing: the USB has no audio under Contents",
                            counted(
                                presence.missing.len(),
                                "indexed audio file",
                                "indexed audio files"
                            )
                        ),
                        reason: "Looks like a copy of the databases without the music. Copy the audio files back instead of removing every track.".to_string(),
                        link: Some("event-log".to_string()),
                    });
                }
            } else {
                let missing_keys = presence.missing_keys();
                let missing_audio_track_ids = parsed
                    .tracks
                    .iter()
                    .filter(|t| missing_keys.contains(&audio_path_key(&t.track_file_path)))
                    .map(|t| t.id)
                    .collect::<HashSet<u32>>();
                missing_audio_paths = presence.missing.clone();
                unindexed_audio_files = presence.unindexed.clone();

                // A copy of an indexed file sitting next to it is not a new
                // track: offer to delete it rather than add the same track to
                // a playlist a second time. Unindexed files are in no
                // playlist, history included, so nothing referenced is deleted.
                let indexed_on_usb = presence.indexed_on_usb.iter().collect::<Vec<_>>();
                duplicate_audio_copies =
                    find_duplicate_audio_copies(&usb_root, &unindexed_audio_files, &indexed_on_usb);
                let copies = duplicate_audio_copies
                    .iter()
                    .map(|c| c.copy_path.as_str())
                    .collect::<HashSet<_>>();
                unindexed_audio_files.retain(|p| !copies.contains(p.as_str()));

                // A missing reference and an unindexed file are often the same
                // track under a new path (renamed file, moved folder). Pair
                // them first so the track is relinked -- keeping its
                // playlists, cues and history -- rather than removed from its
                // playlists and re-added to the "Unindexed" one. The other two
                // fixes then only see what stayed unmatched.
                if !missing_audio_track_ids.is_empty() && !unindexed_audio_files.is_empty() {
                    let missing = parsed
                        .tracks
                        .iter()
                        .filter(|t| missing_audio_track_ids.contains(&t.id))
                        .map(|t| {
                            let artist = parsed
                                .artists
                                .get(&t.artist_id)
                                .map(String::as_str)
                                .unwrap_or("");
                            (
                                t.id,
                                t.track_file_path.clone(),
                                content_fingerprint_key(
                                    t.file_size_bytes.map(i64::from),
                                    &t.title,
                                    artist,
                                ),
                            )
                        })
                        .collect::<Vec<_>>();
                    let unindexed = unindexed_audio_files
                        .iter()
                        .map(|rel| {
                            let fingerprint = resolve_usb_side_path(&usb_root, rel)
                                .and_then(|abs| scan_audio_file(&abs).ok())
                                .and_then(|t| {
                                    content_fingerprint_key(t.file_size_bytes, &t.title, &t.artist)
                                });
                            (rel.clone(), fingerprint)
                        })
                        .collect::<Vec<_>>();
                    moved_audio_relinks = match_moved_audio(&missing, &unindexed);
                }
                let relinked_files = moved_audio_relinks
                    .iter()
                    .map(|r| r.new_path.as_str())
                    .collect::<HashSet<_>>();
                unindexed_audio_files.retain(|p| !relinked_files.contains(p.as_str()));
                let relinked_keys = moved_audio_relinks
                    .iter()
                    .map(|r| audio_path_key(&r.old_path))
                    .collect::<HashSet<_>>();
                missing_audio_paths.retain(|p| !relinked_keys.contains(&audio_path_key(p)));

                // A track in a history playlist is never removed, even when
                // its file is gone: the history is the record of what was
                // played. Listed for the user instead.
                let history_keys = missing_audio_history_keys(parsed, edb_conn.as_ref());
                let (in_history, removable): (Vec<String>, Vec<String>) = missing_audio_paths
                    .drain(..)
                    .partition(|p| history_keys.contains(&audio_path_key(p)));
                missing_audio_paths = removable;
                if !in_history.is_empty() {
                    unsupported_items.push(RepairUnsupportedItem {
                        issue: format!(
                            "{} in a history playlist with missing audio files",
                            counted(in_history.len(), "track", "tracks")
                        ),
                        reason: "Tracks in a history playlist are not removed. Copy the files back to the USB (Event Log lists them).".to_string(),
                        link: Some("event-log".to_string()),
                    });
                    for path in &in_history {
                        warnings.push(logging::log(
                            Level::Warn,
                            "usb-diagnostics",
                            "usb.diagnostics.missing-history-audio",
                            format!("history track's audio file missing: {path}"),
                        ));
                    }
                }
            }
        }

        if !missing_audio_paths.is_empty() {
            detected_issues.push(format!(
                "{} to missing audio files under Contents",
                counted(
                    missing_audio_paths.len(),
                    "track reference",
                    "track references"
                )
            ));
        }
        if !duplicate_audio_copies.is_empty() {
            detected_issues.push(format!(
                "{} under Contents: unindexed copies of an indexed file in the same folder",
                counted(duplicate_audio_copies.len(), "audio file", "audio files")
            ));
            estimated_file_deletes += duplicate_audio_copies.len();
            proposed_fixes.push(RepairFixProposal {
                destructive: true,
                estimated_writes: 0,
                estimated_deletes: duplicate_audio_copies.len(),
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    DUPLICATE_AUDIO_COPIES_FIX_ID,
                    "Remove Duplicate Audio Copies",
                    format!(
                        "Audio files on the USB in no playlist, each a byte-for-byte copy of a track in the same folder: {}. Deletes each copy; the original track keeps its playlists, cues and analysis. See Event Log for the file list.",
                        duplicate_audio_copies.len()
                    ),
                )
            });
            for copy in &duplicate_audio_copies {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.duplicate-audio",
                    format!(
                        "duplicate audio file: {} (copy of {})",
                        copy.copy_path, copy.original_path
                    ),
                ));
            }
        }
        if !unindexed_audio_files.is_empty() {
            detected_issues.push(format!(
                "{} under Contents missing from the canonical-path indexed set (PDB/eDB)",
                counted(unindexed_audio_files.len(), "audio file", "audio files")
            ));
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: 2,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    UNINDEXED_AUDIO_PLAYLIST_FIX_ID,
                    "Add Unindexed Audio to a Playlist",
                    format!(
                        "Audio files on the USB in no playlist (e.g. after restoring the databases from a backup): {}. Adds each to a USB playlist \"{UNINDEXED_AUDIO_PLAYLIST_NAME}\", reusing its analysis files; nothing is copied or deleted. See Event Log for the file list.",
                        unindexed_audio_files.len()
                    ),
                )
            });
            for path in &unindexed_audio_files {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.unindexed-audio",
                    format!("unindexed audio file: {path}"),
                ));
            }
        }
        if !moved_audio_relinks.is_empty() {
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: 2,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    RELINK_MOVED_AUDIO_FIX_ID,
                    "Relink Moved Audio Files",
                    format!(
                        "Tracks pointing to a missing file while the same track is on the USB under another name or folder: {}. Points each to the file found; it keeps its playlists, cues and analysis. See Event Log for the list.",
                        moved_audio_relinks.len()
                    ),
                )
            });
            for relink in &moved_audio_relinks {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.moved-audio",
                    format!("moved audio: {} -> {}", relink.old_path, relink.new_path),
                ));
            }
        }
        // Safe without a manual review: every missing reference that had a
        // candidate file on the USB was paired above and is relinked instead.
        if !missing_audio_paths.is_empty() {
            proposed_fixes.push(RepairFixProposal {
                destructive: true,
                estimated_writes: 0,
                estimated_deletes: missing_audio_paths.len(),
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID,
                    "Remove Missing Audio References",
                    format!(
                        "Audio files no longer on the USB: {}. Removes their tracks from both databases (track and content rows, playlist entries). Tracks in a history playlist are kept. See Event Log for the file list.",
                        missing_audio_paths.len()
                    ),
                )
            });
            for path in &missing_audio_paths {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.missing-audio",
                    format!("missing-audio reference: {path}"),
                ));
            }
        }

        if let Some(parsed) = parsed_pdb.as_ref() {
            let (history_rows, history_content_rows) = derive_history_sync_payload(parsed);
            if !history_rows.is_empty()
                && let Some(conn) = edb_conn.as_ref()
                && table_exists(conn, "history")
                && table_exists(conn, "history_content")
            {
                let current_history_count = conn
                    .query_row("SELECT COUNT(*) FROM history", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize;
                let current_history_content_count = conn
                    .query_row("SELECT COUNT(*) FROM history_content", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize;
                let target_history_count = history_rows.len();
                let target_history_content_count = history_content_rows.len();
                if current_history_count != target_history_count
                    || current_history_content_count != target_history_content_count
                {
                    sync_edb_history_needed = true;
                    sync_edb_history_supported = true;
                    detected_issues.push(format!(
                                "eDB history differs from PDB-derived payload (history {current_history_count}->{target_history_count}, history_content {current_history_content_count}->{target_history_content_count})"
                            ));
                    proposed_fixes.push(RepairFixProposal {
                        destructive: current_history_count > 0 || current_history_content_count > 0,
                        estimated_writes: target_history_count + target_history_content_count,
                        estimated_deletes: current_history_count + current_history_content_count,
                        ..fix_proposal(
                            SYNC_EDB_HISTORY_FROM_PDB_FIX_ID,
                            "Sync eDB History Tables From PDB",
                            "Optional fix: replace eDB history/history_content rows using current PDB history payload mapping.",
                        )
                    });
                }
            }
        }

        let mut strict_upgrade_targets = HashSet::<String>::new();
        let mut pdb_duplicate_entry_count: usize = 0;
        match parity {
            Ok(report) => {
                let strict_count = report
                    .playlist_details
                    .iter()
                    .filter(|d| playlist_requires_strict_upgrade(d))
                    .count();
                strict_upgrade_targets = report
                    .playlist_details
                    .iter()
                    .filter(|d| playlist_requires_strict_upgrade(d))
                    .map(|d| canonicalize_playlist_name(&d.name))
                    .collect();
                if strict_count > 0 {
                    detected_issues.push(format!(
                        "{} failing strict parity",
                        counted(strict_count, "playlist", "playlists")
                    ));
                    // A dry run of the apply: same collect, merge and
                    // writers, eDB rolled back and PDB bytes dropped.
                    let mut dry_run_warnings = Vec::<WarningEntry>::new();
                    let (estimated_writes, planned) = match run_strict_parity_upgrade(
                        &usb_root,
                        Some(&strict_upgrade_targets),
                        None,
                        &mut dry_run_warnings,
                        false,
                    ) {
                        Ok(dry) => (
                            dry.edb_rows_inserted
                                + dry.edb_rows_updated
                                + dry.pdb_tracks_inserted
                                + dry.pdb_tracks_changed
                                + dry.pdb_playlists_inserted
                                + dry.duplicate_entries_removed,
                            format!(" {}.", strict_parity_summary(&dry)),
                        ),
                        Err(err) => (0, format!(" Dry run failed: {err}.")),
                    };
                    proposed_fixes.push(RepairFixProposal {
                        estimated_writes,
                        ..fix_proposal(
                            STRICT_PARITY_UPGRADE_FIX_ID,
                            "Upgrade Export Data To Strict Parity",
                            format!(
                                "Collect the failing playlists from both eDB and PDB, merge metadata from both sides, and write both databases once through the export writers.{planned}"
                            ),
                        )
                    });
                }

                pdb_duplicate_entry_count = report
                    .playlist_details
                    .iter()
                    .map(|d| d.pdb_duplicate_entries)
                    .sum();
                if pdb_duplicate_entry_count > 0 {
                    detected_issues.push(format!(
                        "{} detected",
                        counted(
                            pdb_duplicate_entry_count,
                            "duplicate PDB playlist entry",
                            "duplicate PDB playlist entries"
                        )
                    ));
                    proposed_fixes.push(fix_proposal(
                        PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID,
                        "Remove Duplicate PDB Playlist Entries",
                        format!(
                            "Remove {} left \
                             behind on shared pages from repeated exports of the same playlist over time. \
                             This only cleans up duplicate rows already in the PDB — it does not merge \
                             playlists between eDB and PDB or otherwise rewrite the export. For that broader \
                             repair, use \"Upgrade Export Data To Strict Parity\" instead (or in addition).",
                            counted(
                                pdb_duplicate_entry_count,
                                "stale duplicate playlist_entries row",
                                "stale duplicate playlist_entries rows"
                            )
                        ),
                    ));
                }
                warnings.extend(report.warnings);
            }
            Err(err) => {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.parity-preview-unavailable",
                    format!("parity preview unavailable: {err}"),
                ));
            }
        }

        let engine_library_fix_needed = self.engine_library_needs_fix(&usb_root);
        if engine_library_fix_needed {
            proposed_fixes.push(RepairFixProposal {
                destructive: true,
                ..fix_proposal(
                    ENGINE_LIBRARY_FIX_ID,
                    "Keep Engine DJ Library Up to Date",
                    "This USB has an Engine DJ library (Denon Prime, Numark Mixstream). \
                     Turns on \"Write Engine DJ library\" and rebuilds it now from this USB's \
                     rekordbox data; from then on it's rebuilt whenever this app changes a USB. \
                     Play history is kept, but changes made on the player itself (cues, loops, \
                     playlists created there) are replaced every time.",
                )
            });
        }

        // Display order should match the actual apply order below (see
        // REPAIR_FIX_DISPLAY_ORDER), not the incidental order each fix
        // happened to be detected in.
        proposed_fixes.sort_by_key(|f| {
            REPAIR_FIX_DISPLAY_ORDER
                .iter()
                .position(|id| *id == f.id)
                .unwrap_or(usize::MAX)
        });
        for fix in &mut proposed_fixes {
            fix.always_applied = fix_always_applied(&fix.id);
            fix.bookkeeping = fix_is_bookkeeping(&fix.id);
        }

        let mut selected = if req.selected_fix_ids.is_empty() {
            proposed_fixes
                .iter()
                // Both change more than the USB's rekordbox data, so only on request.
                .filter(|f| {
                    f.supported
                        && f.id != SYNC_EDB_HISTORY_FROM_PDB_FIX_ID
                        && f.id != ENGINE_LIBRARY_FIX_ID
                })
                .map(|f| f.id.clone())
                .collect::<std::collections::HashSet<_>>()
        } else {
            req.selected_fix_ids.iter().cloned().collect()
        };
        // Bookkeeping is never offered for selection; it rides along with
        // every repair that runs.
        if req.apply {
            selected.extend(
                proposed_fixes
                    .iter()
                    .filter(|f| f.bookkeeping)
                    .map(|f| f.id.clone()),
            );
        }
        let selected_fix_count = proposed_fixes
            .iter()
            // The Engine DJ library is rebuilt after database write-back and
            // has its own 95% progress step outside the normal apply window.
            .filter(|fix| {
                fix.supported && fix.id != ENGINE_LIBRARY_FIX_ID && selected.contains(&fix.id)
            })
            .count();

        let found = DetectedRepairs {
            empty_analysis_paths,
            parsed_pdb,
            bpm_key_mismatches,
            analysis_ownership,
            app_master_db_id,
            app_beat_grids,
            missing_seek_data,
            pdb_wrong_history_shape_pages,
            pdb_ec_conflicts,
            pdb_truncated_chains,
            pdb_torn_growth,
            pdb_t19_template_state,
            pdb_misaligned_track_ids,
            pdb_misaligned_album_ids,
            pdb_misaligned_artist_ids,
            pdb_header_compatibility_repair,
            missing_audio_paths,
            unindexed_audio_files,
            duplicate_audio_copies,
            moved_audio_relinks,
            sync_edb_history_supported,
            sync_edb_history_needed,
            strict_upgrade_targets,
            pdb_duplicate_entry_count,
            engine_library_fix_needed,
        };
        let outcomes = if req.apply {
            on_progress(70, 100, "USB: Applying selected fixes");
            let mut progress = ApplyProgress::new(&mut on_progress, selected_fix_count);
            self.apply_selected_fixes(
                &usb_root,
                &selected,
                found,
                &mut warnings,
                &mut estimated_file_writes,
                &mut progress,
            )
        } else {
            FixOutcomes::default()
        };

        detected_issues.sort();
        detected_issues.dedup();
        {
            let mut seen = HashSet::<String>::new();
            unsupported_items.retain(|item| seen.insert(format!("{}|{}", item.issue, item.reason)));
        }

        // The outcome of every fix also goes to the Event Log, so a log a user
        // sends shows what the repair actually did.
        if req.apply {
            let outcomes = [
                (
                    Level::Info,
                    "usb.repair.applied",
                    "applied",
                    &outcomes.applied,
                ),
                (
                    Level::Info,
                    "usb.repair.skipped",
                    "skipped",
                    &outcomes.skipped,
                ),
                (
                    Level::Error,
                    "usb.repair.failed",
                    "failed",
                    &outcomes.failed,
                ),
            ];
            for (level, code, outcome, fixes) in outcomes {
                for fix in fixes {
                    warnings.push(logging::log(
                        level,
                        "usb-repair",
                        code,
                        format!("repair {outcome}: {fix}"),
                    ));
                }
            }
        }

        let diagnostics_after_apply = if req.apply {
            on_progress(96, 100, "USB: Re-running diagnostics after repair");
            match self.run_usb_diagnostics_with_progress(
                RunUsbDiagnosticsRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                },
                |c, t, m| {
                    let pct = 96 + ((c * 3) / t.max(1)).min(3);
                    on_progress(pct, 100, m);
                },
            ) {
                Ok(data) => Some(Box::new(data)),
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.post-diagnostics-failed",
                        format!("Post-repair diagnostics failed: {err}"),
                    ));
                    None
                }
            }
        } else {
            None
        };

        on_progress(
            100,
            100,
            if req.apply {
                "USB: Repair apply complete"
            } else {
                "USB: Repair preview ready"
            },
        );
        let (summary, overview) = repair_overview(&proposed_fixes, &unsupported_items);
        Ok(RepairUsbDiagnosticsData {
            summary,
            overview,
            detected_issues,
            proposed_fixes,
            unsupported_items,
            applied_fixes: outcomes.applied,
            skipped_fixes: outcomes.skipped,
            failed_fixes: outcomes.failed,
            estimated_file_writes,
            estimated_file_deletes,
            warnings,
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics: diagnostics_after_apply,
            engine_library_enabled: self.engine_library_enabled(),
        })
    }

    /// Applies every selected fix, in apply order, to what the detect phase
    /// found.
    fn apply_selected_fixes(
        &self,
        usb_root: &Path,
        selected: &HashSet<String>,
        found: DetectedRepairs,
        warnings: &mut Vec<WarningEntry>,
        estimated_file_writes: &mut usize,
        progress: &mut ApplyProgress<'_>,
    ) -> FixOutcomes {
        let DetectedRepairs {
            empty_analysis_paths,
            parsed_pdb,
            bpm_key_mismatches,
            analysis_ownership,
            app_master_db_id,
            app_beat_grids,
            missing_seek_data,
            pdb_wrong_history_shape_pages,
            pdb_ec_conflicts,
            pdb_truncated_chains,
            pdb_torn_growth,
            pdb_t19_template_state,
            pdb_misaligned_track_ids,
            pdb_misaligned_album_ids,
            pdb_misaligned_artist_ids,
            pdb_header_compatibility_repair,
            missing_audio_paths,
            unindexed_audio_files,
            duplicate_audio_copies,
            moved_audio_relinks,
            sync_edb_history_supported,
            sync_edb_history_needed,
            strict_upgrade_targets,
            pdb_duplicate_entry_count,
            engine_library_fix_needed,
        } = found;
        let mut outcomes = FixOutcomes::default();

        progress.report(70, "USB: Backing up databases before repair");
        warnings.extend(
            self.backup_usb_databases_with_retention(usb_root, "Before repair", None)
                .into_iter()
                .map(|message| {
                    logging::log(
                        Level::Info,
                        "usb-diagnostics",
                        "usb.diagnostics.backup",
                        message,
                    )
                }),
        );

        // Open the eDB read-write once for every fix below instead of
        // each fix independently re-staging/re-opening it -- SQLCipher
        // key negotiation is the expensive part. Individual fixes fall
        // back to opening their own connection if this fails.
        progress.report(70, "USB: Opening repair databases");
        let mut fix_edb_conn = open_edb_rw(usb_root, warnings);
        let mut pdb = match StagedPdb::load(usb_root) {
            Ok(pdb) => pdb,
            Err(err) => {
                outcomes.failed.push(format!(
                    "Reading the staged PDB failed, nothing was applied: {err}"
                ));
                return outcomes;
            }
        };

        if selected.contains(EMPTY_ANALYSIS_FILES_FIX_ID) {
            progress.start("Fix Empty Analysis Files");
            match pdb.on_file(|| {
                self.apply_fix_empty_analysis_files(
                    usb_root,
                    &empty_analysis_paths,
                    fix_edb_conn.as_mut(),
                    parsed_pdb.as_ref(),
                    warnings,
                )
            }) {
                Ok((fixed, skipped, failed, write_count)) => {
                    *estimated_file_writes = (*estimated_file_writes).max(write_count);
                    if failed > 0 {
                        outcomes.failed.push(format!(
                            "Fix Empty Analysis Files: fixed {fixed}, skipped {skipped}, failed {failed}"
                        ));
                    } else if fixed > 0 {
                        outcomes.applied.push(format!(
                            "Fix Empty Analysis Files: fixed {fixed}, skipped {skipped}"
                        ));
                    } else {
                        outcomes
                            .skipped
                            .push("Fix Empty Analysis Files: nothing to apply".to_string());
                    }
                }
                Err(err) => outcomes
                    .failed
                    .push(format!("Fix Empty Analysis Files failed: {err}")),
            }
            progress.finish();
        } else if !empty_analysis_paths.is_empty() {
            outcomes
                .skipped
                .push("Fix Empty Analysis Files: not selected".to_string());
        }

        // Unlike the other structural repairs (which are safe to run after
        // strict parity, since write_pdb's own additive writes establish
        // correct page structure), a truncated table chain must be fixed
        // *before* strict parity: write_pdb's additive track-append path
        // hard-fails on a table whose declared last page doesn't physically
        // exist, so every strict-parity mutation that needs a new/changed
        // track row would otherwise fail (or only succeed once enough other
        // playlists happen to have organically grown the chain past it).
        outcomes.record_with_progress(
            progress,
            "Repair Truncated Table Chain",
            selected.contains(PDB_TRUNCATED_TABLE_CHAIN_FIX_ID),
            !pdb_truncated_chains.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_truncated_table_chain_repair(bytes, &pdb_truncated_chains)
                })
                .map(|n| format!("fixed {}", counted(n, "table", "tables")))
            },
        );

        // Also unlike the other structural repairs, torn-growth-pages must be
        // fixed *before* strict parity, for a different reason: its
        // `truncate_to_pages` boundary was computed once, up front, against
        // the pre-repair file. Strict parity's additive writer legitimately
        // grows the file past that boundary while merging playlists/tracks
        // (`append_rows_to_chain_in_place`); if this repair ran afterward
        // using the now-stale boundary, it would truncate the file back down
        // and silently drop everything strict parity had just written.
        // Neither `fix_empty_analysis_files` nor the truncated-table-chain
        // repair above grow the PDB file, so running this immediately after
        // them (and still before strict parity) keeps the boundary accurate.
        outcomes.record_with_progress(
            progress,
            "Repair Torn Growth Pages",
            selected.contains(PDB_TORN_GROWTH_PAGES_FIX_ID),
            !pdb_torn_growth.is_empty(),
            || {
                pdb.patch(|bytes| apply_pdb_torn_growth_pages_repair(bytes, &pdb_torn_growth))
                    .map(|n| {
                        format!(
                            "zeroed {}{}",
                            counted(n, "page", "pages"),
                            if pdb_torn_growth.truncate_to_pages.is_some() {
                                ", truncated unpopulated tail"
                            } else {
                                ""
                            }
                        )
                    })
            },
        );

        // Before strict parity, so its additive write already finds t19
        // counting the tracks and leaves it alone (parity's own additive
        // writes keep it in sync if they add tracks). Grows nothing, so the torn-growth
        // boundary above stays valid.
        outcomes.record_with_progress(
            progress,
            "Activate PDB History Page",
            selected.contains(PDB_T19_TEMPLATE_STATE_FIX_ID),
            pdb_t19_template_state,
            || {
                let property = crate::service::export_helpers::edb_t19_property(usb_root);
                pdb.patch(|bytes| apply_pdb_t19_template_state_repair(bytes, property.as_ref()))
                    .map(|activated| {
                        if activated {
                            "rewrote tt=19 page for the track count".to_string()
                        } else {
                            "already matches the track count".to_string()
                        }
                    })
            },
        );

        // Strict parity runs after the two structural prerequisites above so
        // that write_pdb establishes correct page structure before the
        // remaining structural repairs (especially stale B-tree) run over
        // the final result in one clean pass.
        if selected.contains(STRICT_PARITY_UPGRADE_FIX_ID) {
            let force_all = strict_repair_force_all_enabled();
            if strict_upgrade_targets.is_empty() && !force_all {
                outcomes
                    .skipped
                    .push("Upgrade Export Data To Strict Parity: nothing to apply".to_string());
            } else {
                let target_names = if force_all {
                    None
                } else {
                    Some(&strict_upgrade_targets)
                };
                progress.start("Upgrade Export Data To Strict Parity");
                match pdb.on_file(|| {
                    self.apply_strict_parity_upgrade(
                        usb_root,
                        target_names,
                        fix_edb_conn.as_mut(),
                        warnings,
                    )
                }) {
                    Ok(result) if result.merged_playlists > 0 => {
                        outcomes.applied.push(format!(
                            "Upgrade Export Data To Strict Parity: {}",
                            strict_parity_summary(&result)
                        ));
                    }
                    Ok(_) => outcomes
                        .skipped
                        .push("Upgrade Export Data To Strict Parity: nothing to apply".to_string()),
                    Err(err) => outcomes.failed.push(format!(
                        "Upgrade Export Data To Strict Parity failed: {err}"
                    )),
                }
                progress.finish();
            }
        } else if !strict_upgrade_targets.is_empty() {
            outcomes
                .skipped
                .push("Upgrade Export Data To Strict Parity: not selected".to_string());
        }

        if selected.contains(PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID) {
            progress.start("Remove Duplicate PDB Playlist Entries");
            match pdb.patch(|bytes| apply_pdb_duplicate_playlist_entries_repair(bytes)) {
                Ok(0) => outcomes
                    .skipped
                    .push("Remove Duplicate PDB Playlist Entries: nothing to apply".to_string()),
                Ok(n) => outcomes.applied.push(format!(
                    "Remove Duplicate PDB Playlist Entries: removed {}",
                    counted(n, "duplicate entry", "duplicate entries")
                )),
                Err(err) => outcomes.failed.push(format!(
                    "Remove Duplicate PDB Playlist Entries failed: {err}"
                )),
            }
            progress.finish();
        } else if pdb_duplicate_entry_count > 0 {
            outcomes
                .skipped
                .push("Remove Duplicate PDB Playlist Entries: not selected".to_string());
        }

        // rekordbox bookkeeping runs with every apply, so it always follows
        // the fixes above that rewrite the PDB (strict parity appends rows
        // and pages). Detect it again on the current bytes instead of
        // patching the preview's page snapshot.
        let pdb_zero_tranrf_pages = detect_pdb_zero_tranrf_all_tables(&pdb.bytes);
        let pdb_wrong_flags_pages = detect_pdb_wrong_page_flags(&pdb.bytes);
        let pdb_sentinel_u5_pages = detect_pdb_sentinel_u5_on_data_pages(&pdb.bytes);
        let pdb_wrong_track_u5_pages = detect_pdb_wrong_track_u5(&pdb.bytes);
        let pdb_t00_multipage_active_pages = detect_pdb_t00_multipage_active_pages(&pdb.bytes);
        let pdb_tombstoned_playlist_ids = detect_pdb_tombstoned_playlist_tree_ids(&pdb.bytes);
        let pdb_wrong_playlist_tree_shape_pages = detect_pdb_wrong_playlist_tree_shape(&pdb.bytes);
        // The page and row targets the preview found are stale once the
        // fixes above wrote the PDB (strict parity, the audio fixes); find
        // them again on the current bytes. A target the writers already fixed
        // is then simply gone.
        let pdb_wrong_history_shape_pages = if pdb_wrong_history_shape_pages.is_empty() {
            pdb_wrong_history_shape_pages
        } else {
            detect_pdb_wrong_history_page_shape(&pdb.bytes)
        };
        let pdb_ec_conflicts = if pdb_ec_conflicts.is_empty() {
            pdb_ec_conflicts
        } else {
            detect_pdb_ec_data_page_conflicts(&pdb.bytes)
        };
        let pdb_misaligned_slots = if pdb_misaligned_track_ids.is_empty()
            && pdb_misaligned_album_ids.is_empty()
            && pdb_misaligned_artist_ids.is_empty()
        {
            Vec::new()
        } else {
            detect_pdb_string_misalignment(&pdb.bytes)
        };
        let pdb_misaligned_track_ids = misaligned_row_ids(&pdb_misaligned_slots, 0);
        let pdb_misaligned_album_ids = misaligned_row_ids(&pdb_misaligned_slots, 3);
        let pdb_misaligned_artist_ids = misaligned_row_ids(&pdb_misaligned_slots, 2);
        outcomes.record_with_progress(
            progress,
            "Repair Data Pages With Zero tranrf",
            selected.contains(PDB_ZERO_TRANRF_FIX_ID),
            !pdb_zero_tranrf_pages.is_empty(),
            || {
                pdb.patch(|bytes| apply_pdb_zero_tranrf_repair(bytes, &pdb_zero_tranrf_pages))
                    .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair PDB Data Page Flags",
            selected.contains(PDB_WRONG_PAGE_FLAGS_FIX_ID),
            !pdb_wrong_flags_pages.is_empty(),
            || {
                pdb.patch(|bytes| apply_pdb_wrong_page_flags_repair(bytes, &pdb_wrong_flags_pages))
                    .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair PDB Data Pages With Sentinel u5",
            selected.contains(PDB_SENTINEL_U5_FIX_ID),
            !pdb_sentinel_u5_pages.is_empty(),
            || {
                pdb.patch(|bytes| apply_pdb_sentinel_u5_repair(bytes, &pdb_sentinel_u5_pages))
                    .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair PDB History/Columns Page Footer Shape",
            selected.contains(PDB_WRONG_HISTORY_SHAPE_FIX_ID),
            !pdb_wrong_history_shape_pages.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_wrong_history_page_shape_repair(bytes, &pdb_wrong_history_shape_pages)
                })
                .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair PDB Track Page Footer Shape",
            selected.contains(PDB_WRONG_TRACK_U5_FIX_ID),
            !pdb_wrong_track_u5_pages.is_empty(),
            || {
                pdb.patch(|bytes| apply_pdb_wrong_track_u5_repair(bytes, &pdb_wrong_track_u5_pages))
                    .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair Track Page Flag (Multi-Page Chain)",
            selected.contains(PDB_T00_MULTIPAGE_ACTIVE_FIX_ID),
            !pdb_t00_multipage_active_pages.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_t00_multipage_active_repair(bytes, &pdb_t00_multipage_active_pages)
                })
                .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair Tombstoned Playlist Tree IDs",
            selected.contains(PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID),
            !pdb_tombstoned_playlist_ids.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_tombstoned_playlist_tree_id_repair(
                        bytes,
                        &pdb_tombstoned_playlist_ids,
                    )
                })
                .map(|n| format!("zeroed id in {}", counted(n, "slot", "slots")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair PDB Playlist-Tree Page Shape",
            selected.contains(PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID),
            !pdb_wrong_playlist_tree_shape_pages.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_wrong_playlist_tree_shape_repair(
                        bytes,
                        &pdb_wrong_playlist_tree_shape_pages,
                    )
                })
                .map(|n| format!("patched {}", counted(n, "page", "pages")))
            },
        );

        // After the page fixes above: resealing a track page can leave its
        // sentinel's B-tree stale.
        let pdb_stale_sentinel_btree_pages = detect_pdb_stale_sentinel_btree(&pdb.bytes);
        outcomes.record_with_progress(
            progress,
            "Repair Stale Sentinel B-Tree",
            selected.contains(PDB_STALE_SENTINEL_BTREE_FIX_ID),
            !pdb_stale_sentinel_btree_pages.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_stale_sentinel_btree_repair(bytes, &pdb_stale_sentinel_btree_pages)
                })
                .map(|n| format!("reset {}", counted(n, "sentinel page", "sentinel pages")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair Table Write-Pointer Conflict",
            selected.contains(PDB_EC_CONFLICT_FIX_ID),
            !pdb_ec_conflicts.is_empty(),
            || {
                pdb.patch(|bytes| apply_pdb_ec_data_page_conflict_repair(bytes, &pdb_ec_conflicts))
                    .map(|n| format!("fixed {}", counted(n, "table", "tables")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair Track String Alignment",
            selected.contains(PDB_TRACK_STRING_ALIGNMENT_FIX_ID),
            !pdb_misaligned_track_ids.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_track_string_alignment_repair(bytes, &pdb_misaligned_track_ids)
                })
                .map(|n| format!("fixed {}", counted(n, "row", "rows")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair Album String Alignment",
            selected.contains(PDB_ALBUM_STRING_ALIGNMENT_FIX_ID),
            !pdb_misaligned_album_ids.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_album_string_alignment_repair(bytes, &pdb_misaligned_album_ids)
                })
                .map(|n| format!("fixed {}", counted(n, "row", "rows")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Repair Artist Name Layout",
            selected.contains(PDB_ARTIST_NAME_LAYOUT_FIX_ID),
            !pdb_misaligned_artist_ids.is_empty(),
            || {
                pdb.patch(|bytes| {
                    apply_pdb_artist_name_layout_repair(bytes, &pdb_misaligned_artist_ids)
                })
                .map(|n| format!("fixed {}", counted(n, "row", "rows")))
            },
        );

        if selected.contains(PDB_HEADER_COMPATIBILITY_FIX_ID) {
            if let Some(repair) = pdb_header_compatibility_repair.as_ref() {
                progress.start("Repair PDB Header Compatibility Field");
                match pdb.patch(|bytes| apply_pdb_header_compatibility_repair(bytes, repair)) {
                    Ok(true) => outcomes.applied.push(format!(
                        "Repair PDB Header Compatibility Field: wrote 0x10..0x14 {}->{}",
                        repair.current_value, repair.target_value,
                    )),
                    Ok(false) => outcomes.skipped.push(
                        "Repair PDB Header Compatibility Field: nothing to apply".to_string(),
                    ),
                    Err(err) => outcomes.failed.push(format!(
                        "Repair PDB Header Compatibility Field failed: {err}"
                    )),
                }
                progress.finish();
            } else {
                outcomes
                    .skipped
                    .push("Repair PDB Header Compatibility Field: nothing to apply".to_string());
            }
        } else if pdb_header_compatibility_repair.is_some() {
            outcomes
                .skipped
                .push("Repair PDB Header Compatibility Field: not selected".to_string());
        }

        outcomes.record_with_progress(
            progress,
            "Relink Moved Audio Files",
            selected.contains(RELINK_MOVED_AUDIO_FIX_ID),
            !moved_audio_relinks.is_empty(),
            || {
                pdb.on_file(|| {
                    self.apply_fix_relink_moved_audio(
                        usb_root,
                        &moved_audio_relinks,
                        fix_edb_conn.as_mut(),
                        warnings,
                    )
                })
                .map(|relinked| format!("relinked {}", counted(relinked, "track", "tracks")))
            },
        );

        outcomes.record_with_progress(
            progress,
            "Add Unindexed Audio to a Playlist",
            selected.contains(UNINDEXED_AUDIO_PLAYLIST_FIX_ID),
            !unindexed_audio_files.is_empty(),
            || {
                pdb.on_file(|| {
                    self.apply_fix_add_unindexed_audio_playlist(
                        usb_root,
                        &unindexed_audio_files,
                        fix_edb_conn.as_mut(),
                        warnings,
                    )
                })
                .map(|(added, analysed, without_analysis)| {
                    let mut notes = Vec::new();
                    if analysed > 0 {
                        notes.push(format!("{analysed} analysed from the audio"));
                    }
                    if without_analysis > 0 {
                        notes.push(format!("{without_analysis} without analysis files"));
                    }
                    let note = if notes.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", notes.join(", "))
                    };
                    format!(
                        "added {} to \"{UNINDEXED_AUDIO_PLAYLIST_NAME}\"{note}",
                        counted(added, "track", "tracks")
                    )
                })
            },
        );

        outcomes.record_with_progress(
            progress,
            "Remove Duplicate Audio Copies",
            selected.contains(DUPLICATE_AUDIO_COPIES_FIX_ID),
            !duplicate_audio_copies.is_empty(),
            || {
                apply_remove_duplicate_audio_copies(usb_root, &duplicate_audio_copies, warnings)
                    .map(|deleted| format!("deleted {}", counted(deleted, "file", "files")))
            },
        );

        if selected.contains(REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID) {
            progress.start("Remove Missing Audio References");
            match pdb.on_file(|| {
                self.apply_fix_remove_missing_audio_references(
                    usb_root,
                    &missing_audio_paths,
                    fix_edb_conn.as_mut(),
                    warnings,
                )
            }) {
                Ok((removed_db_content, removed_db_playlist_links, removed_pdb)) => {
                    if removed_db_content > 0
                        || removed_db_playlist_links > 0
                        || removed_pdb != PdbTrackRemoval::default()
                    {
                        outcomes.applied.push(format!(
                                "Remove Missing Audio References: removed content rows {removed_db_content}, playlist_content rows {removed_db_playlist_links}, PDB playlist entries {}, PDB track rows {}",
                                removed_pdb.playlist_entries, removed_pdb.track_rows
                            ));
                    } else {
                        outcomes
                            .skipped
                            .push("Remove Missing Audio References: nothing to apply".to_string());
                    }
                }
                Err(err) => outcomes
                    .failed
                    .push(format!("Remove Missing Audio References failed: {err}")),
            }
            progress.finish();
        } else if !missing_audio_paths.is_empty() {
            outcomes
                .skipped
                .push("Remove Missing Audio References: not selected".to_string());
        }

        outcomes.record_with_progress(
            progress,
            "Give Tracks Their Own Analysis Files",
            selected.contains(SHARED_ANALYSIS_FIX_ID),
            !analysis_ownership.shared.is_empty(),
            || {
                pdb.on_file(|| {
                    apply_separate_shared_analysis(
                        usb_root,
                        app_master_db_id,
                        fix_edb_conn.as_mut(),
                        warnings,
                    )
                })
                .map(|(moved, kept)| {
                    format!(
                        "gave {} a new analysis file, {kept} kept the original",
                        counted(moved, "track", "tracks")
                    )
                })
            },
        );
        outcomes.record_with_progress(
            progress,
            "Add Track Path to Analysis Files",
            selected.contains(ANALYSIS_TRACK_PATH_FIX_ID),
            !analysis_ownership.unmarked.is_empty(),
            || {
                pdb.on_file(|| apply_add_analysis_track_path(usb_root, app_master_db_id))
                    .map(|stamped| {
                        format!(
                            "named the track in {}",
                            counted(stamped, "analysis file", "analysis files")
                        )
                    })
            },
        );

        let bpm_selected = selected.contains(BPM_KEY_MISMATCH_FIX_ID);
        let bpm_found = !bpm_key_mismatches.is_empty();
        if bpm_selected && bpm_found {
            progress.start("Fix BPM/Key Mismatch");
        }
        outcomes.record("Fix BPM/Key Mismatch", bpm_selected, bpm_found, || {
            pdb.on_file(|| {
                apply_bpm_key_mismatch_repair(
                    usb_root,
                    app_master_db_id,
                    fix_edb_conn.as_mut(),
                    warnings,
                    &mut |done, total| progress.item_done(done, total),
                )
            })
            .map(|(anlz_fixed, edb_fixed)| {
                format!(
                    "rebuilt {}, updated {}",
                    counted(anlz_fixed, "beat grid", "beat grids"),
                    counted(edb_fixed, "eDB row", "eDB rows")
                )
            })
        });
        if bpm_selected && bpm_found {
            progress.finish();
        }

        let grid_selected = selected.contains(BEAT_GRID_HEADER_FIX_ID);
        let grid_found = !app_beat_grids.is_empty();
        if grid_selected && grid_found {
            progress.start("Fix Beat Grid");
        }
        outcomes.record("Fix Beat Grid", grid_selected, grid_found, || {
            pdb.on_file(|| {
                apply_beat_grid_repair(usb_root, warnings, &mut |done, total| {
                    progress.item_done(done, total)
                })
            })
            .map(|fixed| format!("fixed {}", counted(fixed, "bundle", "bundles")))
        });
        if grid_selected && grid_found {
            progress.finish();
        }

        for (format, missing) in &missing_seek_data {
            let selected_format = selected.contains(format.fix_id());
            let found = !missing.is_empty();
            if selected_format && found {
                progress.start(format.title());
            }
            outcomes.record(format.title(), selected_format, found, || {
                pdb.on_file(|| {
                    apply_missing_seek_data_repair(
                        usb_root,
                        *format,
                        warnings,
                        &mut |done, total| progress.item_done(done, total),
                    )
                })
                .map(|(fixed, skipped)| format!("fixed {fixed}, skipped {skipped}"))
            });
            if selected_format && found {
                progress.finish();
            }
        }

        if selected.contains(SYNC_EDB_HISTORY_FROM_PDB_FIX_ID) {
            if !sync_edb_history_supported || !sync_edb_history_needed {
                outcomes
                    .skipped
                    .push("Sync eDB History Tables From PDB: nothing to apply".to_string());
            } else if let Some(parsed) = parsed_pdb.as_ref() {
                progress.start("Sync eDB History Tables From PDB");
                match self.apply_fix_sync_edb_history_from_pdb(
                    usb_root,
                    parsed,
                    fix_edb_conn.as_mut(),
                    warnings,
                ) {
                    Ok((history_written, history_content_written)) => {
                        outcomes.applied.push(format!(
                            "Sync eDB History Tables From PDB: wrote history {history_written}, history_content {history_content_written}"
                        ));
                    }
                    Err(err) => outcomes
                        .failed
                        .push(format!("Sync eDB History Tables From PDB failed: {err}")),
                }
                progress.finish();
            } else {
                outcomes
                    .skipped
                    .push("Sync eDB History Tables From PDB: PDB unavailable".to_string());
            }
        } else if sync_edb_history_needed {
            outcomes
                .skipped
                .push("Sync eDB History Tables From PDB: not selected".to_string());
        }

        // Checkpoint the shared eDB connection's WAL into the main db
        // file before write-back reads it off disk below -- write-back
        // copies raw file bytes, not through SQLite, so any change still
        // sitting in the WAL would otherwise be silently dropped.
        if let Some(conn) = fix_edb_conn.as_ref() {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        drop(fix_edb_conn);

        // All selected fixes above changed only the staged copies (see
        // usb_staging.rs); save them to the USB here, both or neither.
        progress.report(95, "USB: Saving repaired databases");
        let saved = write_back_both_or_neither(usb_root, &mut pdb);
        if let Err(err) = &saved {
            outcomes.not_saved(err);
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.write-back-failed",
                format!("Saving the repaired databases to the USB failed: {err}"),
            ));
        }

        // After the write-back, so it reads the repaired PDB.
        if saved.is_err() {
            if selected.contains(ENGINE_LIBRARY_FIX_ID) && engine_library_fix_needed {
                outcomes.failed.push(
                    "Keep Engine DJ Library Up to Date: not run, the repaired databases were not saved"
                        .to_string(),
                );
            }
        } else if selected.contains(ENGINE_LIBRARY_FIX_ID) && engine_library_fix_needed {
            progress.report(95, "USB: Applying Write Engine DJ Library");
            match self
                .enable_engine_library()
                .and_then(|()| crate::service::rebuild_engine_library(usb_root))
            {
                Ok(summary) => {
                    warnings.extend(summary.warnings);
                    outcomes.applied.push(format!(
                        "Keep Engine DJ Library Up to Date: setting on, rebuilt ({}, {})",
                        counted(summary.tracks, "track", "tracks"),
                        counted(summary.playlists, "playlist", "playlists")
                    ));
                }
                Err(err) => outcomes
                    .failed
                    .push(format!("Keep Engine DJ Library Up to Date failed: {err}")),
            }
        } else {
            if engine_library_fix_needed {
                outcomes
                    .skipped
                    .push("Keep Engine DJ Library Up to Date: not selected".to_string());
            }
            warnings.extend(self.refresh_engine_library(usb_root));
        }

        outcomes
    }
}
