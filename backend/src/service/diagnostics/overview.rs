//! Display layout: grouping checks into sections and writing the overview lines.

use super::*;
use crate::i18n::{Text, tr};

pub(super) const DISPLAY_PDB: usize = 0;
pub(super) const DISPLAY_EDB: usize = 1;
pub(super) const DISPLAY_ENGINE: usize = 2;
pub(super) const DISPLAY_PLAYLISTS: usize = 3;
pub(super) const DISPLAY_ANALYSIS: usize = 4;
pub(super) const DISPLAY_LOW_LEVEL: usize = 5;

/// One section of the diagnostics report as the user sees it.
pub(super) struct DisplaySection {
    /// Heading of the box this section is a column in; consecutive sections
    /// with the same group share the box.
    group: fn() -> Text,
    title: fn() -> Text,
    /// Check labels this section gathers, in display order.
    pub(super) labels: &'static [fn() -> Text],
}

/// Display sections of the diagnostics report, in render order. Checks are
/// computed per diagnose stage (PDB, eDB, contents, ...) and regrouped here by
/// what they tell the user: the databases on the USB first, then playlists and
/// analysis, raw format counters last. `DISPLAY_*` index into it.
pub(super) const DISPLAY_LAYOUT: [DisplaySection; 7] = [
    DisplaySection {
        group: tr::diagnostics_group_databases,
        title: tr::diagnostics_section_pdb,
        labels: &[
            tr::diagnostics_check_pdb_exists,
            tr::diagnostics_check_pdb_parseable,
            tr::diagnostics_check_pdb_header,
            tr::diagnostics_check_pdb_page_headers,
            tr::diagnostics_check_pdb_structure,
            tr::diagnostics_check_string_alignment,
            T19_TEMPLATE_STATE_LABEL,
            tr::diagnostics_check_parse_warnings,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_databases,
        title: tr::diagnostics_section_edb,
        labels: &[
            tr::diagnostics_check_edb,
            tr::diagnostics_check_edb_history,
            tr::diagnostics_check_pdb_paths,
            tr::diagnostics_check_edb_paths,
            tr::diagnostics_check_path_agreement,
            tr::diagnostics_check_edb_analysis,
            tr::diagnostics_check_bpm_key,
            tr::diagnostics_check_key_overlap,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_databases,
        title: tr::diagnostics_section_engine,
        labels: &[],
    },
    DisplaySection {
        group: tr::diagnostics_group_library,
        title: tr::diagnostics_section_playlists,
        labels: &[
            tr::diagnostics_check_playlist_data,
            tr::diagnostics_check_overall_resolution,
            tr::diagnostics_check_db_only_playlists,
            tr::diagnostics_check_pdb_playlists,
            tr::diagnostics_check_orphaned_entries,
            tr::diagnostics_check_pdb_history,
            tr::diagnostics_check_operational,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_library,
        title: tr::diagnostics_section_analysis,
        labels: &[
            tr::diagnostics_check_pdb_analysis,
            tr::diagnostics_check_beat_grid,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_low_level,
        title: tr::diagnostics_section_player_counters,
        labels: &[
            tr::diagnostics_check_predicted_counter,
            tr::diagnostics_check_shape_mode,
            tr::diagnostics_check_cross_check,
            tr::diagnostics_check_history_pointers,
            tr::diagnostics_check_primitive_signal,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_low_level,
        title: tr::diagnostics_section_page_format,
        labels: &[
            tr::diagnostics_check_num_rl_pages,
            tr::diagnostics_check_nrs_wrapping,
        ],
    },
];

/// Regroups report sections into `layout` order (`layout`,
/// `PARITY_LAYOUT`).
/// `sources` pairs each stage section with the display section that receives
/// its checks whose label is not in `layout` (e.g. the per-playlist
/// resolution rows). Within a display section, listed labels come first in
/// layout order, unlisted ones after in arrival order. A display section's
/// status is the worst of its checks, plus the stage status of every source
/// whose checks all landed in it (keeps e.g. the player-counter confidence
/// warning, which no single check carries). Empty sections are dropped;
/// consecutive sections sharing a `group` come out as one group.
pub(super) fn regroup_sections(
    layout: &[DisplaySection],
    sources: &[(&DiagSection, usize)],
) -> Vec<DiagSectionGroup> {
    let mut grouped: Vec<Vec<(usize, DiagCheck)>> = vec![Vec::new(); layout.len()];
    let mut statuses: Vec<Vec<DiagStatus>> = vec![Vec::new(); layout.len()];
    for (section, default_target) in sources {
        let mut targets = HashSet::new();
        for check in &section.checks {
            let (target, rank) = layout
                .iter()
                .enumerate()
                .find_map(|(i, display)| {
                    display
                        .labels
                        .iter()
                        .position(|label| !check.key.is_empty() && label().key() == check.key)
                        .map(|rank| (i, rank))
                })
                .unwrap_or((*default_target, usize::MAX));
            targets.insert(target);
            statuses[target].push(check.status.clone());
            grouped[target].push((rank, check.clone()));
        }
        if targets.len() <= 1 {
            let target = targets.into_iter().next().unwrap_or(*default_target);
            statuses[target].push(section.status.clone());
        }
    }
    let mut groups: Vec<DiagSectionGroup> = Vec::new();
    // The catalog key of the last group's title: sections join it by key, not translated text.
    let mut last_group_key = "";
    for (display, (mut checks, statuses)) in layout.iter().zip(grouped.into_iter().zip(statuses)) {
        if checks.is_empty() {
            continue;
        }
        checks.sort_by_key(|(rank, _)| *rank);
        let title = (display.title)();
        let section = DiagSection {
            key: title.key(),
            title: title.render(),
            status: DiagStatus::worst_of(&statuses.iter().collect::<Vec<_>>()),
            checks: checks.into_iter().map(|(_, c)| c).collect(),
            counts: None,
        };
        let group = (display.group)();
        match groups.last_mut() {
            Some(last) if last_group_key == group.key() => last.sections.push(section),
            _ => groups.push(DiagSectionGroup {
                title: group.render(),
                sections: vec![section],
            }),
        }
        last_group_key = group.key();
    }
    groups
}

/// Counts behind the plain-language overview that the display sections only
/// carry as preformatted detail text.
pub(super) struct OverviewFacts {
    pub(super) edb_file_present: bool,
    pub(super) pdb_tracks: usize,
    pub(super) edb_tracks: usize,
    /// Tracks in PDB but not in eDB (history-only tracks excluded).
    pub(super) pdb_only_tracks: usize,
    pub(super) edb_only_tracks: usize,
    pub(super) playlists: usize,
    pub(super) playlist_entries: usize,
    pub(super) resolved_playlist_entries: usize,
}

/// The short "what does this mean for me" summary shown above the technical
/// sections: one row per thing a DJ cares about, in everyday words. Built from
/// the display sections so its status always agrees with them.
pub(super) fn diagnostics_overview(
    groups: &[DiagSectionGroup],
    facts: &OverviewFacts,
) -> Vec<DiagOverviewItem> {
    let section = |index: usize| {
        groups
            .iter()
            .flat_map(|g| &g.sections)
            .find(|s| s.key == (DISPLAY_LAYOUT[index].title)().key())
    };
    let check_status = |sec: Option<&DiagSection>, label: fn() -> Text| {
        let key = label().key();
        sec.and_then(|s| s.checks.iter().find(|c| c.key == key))
            .map(|c| c.status.clone())
    };
    let is_pass = |status: &DiagStatus| matches!(status, DiagStatus::Pass);
    let item = |title: Text, status: Option<DiagStatus>, detail: String| DiagOverviewItem {
        title: title.render(),
        status,
        detail,
    };
    let with_hint = |problems: Vec<Text>| with_preview_fixes_hint(&problems);
    let mut items = Vec::new();

    // Older Pioneer players read export.pdb.
    let pdb = section(DISPLAY_PDB);
    let pdb_status = pdb.map_or(DiagStatus::Fail, |s| s.status.clone());
    let pdb_unreadable = [
        tr::diagnostics_check_pdb_exists as fn() -> Text,
        tr::diagnostics_check_pdb_parseable,
    ]
    .into_iter()
    .any(|label| !check_status(pdb, label).is_some_and(|s| is_pass(&s)));
    // The repair rebuilds a missing database from the other one.
    let pdb_missing =
        !check_status(pdb, tr::diagnostics_check_pdb_exists).is_some_and(|s| is_pass(&s));
    let pdb_detail = if pdb_missing && facts.edb_file_present {
        with_hint(vec![tr::diagnostics_overview_pdb_missing()])
    } else if pdb_unreadable {
        tr::diagnostics_overview_pdb_unreadable().render()
    } else {
        match pdb_status {
            DiagStatus::Pass => {
                tr::diagnostics_overview_pdb_ready(facts.playlists, facts.pdb_tracks).render()
            }
            DiagStatus::Warn
                if check_status(pdb, T19_TEMPLATE_STATE_LABEL).is_some_and(|s| !is_pass(&s)) =>
            {
                with_hint(vec![tr::diagnostics_overview_pdb_track_count()])
            }
            DiagStatus::Warn => with_hint(vec![tr::diagnostics_overview_pdb_irregular()]),
            DiagStatus::Fail => with_hint(vec![tr::diagnostics_overview_pdb_errors()]),
        }
    };
    items.push(item(
        tr::diagnostics_area_older_players(),
        Some(pdb_status),
        pdb_detail,
    ));

    // Newer Pioneer players also read exportLibrary.db.
    let edb = section(DISPLAY_EDB);
    let edb_status = edb.map_or(DiagStatus::Warn, |s| s.status.clone());
    let edb_detail = if !facts.edb_file_present && !pdb_missing {
        with_hint(vec![tr::diagnostics_overview_edb_missing()])
    } else if !facts.edb_file_present {
        tr::diagnostics_overview_edb_not_yet().render()
    } else if !check_status(edb, tr::diagnostics_check_edb).is_some_and(|s| is_pass(&s)) {
        tr::diagnostics_overview_edb_unreadable().render()
    } else {
        let mut problems = Vec::new();
        if facts.pdb_only_tracks > 0 {
            problems.push(tr::diagnostics_overview_edb_pdb_only(facts.pdb_only_tracks));
        }
        if facts.edb_only_tracks > 0 {
            problems.push(tr::diagnostics_overview_edb_edb_only(facts.edb_only_tracks));
        }
        if !check_status(edb, tr::diagnostics_check_bpm_key).is_none_or(|s| is_pass(&s)) {
            problems.push(tr::diagnostics_overview_edb_bpm_key());
        }
        if !check_status(edb, tr::diagnostics_check_edb_analysis).is_none_or(|s| is_pass(&s)) {
            problems.push(tr::diagnostics_overview_edb_analysis());
        }
        if problems.is_empty() && !is_pass(&edb_status) {
            problems.push(tr::diagnostics_overview_edb_irregular());
        }
        if problems.is_empty() {
            tr::diagnostics_overview_edb_ready(facts.edb_tracks).render()
        } else {
            with_hint(problems)
        }
    };
    items.push(item(
        tr::diagnostics_area_newer_players(),
        Some(edb_status),
        edb_detail,
    ));

    let playlists = section(DISPLAY_PLAYLISTS);
    let playlists_status = playlists.map_or(DiagStatus::Pass, |s| s.status.clone());
    let missing_entries = facts
        .playlist_entries
        .saturating_sub(facts.resolved_playlist_entries);
    let playlists_detail = if is_pass(&playlists_status) {
        tr::diagnostics_overview_playlists_ready(facts.playlists).render()
    } else if missing_entries > 0 {
        with_hint(vec![tr::diagnostics_overview_playlists_missing(
            missing_entries,
        )])
    } else {
        with_hint(vec![tr::diagnostics_overview_playlists_mismatch()])
    };
    items.push(item(
        tr::diagnostics_area_playlists(),
        Some(playlists_status),
        playlists_detail,
    ));

    let analysis = section(DISPLAY_ANALYSIS);
    let analysis_status = analysis.map_or(DiagStatus::Pass, |s| s.status.clone());
    let analysis_detail = if is_pass(&analysis_status) {
        tr::diagnostics_overview_analysis_ready().render()
    } else {
        with_hint(vec![tr::diagnostics_overview_analysis_problems()])
    };
    items.push(item(
        tr::diagnostics_area_analysis(),
        Some(analysis_status),
        analysis_detail,
    ));

    let engine = section(DISPLAY_ENGINE);
    let engine_item = match engine {
        None => item(
            tr::diagnostics_area_engine(),
            None,
            tr::diagnostics_overview_engine_next_change().render(),
        ),
        Some(_) if check_status(engine, tr::diagnostics_check_engine_missing).is_some() => item(
            tr::diagnostics_area_engine(),
            None,
            tr::diagnostics_overview_engine_not_set_up().render(),
        ),
        Some(sec) => {
            let mut problems = Vec::new();
            if !check_status(engine, tr::diagnostics_check_engine_upkeep)
                .is_none_or(|s| is_pass(&s))
            {
                problems.push(tr::diagnostics_overview_engine_updates_off());
            }
            if !check_status(engine, tr::diagnostics_check_engine_sync).is_none_or(|s| is_pass(&s))
            {
                problems.push(tr::diagnostics_overview_engine_out_of_date());
            }
            let detail = if problems.is_empty() {
                tr::diagnostics_overview_engine_ready().render()
            } else {
                with_hint(problems)
            };
            item(
                tr::diagnostics_area_engine(),
                Some(sec.status.clone()),
                detail,
            )
        }
    };
    items.push(engine_item);
    items
}

pub(super) const PARITY_PLAYLISTS: usize = 0;
pub(super) const PARITY_TRACK_INFO: usize = 1;
pub(super) const PARITY_FILES: usize = 2;

/// Strict-parity checks grouped by what they compare. `PARITY_*` index into it.
pub(super) const PARITY_LAYOUT: [DisplaySection; 3] = [
    DisplaySection {
        group: tr::diagnostics_group_comparison,
        title: tr::diagnostics_section_playlists,
        labels: &[
            tr::diagnostics_check_parity_identity,
            tr::diagnostics_check_parity_membership,
            tr::diagnostics_check_parity_ordering,
            tr::diagnostics_check_parity_duplicates,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_comparison,
        title: tr::diagnostics_section_track_details,
        labels: &[
            tr::diagnostics_check_parity_pdb_metadata,
            tr::diagnostics_check_parity_edb_metadata,
            tr::diagnostics_check_parity_dictionary,
            tr::diagnostics_check_parity_artwork,
            tr::diagnostics_check_parity_rating_colour,
        ],
    },
    DisplaySection {
        group: tr::diagnostics_group_comparison,
        title: tr::diagnostics_section_music_files,
        labels: &[
            tr::diagnostics_check_parity_files,
            tr::diagnostics_check_parity_paths,
        ],
    },
];

/// Report-level rows the badge and the overview already cover.
pub(super) const PARITY_HEADLINE_CHECKS: [fn() -> Text; 2] = [
    tr::diagnostics_check_parity_overall,
    tr::diagnostics_check_parity_section,
];

/// The strict-parity checks regrouped into `PARITY_LAYOUT` columns.
pub(super) fn parity_section_groups(checks: &[DiagCheck]) -> Vec<DiagSectionGroup> {
    let source = DiagSection {
        key: "",
        title: "Strict parity".to_string(),
        status: DiagStatus::Pass,
        checks: checks
            .iter()
            .filter(|c| {
                !PARITY_HEADLINE_CHECKS
                    .iter()
                    .any(|label| label().key() == c.key)
            })
            .cloned()
            .collect(),
        counts: None,
    };
    regroup_sections(&PARITY_LAYOUT, &[(&source, PARITY_TRACK_INFO)])
}

/// Plain-language summary of the strict-parity report: whether older and
/// newer Pioneer players see the same playlists, track details and files.
pub(super) fn parity_overview(
    groups: &[DiagSectionGroup],
    totals: &ParityTotals,
    playlists: usize,
    raw_coverage: Option<&StrictRawCoverageParity>,
) -> Vec<DiagOverviewItem> {
    let section = |index: usize| {
        groups
            .iter()
            .flat_map(|g| &g.sections)
            .find(|s| s.key == (PARITY_LAYOUT[index].title)().key())
    };
    let status_of = |index: usize| {
        DiagStatus::worst_of(
            &section(index)
                .map(|s| s.checks.iter().map(|c| &c.status).collect::<Vec<_>>())
                .unwrap_or_default(),
        )
    };
    let row =
        |title: Text, status: DiagStatus, problems: Vec<Text>, ready: Text| DiagOverviewItem {
            title: title.render(),
            status: Some(status),
            detail: if problems.is_empty() {
                ready.render()
            } else {
                with_preview_fixes_hint(&problems)
            },
        };
    let problem = |count: usize, text: fn(usize) -> Text| (count > 0).then(|| text(count));

    let playlist_problems = [
        problem(
            totals.playlist_id_mismatches,
            tr::parity_overview_playlist_ids,
        ),
        problem(totals.total_only_in_pdb, tr::parity_overview_only_in_pdb),
        problem(totals.total_only_in_edb, tr::parity_overview_only_in_edb),
        problem(totals.order_mismatches, tr::parity_overview_order),
        problem(
            totals.total_duplicate_entries,
            tr::parity_overview_duplicates,
        ),
    ];
    let track_problems = [
        problem(
            totals.total_pdb_missing_core_metadata,
            tr::parity_overview_pdb_details,
        ),
        problem(
            totals.total_edb_missing_core_metadata,
            tr::parity_overview_edb_details,
        ),
        problem(
            totals.total_dictionary_id_issues,
            tr::parity_overview_dictionary,
        ),
        problem(totals.total_artwork_mismatches, tr::parity_overview_artwork),
        problem(
            totals.rating_colour_mismatches,
            tr::parity_overview_rating_colour,
        ),
    ];
    let file_problems = [
        raw_coverage.and_then(|r| problem(r.missing_count, tr::parity_overview_files_missing)),
        raw_coverage.and_then(|r| problem(r.extra_count, tr::parity_overview_files_extra)),
        problem(totals.total_path_mismatches, tr::parity_overview_paths),
    ];

    vec![
        row(
            tr::diagnostics_area_playlists(),
            status_of(PARITY_PLAYLISTS),
            playlist_problems.into_iter().flatten().collect(),
            tr::parity_overview_playlists_ready(playlists),
        ),
        row(
            tr::diagnostics_area_track_details(),
            status_of(PARITY_TRACK_INFO),
            track_problems.into_iter().flatten().collect(),
            tr::parity_overview_tracks_ready(),
        ),
        row(
            tr::diagnostics_area_music_files(),
            status_of(PARITY_FILES),
            file_problems.into_iter().flatten().collect(),
            tr::parity_overview_files_ready(),
        ),
    ]
}

/// An overview row's problems, one sentence each, and the hint that Preview
/// Fixes can repair them.
fn with_preview_fixes_hint(problems: &[Text]) -> String {
    problems
        .iter()
        .chain([&tr::diagnostics_preview_fixes_hint()])
        .map(Text::render)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Short "Issues" badges for a strict-parity playlist row (previously
/// `formatParityIssues` in `vanilla-ui/components/usb/actions.mjs`). A `+PDB` /
/// `+eDB` membership badge only shows when the other side actually has tracks,
/// so a wholly-missing playlist isn't double-reported.
pub(super) fn parity_issue_labels(d: &UsbParityPlaylistDetail) -> Vec<String> {
    /// A badge's count (0: no badge) and its text.
    type Badge = (usize, fn(usize) -> Text);
    let counts: [Badge; 10] = [
        (
            if d.edb_tracks > 0 { d.only_in_pdb } else { 0 },
            tr::parity_issue_only_in_pdb,
        ),
        (
            if d.pdb_tracks > 0 { d.only_in_edb } else { 0 },
            tr::parity_issue_only_in_edb,
        ),
        (d.pdb_duplicate_entries, tr::parity_issue_duplicates),
        (d.pdb_missing_core_metadata, tr::parity_issue_pdb_gaps),
        (d.edb_missing_core_metadata, tr::parity_issue_edb_gaps),
        (d.path_mismatch_tracks, tr::parity_issue_path_mismatch),
        (d.dictionary_id_issue_tracks, tr::parity_issue_dictionary),
        (d.artwork_mismatch_tracks, tr::parity_issue_artwork),
        (
            d.rating_colour_mismatch_tracks,
            tr::parity_issue_rating_colour,
        ),
        (d.field_note_tracks, tr::parity_issue_field_notes),
    ];
    let flags: [(bool, fn() -> Text); 3] = [
        (!d.playlist_id_match, tr::parity_issue_id_mismatch),
        (!d.sort_order_match, tr::parity_issue_sort_mismatch),
        (d.order_mismatch, tr::parity_issue_order_mismatch),
    ];
    // Badge order: membership, then the playlist-wide flags, then the rest.
    let (membership, rest) = counts.split_at(2);
    let badges = |items: &[Badge]| {
        items
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, text)| text(*count).render())
            .collect::<Vec<_>>()
    };
    let mut labels = badges(membership);
    labels.extend(
        flags
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, text)| text().render()),
    );
    labels.extend(badges(rest));
    labels
}
