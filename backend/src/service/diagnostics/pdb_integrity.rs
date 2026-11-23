//! PDB integrity checks.

use super::*;
use crate::i18n::{Text, tr};

/// `parse` reads the PDB at `pdb_path` (the service's shared parse).
pub(crate) fn diagnose_pdb_integrity(
    pdb_path: &std::path::Path,
    parse: impl FnOnce(
        &std::path::Path,
    ) -> BackendResult<std::sync::Arc<crate::service::usb::cache::StagedPdbParse>>,
) -> (
    DiagSection,
    Option<std::sync::Arc<crate::service::usb::cache::StagedPdbParse>>,
) {
    let mut checks = Vec::new();

    if !pdb_path.exists() {
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_exists(),
            DiagStatus::Fail,
            tr::error_file_not_found(pdb_path.display()),
        ));
        return (
            DiagSection {
                key: "",
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

    let staged = match parse(pdb_path) {
        Ok(staged) => staged,
        Err(err) => {
            checks.push(DiagCheck::new(
                tr::diagnostics_check_pdb_parseable(),
                DiagStatus::Fail,
                tr::diagnostics_check_pdb_parseable_error(err),
            ));
            return (
                DiagSection {
                    key: "",
                    title: "PDB Integrity".to_string(),
                    status: DiagStatus::Fail,
                    checks,
                    counts: None,
                },
                None,
            );
        }
    };
    let parsed = &*staged.parsed;
    let diag = &staged.diagnostics;

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
            logging::debug(
                "usb-diagnostics",
                format!(
                    "PDB header compatibility field is {header_compat}; run repair_pdb_header_compatibility_field to restore a compatible value."
                ),
            );
        }
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_header(),
            header_compat_status,
            detail,
        ));
    }

    {
        // rekordbox's own page state (flags 0x24/0x34, footers that follow
        // the transaction rule, the 0x1FFF placeholder) is valid and never
        // reported. These two are damage: from old app repairs, or from a
        // rekordbox write that crashed halfway.
        let damaged = crate::service::repair::detect_pdb_damaged_page_headers(&bytes);
        let inconsistent = crate::service::repair::detect_pdb_inconsistent_page_footers(&bytes);
        let mut parts: Vec<Text> = Vec::new();
        if !damaged.is_empty() {
            parts.push(tr::diagnostics_check_pdb_page_headers_damaged(
                damaged.len(),
            ));
        }
        if !inconsistent.is_empty() {
            parts.push(tr::diagnostics_check_pdb_page_headers_footer(
                inconsistent.len(),
            ));
        }
        let (status, detail) = if parts.is_empty() {
            (
                DiagStatus::Pass,
                tr::diagnostics_check_pdb_page_headers_valid(),
            )
        } else {
            logging::debug(
                "usb-diagnostics",
                format!(
                    "damaged PDB page headers: {}; run repair_pdb_damaged_page_headers / repair_pdb_page_footer_transaction to fix.",
                    join_english(&parts)
                ),
            );
            (
                DiagStatus::Fail,
                tr::diagnostics_check_pdb_structure_fails(join_rendered(&parts, "; ")),
            )
        };
        checks.push(DiagCheck::new(
            tr::diagnostics_check_pdb_page_headers(),
            status,
            detail,
        ));
    }

    {
        let stale_btree = crate::service::repair::detect_pdb_stale_sentinel_btree(&bytes);
        let ec_conflicts = crate::service::repair::detect_pdb_ec_data_page_conflicts(&bytes);
        let truncated_chains = crate::service::repair::detect_pdb_truncated_table_chains(&bytes);
        let torn_growth = crate::service::repair::detect_pdb_torn_growth_pages(&bytes);

        // Hard failures: conditions that cause player or DJ software rejection.
        let mut fail_parts: Vec<Text> = Vec::new();
        // rekordbox transaction bookkeeping: real rekordbox exports carry these
        // and play fine, so they never warn (see the page-headers check above).
        let mut bookkeeping_parts: Vec<Text> = Vec::new();

        if !stale_btree.is_empty() {
            bookkeeping_parts.push(tr::diagnostics_check_pdb_structure_stale_btree(
                stale_btree.len(),
            ));
        }
        if !bookkeeping_parts.is_empty() {
            logging::debug(
                "usb-diagnostics",
                format!(
                    "rekordbox transaction bookkeeping in PDB tables: {}; normalized whenever a repair runs.",
                    join_english(&bookkeeping_parts)
                ),
            );
        }
        if !ec_conflicts.is_empty() {
            logging::debug(
                "usb-diagnostics",
                format!(
                    "{} with empty_candidate pointing to another table's data page \
                     (may cause write-pointer conflict rejection); run repair_pdb_ec_data_page_conflict to fix.",
                    counted(ec_conflicts.len(), "table", "tables")
                ),
            );
            fail_parts.push(tr::diagnostics_check_pdb_structure_ec_conflict(
                ec_conflicts.len(),
            ));
        }
        if !truncated_chains.is_empty() {
            logging::debug(
                "usb-diagnostics",
                format!(
                    "{} with a declared last page beyond the physical end of the file \
                     (interrupted export); run repair_pdb_truncated_table_chain to fix.",
                    counted(truncated_chains.len(), "table", "tables")
                ),
            );
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
            logging::debug(
                "usb-diagnostics",
                format!(
                    "PDB shows signs of an interrupted export (e.g. USB disconnected mid-write): \
                     {ec_desc}{tail_suffix}; run repair_pdb_torn_growth_pages to fix."
                ),
            );
            fail_parts.push(if ec_count > 0 {
                tr::diagnostics_check_pdb_structure_torn_pages(ec_count)
            } else {
                tr::diagnostics_check_pdb_structure_torn_tail()
            });
        }
        let integrity_status = if fail_parts.is_empty() {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        };
        let detail = if fail_parts.is_empty() {
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
            tr::diagnostics_check_pdb_structure_fails(join_rendered(&fail_parts, "; ")).render()
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
            tr::diagnostics_check_history_empty()
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

    // Empty copies of a playlist with tracks: the repair removes them before
    // strict parity, which matches playlists by name.
    let empty_duplicates = crate::service::repair::detect_pdb_empty_duplicate_playlists(parsed);
    for playlist in &empty_duplicates {
        logging::debug(
            "usb-diagnostics",
            format!(
                "empty PDB playlist {} (id {}) has the name of a playlist with tracks",
                playlist.name, playlist.id
            ),
        );
    }
    checks.push(DiagCheck::new(
        tr::diagnostics_check_empty_duplicate_playlists(),
        if empty_duplicates.is_empty() {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        tr::diagnostics_check_empty_duplicate_playlists_detail(empty_duplicates.len()),
    ));

    // Parse warnings
    if !parsed.warnings.is_empty() {
        for message in &parsed.warnings {
            logging::debug("usb-diagnostics", message);
        }
        checks.push(DiagCheck::new(
            tr::diagnostics_check_parse_warnings(),
            DiagStatus::Warn,
            tr::diagnostics_check_parse_warnings_detail(parsed.warnings.len()),
        ));
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (
        DiagSection {
            key: "",
            title: "PDB Integrity".to_string(),
            status,
            checks,
            counts: None,
        },
        Some(staged),
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
