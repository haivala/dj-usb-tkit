//! The repair run: detect issues, propose fixes, apply the selected ones.

use super::*;

/// A supported, non-destructive proposal that writes one file -- the shape
/// most fixes share. Fixes that differ override fields with struct-update
/// syntax; `always_applied` is filled in once every proposal is collected.
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
    pdb_sentinel_u5_pages: Vec<SentinelU5Page>,
    pdb_wrong_flags_pages: Vec<WrongFlagsPage>,
    pdb_zero_tranrf_pages: Vec<ZeroTranrfPage>,
    pdb_wrong_history_shape_pages: Vec<WrongShapeHistoryPage>,
    pdb_tombstoned_playlist_ids: Vec<TombstonedRowId>,
    pdb_wrong_track_u5_pages: Vec<WrongTrackU5Page>,
    pdb_t00_multipage_active_pages: Vec<T00MultipageActivePage>,
    pdb_stale_sentinel_btree_pages: Vec<StaleSentinelBtree>,
    pdb_wrong_playlist_tree_shape_pages: Vec<WrongPlaylistTreeShapePage>,
    pdb_ec_conflicts: Vec<EcDataPageConflict>,
    pdb_truncated_chains: Vec<TruncatedTableChain>,
    pdb_torn_growth: PdbTornGrowthReport,
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

/// One line per fix: applied, skipped or failed.
#[derive(Default)]
pub(super) struct FixOutcomes {
    pub(super) applied: Vec<String>,
    pub(super) skipped: Vec<String>,
    pub(super) failed: Vec<String>,
}

impl FixOutcomes {
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
                    let pct = 5 + ((c * 50) / t.max(1)).min(50);
                    on_progress(pct, 100, m);
                },
            )?;

        on_progress(60, 100, "USB: Building parity guidance");
        let parity = self.run_usb_parity_report_with_progress_impl(
            RunUsbParityReportRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            },
            edb_conn.as_ref(),
            Some(&staged_pdb_path),
            parsed_pdb_from_diagnostics.as_ref(),
            |c, t, m| {
                let pct = 60 + ((c * 20) / t.max(1)).min(20);
                on_progress(pct, 100, m);
            },
        );

        on_progress(85, 100, "USB: Collecting repair opportunities");
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
                "{} empty USB analysis file(s) detected",
                empty_analysis_paths.len()
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
                "{} malformed entry/entries detected under USBANLZ",
                malformed_analysis_entries.len()
            ));
            unsupported_items.push(RepairUnsupportedItem {
                issue: format!(
                    "{} malformed USBANLZ entry/entries",
                    malformed_analysis_entries.len()
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
                "{} track(s) with ANLZ beat-grid tempo or eDB BPM/key differing from the PDB",
                bpm_key_mismatches.len()
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
                "{} analysis file(s) shared by tracks that need their own ({movers} track(s))",
                analysis_ownership.shared.len()
            ));
            proposed_fixes.push(RepairFixProposal {
                destructive: true,
                estimated_writes: movers * 3,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    SHARED_ANALYSIS_FIX_ID,
                    "Give Tracks Their Own Analysis Files",
                    format!(
                        "{movers} track(s) use another track's analysis file, so players can \
                         show that track's waveform and beat grid for them. Each gets its own \
                         file in its folder (ANLZ0001 and on, as rekordbox names them), analysed \
                         from its audio with its BPM; the file's own track keeps it. Their old \
                         cues stay with the other track. See Event Log for the tracks."
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
                "{count} analysis file(s) without the track path players match them by"
            ));
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: count * 3,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    ANALYSIS_TRACK_PATH_FIX_ID,
                    "Add Track Path to Analysis Files",
                    format!(
                        "{count} analysis file(s) don't name their track (PPTH), which players \
                         use to find a track's file when two tracks hash to the same folder. \
                         Writes the track path into each; nothing else changes. See Event Log \
                         for the files."
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
                "{} analysis bundle(s) with a beat grid in the pre-0.3.7 format",
                app_beat_grids.len()
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
                    // Files the fix would skip on every pass are listed, not proposed.
                    let (fixable, unfixable) = partition_fixable_seek_data(missing);
                    if !unfixable.is_empty() {
                        unsupported_items.push(RepairUnsupportedItem {
                            issue: format!(
                                "{} {} track(s) without seek data that can't be added",
                                unfixable.len(),
                                format.name()
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
                "{} {} track(s) without the seek data rekordbox writes",
                missing.len(),
                format.name()
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
        let pdb_sentinel_u5_pages = detect_pdb_sentinel_u5_on_data_pages(&staged_pdb_path);
        let pdb_wrong_flags_pages = detect_pdb_wrong_page_flags(&staged_pdb_path);
        let pdb_zero_tranrf_pages = detect_pdb_zero_tranrf_all_tables(&staged_pdb_path);
        let pdb_wrong_history_shape_pages = detect_pdb_wrong_history_page_shape(&staged_pdb_path);
        let pdb_tombstoned_playlist_ids = detect_pdb_tombstoned_playlist_tree_ids(&staged_pdb_path);
        let pdb_wrong_track_u5_pages = detect_pdb_wrong_track_u5(&staged_pdb_path);
        let pdb_t00_multipage_active_pages =
            detect_pdb_t00_multipage_active_pages(&staged_pdb_path);
        let pdb_stale_sentinel_btree_pages = detect_pdb_stale_sentinel_btree(&staged_pdb_path);
        let pdb_wrong_playlist_tree_shape_pages =
            detect_pdb_wrong_playlist_tree_shape(&staged_pdb_path);
        let pdb_ec_conflicts = detect_pdb_ec_data_page_conflicts(&staged_pdb_path);
        let pdb_truncated_chains = detect_pdb_truncated_table_chains(&staged_pdb_path);
        let pdb_torn_growth = detect_pdb_torn_growth_pages(&staged_pdb_path);
        let pdb_misaligned_slots = detect_pdb_string_misalignment(&staged_pdb_path);
        let pdb_misaligned_track_ids: Vec<u32> = {
            let mut ids: Vec<u32> = pdb_misaligned_slots
                .iter()
                .filter(|m| m.table_type == 0)
                .map(|m| m.id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort_unstable();
            ids
        };
        let pdb_misaligned_album_ids: Vec<u32> = {
            let mut ids: Vec<u32> = pdb_misaligned_slots
                .iter()
                .filter(|m| m.table_type == 3)
                .map(|m| m.id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort_unstable();
            ids
        };
        let pdb_misaligned_artist_ids: Vec<u32> = {
            let mut ids: Vec<u32> = pdb_misaligned_slots
                .iter()
                .filter(|m| m.table_type == 2)
                .map(|m| m.id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort_unstable();
            ids
        };
        if !pdb_zero_tranrf_pages.is_empty() {
            detected_issues.push(format!(
                "{} data page(s) with tranrf=0 and active rows (may be rejected by some DJ software versions)",
                pdb_zero_tranrf_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_ZERO_TRANRF_FIX_ID,
                "Repair Data Pages With Zero tranrf",
                format!(
                    "Set tranrf = rowpf on {} data page(s) where tranrf is zero but active rows \
                     exist. Some DJ software versions reject databases where tranrf is cleared on \
                     a live data page.",
                    pdb_zero_tranrf_pages.len()
                ),
            ));
        }
        if !pdb_wrong_flags_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB data page(s) with wrong page_flags byte (firmware may reject as corrupted)",
                pdb_wrong_flags_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_PAGE_FLAGS_FIX_ID,
                "Repair PDB Data Page Flags",
                format!(
                    "Patch page_flags byte (0x1b) on {} data page(s) to the correct per-table value \
                     (tracks/history=0x34, all others=0x24).",
                    pdb_wrong_flags_pages.len()
                ),
            ));
        }
        if !pdb_sentinel_u5_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB data page(s) with sentinel u5=0x1FFF (may be rejected by some DJ software and player firmware)",
                pdb_sentinel_u5_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_SENTINEL_U5_FIX_ID,
                "Repair PDB Data Pages With Sentinel u5",
                format!(
                    "Patch u5/num_rl header fields on {} data page(s) from sentinel 0x1FFF to the correct per-table value. \
                     Some DJ software and player firmware versions may reject data pages with u5=0x1FFF.",
                    pdb_sentinel_u5_pages.len()
                ),
            ));
        }
        if !pdb_wrong_history_shape_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB history/columns page(s) with wrong u5/num_rl shape (may be rejected as corrupted)",
                pdb_wrong_history_shape_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_HISTORY_SHAPE_FIX_ID,
                "Repair PDB History/Columns Page Footer Shape",
                format!(
                    "Patch u5/num_rl footer fields on {} tt=16/17/18 data page(s) from the wrong \
                     (1, nrs-1) shape to the correct (nrs, 0) convention.",
                    pdb_wrong_history_shape_pages.len()
                ),
            ));
        }
        if !pdb_wrong_track_u5_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB track page(s) with u5/num_rl shape outside expected values",
                pdb_wrong_track_u5_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_TRACK_U5_FIX_ID,
                "Repair PDB Track Page Footer Shape",
                format!(
                    "Normalise u5/num_rl footer fields on {} tt=0 track page(s) to (1, nrs-1), \
                     the format written by the current writer. (2, 0) is also accepted by player \
                     firmware and left untouched on pages that already have it, but this repair \
                     targets the current writer's own convention.",
                    pdb_wrong_track_u5_pages.len()
                ),
            ));
        }
        if !pdb_t00_multipage_active_pages.is_empty() {
            detected_issues.push(format!(
                "{} tt=0 track page(s) with ACTV (0x34) flag in a multi-page chain \
                 (expected SEAL 0x24; may be flagged as corrupted by some DJ software versions)",
                pdb_t00_multipage_active_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_T00_MULTIPAGE_ACTIVE_FIX_ID,
                "Repair Track Page Flag (Multi-Page Chain)",
                format!(
                    "Set flags=0x24 (SEAL) and u5/num_rl=(1, nrs-1) on {} tt=0 track page(s). \
                     Single-page track chains use ACTV (0x34); multi-page chains are expected \
                     to use SEAL (0x24). Some DJ software versions may flag ACTV pages in a \
                     multi-page chain as corrupted.",
                    pdb_t00_multipage_active_pages.len()
                ),
            ));
        }
        if !pdb_tombstoned_playlist_ids.is_empty() {
            detected_issues.push(format!(
                "{} tombstoned row(s) with non-zero id in tracks/playlist_tree (may cause duplicate-id rejection)",
                pdb_tombstoned_playlist_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID,
                "Repair Tombstoned Row IDs",
                format!(
                    "Zero out the id field in {} tombstoned slot(s) (tracks + playlist_tree) to \
                     prevent duplicate-id rejection when active and dead rows share the same id.",
                    pdb_tombstoned_playlist_ids.len()
                ),
            ));
        }
        if !pdb_stale_sentinel_btree_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB sentinel page(s) with stale B-tree index (rejected as corrupted — \
                 B-tree entries reference wrong pages after additive export grew the table)",
                pdb_stale_sentinel_btree_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_STALE_SENTINEL_BTREE_FIX_ID,
                "Repair Stale Sentinel B-Tree Index",
                format!(
                    "Reset the B-tree index area in {} sentinel page(s) to the empty state \
                     (num_entries=0, first_empty=0x1FFF). The DJ software uses the B-tree to validate \
                     the table chain; stale entries referencing wrong pages cause a 'corrupted' \
                     error. player firmware walks the page chain directly and is unaffected. \
                     Fresh exports already write empty B-trees.",
                    pdb_stale_sentinel_btree_pages.len()
                ),
            ));
        }
        if !pdb_wrong_playlist_tree_shape_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB playlist_tree page(s) with (u5, num_rl) shape outside expected (nrs, 0)",
                pdb_wrong_playlist_tree_shape_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID,
                "Repair PDB Playlist-Tree Page Footer Shape",
                format!(
                    "Set u5=nrs and num_rl=0 on {} tt=7 (playlist_tree) data page(s). \
                     The export wrote num_rl=1 instead of 0; the expected shape for this \
                     table is (nrs, 0).",
                    pdb_wrong_playlist_tree_shape_pages.len()
                ),
            ));
        }
        if !pdb_ec_conflicts.is_empty() {
            detected_issues.push(format!(
                "{} table(s) with empty_candidate pointing to another table's data page \
                 (rejected as corrupted — aliased write pointer)",
                pdb_ec_conflicts.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_EC_CONFLICT_FIX_ID,
                "Repair Table Write-Pointer Conflict",
                format!(
                    "{} table(s) have an empty_candidate pointer targeting a page already \
                     used as data by a different table. The DJ software validates this invariant \
                     and reports 'database corrupted' when it finds a conflict. The repair \
                     reassigns each conflicting empty_candidate to a free page beyond the \
                     physical file and updates next_unused_page in the header.",
                    pdb_ec_conflicts.len()
                ),
            ));
        }
        if !pdb_truncated_chains.is_empty() {
            detected_issues.push(format!(
                "{} table(s) with a declared last page beyond the physical end of the file \
                 (interrupted export left growth pointers ahead of the actual written data)",
                pdb_truncated_chains.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_TRUNCATED_TABLE_CHAIN_FIX_ID,
                "Repair Truncated Table Chain",
                format!(
                    "{} table(s) declare a last page that does not physically exist in the file \
                     (an additive-growth write was interrupted after the table pointer was \
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
                 {} empty_candidate page(s) hold non-zero garbage instead of a blank reusable page{}",
                pdb_torn_growth.garbage_ec_pages.len(),
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
                     growing the PDB: {} table empty_candidate page(s) were earmarked as the \
                     next slot to write but never received data, leaving non-zero garbage \
                     instead of the blank page the writer requires before reuse{}. This zeroes \
                     those page(s){} and recomputes seqdb. No table chain, page index, or \
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
        if !pdb_misaligned_track_ids.is_empty() {
            detected_issues.push(format!(
                "{} track row(s) with a misaligned UTF-16 string slot (freezes/comm-errors CDJ hardware while browsing)",
                pdb_misaligned_track_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_TRACK_STRING_ALIGNMENT_FIX_ID,
                "Repair Track String Alignment",
                format!(
                    "Re-encode {} track row(s) whose title/filename/other UTF-16 string slot \
                     doesn't start at a 4-byte aligned offset. MIPS-based CDJ hardware requires \
                     4-byte aligned reads for the UTF-16 string header; a misaligned slot \
                     freezes/comm-errors the player while browsing.",
                    pdb_misaligned_track_ids.len()
                ),
            ));
        }
        if !pdb_misaligned_album_ids.is_empty() {
            detected_issues.push(format!(
                "{} album row(s) with a misaligned UTF-16 name slot (freezes/comm-errors CDJ hardware while browsing)",
                pdb_misaligned_album_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_ALBUM_STRING_ALIGNMENT_FIX_ID,
                "Repair Album String Alignment",
                format!(
                    "Re-encode {} album row(s) whose UTF-16 name slot doesn't start at a 4-byte \
                     aligned offset. Same class of issue as the track string alignment repair.",
                    pdb_misaligned_album_ids.len()
                ),
            ));
        }
        if !pdb_misaligned_artist_ids.is_empty() {
            detected_issues.push(format!(
                "{} artist row(s) with the name at a different offset than rekordbox uses \
                 (Engine DJ players show these tracks without an artist; a misaligned UTF-16 \
                 name also freezes CDJ hardware while browsing)",
                pdb_misaligned_artist_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(fix_proposal(
                PDB_ARTIST_NAME_LAYOUT_FIX_ID,
                "Repair Artist Name Layout",
                format!(
                    "Re-encode {} artist row(s) the way rekordbox writes them: a plain ASCII \
                     name at offset 10, a UTF-16 name at the 4-byte aligned offset 12. Older \
                     versions of this app put every artist name at 12, which an Engine DJ \
                     player's own import of export.pdb reads as no artist.",
                    pdb_misaligned_artist_ids.len()
                ),
            ));
        }
        let pdb_header_compatibility_repair =
            detect_pdb_header_compatibility_repair(&staged_pdb_path);
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
                            "{} indexed audio file(s) missing: the USB has no audio under Contents",
                            presence.missing.len()
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
                            "{} track(s) in a history playlist point to missing audio files",
                            in_history.len()
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
                "{} track reference(s) point to missing audio files under Contents",
                missing_audio_paths.len()
            ));
        }
        if !duplicate_audio_copies.is_empty() {
            detected_issues.push(format!(
                "{} audio file(s) under Contents are unindexed copies of an indexed file in the same folder",
                duplicate_audio_copies.len()
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
                        "{} audio file(s) are on the USB but in no playlist, and each is a byte-for-byte copy of a track in the same folder. Deletes the copies; the tracks keep their playlists, cues and analysis. See Event Log for the file list.",
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
                "{} audio file(s) exist under Contents but are missing from the canonical-path indexed set (PDB/eDB)",
                unindexed_audio_files.len()
            ));
            proposed_fixes.push(RepairFixProposal {
                estimated_writes: 2,
                link: Some("event-log".to_string()),
                ..fix_proposal(
                    UNINDEXED_AUDIO_PLAYLIST_FIX_ID,
                    "Add Unindexed Audio to a Playlist",
                    format!(
                        "{} audio file(s) are on the USB but in no playlist (e.g. after restoring the databases from a backup). Adds them to a USB playlist \"{UNINDEXED_AUDIO_PLAYLIST_NAME}\", reusing their analysis files; nothing is copied or deleted. See Event Log for the file list.",
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
                        "{} track(s) point to a missing file while the same track is on the USB under another name or folder. Points them to the file found; the tracks keep their playlists, cues and analysis. See Event Log for the pairs.",
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
                        "Remove the tracks for {} audio file(s) that no longer exist on the USB from both databases (track and content rows, playlist entries). Tracks in a history playlist are kept. See Event Log for the file list.",
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
                    detected_issues.push(format!("{strict_count} playlist(s) fail strict parity"));
                    proposed_fixes.push(RepairFixProposal {
                        estimated_writes: 0,
                        ..fix_proposal(
                            STRICT_PARITY_UPGRADE_FIX_ID,
                            "Upgrade Export Data To Strict Parity",
                            "Collect all playlists from both eDB and PDB, merge metadata from both sides, and rewrite both databases once.",
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
                        "{pdb_duplicate_entry_count} duplicate PDB playlist entry/entries detected"
                    ));
                    proposed_fixes.push(fix_proposal(
                        PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID,
                        "Remove Duplicate PDB Playlist Entries",
                        format!(
                            "Remove {pdb_duplicate_entry_count} stale duplicate playlist_entries row(s) left \
                             behind on shared pages from repeated exports of the same playlist over time. \
                             This only cleans up duplicate rows already in the PDB — it does not merge \
                             playlists between eDB and PDB or otherwise rewrite the export. For that broader \
                             repair, use \"Upgrade Export Data To Strict Parity\" instead (or in addition)."
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
        }

        let selected = if req.selected_fix_ids.is_empty() {
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

        let found = DetectedRepairs {
            empty_analysis_paths,
            parsed_pdb,
            bpm_key_mismatches,
            analysis_ownership,
            app_master_db_id,
            app_beat_grids,
            missing_seek_data,
            pdb_sentinel_u5_pages,
            pdb_wrong_flags_pages,
            pdb_zero_tranrf_pages,
            pdb_wrong_history_shape_pages,
            pdb_tombstoned_playlist_ids,
            pdb_wrong_track_u5_pages,
            pdb_t00_multipage_active_pages,
            pdb_stale_sentinel_btree_pages,
            pdb_wrong_playlist_tree_shape_pages,
            pdb_ec_conflicts,
            pdb_truncated_chains,
            pdb_torn_growth,
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
            self.apply_selected_fixes(
                &usb_root,
                &selected,
                found,
                &mut warnings,
                &mut estimated_file_writes,
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
            on_progress(95, 100, "USB: Re-running diagnostics after repair");
            match self.run_usb_diagnostics_with_progress(
                RunUsbDiagnosticsRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                },
                |c, t, m| {
                    let pct = 95 + ((c * 4) / t.max(1)).min(4);
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
        Ok(RepairUsbDiagnosticsData {
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
    ) -> FixOutcomes {
        let DetectedRepairs {
            empty_analysis_paths,
            parsed_pdb,
            bpm_key_mismatches,
            analysis_ownership,
            app_master_db_id,
            app_beat_grids,
            missing_seek_data,
            pdb_sentinel_u5_pages,
            pdb_wrong_flags_pages,
            pdb_zero_tranrf_pages,
            pdb_wrong_history_shape_pages,
            pdb_tombstoned_playlist_ids,
            pdb_wrong_track_u5_pages,
            pdb_t00_multipage_active_pages,
            pdb_stale_sentinel_btree_pages,
            pdb_wrong_playlist_tree_shape_pages,
            pdb_ec_conflicts,
            pdb_truncated_chains,
            pdb_torn_growth,
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
        let mut fix_edb_conn = open_edb_rw(usb_root, warnings);

        if selected.contains(EMPTY_ANALYSIS_FILES_FIX_ID) {
            match self.apply_fix_empty_analysis_files(
                usb_root,
                &empty_analysis_paths,
                fix_edb_conn.as_mut(),
                parsed_pdb.as_ref(),
                warnings,
            ) {
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
        outcomes.record(
            "Repair Truncated Table Chain",
            selected.contains(PDB_TRUNCATED_TABLE_CHAIN_FIX_ID),
            !pdb_truncated_chains.is_empty(),
            || {
                apply_pdb_truncated_table_chain_repair(usb_root, &pdb_truncated_chains)
                    .map(|n| format!("fixed {n} table(s)"))
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
        outcomes.record(
            "Repair Torn Growth Pages",
            selected.contains(PDB_TORN_GROWTH_PAGES_FIX_ID),
            !pdb_torn_growth.is_empty(),
            || {
                apply_pdb_torn_growth_pages_repair(usb_root, &pdb_torn_growth).map(|n| {
                    format!(
                        "zeroed {n} page(s){}",
                        if pdb_torn_growth.truncate_to_pages.is_some() {
                            ", truncated unpopulated tail"
                        } else {
                            ""
                        }
                    )
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
                match self.apply_strict_parity_upgrade(
                    usb_root,
                    target_names,
                    fix_edb_conn.as_mut(),
                    warnings,
                ) {
                    Ok(result) => {
                        if result.failed_playlists > 0 {
                            outcomes.failed.push(format!(
                            "Upgrade Export Data To Strict Parity: merged {} playlist(s), wrote {} eDB playlist(s), removed {} duplicate PDB entry/entries, {} failed{}",
                            result.merged_playlists,
                            result.edb_playlists_written,
                            result.duplicate_entries_removed,
                            result.failed_playlists,
                            if result.artwork_patch_incomplete {
                                " (artwork parity incomplete)"
                            } else {
                                ""
                            }
                        ));
                        } else if result.merged_playlists > 0 {
                            outcomes.applied.push(format!(
                            "Upgrade Export Data To Strict Parity: merged {} playlist(s), wrote {} eDB playlist(s), removed {} duplicate PDB entry/entries{}",
                            result.merged_playlists,
                            result.edb_playlists_written,
                            result.duplicate_entries_removed,
                            if result.artwork_patch_incomplete {
                                " (artwork parity incomplete)"
                            } else {
                                ""
                            }
                        ));
                        } else {
                            outcomes.skipped.push(
                                "Upgrade Export Data To Strict Parity: nothing to apply"
                                    .to_string(),
                            );
                        }
                    }
                    Err(err) => outcomes.failed.push(format!(
                        "Upgrade Export Data To Strict Parity failed: {err}"
                    )),
                }
            }
        } else if !strict_upgrade_targets.is_empty() {
            outcomes
                .skipped
                .push("Upgrade Export Data To Strict Parity: not selected".to_string());
        }

        if selected.contains(PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID) {
            match apply_pdb_duplicate_playlist_entries_repair(usb_root) {
                Ok(0) => outcomes
                    .skipped
                    .push("Remove Duplicate PDB Playlist Entries: nothing to apply".to_string()),
                Ok(n) => outcomes.applied.push(format!(
                    "Remove Duplicate PDB Playlist Entries: removed {n} duplicate entry/entries"
                )),
                Err(err) => outcomes.failed.push(format!(
                    "Remove Duplicate PDB Playlist Entries failed: {err}"
                )),
            }
        } else if pdb_duplicate_entry_count > 0 {
            outcomes
                .skipped
                .push("Remove Duplicate PDB Playlist Entries: not selected".to_string());
        }

        outcomes.record(
            "Repair Data Pages With Zero tranrf",
            selected.contains(PDB_ZERO_TRANRF_FIX_ID),
            !pdb_zero_tranrf_pages.is_empty(),
            || {
                apply_pdb_zero_tranrf_repair(usb_root, &pdb_zero_tranrf_pages)
                    .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair PDB Data Page Flags",
            selected.contains(PDB_WRONG_PAGE_FLAGS_FIX_ID),
            !pdb_wrong_flags_pages.is_empty(),
            || {
                apply_pdb_wrong_page_flags_repair(usb_root, &pdb_wrong_flags_pages)
                    .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair PDB Data Pages With Sentinel u5",
            selected.contains(PDB_SENTINEL_U5_FIX_ID),
            !pdb_sentinel_u5_pages.is_empty(),
            || {
                apply_pdb_sentinel_u5_repair(usb_root, &pdb_sentinel_u5_pages)
                    .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair PDB History/Columns Page Footer Shape",
            selected.contains(PDB_WRONG_HISTORY_SHAPE_FIX_ID),
            !pdb_wrong_history_shape_pages.is_empty(),
            || {
                apply_pdb_wrong_history_page_shape_repair(usb_root, &pdb_wrong_history_shape_pages)
                    .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair PDB Track Page Footer Shape",
            selected.contains(PDB_WRONG_TRACK_U5_FIX_ID),
            !pdb_wrong_track_u5_pages.is_empty(),
            || {
                apply_pdb_wrong_track_u5_repair(usb_root, &pdb_wrong_track_u5_pages)
                    .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair Track Page Flag (Multi-Page Chain)",
            selected.contains(PDB_T00_MULTIPAGE_ACTIVE_FIX_ID),
            !pdb_t00_multipage_active_pages.is_empty(),
            || {
                apply_pdb_t00_multipage_active_repair(usb_root, &pdb_t00_multipage_active_pages)
                    .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair Tombstoned Playlist Tree IDs",
            selected.contains(PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID),
            !pdb_tombstoned_playlist_ids.is_empty(),
            || {
                apply_pdb_tombstoned_playlist_tree_id_repair(usb_root, &pdb_tombstoned_playlist_ids)
                    .map(|n| format!("zeroed id in {n} slot(s)"))
            },
        );

        outcomes.record(
            "Repair PDB Playlist-Tree Page Shape",
            selected.contains(PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID),
            !pdb_wrong_playlist_tree_shape_pages.is_empty(),
            || {
                apply_pdb_wrong_playlist_tree_shape_repair(
                    usb_root,
                    &pdb_wrong_playlist_tree_shape_pages,
                )
                .map(|n| format!("patched {n} page(s)"))
            },
        );

        outcomes.record(
            "Repair Stale Sentinel B-Tree",
            selected.contains(PDB_STALE_SENTINEL_BTREE_FIX_ID),
            !pdb_stale_sentinel_btree_pages.is_empty(),
            || {
                apply_pdb_stale_sentinel_btree_repair(usb_root, &pdb_stale_sentinel_btree_pages)
                    .map(|n| format!("reset {n} sentinel page(s)"))
            },
        );

        outcomes.record(
            "Repair Table Write-Pointer Conflict",
            selected.contains(PDB_EC_CONFLICT_FIX_ID),
            !pdb_ec_conflicts.is_empty(),
            || {
                apply_pdb_ec_data_page_conflict_repair(usb_root, &pdb_ec_conflicts)
                    .map(|n| format!("fixed {n} table(s)"))
            },
        );

        outcomes.record(
            "Repair Track String Alignment",
            selected.contains(PDB_TRACK_STRING_ALIGNMENT_FIX_ID),
            !pdb_misaligned_track_ids.is_empty(),
            || {
                apply_pdb_track_string_alignment_repair(usb_root, &pdb_misaligned_track_ids)
                    .map(|n| format!("fixed {n} row(s)"))
            },
        );

        outcomes.record(
            "Repair Album String Alignment",
            selected.contains(PDB_ALBUM_STRING_ALIGNMENT_FIX_ID),
            !pdb_misaligned_album_ids.is_empty(),
            || {
                apply_pdb_album_string_alignment_repair(usb_root, &pdb_misaligned_album_ids)
                    .map(|n| format!("fixed {n} row(s)"))
            },
        );

        outcomes.record(
            "Repair Artist Name Layout",
            selected.contains(PDB_ARTIST_NAME_LAYOUT_FIX_ID),
            !pdb_misaligned_artist_ids.is_empty(),
            || {
                apply_pdb_artist_name_layout_repair(usb_root, &pdb_misaligned_artist_ids)
                    .map(|n| format!("fixed {n} row(s)"))
            },
        );

        if selected.contains(PDB_HEADER_COMPATIBILITY_FIX_ID) {
            if let Some(repair) = pdb_header_compatibility_repair.as_ref() {
                match apply_pdb_header_compatibility_repair(usb_root, repair) {
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

        outcomes.record(
            "Relink Moved Audio Files",
            selected.contains(RELINK_MOVED_AUDIO_FIX_ID),
            !moved_audio_relinks.is_empty(),
            || {
                self.apply_fix_relink_moved_audio(
                    usb_root,
                    &moved_audio_relinks,
                    fix_edb_conn.as_mut(),
                    warnings,
                )
                .map(|relinked| format!("relinked {relinked} track(s)"))
            },
        );

        outcomes.record(
            "Add Unindexed Audio to a Playlist",
            selected.contains(UNINDEXED_AUDIO_PLAYLIST_FIX_ID),
            !unindexed_audio_files.is_empty(),
            || {
                self.apply_fix_add_unindexed_audio_playlist(
                    usb_root,
                    &unindexed_audio_files,
                    fix_edb_conn.as_mut(),
                    warnings,
                )
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
                    format!("added {added} track(s) to \"{UNINDEXED_AUDIO_PLAYLIST_NAME}\"{note}")
                })
            },
        );

        outcomes.record(
            "Remove Duplicate Audio Copies",
            selected.contains(DUPLICATE_AUDIO_COPIES_FIX_ID),
            !duplicate_audio_copies.is_empty(),
            || {
                apply_remove_duplicate_audio_copies(usb_root, &duplicate_audio_copies, warnings)
                    .map(|deleted| format!("deleted {deleted} file(s)"))
            },
        );

        if selected.contains(REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID) {
            match self.apply_fix_remove_missing_audio_references(
                usb_root,
                &missing_audio_paths,
                fix_edb_conn.as_mut(),
                warnings,
            ) {
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
        } else if !missing_audio_paths.is_empty() {
            outcomes
                .skipped
                .push("Remove Missing Audio References: not selected".to_string());
        }

        outcomes.record(
            "Give Tracks Their Own Analysis Files",
            selected.contains(SHARED_ANALYSIS_FIX_ID),
            !analysis_ownership.shared.is_empty(),
            || {
                apply_separate_shared_analysis(
                    usb_root,
                    app_master_db_id,
                    fix_edb_conn.as_mut(),
                    warnings,
                )
                .map(|(moved, kept)| {
                    format!("gave {moved} track(s) their own analysis file, {kept} kept theirs")
                })
            },
        );
        outcomes.record(
            "Add Track Path to Analysis Files",
            selected.contains(ANALYSIS_TRACK_PATH_FIX_ID),
            !analysis_ownership.unmarked.is_empty(),
            || {
                apply_add_analysis_track_path(usb_root, app_master_db_id)
                    .map(|stamped| format!("named the track in {stamped} analysis file(s)"))
            },
        );

        outcomes.record(
            "Fix BPM/Key Mismatch",
            selected.contains(BPM_KEY_MISMATCH_FIX_ID),
            !bpm_key_mismatches.is_empty(),
            || {
                apply_bpm_key_mismatch_repair(
                    usb_root,
                    app_master_db_id,
                    fix_edb_conn.as_mut(),
                    warnings,
                )
                .map(|(anlz_fixed, edb_fixed)| {
                    format!("rebuilt {anlz_fixed} beat grid(s), updated {edb_fixed} eDB row(s)")
                })
            },
        );

        outcomes.record(
            "Fix Beat Grid",
            selected.contains(BEAT_GRID_HEADER_FIX_ID),
            !app_beat_grids.is_empty(),
            || {
                apply_beat_grid_repair(usb_root, warnings)
                    .map(|fixed| format!("fixed {fixed} bundle(s)"))
            },
        );

        for (format, missing) in &missing_seek_data {
            outcomes.record(
                format.title(),
                selected.contains(format.fix_id()),
                !missing.is_empty(),
                || {
                    apply_missing_seek_data_repair(usb_root, *format, warnings)
                        .map(|(fixed, skipped)| format!("fixed {fixed}, skipped {skipped}"))
                },
            );
        }

        if selected.contains(SYNC_EDB_HISTORY_FROM_PDB_FIX_ID) {
            if !sync_edb_history_supported || !sync_edb_history_needed {
                outcomes
                    .skipped
                    .push("Sync eDB History Tables From PDB: nothing to apply".to_string());
            } else if let Some(parsed) = parsed_pdb.as_ref() {
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

        // All selected fixes above write to their local staged copies only
        // (see usb_staging.rs) -- flush the accumulated local changes back
        // to the USB drive here, once per database, instead of once per
        // fix.
        if let Err(err) = usb_staging::write_back_if_changed(usb_root, usb_staging::DbKind::Pdb) {
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.pdb-write-back-failed",
                format!("PDB write-back to USB failed: {err}"),
            ));
        }
        if let Err(err) = usb_staging::write_back_if_changed(usb_root, usb_staging::DbKind::Edb) {
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.edb-write-back-failed",
                format!("eDB write-back to USB failed: {err}"),
            ));
        }

        // After the write-back, so it reads the repaired PDB.
        if selected.contains(ENGINE_LIBRARY_FIX_ID) && engine_library_fix_needed {
            match self
                .enable_engine_library()
                .and_then(|()| crate::service::rebuild_engine_library(usb_root))
            {
                Ok(summary) => {
                    warnings.extend(summary.warnings);
                    outcomes.applied.push(format!(
                        "Keep Engine DJ Library Up to Date: setting on, rebuilt ({} tracks, {} playlists)",
                        summary.tracks, summary.playlists
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
