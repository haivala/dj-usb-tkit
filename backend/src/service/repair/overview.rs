//! The plain-language summary of a repair preview: what Apply Fixes changes,
//! grouped by what a DJ cares about, above the technical fix list.

use super::*;
use crate::i18n::{Text, tr};

/// Overview rows, in render order. Titles match the diagnostics overview
/// (`service::diagnostics::diagnostics_overview`) where the area is the same.
const AREA_OLDER_PLAYERS: usize = 0;
const AREA_NEWER_PLAYERS: usize = 1;
const AREA_PLAYLISTS: usize = 2;
const AREA_MUSIC_FILES: usize = 3;
const AREA_ANALYSIS: usize = 4;
const AREA_ENGINE: usize = 5;
const AREA_TITLES: [fn() -> Text; 6] = [
    tr::diagnostics_area_older_players,
    tr::diagnostics_area_newer_players,
    tr::diagnostics_area_playlists,
    tr::diagnostics_area_music_files,
    tr::diagnostics_area_analysis,
    tr::diagnostics_area_engine,
];

/// The area a fix belongs to and what it does, in everyday words. Bookkeeping
/// fixes have no line: they are never shown.
fn fix_overview_line(fix_id: &str) -> Option<(usize, fn() -> Text)> {
    let line: (usize, fn() -> Text) = match fix_id {
        PDB_TRUNCATED_TABLE_CHAIN_FIX_ID | PDB_TORN_GROWTH_PAGES_FIX_ID => {
            (AREA_OLDER_PLAYERS, tr::repair_overview_interrupted_export)
        }
        PDB_T19_TEMPLATE_STATE_FIX_ID => (AREA_OLDER_PLAYERS, tr::repair_overview_track_count),
        PDB_WRONG_HISTORY_SHAPE_FIX_ID | PDB_EC_CONFLICT_FIX_ID => {
            (AREA_OLDER_PLAYERS, tr::repair_overview_rekordbox_corrupted)
        }
        PDB_HEADER_COMPATIBILITY_FIX_ID => (AREA_OLDER_PLAYERS, tr::repair_overview_header),
        PDB_TRACK_STRING_ALIGNMENT_FIX_ID | PDB_ALBUM_STRING_ALIGNMENT_FIX_ID => {
            (AREA_OLDER_PLAYERS, tr::repair_overview_string_alignment)
        }
        PDB_ARTIST_NAME_LAYOUT_FIX_ID => (AREA_OLDER_PLAYERS, tr::repair_overview_artist_names),
        SYNC_EDB_HISTORY_FROM_PDB_FIX_ID => (AREA_NEWER_PLAYERS, tr::repair_overview_history),
        STRICT_PARITY_UPGRADE_FIX_ID => (AREA_PLAYLISTS, tr::repair_overview_strict_parity),
        PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID => {
            (AREA_PLAYLISTS, tr::repair_overview_duplicate_entries)
        }
        RELINK_MOVED_AUDIO_FIX_ID => (AREA_MUSIC_FILES, tr::repair_overview_relink),
        UNINDEXED_AUDIO_PLAYLIST_FIX_ID => (AREA_MUSIC_FILES, tr::repair_overview_unindexed),
        DUPLICATE_AUDIO_COPIES_FIX_ID => (AREA_MUSIC_FILES, tr::repair_overview_duplicate_copies),
        REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID => {
            (AREA_MUSIC_FILES, tr::repair_overview_missing_audio)
        }
        EMPTY_ANALYSIS_FILES_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_empty_analysis),
        SHARED_ANALYSIS_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_shared_analysis),
        ANALYSIS_TRACK_PATH_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_analysis_path),
        BPM_KEY_MISMATCH_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_bpm_key),
        BEAT_GRID_HEADER_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_beat_grid),
        MISSING_MP3_SEEK_DATA_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_mp3_seek),
        MISSING_FLAC_SEEK_DATA_FIX_ID => (AREA_ANALYSIS, tr::repair_overview_flac_seek),
        ENGINE_LIBRARY_FIX_ID => (AREA_ENGINE, tr::repair_overview_engine),
        _ => return None,
    };
    Some(line)
}

/// The headline above the overview and one row per area Apply Fixes touches,
/// plus a row for what it can't fix. Empty rows when there is nothing to show.
pub(super) fn repair_overview(
    fixes: &[RepairFixProposal],
    unsupported: &[RepairUnsupportedItem],
) -> (String, Vec<DiagOverviewItem>) {
    let fixable: Vec<&RepairFixProposal> = fixes
        .iter()
        .filter(|f| f.supported && !f.bookkeeping)
        .collect();
    let not_fixable = fixes
        .iter()
        .filter(|f| !f.supported && !f.bookkeeping)
        .count()
        + unsupported.len();
    if fixable.is_empty() && not_fixable == 0 {
        return (tr::repair_summary_no_issues().render(), Vec::new());
    }

    let mut lines: Vec<Vec<String>> = vec![Vec::new(); AREA_TITLES.len()];
    let mut unlisted = 0usize;
    for fix in &fixable {
        match fix_overview_line(&fix.id) {
            Some((area, line)) => {
                let line = line().render();
                if !lines[area].contains(&line) {
                    lines[area].push(line);
                }
            }
            None => unlisted += 1,
        }
    }
    if unlisted > 0 {
        lines[AREA_OLDER_PLAYERS].push(tr::repair_overview_other().render());
    }
    let mut items: Vec<DiagOverviewItem> = AREA_TITLES
        .iter()
        .zip(lines)
        .filter(|(_, lines)| !lines.is_empty())
        .map(|(title, lines)| DiagOverviewItem {
            title: title().render(),
            status: Some(DiagStatus::Warn),
            detail: lines.join(" "),
        })
        .collect();
    if not_fixable > 0 {
        items.push(DiagOverviewItem {
            title: tr::repair_overview_needs_attention().render(),
            status: Some(DiagStatus::Fail),
            detail: tr::repair_overview_not_fixable(not_fixable).render(),
        });
    }

    let summary = if fixable.is_empty() {
        tr::repair_summary_nothing_fixable()
    } else {
        tr::repair_summary(fixable.len())
    }
    .render();
    (summary, items)
}
