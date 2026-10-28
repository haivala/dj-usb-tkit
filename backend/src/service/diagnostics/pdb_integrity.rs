//! PDB integrity checks.

use super::*;

pub(crate) fn diagnose_pdb_integrity(
    pdb_path: &std::path::Path,
    raw_warnings: &mut Vec<WarningEntry>,
) -> (
    DiagSection,
    Option<(
        crate::pdb_reader::ParsedPdb,
        crate::pdb_reader::PdbDiagnostics,
    )>,
) {
    let mut checks = Vec::new();

    if !pdb_path.exists() {
        checks.push(DiagCheck {
            label: "PDB exists".to_string(),
            status: DiagStatus::Fail,
            detail: format!("File not found: {}", pdb_path.display()),
            link: None,
        });
        return (
            DiagSection {
                title: "PDB Integrity".to_string(),
                status: DiagStatus::Fail,
                checks,
                counts: None,
            },
            None,
        );
    }
    checks.push(DiagCheck {
        label: "PDB exists".to_string(),
        status: DiagStatus::Pass,
        detail: "Found".to_string(),
        link: None,
    });

    let (parsed, diag) = match parse_pdb_with_diagnostics(pdb_path) {
        Ok(result) => result,
        Err(err) => {
            checks.push(DiagCheck {
                label: "PDB parseable".to_string(),
                status: DiagStatus::Fail,
                detail: format!("Parse error: {err}"),
                link: None,
            });
            return (
                DiagSection {
                    title: "PDB Integrity".to_string(),
                    status: DiagStatus::Fail,
                    checks,
                    counts: None,
                },
                None,
            );
        }
    };

    checks.push(DiagCheck {
        label: "PDB parseable".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} tracks, {} artists, {} albums, {} genres, {} labels, {} keys, {} artworks",
            parsed.tracks.len(),
            parsed.artists.len(),
            parsed.albums.len(),
            parsed.genres.len(),
            parsed.labels.len(),
            parsed.keys.len(),
            parsed.artworks.len(),
        ),
        link: None,
    });

    // The detectors below all look at the same bytes; read the file once.
    let bytes = std::fs::read(pdb_path).unwrap_or_default();
    if let Some(header_compat) =
        crate::service::repair::pdb_header_compatibility_value_from_bytes(&bytes)
    {
        // docs/PDB.md "File header 0x10 compatibility field": both 1 and 5 are
        // confirmed accepted by every tested validator. Comparing against a
        // backup snapshot's value here would flag "drift" every time a fresh
        // export (writer always emits 5) follows a prior repair application
        // (which patches to 1) — an unwinnable oscillation, not a real issue.
        let invalid_current =
            !crate::service::repair::is_known_pdb_header_compatibility_value(header_compat);
        let header_compat_status = if invalid_current {
            DiagStatus::Warn
        } else {
            DiagStatus::Pass
        };
        let detail = if invalid_current {
            format!("value {header_compat} at bytes 0x10..0x14; expected a known-compatible value")
        } else {
            format!("value {header_compat} at bytes 0x10..0x14")
        };
        if matches!(header_compat_status, DiagStatus::Warn) {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-header-compatibility",
                format!(
                    "PDB header compatibility field is {header_compat}; run repair_pdb_header_compatibility_field to restore a compatible value."
                ),
            ));
        }
        checks.push(DiagCheck {
            label: "PDB header compatibility".to_string(),
            status: header_compat_status,
            detail,
            link: None,
        });
    }

    {
        // All three are rekordbox transaction bookkeeping: real rekordbox
        // exports carry them and play fine, so they never warn. Repair USB
        // still normalizes them whenever it runs (see
        // `repair::fix_is_bookkeeping`); our own writer never produces them.
        let bad_u5 = crate::service::repair::detect_pdb_sentinel_u5_on_data_pages(&bytes);
        let bad_flags = crate::service::repair::detect_pdb_wrong_page_flags(&bytes);
        let bad_tranrf = crate::service::repair::detect_pdb_zero_tranrf_all_tables(&bytes);
        let mut parts = Vec::new();
        if !bad_u5.is_empty() {
            parts.push(format!("{} page(s) with sentinel u5=0x1FFF", bad_u5.len()));
        }
        if !bad_flags.is_empty() {
            parts.push(format!(
                "{} page(s) with non-default page_flags",
                bad_flags.len()
            ));
        }
        if !bad_tranrf.is_empty() {
            parts.push(format!(
                "{} data page(s) with tranrf=0 on last-row group",
                bad_tranrf.len()
            ));
        }
        let detail = if parts.is_empty() {
            "u5, page_flags, and tranrf are valid on all data pages".to_string()
        } else {
            raw_warnings.push(logging::log(
                Level::Info,
                "usb-diagnostics",
                "usb.diagnostics.pdb-page-header-bookkeeping",
                format!(
                    "rekordbox transaction bookkeeping on PDB data pages: {}; normalized whenever a repair runs.",
                    parts.join("; ")
                ),
            ));
            format!(
                "valid; rekordbox transaction bookkeeping: {}",
                parts.join("; ")
            )
        };
        checks.push(DiagCheck {
            label: "PDB page headers".to_string(),
            status: DiagStatus::Pass,
            detail,
            link: None,
        });
    }

    {
        let stale_btree = crate::service::repair::detect_pdb_stale_sentinel_btree(&bytes);
        let wrong_pl_tree = crate::service::repair::detect_pdb_wrong_playlist_tree_shape(&bytes);
        let tombstoned = crate::service::repair::detect_pdb_tombstoned_playlist_tree_ids(&bytes);
        let wrong_track_u5 = crate::service::repair::detect_pdb_wrong_track_u5(&bytes);
        let wrong_history = crate::service::repair::detect_pdb_wrong_history_page_shape(&bytes);
        let t00_multipage_active =
            crate::service::repair::detect_pdb_t00_multipage_active_pages(&bytes);
        let ec_conflicts = crate::service::repair::detect_pdb_ec_data_page_conflicts(&bytes);
        let truncated_chains = crate::service::repair::detect_pdb_truncated_table_chains(&bytes);
        let torn_growth = crate::service::repair::detect_pdb_torn_growth_pages(&bytes);

        // Hard failures: conditions that cause player or DJ software rejection.
        let mut fail_parts = Vec::new();
        // Soft warnings: player-tolerant shapes rekordbox itself never writes.
        let mut warn_parts = Vec::new();
        // rekordbox transaction bookkeeping: real rekordbox exports carry these
        // and play fine, so they never warn (see the page-headers check above).
        let mut bookkeeping_parts = Vec::new();

        if !stale_btree.is_empty() {
            bookkeeping_parts.push(format!("{} sentinel(s) stale b-tree", stale_btree.len()));
        }
        if !wrong_pl_tree.is_empty() {
            bookkeeping_parts.push(format!(
                "{} playlist_tree page(s) with non-default shape",
                wrong_pl_tree.len()
            ));
        }
        if !tombstoned.is_empty() {
            bookkeeping_parts.push(format!(
                "{} tombstoned row(s) non-zero id",
                tombstoned.len()
            ));
        }
        if !wrong_track_u5.is_empty() {
            bookkeeping_parts.push(format!(
                "{} track page(s) with non-default shape",
                wrong_track_u5.len()
            ));
        }
        if !t00_multipage_active.is_empty() {
            bookkeeping_parts.push(format!(
                "{} track page(s) ACTV in multi-page chain",
                t00_multipage_active.len()
            ));
        }
        if !bookkeeping_parts.is_empty() {
            raw_warnings.push(logging::log(
                Level::Info,
                "usb-diagnostics",
                "usb.diagnostics.pdb-structural-bookkeeping",
                format!(
                    "rekordbox transaction bookkeeping in PDB tables: {}; normalized whenever a repair runs.",
                    bookkeeping_parts.join("; ")
                ),
            ));
        }
        if !wrong_history.is_empty() {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-wrong-history-shape",
                format!(
                    "{} history page(s) with wrong u5/num_rl shape; run repair_pdb_wrong_history_page_shape to fix.",
                    wrong_history.len()
                ),
            ));
            warn_parts.push(format!(
                "{} history page(s) wrong shape",
                wrong_history.len()
            ));
        }
        if !ec_conflicts.is_empty() {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-ec-data-page-conflict",
                format!(
                    "{} table(s) with empty_candidate pointing to another table's data page \
                     (may cause write-pointer conflict rejection); run repair_pdb_ec_data_page_conflict to fix.",
                    ec_conflicts.len()
                ),
            ));
            fail_parts.push(format!(
                "{} table(s) write-pointer conflict",
                ec_conflicts.len()
            ));
        }
        if !truncated_chains.is_empty() {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-truncated-table-chain",
                format!(
                    "{} table(s) declare a last page beyond the physical end of the file \
                     (interrupted export); run repair_pdb_truncated_table_chain to fix.",
                    truncated_chains.len()
                ),
            ));
            fail_parts.push(format!(
                "{} table(s) with declared last page beyond physical file end",
                truncated_chains.len()
            ));
        }
        if !torn_growth.is_empty() {
            let ec_count = torn_growth.garbage_ec_pages.len();
            let ec_desc = if ec_count > 0 {
                format!("{ec_count} empty_candidate page(s) hold garbage instead of a blank page")
            } else {
                "the file has a never-populated tail".to_string()
            };
            let tail_suffix = if ec_count > 0 && torn_growth.truncate_to_pages.is_some() {
                " and the file has a never-populated tail"
            } else {
                ""
            };
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-torn-growth",
                format!(
                    "PDB shows signs of an interrupted export (e.g. USB disconnected mid-write): \
                     {ec_desc}{tail_suffix}; run repair_pdb_torn_growth_pages to fix."
                ),
            ));
            fail_parts.push(if ec_count > 0 {
                format!("{ec_count} empty_candidate page(s) torn from interrupted export")
            } else {
                "PDB tail torn from interrupted export".to_string()
            });
        }
        let integrity_status = if !fail_parts.is_empty() {
            DiagStatus::Fail
        } else if !warn_parts.is_empty() {
            DiagStatus::Warn
        } else {
            DiagStatus::Pass
        };
        let detail = if fail_parts.is_empty() && warn_parts.is_empty() {
            let ok = "sentinel b-tree, page shapes, row IDs, and write pointers OK".to_string();
            if bookkeeping_parts.is_empty() {
                ok
            } else {
                format!(
                    "{ok}; rekordbox transaction bookkeeping: {}",
                    bookkeeping_parts.join("; ")
                )
            }
        } else {
            let mut segments = Vec::new();
            if !fail_parts.is_empty() {
                segments.push(fail_parts.join("; ") + " — run repair_usb_diagnostics to fix");
            }
            if !warn_parts.is_empty() {
                segments.push(warn_parts.join("; ") + " — player-tolerant; repair available");
            }
            segments.join(" | ")
        };
        checks.push(DiagCheck {
            label: "PDB structural integrity".to_string(),
            status: integrity_status,
            detail,
            link: None,
        });
    }

    checks.push(DiagCheck {
        label: "PDB playlists".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} tree nodes, {} entries",
            parsed.playlist_tree.len(),
            parsed.playlist_entries.len(),
        ),
        link: None,
    });

    // History playlists in PDB. Fresh/never-played exports ship t17/t18 with
    // a fixed block of blank template/seed rows (no name, no track link)
    // alongside any real recorded sessions; only count rows that look like
    // real rekordbox-recorded history, matching the same rule
    // `derive_history_sync_payload` already uses for the eDB history-sync
    // repair.
    {
        let (hp, he) = count_named_history_rows(&parsed.history_playlists, &parsed.history_entries);
        let hr = parsed.history_rows.len();
        if hp > 0 || he > 0 || hr > 0 {
            checks.push(DiagCheck {
                label: "PDB history".to_string(),
                status: DiagStatus::Pass,
                detail: format!("{hp} playlists, {he} entries, {hr} rows"),
                link: None,
            });
        } else {
            checks.push(DiagCheck {
                label: "PDB history".to_string(),
                status: DiagStatus::Pass,
                detail: "empty".to_string(),
                link: None,
            });
        }
    }

    // num_rl=8191 is a valid external-library sentinel; treat as compatibility info.
    checks.push(DiagCheck {
        label: "num_rl=8191 pages".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} of {} pages (using nrs fallback)",
            diag.pages_with_num_rl_8191, diag.total_pages
        ),
        link: None,
    });

    // nrs u8 wrapping is expected on some pages; recovery is part of normal parsing.
    checks.push(DiagCheck {
        label: "nrs wrapping".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} pages with row count exceeding nrs header",
            diag.nrs_wrapping_pages
        ),
        link: None,
    });

    // Orphaned playlist entries
    let track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    let orphaned: Vec<u32> = parsed
        .playlist_entries
        .iter()
        .filter(|e| !track_ids.contains(&e.track_id))
        .map(|e| e.track_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let orphan_status = if orphaned.is_empty() {
        DiagStatus::Pass
    } else {
        DiagStatus::Warn
    };
    checks.push(DiagCheck {
        label: "Orphaned entries".to_string(),
        status: orphan_status,
        detail: format!(
            "{} entries reference {} track IDs not in PDB",
            parsed
                .playlist_entries
                .iter()
                .filter(|e| !track_ids.contains(&e.track_id))
                .count(),
            orphaned.len()
        ),
        link: None,
    });

    // Parse warnings
    if !parsed.warnings.is_empty() {
        raw_warnings.extend(parsed.warnings.iter().map(|message| {
            logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-parse",
                message.clone(),
            )
        }));
        checks.push(DiagCheck {
            label: "Parse warnings".to_string(),
            status: DiagStatus::Warn,
            detail: format!("{} warnings during parsing", parsed.warnings.len()),
            link: None,
        });
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (
        DiagSection {
            title: "PDB Integrity".to_string(),
            status,
            checks,
            counts: None,
        },
        Some((parsed, diag)),
    )
}
