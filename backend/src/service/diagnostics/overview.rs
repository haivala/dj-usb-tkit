//! Display layout: grouping checks into sections and writing the overview lines.

use super::*;

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
    group: &'static str,
    title: &'static str,
    /// Check labels this section gathers, in display order.
    labels: &'static [&'static str],
}

/// Display sections of the diagnostics report, in render order. Checks are
/// computed per diagnose stage (PDB, eDB, contents, ...) and regrouped here by
/// what they tell the user: the databases on the USB first, then playlists and
/// analysis, raw format counters last. `DISPLAY_*` index into it.
pub(super) const DISPLAY_LAYOUT: [DisplaySection; 7] = [
    DisplaySection {
        group: "Databases",
        title: "PDB",
        labels: &[
            "PDB exists",
            "PDB parseable",
            "PDB header compatibility",
            "PDB page headers",
            "PDB structural integrity",
            "Track/album string alignment",
            "Parse warnings",
        ],
    },
    DisplaySection {
        group: "Databases",
        title: "eDB",
        labels: &[
            "eDB",
            "eDB history",
            "PDB indexed paths",
            "eDB indexed paths",
            "PDB/eDB path agreement",
            "eDB analysis refs",
            "BPM/key consistency",
            "PDB vs eDB key overlap (informational)",
        ],
    },
    DisplaySection {
        group: "Databases",
        title: "Engine DJ",
        labels: &[],
    },
    DisplaySection {
        group: "Library",
        title: "Playlists",
        labels: &[
            "Playlist data",
            "Overall resolution",
            "DB-only playlists",
            "PDB playlists",
            "Orphaned entries",
            "PDB history",
            "Operational interpretation",
        ],
    },
    DisplaySection {
        group: "Library",
        title: "Analysis",
        labels: &["PDB analysis refs", "Beat grid format"],
    },
    DisplaySection {
        group: "Low-level",
        title: "Player counters",
        labels: &[
            "Predicted player counter",
            "Shape mode",
            "Cross-check",
            "History pointers",
            "Primitive signal",
        ],
    },
    DisplaySection {
        group: "Low-level",
        title: "Page format",
        labels: &["num_rl=8191 pages", "nrs wrapping"],
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
                        .position(|l| *l == check.label)
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
    for (display, (mut checks, statuses)) in layout.iter().zip(grouped.into_iter().zip(statuses)) {
        if checks.is_empty() {
            continue;
        }
        checks.sort_by_key(|(rank, _)| *rank);
        let section = DiagSection {
            title: display.title.to_string(),
            status: DiagStatus::worst_of(&statuses.iter().collect::<Vec<_>>()),
            checks: checks.into_iter().map(|(_, c)| c).collect(),
            counts: None,
        };
        match groups.last_mut() {
            Some(last) if last.title == display.group => last.sections.push(section),
            _ => groups.push(DiagSectionGroup {
                title: display.group.to_string(),
                sections: vec![section],
            }),
        }
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

pub(super) const PREVIEW_FIXES_HINT: &str = "Preview Fixes shows what can be repaired.";

/// `count` followed by the singular or plural phrase, for overview text
/// ("1 track is …" / "3 tracks are …").
pub(crate) fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
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
            .find(|s| s.title == DISPLAY_LAYOUT[index].title)
    };
    let check_status = |sec: Option<&DiagSection>, label: &str| {
        sec.and_then(|s| s.checks.iter().find(|c| c.label == label))
            .map(|c| c.status.clone())
    };
    let is_pass = |status: &DiagStatus| matches!(status, DiagStatus::Pass);
    let item = |title: &str, status: Option<DiagStatus>, detail: String| DiagOverviewItem {
        title: title.to_string(),
        status,
        detail,
    };
    let with_hint = |mut sentences: Vec<String>| {
        sentences.push(PREVIEW_FIXES_HINT.to_string());
        sentences.join(" ")
    };
    let mut items = Vec::new();

    // Older Pioneer players read export.pdb.
    let pdb = section(DISPLAY_PDB);
    let pdb_status = pdb.map_or(DiagStatus::Fail, |s| s.status.clone());
    let pdb_unreadable = ["PDB exists", "PDB parseable"]
        .iter()
        .any(|label| !check_status(pdb, label).is_some_and(|s| is_pass(&s)));
    let pdb_detail = if pdb_unreadable {
        "The player database is missing or can't be read, so these players won't show any music."
            .to_string()
    } else {
        match pdb_status {
            DiagStatus::Pass => format!(
                "Ready: {}, {}.",
                counted(facts.playlists, "playlist", "playlists"),
                counted(facts.pdb_tracks, "track", "tracks")
            ),
            DiagStatus::Warn => with_hint(vec![
                "Readable, but parts of the player database are irregular and desktop DJ software may complain."
                    .to_string(),
            ]),
            DiagStatus::Fail => with_hint(vec![
                "The player database has errors that can make these players freeze or show wrong data."
                    .to_string(),
            ]),
        }
    };
    items.push(item("Older Pioneer players", Some(pdb_status), pdb_detail));

    // Newer Pioneer players also read exportLibrary.db.
    let edb = section(DISPLAY_EDB);
    let edb_status = edb.map_or(DiagStatus::Warn, |s| s.status.clone());
    let edb_detail = if !facts.edb_file_present {
        "This USB has no database for newer players yet. Exporting to it creates one.".to_string()
    } else if !check_status(edb, "eDB").is_some_and(|s| is_pass(&s)) {
        "The database newer players read can't be opened, so they may not show this USB's music."
            .to_string()
    } else {
        let mut problems = Vec::new();
        if facts.pdb_only_tracks > 0 {
            problems.push(counted(
                facts.pdb_only_tracks,
                "track is missing from the database newer players read, so it may not show up there.",
                "tracks are missing from the database newer players read, so they may not show up there.",
            ));
        }
        if facts.edb_only_tracks > 0 {
            problems.push(counted(
                facts.edb_only_tracks,
                "track is only in the newer players' database, so older Pioneer players won't show it.",
                "tracks are only in the newer players' database, so older Pioneer players won't show them.",
            ));
        }
        if !check_status(edb, "BPM/key consistency").is_none_or(|s| is_pass(&s)) {
            problems.push(
                "Some tracks may show a different BPM or key on newer players than on older ones."
                    .to_string(),
            );
        }
        if !check_status(edb, "eDB analysis refs").is_none_or(|s| is_pass(&s)) {
            problems.push(
                "Some tracks may have no waveform or beat grid on newer players.".to_string(),
            );
        }
        if problems.is_empty() && !is_pass(&edb_status) {
            problems.push("Parts of the newer players' database are irregular.".to_string());
        }
        if problems.is_empty() {
            format!("Ready: {}.", counted(facts.edb_tracks, "track", "tracks"))
        } else {
            with_hint(problems)
        }
    };
    items.push(item("Newer Pioneer players", Some(edb_status), edb_detail));

    let playlists = section(DISPLAY_PLAYLISTS);
    let playlists_status = playlists.map_or(DiagStatus::Pass, |s| s.status.clone());
    let missing_entries = facts
        .playlist_entries
        .saturating_sub(facts.resolved_playlist_entries);
    let playlists_detail = if is_pass(&playlists_status) {
        format!(
            "Every track in your {} is found.",
            counted(facts.playlists, "playlist", "playlists")
        )
    } else if missing_entries > 0 {
        with_hint(vec![counted(
            missing_entries,
            "track in your playlists can't be found.",
            "tracks in your playlists can't be found.",
        )])
    } else {
        with_hint(vec![
            "Some playlist data doesn't line up between the databases.".to_string(),
        ])
    };
    items.push(item("Playlists", Some(playlists_status), playlists_detail));

    let analysis = section(DISPLAY_ANALYSIS);
    let analysis_status = analysis.map_or(DiagStatus::Pass, |s| s.status.clone());
    let analysis_detail = if is_pass(&analysis_status) {
        "Every track has waveform and beat grid data.".to_string()
    } else {
        with_hint(vec![
            "Some tracks' waveform or beat grid data is missing or outdated.".to_string(),
        ])
    };
    items.push(item(
        "Waveforms & beat grids",
        Some(analysis_status),
        analysis_detail,
    ));

    let engine = section(DISPLAY_ENGINE);
    let engine_item = match engine {
        None => item(
            "Denon / Numark players",
            None,
            "Will be added the next time this app changes the USB.".to_string(),
        ),
        Some(_) if check_status(engine, "Not on this USB").is_some() => item(
            "Denon / Numark players",
            None,
            "Not set up. Only needed for Denon Prime / Numark Mixstream players: turn on \
             Settings → Export → \"Write Engine DJ library\"."
                .to_string(),
        ),
        Some(sec) => {
            let mut problems = Vec::new();
            if !check_status(engine, "Kept up to date").is_none_or(|s| is_pass(&s)) {
                problems.push(
                    "Updates are turned off, so changes made in this app don't reach these players."
                        .to_string(),
                );
            }
            if !check_status(engine, "Matches export.pdb").is_none_or(|s| is_pass(&s)) {
                problems
                    .push("Out of date, so the player will analyze every track again.".to_string());
            }
            let detail = if problems.is_empty() {
                "Ready.".to_string()
            } else {
                with_hint(problems)
            };
            item("Denon / Numark players", Some(sec.status.clone()), detail)
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
        group: "Comparison",
        title: "Playlists",
        labels: &[
            "Playlist identity parity",
            "Playlist membership parity",
            "Playlist ordering parity",
            "Duplicate PDB entries",
        ],
    },
    DisplaySection {
        group: "Comparison",
        title: "Track details",
        labels: &[
            "PDB metadata completeness",
            "eDB source completeness",
            "PDB dictionary id resolution",
            "Artwork presence parity",
            "Rating and colour parity",
        ],
    },
    DisplaySection {
        group: "Comparison",
        title: "Music files",
        labels: &[
            "Indexed audio file presence",
            "Media and analysis path parity",
        ],
    },
];

/// Report-level rows the badge and the overview already cover.
pub(super) const PARITY_HEADLINE_CHECKS: [&str; 2] = [
    "Overall player parity status",
    "Parity-report section (required)",
];

/// The strict-parity checks regrouped into `PARITY_LAYOUT` columns.
pub(super) fn parity_section_groups(checks: &[DiagCheck]) -> Vec<DiagSectionGroup> {
    let source = DiagSection {
        title: "Strict parity".to_string(),
        status: DiagStatus::Pass,
        checks: checks
            .iter()
            .filter(|c| !PARITY_HEADLINE_CHECKS.contains(&c.label.as_str()))
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
            .find(|s| s.title == PARITY_LAYOUT[index].title)
    };
    let status_of = |index: usize| {
        DiagStatus::worst_of(
            &section(index)
                .map(|s| s.checks.iter().map(|c| &c.status).collect::<Vec<_>>())
                .unwrap_or_default(),
        )
    };
    let row = |title: &str, status: DiagStatus, problems: Vec<String>, ready: String| {
        let detail = if problems.is_empty() {
            ready
        } else {
            let mut sentences = problems;
            sentences.push(PREVIEW_FIXES_HINT.to_string());
            sentences.join(" ")
        };
        DiagOverviewItem {
            title: title.to_string(),
            status: Some(status),
            detail,
        }
    };
    let problem =
        |count: usize, one: &str, many: &str| (count > 0).then(|| counted(count, one, many));

    let playlist_problems = [
        problem(
            totals.playlist_id_mismatches,
            "playlist doesn't line up between the two databases.",
            "playlists don't line up between the two databases.",
        ),
        problem(
            totals.total_only_in_pdb,
            "playlist track is only in the database older Pioneer players read.",
            "playlist tracks are only in the database older Pioneer players read.",
        ),
        problem(
            totals.total_only_in_edb,
            "playlist track is only in the database newer Pioneer players read.",
            "playlist tracks are only in the database newer Pioneer players read.",
        ),
        problem(
            totals.order_mismatches,
            "playlist is in a different order depending on the player.",
            "playlists are in a different order depending on the player.",
        ),
        problem(
            totals.total_duplicate_entries,
            "track is listed twice in a playlist on older Pioneer players.",
            "tracks are listed twice in a playlist on older Pioneer players.",
        ),
    ];
    let track_problems = [
        problem(
            totals.total_pdb_missing_core_metadata,
            "track is missing track details on older Pioneer players.",
            "tracks are missing track details on older Pioneer players.",
        ),
        problem(
            totals.total_edb_missing_core_metadata,
            "track is missing track details in the database newer Pioneer players read.",
            "tracks are missing track details in the database newer Pioneer players read.",
        ),
        problem(
            totals.total_dictionary_id_issues,
            "track refers to an artist, album or key that's missing on older Pioneer players.",
            "tracks refer to an artist, album or key that's missing on older Pioneer players.",
        ),
        problem(
            totals.total_artwork_mismatches,
            "track has artwork on one kind of Pioneer player but not the other.",
            "tracks have artwork on one kind of Pioneer player but not the other.",
        ),
        problem(
            totals.rating_colour_mismatches,
            "track has a different rating or colour on older Pioneer players.",
            "tracks have a different rating or colour on older Pioneer players.",
        ),
    ];
    let file_problems = [
        raw_coverage.and_then(|r| {
            problem(
                r.missing_count,
                "audio file is missing from the USB, so that track won't play.",
                "audio files are missing from the USB, so those tracks won't play.",
            )
        }),
        raw_coverage.and_then(|r| {
            problem(
                r.extra_count,
                "audio file on the USB isn't in any playlist.",
                "audio files on the USB aren't in any playlist.",
            )
        }),
        problem(
            totals.total_path_mismatches,
            "track points to a different audio or waveform file depending on the player.",
            "tracks point to a different audio or waveform file depending on the player.",
        ),
    ];

    vec![
        row(
            "Playlists",
            status_of(PARITY_PLAYLISTS),
            playlist_problems.into_iter().flatten().collect(),
            format!(
                "Both databases have the same {}, with the same tracks in the same order.",
                counted(playlists, "playlist", "playlists")
            ),
        ),
        row(
            "Track details",
            status_of(PARITY_TRACK_INFO),
            track_problems.into_iter().flatten().collect(),
            "Every playlist track has complete details and the same artwork in both databases."
                .to_string(),
        ),
        row(
            "Music files",
            status_of(PARITY_FILES),
            file_problems.into_iter().flatten().collect(),
            "Every playlist track's audio file is on the USB, and both databases point to the \
             same files."
                .to_string(),
        ),
    ]
}

/// Short "Issues" badges for a strict-parity playlist row (previously
/// `formatParityIssues` in `vanilla-ui/components/usb/actions.mjs`). A `+PDB` /
/// `+eDB` membership badge only shows when the other side actually has tracks,
/// so a wholly-missing playlist isn't double-reported.
pub(super) fn parity_issue_labels(d: &UsbParityPlaylistDetail) -> Vec<String> {
    let mut labels = Vec::new();
    if d.only_in_pdb > 0 && d.edb_tracks > 0 {
        labels.push(format!("+PDB {}", d.only_in_pdb));
    }
    if d.only_in_edb > 0 && d.pdb_tracks > 0 {
        labels.push(format!("+eDB {}", d.only_in_edb));
    }
    if !d.playlist_id_match {
        labels.push("id mismatch".to_string());
    }
    if !d.sort_order_match {
        labels.push("sort mismatch".to_string());
    }
    if d.order_mismatch {
        labels.push("order mismatch".to_string());
    }
    if d.pdb_duplicate_entries > 0 {
        labels.push(format!("dup PDB {}", d.pdb_duplicate_entries));
    }
    if d.pdb_missing_core_metadata > 0 {
        labels.push(format!("PDB gaps {}", d.pdb_missing_core_metadata));
    }
    if d.edb_missing_core_metadata > 0 {
        labels.push(format!("eDB gaps {}", d.edb_missing_core_metadata));
    }
    if d.path_mismatch_tracks > 0 {
        labels.push(format!("path mismatch {}", d.path_mismatch_tracks));
    }
    if d.dictionary_id_issue_tracks > 0 {
        labels.push(format!("dict issues {}", d.dictionary_id_issue_tracks));
    }
    if d.artwork_mismatch_tracks > 0 {
        labels.push(format!("art mismatch {}", d.artwork_mismatch_tracks));
    }
    if d.rating_colour_mismatch_tracks > 0 {
        labels.push(format!(
            "rating/colour mismatch {}",
            d.rating_colour_mismatch_tracks
        ));
    }
    labels
}
