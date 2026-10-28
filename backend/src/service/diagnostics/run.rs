//! The diagnostics and parity report entry points.

use super::*;

impl BackendService {
    pub fn run_usb_diagnostics(
        &self,
        req: RunUsbDiagnosticsRequest,
    ) -> BackendResult<RunUsbDiagnosticsData> {
        self.run_usb_diagnostics_with_progress(req, |_, _, _| {})
    }

    pub fn run_usb_diagnostics_with_progress<F>(
        &self,
        req: RunUsbDiagnosticsRequest,
        on_progress: F,
    ) -> BackendResult<RunUsbDiagnosticsData>
    where
        F: FnMut(usize, usize, &str),
    {
        self.run_usb_diagnostics_with_progress_impl(req, None, None, on_progress)
            .map(|(data, _)| data)
    }

    /// Same as `run_usb_diagnostics_with_progress`, but lets a caller that
    /// already has an open eDB connection and/or a locally-staged PDB path
    /// (e.g. `repair_usb_diagnostics_with_progress`, which runs this as a
    /// baseline alongside its own eDB/PDB reads) share them instead of each
    /// paying the USB stat/copy/key-negotiation cost separately. Passing
    /// `None` for either opens/stages it internally exactly as before.
    /// Returns the diagnostics report alongside the `ParsedPdb` it parsed
    /// internally (if parsing succeeded), so a caller like
    /// `repair_usb_diagnostics_with_progress` that needs the parsed PDB for
    /// further work can reuse it instead of re-parsing the same unchanged
    /// file a second time.
    pub(crate) fn run_usb_diagnostics_with_progress_impl<F>(
        &self,
        req: RunUsbDiagnosticsRequest,
        edb_conn: Option<&rusqlite::Connection>,
        staged_pdb_path: Option<&std::path::Path>,
        mut on_progress: F,
    ) -> BackendResult<(RunUsbDiagnosticsData, Option<crate::pdb_reader::ParsedPdb>)>
    where
        F: FnMut(usize, usize, &str),
    {
        const SLOW_USB_STAGE_MS: u128 = 3_000;
        let start = std::time::Instant::now();
        let mut stage_started = std::time::Instant::now();
        let mut note_stage = |name: &str, raw_warnings: &mut Vec<WarningEntry>| {
            let elapsed = stage_started.elapsed().as_millis();
            raw_warnings.push(logging::log(
                Level::Info,
                "usb-diagnostics",
                "usb.diagnostics.stage-timing",
                format!("stage timing: {name}: {elapsed}ms"),
            ));
            if elapsed >= SLOW_USB_STAGE_MS {
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.slow-media",
                    format!("slow-media suspected: stage '{name}' took {elapsed}ms"),
                ));
            }
            stage_started = std::time::Instant::now();
        };
        on_progress(2, 100, "USB: Resolving root");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut raw_warnings = Vec::<WarningEntry>::new();
        raw_warnings.push(logging::log(
            Level::Info,
            "usb-diagnostics",
            "usb.diagnostics.info",
            format!("USB root: {}", usb_root.display()),
        ));
        note_stage("resolve usb root", &mut raw_warnings);

        let pdb_path = match staged_pdb_path {
            Some(p) => p.to_path_buf(),
            None => crate::service::usb_staging::stage_pdb(&usb_root)?,
        };

        // --- 1. PDB Integrity ---
        on_progress(10, 100, "USB: Checking PDB integrity");
        let (pdb_integrity, parsed_opt) = diagnose_pdb_integrity(&pdb_path, &mut raw_warnings);
        note_stage("pdb integrity", &mut raw_warnings);

        // --- 2. DB Access ---
        on_progress(30, 100, "USB: Checking database access");
        let edb_path = vendor_edb_path(&usb_root);
        let mut edb_open_warnings = Vec::<WarningEntry>::new();
        let owned_edb_conn: Option<rusqlite::Connection> = if edb_conn.is_some() {
            None
        } else if edb_path.exists() {
            open_edb_from_usb_root(&usb_root, &mut edb_open_warnings)
        } else {
            None
        };
        let conn: Option<&rusqlite::Connection> = edb_conn.or(owned_edb_conn.as_ref());
        let mut edb_access = diagnose_edb_access(&edb_path, conn, &edb_open_warnings);
        raw_warnings.append(&mut edb_open_warnings);
        let edb_playlists = conn.and_then(|c| {
            try_read_playlists_with_metadata_from_edb_db_only_with_conn(
                c,
                &usb_root,
                &mut raw_warnings,
            )
        });
        let edb_playlist_tracks = edb_playlists.map(|m| {
            m.into_iter()
                .map(|(name, playlist)| (name, playlist.tracks))
                .collect::<HashMap<_, _>>()
        });

        // eDB history counts
        {
            let (edb_h, edb_hc) = if let Some(conn) = conn {
                let h = if table_exists(conn, "history") {
                    conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0))
                        .unwrap_or(0)
                        .max(0) as usize
                } else {
                    0
                };
                let hc = if table_exists(conn, "history_content") {
                    conn.query_row("SELECT COUNT(*) FROM history_content", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap_or(0)
                    .max(0) as usize
                } else {
                    0
                };
                (h, hc)
            } else {
                (0, 0)
            };
            if edb_h > 0 || edb_hc > 0 {
                edb_access.checks.push(DiagCheck {
                    label: "eDB history".to_string(),
                    status: DiagStatus::Pass,
                    detail: format!("{edb_h} playlists, {edb_hc} entries"),
                    link: None,
                });
            } else {
                edb_access.checks.push(DiagCheck {
                    label: "eDB history".to_string(),
                    status: DiagStatus::Pass,
                    detail: "empty".to_string(),
                    link: None,
                });
            }
        }

        note_stage("db access", &mut raw_warnings);

        // --- 3. Contents Integrity ---
        on_progress(50, 100, "USB: Checking contents integrity (DB-only)");
        let pdb_indexed_paths = parsed_opt
            .as_ref()
            .map(|p| {
                let parsed = &p.0;
                parsed
                    .tracks
                    .iter()
                    .map(|t| audio_path_key(&t.track_file_path))
                    .filter(|p| !p.is_empty())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let edb_indexed_paths = conn
            .map(|c| {
                collect_edb_audio_paths(c, &mut raw_warnings)
                    .iter()
                    .map(|p| audio_path_key(p))
                    .filter(|p| !p.is_empty())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();

        // Identify tracks that exist in PDB history playlists but not in any regular playlist.
        // These are written by players during playback and are expected to be absent from eDB.
        let history_only_pdb_paths = if let Some((parsed, _)) = parsed_opt.as_ref() {
            let regular_ids: HashSet<u32> =
                parsed.playlist_entries.iter().map(|e| e.track_id).collect();
            let history_only_ids: HashSet<u32> = parsed
                .history_entries
                .iter()
                .filter_map(|e| e.track_id)
                .filter(|id| !regular_ids.contains(id))
                .collect();
            let hp_name: HashMap<u32, &str> = parsed
                .history_playlists
                .iter()
                .map(|hp| (hp.id, hp.name.as_str()))
                .collect();
            let mut out = HashSet::new();
            for track in &parsed.tracks {
                if !history_only_ids.contains(&track.id) {
                    continue;
                }
                let norm = normalize_path_for_strict_presence_match(&track.track_file_path);
                if norm.is_empty() {
                    continue;
                }
                let pl_names: std::collections::BTreeSet<&str> = parsed
                    .history_entries
                    .iter()
                    .filter(|e| e.track_id == Some(track.id))
                    .filter_map(|e| hp_name.get(&e.playlist_id).copied())
                    .collect();
                let pl_str = if pl_names.is_empty() {
                    "unknown history playlist".to_string()
                } else {
                    pl_names.into_iter().collect::<Vec<_>>().join(", ")
                };
                raw_warnings.push(logging::log(
                    Level::Info,
                    "usb-diagnostics",
                    "usb.diagnostics.history-only",
                    format!("history-only track: {norm} ({pl_str})"),
                ));
                out.insert(audio_path_key(&norm));
            }
            out
        } else {
            HashSet::new()
        };

        let true_pdb_only_count = pdb_indexed_paths
            .difference(&edb_indexed_paths)
            .filter(|p| !history_only_pdb_paths.contains(*p))
            .count();
        let history_only_count = pdb_indexed_paths
            .difference(&edb_indexed_paths)
            .filter(|p| history_only_pdb_paths.contains(*p))
            .count();
        let edb_only_count = edb_indexed_paths.difference(&pdb_indexed_paths).count();
        let mut contents_integrity = diagnose_contents_integrity_db_only(
            pdb_indexed_paths.len(),
            edb_indexed_paths.len(),
            true_pdb_only_count,
            edb_only_count,
            history_only_count,
        );

        // String alignment: a track/album row whose UTF-16 string slot isn't 4-byte aligned
        // freezes/comm-errors CDJ hardware while browsing (see `detect_pdb_string_misalignment`
        // for the full MIPS-alignment explanation). `dump_pdb_anomalies`-style checks only look
        // at page/table shape, so this is otherwise invisible on read.
        {
            let (artist_layout, misaligned): (Vec<_>, Vec<_>) =
                crate::service::repair::detect_pdb_string_misalignment(
                    &std::fs::read(&pdb_path).unwrap_or_default(),
                )
                .into_iter()
                .partition(|m| m.slot_label == "ascii_name_offset");
            let status = if misaligned.is_empty() {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            };
            let detail = if misaligned.is_empty() {
                "All UTF-16 and long ASCII string slots are 4-byte aligned".to_string()
            } else {
                let examples = misaligned
                    .iter()
                    .take(3)
                    .map(|m| {
                        format!(
                            "{} #{} ({})",
                            table_type_label(m.table_type),
                            m.id,
                            m.slot_label
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} row(s) with a misaligned UTF-16 or long ASCII string slot — \
                     freezes/comm-errors CDJ hardware while browsing; e.g. {examples}; run repair_pdb_track_string_alignment \
                     / repair_pdb_album_string_alignment / repair_pdb_artist_name_layout to fix",
                    misaligned.len()
                )
            };
            if matches!(status, DiagStatus::Fail) {
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.pdb-string-misalignment",
                    detail.clone(),
                ));
            }
            contents_integrity.checks.push(DiagCheck {
                label: "Track/album string alignment".to_string(),
                status,
                detail,
                link: None,
            });

            // Artist rows with a short ASCII name at 12 instead of rekordbox's 10: CDJs read
            // them fine, but an Engine DJ player's own import of export.pdb shows no artist.
            let (status, detail) = if artist_layout.is_empty() {
                (
                    DiagStatus::Pass,
                    "All artist names are where rekordbox puts them".to_string(),
                )
            } else {
                let detail = format!(
                    "{} artist row(s) with the name at a different offset than rekordbox uses; \
                     an Engine DJ player importing export.pdb shows these tracks without an \
                     artist; run repair_pdb_artist_name_layout to fix",
                    artist_layout.len()
                );
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.pdb-artist-name-layout",
                    detail.clone(),
                ));
                (DiagStatus::Warn, detail)
            };
            contents_integrity.checks.push(DiagCheck {
                label: "Artist name layout".to_string(),
                status,
                detail,
                link: None,
            });
            contents_integrity.status = DiagStatus::worst_of(
                &contents_integrity
                    .checks
                    .iter()
                    .map(|c| &c.status)
                    .collect::<Vec<_>>(),
            );
        }
        note_stage("contents integrity", &mut raw_warnings);

        // --- 4. Analysis Integrity ---
        on_progress(70, 100, "USB: Checking analysis references");
        let edb_content_rows = conn.and_then(|c| {
            c.query_row("SELECT COUNT(*) FROM content", [], |r| r.get::<_, i64>(0))
                .ok()
                .map(|n| n.max(0) as usize)
        });
        let mut analysis_integrity = diagnose_analysis_integrity(
            parsed_opt.as_ref().map(|p| &p.0),
            edb_playlist_tracks.as_ref(),
            edb_content_rows,
        );
        if let Some((parsed, _)) = parsed_opt.as_ref() {
            // Database-only: the ANLZ beat grids are compared by the repair
            // preview, which scans the analysis files anyway.
            let mismatches = detect_bpm_key_mismatches(&usb_root, parsed, conn, false, None);
            let (status, detail) = if mismatches.is_empty() {
                (
                    DiagStatus::Pass,
                    "export.pdb and exportLibrary.db agree on BPM and key for every track"
                        .to_string(),
                )
            } else {
                let examples = mismatches
                    .iter()
                    .take(3)
                    .map(|m| m.track_path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    DiagStatus::Warn,
                    format!(
                        "{} track(s) whose BPM/key in exportLibrary.db differs from export.pdb \
                         -- CDJs may show a different BPM/key than the app; e.g. {examples}; \
                         run the repair (fix_bpm_key_mismatch, which also checks the ANLZ beat \
                         grids) to fix",
                        mismatches.len()
                    ),
                )
            };
            if matches!(status, DiagStatus::Warn) {
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.bpm-key-mismatch",
                    detail.clone(),
                ));
            }
            analysis_integrity.checks.push(DiagCheck {
                label: "BPM/key consistency".to_string(),
                status,
                detail,
                link: None,
            });
            // Database-only, like the BPM check: track rows naming the same
            // bundle. The repair preview also reads each bundle's PPTH.
            let shared = scan_analysis_ownership(&usb_root, parsed, None, false).shared;
            let shared_tracks = shared.iter().map(|b| b.track_ids.len()).sum::<usize>();
            for bundle in &shared {
                let tracks = bundle
                    .track_ids
                    .iter()
                    .filter_map(|id| parsed.tracks.iter().find(|t| t.id == *id))
                    .map(|t| t.track_file_path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.shared-analysis",
                    format!(
                        "shared analysis file: /{} used by {tracks}",
                        bundle
                            .dat
                            .strip_prefix(&usb_root)
                            .unwrap_or(&bundle.dat)
                            .to_string_lossy()
                            .replace('\\', "/")
                    ),
                ));
            }
            analysis_integrity.checks.push(DiagCheck {
                label: "Shared analysis files".to_string(),
                status: if shared.is_empty() {
                    DiagStatus::Pass
                } else {
                    DiagStatus::Warn
                },
                detail: if shared.is_empty() {
                    "every track has its own analysis file".to_string()
                } else {
                    format!(
                        "{shared_tracks} track(s) share {} analysis file(s), so players can show \
                         one track's waveform and beat grid for another; run the repair \
                         (separate_shared_analysis) to fix",
                        shared.len()
                    )
                },
                link: (!shared.is_empty()).then(|| "event-log".to_string()),
            });
            let beat_grids = diagnose_beat_grid_format(&usb_root, parsed);
            if matches!(beat_grids.status, DiagStatus::Warn) {
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.beat-grid-format",
                    beat_grids.detail.clone(),
                ));
            }
            analysis_integrity.checks.push(beat_grids);
            analysis_integrity.status = DiagStatus::worst_of(
                &analysis_integrity
                    .checks
                    .iter()
                    .map(|c| &c.status)
                    .collect::<Vec<_>>(),
            );
        }
        note_stage("analysis integrity", &mut raw_warnings);

        // --- 5. Playlist Resolution ---
        on_progress(85, 100, "USB: Checking playlist resolution");
        let (playlist_resolution, playlist_details) =
            diagnose_playlist_resolution_with_edb_internal(
                parsed_opt.as_ref().map(|p| &p.0),
                edb_playlist_tracks.as_ref(),
                |done, total, message| {
                    let pct = if total == 0 {
                        98
                    } else {
                        85 + ((done * 13) / total.max(1)).min(13)
                    };
                    on_progress(pct, 100, message);
                },
            );
        note_stage("playlist resolution", &mut raw_warnings);

        {
            let mut menu_warnings = Vec::<WarningEntry>::new();
            if let Ok((_current, _available, divergence)) =
                crate::service::repair::load_usb_player_menu_config_public(
                    &usb_root,
                    &mut menu_warnings,
                )
                && !divergence.is_empty()
            {
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.player-menu-divergence",
                    format!(
                        "PDB and eDB menus disagree ({} active menu items missing from PDB)",
                        divergence.in_edb_visible_only.len()
                    ),
                ));
            }
            raw_warnings.extend(menu_warnings);
            note_stage("player menu divergence", &mut raw_warnings);
        }

        let engine_library_section = self.engine_library_section(&usb_root);
        let overall_status = DiagStatus::worst_of(&[
            &pdb_integrity.status,
            &edb_access.status,
            &contents_integrity.status,
            &analysis_integrity.status,
            &playlist_resolution.status,
            engine_library_section
                .as_ref()
                .map_or(&DiagStatus::Pass, |section| &section.status),
        ]);

        let cdj_counter_snapshot = parsed_opt.as_ref().and_then(|(parsed, _)| {
            compute_player_counter_snapshot(
                &pdb_path,
                parsed,
                crate::service::export_helpers::edb_t19_property(&usb_root).as_ref(),
            )
        });
        let cdj_counter_section = cdj_counter_snapshot
            .as_ref()
            .map(player_counter_snapshot_section);
        let usb_playlist_names = usb_playlist_names_for_export_compare(
            parsed_opt.as_ref().map(|p| &p.0),
            edb_playlist_tracks
                .as_ref()
                .into_iter()
                .flat_map(|m| m.keys().cloned()),
        );
        let local_playlists = self.list_playlists()?.items;
        let conn = self.db.connect()?;
        let prune_stale = export_prune_stale_setting(&conn)?;
        let playlist_usb_export_status = compute_playlist_usb_export_status(
            &local_playlists,
            &usb_playlist_names,
            prune_stale,
            Some(usb_root.to_string_lossy().as_ref()),
        );
        let mut display_sources = vec![
            (&pdb_integrity, DISPLAY_PDB),
            (&playlist_resolution, DISPLAY_PLAYLISTS),
            (&analysis_integrity, DISPLAY_ANALYSIS),
            (&edb_access, DISPLAY_EDB),
            (&contents_integrity, DISPLAY_EDB),
        ];
        display_sources.extend(engine_library_section.iter().map(|s| (s, DISPLAY_ENGINE)));
        display_sources.extend(cdj_counter_section.iter().map(|s| (s, DISPLAY_LOW_LEVEL)));
        let section_groups = regroup_sections(&DISPLAY_LAYOUT, &display_sources);
        let overview = diagnostics_overview(
            &section_groups,
            &OverviewFacts {
                edb_file_present: edb_path.exists(),
                pdb_tracks: parsed_opt.as_ref().map_or(0, |(p, _)| p.tracks.len()),
                edb_tracks: edb_indexed_paths.len(),
                pdb_only_tracks: true_pdb_only_count,
                edb_only_tracks: edb_only_count,
                playlists: playlist_details.len(),
                playlist_entries: playlist_details.iter().map(|p| p.total_entries).sum(),
                resolved_playlist_entries: playlist_details
                    .iter()
                    .map(|p| p.resolved_entries)
                    .sum(),
            },
        );
        let data = RunUsbDiagnosticsData {
            overall_status,
            overview,
            section_groups,
            pdb_integrity,
            edb_access,
            contents_integrity,
            analysis_integrity,
            playlist_resolution,
            playlist_details,
            cdj_counter_snapshot,
            cdj_counter_section,
            engine_library_section,
            warnings: raw_warnings,
            duration_ms: start.elapsed().as_millis() as u64,
            playlist_usb_export_status,
        };
        Ok((data, parsed_opt.map(|(parsed, _)| parsed)))
    }

    pub fn run_usb_parity_report(
        &self,
        req: RunUsbParityReportRequest,
    ) -> BackendResult<RunUsbParityReportData> {
        self.run_usb_parity_report_with_progress(req, |_, _, _| {})
    }

    pub fn run_usb_parity_report_with_progress<F>(
        &self,
        req: RunUsbParityReportRequest,
        on_progress: F,
    ) -> BackendResult<RunUsbParityReportData>
    where
        F: FnMut(usize, usize, &str),
    {
        self.run_usb_parity_report_with_progress_impl(req, None, None, None, on_progress)
    }

    /// Same as `run_usb_parity_report_with_progress`, but lets a caller that
    /// already has an open eDB connection, a locally-staged PDB path, and/or
    /// an already-parsed PDB (e.g. from `run_usb_diagnostics_with_progress_impl`)
    /// share them instead of each paying the USB stat/copy/key-negotiation/parse
    /// cost separately. See `run_usb_diagnostics_with_progress_impl`.
    pub(crate) fn run_usb_parity_report_with_progress_impl<F>(
        &self,
        req: RunUsbParityReportRequest,
        edb_conn: Option<&rusqlite::Connection>,
        staged_pdb_path: Option<&std::path::Path>,
        parsed_pdb: Option<&crate::pdb_reader::ParsedPdb>,
        mut on_progress: F,
    ) -> BackendResult<RunUsbParityReportData>
    where
        F: FnMut(usize, usize, &str),
    {
        const SLOW_USB_STAGE_MS: u128 = 3_000;
        let start = std::time::Instant::now();
        let mut stage_started = std::time::Instant::now();
        let mut note_stage = |name: &str, raw_warnings: &mut Vec<WarningEntry>| {
            let elapsed = stage_started.elapsed().as_millis();
            raw_warnings.push(logging::log(
                Level::Info,
                "usb-diagnostics",
                "usb.diagnostics.stage-timing",
                format!("stage timing: {name}: {elapsed}ms"),
            ));
            if elapsed >= SLOW_USB_STAGE_MS {
                raw_warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.slow-media",
                    format!("slow-media suspected: stage '{name}' took {elapsed}ms"),
                ));
            }
            stage_started = std::time::Instant::now();
        };
        on_progress(2, 100, "USB: Resolving root");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut raw_warnings = vec![logging::log(
            Level::Info,
            "usb-diagnostics",
            "usb.diagnostics.info",
            format!("USB root: {}", usb_root.display()),
        )];
        note_stage("resolve usb root", &mut raw_warnings);

        on_progress(20, 100, "USB: Parsing PDB");
        let pdb_path = match staged_pdb_path {
            Some(p) => p.to_path_buf(),
            None => crate::service::usb_staging::stage_pdb(&usb_root)?,
        };
        let owned_parsed_pdb;
        let parsed: &crate::pdb_reader::ParsedPdb = match parsed_pdb {
            Some(parsed) => parsed,
            None => {
                owned_parsed_pdb = parse_pdb(&pdb_path)?;
                &owned_parsed_pdb
            }
        };
        raw_warnings.extend(parsed.warnings.iter().map(|message| {
            logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-parse",
                message.clone(),
            )
        }));
        note_stage("parse PDB", &mut raw_warnings);

        on_progress(45, 100, "USB: Reading eDB");
        let owned_edb_conn: Option<rusqlite::Connection> = if edb_conn.is_some() {
            None
        } else {
            open_edb_from_usb_root(&usb_root, &mut raw_warnings)
        };
        let conn: Option<&rusqlite::Connection> = edb_conn.or(owned_edb_conn.as_ref());
        let Some(conn) = conn else {
            return Err(BackendError::Internal(
                "parity report requires readable eDB playlist data".to_string(),
            ));
        };
        let edb_playlists =
            try_read_playlists_with_metadata_from_edb_with_conn(conn, &usb_root, &mut raw_warnings)
                .ok_or_else(|| {
                    BackendError::Internal(
                        "parity report requires readable eDB playlist data".to_string(),
                    )
                })?;
        let rating_colour = compare_rating_colour(conn, parsed, &edb_playlists);
        let field_notes = compare_note_fields(conn, parsed, &edb_playlists);
        note_stage("read eDB", &mut raw_warnings);

        on_progress(58, 100, "USB: Checking indexed audio file presence");
        let edb_audio_paths = collect_edb_audio_paths(conn, &mut raw_warnings);
        let presence = scan_audio_presence(&usb_root, parsed, &edb_audio_paths);
        for path in &presence.missing {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.missing-indexed-audio",
                format!("indexed audio file missing from USB: {path}"),
            ));
        }
        for path in &presence.unindexed {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.unindexed-audio",
                format!("unindexed audio file: {path}"),
            ));
        }
        let strict_raw_coverage = evaluate_strict_raw_coverage_parity(
            presence.missing.len(),
            presence.unindexed.len(),
            presence.indexed_count,
        );
        note_stage("strict raw coverage check", &mut raw_warnings);

        on_progress(70, 100, "USB: Comparing playlist sources");
        let (checks, summary_rows, playlist_details, overall_status) = build_usb_parity_comparison(
            parsed,
            &edb_playlists,
            &rating_colour,
            &field_notes,
            Some(strict_raw_coverage.clone()),
        );
        let section_groups = parity_section_groups(&checks);
        let overview = parity_overview(
            &section_groups,
            &ParityTotals {
                rating_colour_mismatches: rating_colour.mismatched_tracks,
                ..ParityTotals::from_details(&playlist_details)
            },
            playlist_details.len(),
            Some(&strict_raw_coverage),
        );
        note_stage("compare playlist sources", &mut raw_warnings);

        on_progress(95, 100, "USB: Finalizing parity report");
        note_stage("finalize", &mut raw_warnings);
        Ok(RunUsbParityReportData {
            overall_status,
            overview,
            section_groups,
            checks,
            summary_rows,
            playlist_details,
            warnings: raw_warnings,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}
