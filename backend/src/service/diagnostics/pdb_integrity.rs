//! PDB integrity checks.

use super::*;
use crate::i18n::{Text, tr};

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
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_exists(),
            DiagStatus::Fail,
            tr::diagnostics_check_pdb_exists_missing(pdb_path.display()),
        ));
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
    checks.push(DiagCheck::new(
        tr::diagnostics_check_pdb_exists(),
        DiagStatus::Pass,
        tr::diagnostics_check_pdb_exists_found(),
    ));

    let (parsed, diag) = match parse_pdb_with_diagnostics(pdb_path) {
        Ok(result) => result,
        Err(err) => {
            checks.push(DiagCheck::new(
                tr::diagnostics_check_pdb_parseable(),
                DiagStatus::Fail,
                tr::diagnostics_check_pdb_parseable_error(err),
            ));
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

    checks.push(DiagCheck::new(
        tr::diagnostics_check_pdb_parseable(),
        DiagStatus::Pass,
        tr::diagnostics_check_pdb_parseable_counts(
            parsed.tracks.len(),
            parsed.artists.len(),
            parsed.albums.len(),
            parsed.genres.len(),
            parsed.labels.len(),
            parsed.keys.len(),
            parsed.artworks.len(),
        ),
    ));

    // The detectors below all look at the same bytes; read the file once.
    let bytes = std::fs::read(pdb_path).unwrap_or_default();
    if let Some(header_compat) =
        crate::service::repair::pdb_header_compatibility_value_from_bytes(&bytes)
    {
        // docs/PDB.md "File header 0x10 compatibility field": rekordbox writes
        // 1, 4 and 5, so none of them warns; any repair that runs still sets
        // 1 and 4 to 5. Comparing against a
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
            tr::diagnostics_check_pdb_header_unknown(header_compat)
        } else {
            tr::diagnostics_check_pdb_header_value(header_compat)
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
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_header(),
            header_compat_status,
            detail,
        ));
    }

    {
        // All three are rekordbox transaction bookkeeping: real rekordbox
        // exports carry them and play fine, so they never warn. Repair USB
        // still normalizes them whenever it runs (see
        // `repair::fix_is_bookkeeping`); our own writer never produces them.
        let bad_u5 = crate::service::repair::detect_pdb_sentinel_u5_on_data_pages(&bytes);
        let bad_flags = crate::service::repair::detect_pdb_wrong_page_flags(&bytes);
        let bad_tranrf = crate::service::repair::detect_pdb_zero_tranrf_all_tables(&bytes);
        let mut parts: Vec<Text> = Vec::new();
        if !bad_u5.is_empty() {
            parts.push(tr::diagnostics_check_pdb_page_headers_sentinel_u5(
                bad_u5.len(),
            ));
        }
        if !bad_flags.is_empty() {
            parts.push(tr::diagnostics_check_pdb_page_headers_page_flags(
                bad_flags.len(),
            ));
        }
        if !bad_tranrf.is_empty() {
            parts.push(tr::diagnostics_check_pdb_page_headers_tranrf(
                bad_tranrf.len(),
            ));
        }
        let detail = if parts.is_empty() {
            tr::diagnostics_check_pdb_page_headers_valid()
        } else {
            raw_warnings.push(logging::log(
                Level::Info,
                "usb-diagnostics",
                "usb.diagnostics.pdb-page-header-bookkeeping",
                format!(
                    "rekordbox transaction bookkeeping on PDB data pages: {}; normalized whenever a repair runs.",
                    join_english(&parts)
                ),
            ));
            tr::diagnostics_check_pdb_page_headers_bookkeeping(join_rendered(&parts, "; "))
        };
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_page_headers(),
            DiagStatus::Pass,
            detail,
        ));
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
        let mut fail_parts: Vec<Text> = Vec::new();
        // Soft warnings: player-tolerant shapes rekordbox itself never writes.
        let mut warn_parts: Vec<Text> = Vec::new();
        // rekordbox transaction bookkeeping: real rekordbox exports carry these
        // and play fine, so they never warn (see the page-headers check above).
        let mut bookkeeping_parts: Vec<Text> = Vec::new();

        if !stale_btree.is_empty() {
            bookkeeping_parts.push(tr::diagnostics_check_pdb_structure_stale_btree(
                stale_btree.len(),
            ));
        }
        if !wrong_pl_tree.is_empty() {
            bookkeeping_parts.push(tr::diagnostics_check_pdb_structure_playlist_tree_shape(
                wrong_pl_tree.len(),
            ));
        }
        if !tombstoned.is_empty() {
            bookkeeping_parts.push(tr::diagnostics_check_pdb_structure_tombstoned(
                tombstoned.len(),
            ));
        }
        if !wrong_track_u5.is_empty() {
            bookkeeping_parts.push(tr::diagnostics_check_pdb_structure_track_page_shape(
                wrong_track_u5.len(),
            ));
        }
        if !t00_multipage_active.is_empty() {
            bookkeeping_parts.push(tr::diagnostics_check_pdb_structure_track_page_active(
                t00_multipage_active.len(),
            ));
        }
        if !bookkeeping_parts.is_empty() {
            raw_warnings.push(logging::log(
                Level::Info,
                "usb-diagnostics",
                "usb.diagnostics.pdb-structural-bookkeeping",
                format!(
                    "rekordbox transaction bookkeeping in PDB tables: {}; normalized whenever a repair runs.",
                    join_english(&bookkeeping_parts)
                ),
            ));
        }
        if !wrong_history.is_empty() {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-wrong-history-shape",
                format!(
                    "{} with wrong u5/num_rl shape; run repair_pdb_wrong_history_page_shape to fix.",
                    counted(wrong_history.len(), "history page", "history pages")
                ),
            ));
            warn_parts.push(tr::diagnostics_check_pdb_structure_history_shape(
                wrong_history.len(),
            ));
        }
        if !ec_conflicts.is_empty() {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-ec-data-page-conflict",
                format!(
                    "{} with empty_candidate pointing to another table's data page \
                     (may cause write-pointer conflict rejection); run repair_pdb_ec_data_page_conflict to fix.",
                    counted(ec_conflicts.len(), "table", "tables")
                ),
            ));
            fail_parts.push(tr::diagnostics_check_pdb_structure_ec_conflict(
                ec_conflicts.len(),
            ));
        }
        if !truncated_chains.is_empty() {
            raw_warnings.push(logging::log(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.pdb-truncated-table-chain",
                format!(
                    "{} with a declared last page beyond the physical end of the file \
                     (interrupted export); run repair_pdb_truncated_table_chain to fix.",
                    counted(truncated_chains.len(), "table", "tables")
                ),
            ));
            fail_parts.push(tr::diagnostics_check_pdb_structure_truncated(
                truncated_chains.len(),
            ));
        }
        if !torn_growth.is_empty() {
            let ec_count = torn_growth.garbage_ec_pages.len();
            let ec_desc = if ec_count > 0 {
                format!(
                    "{} with garbage instead of a blank page",
                    counted(ec_count, "empty_candidate page", "empty_candidate pages")
                )
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
                tr::diagnostics_check_pdb_structure_torn_pages(ec_count)
            } else {
                tr::diagnostics_check_pdb_structure_torn_tail()
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
            if bookkeeping_parts.is_empty() {
                tr::diagnostics_check_pdb_structure_ok().render()
            } else {
                tr::diagnostics_check_pdb_structure_ok_bookkeeping(join_rendered(
                    &bookkeeping_parts,
                    "; ",
                ))
                .render()
            }
        } else {
            let mut segments = Vec::new();
            if !fail_parts.is_empty() {
                segments.push(tr::diagnostics_check_pdb_structure_fails(join_rendered(
                    &fail_parts,
                    "; ",
                )));
            }
            if !warn_parts.is_empty() {
                segments.push(tr::diagnostics_check_pdb_structure_warns(join_rendered(
                    &warn_parts,
                    "; ",
                )));
            }
            join_rendered(&segments, " | ")
        };
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_structure(),
            integrity_status,
            detail,
        ));
    }

    checks.push(DiagCheck::new(
        tr::diagnostics_check_pdb_playlists(),
        DiagStatus::Pass,
        tr::diagnostics_check_pdb_playlists_counts(
            parsed.playlist_tree.len(),
            parsed.playlist_entries.len(),
        ),
    ));

    // History playlists in PDB. Fresh/never-played exports ship t17/t18 with
    // a fixed block of blank template/seed rows (no name, no track link)
    // alongside any real recorded sessions; only count rows that look like
    // real rekordbox-recorded history, matching the same rule
    // `derive_history_sync_payload` already uses for the eDB history-sync
    // repair.
    {
        let (hp, he) = count_named_history_rows(&parsed.history_playlists, &parsed.history_entries);
        let hr = parsed.history_rows.len();
        let detail = if hp > 0 || he > 0 || hr > 0 {
            tr::diagnostics_check_pdb_history_counts(hp, he, hr)
        } else {
            tr::diagnostics_check_pdb_history_empty()
        };
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_history(),
            DiagStatus::Pass,
            detail,
        ));
    }

    // num_rl=8191 is a valid external-library sentinel; treat as compatibility info.
    checks.push(DiagCheck::new(
        tr::diagnostics_check_num_rl_pages(),
        DiagStatus::Pass,
        tr::diagnostics_check_num_rl_pages_detail(diag.pages_with_num_rl_8191, diag.total_pages),
    ));

    // nrs u8 wrapping is expected on some pages; recovery is part of normal parsing.
    checks.push(DiagCheck::new(
        tr::diagnostics_check_nrs_wrapping(),
        DiagStatus::Pass,
        tr::diagnostics_check_nrs_wrapping_detail(diag.nrs_wrapping_pages),
    ));

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
    let orphaned_entries = parsed
        .playlist_entries
        .iter()
        .filter(|e| !track_ids.contains(&e.track_id))
        .count();
    checks.push(DiagCheck::new(
        tr::diagnostics_check_orphaned_entries(),
        orphan_status,
        tr::diagnostics_check_orphaned_entries_detail(orphaned_entries, orphaned.len()),
    ));

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
        checks.push(DiagCheck::new(
            tr::diagnostics_check_parse_warnings(),
            DiagStatus::Warn,
            tr::diagnostics_check_parse_warnings_detail(parsed.warnings.len()),
        ));
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

/// `parts` rendered in the current language, joined with `separator`.
pub(super) fn join_rendered(parts: &[Text], separator: &str) -> String {
    parts
        .iter()
        .map(Text::render)
        .collect::<Vec<_>>()
        .join(separator)
}

/// `parts` in English, joined with "; ", for the log.
pub(super) fn join_english(parts: &[Text]) -> String {
    parts
        .iter()
        .map(Text::render_english)
        .collect::<Vec<_>>()
        .join("; ")
}
