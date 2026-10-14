//! The plain-language summary of a repair preview: what Apply Fixes changes,
//! grouped by what a DJ cares about, above the technical fix list.

use super::*;

/// Overview rows, in render order. Titles match the diagnostics overview
/// (`service::diagnostics::diagnostics_overview`) where the area is the same.
const AREA_OLDER_PLAYERS: usize = 0;
const AREA_NEWER_PLAYERS: usize = 1;
const AREA_PLAYLISTS: usize = 2;
const AREA_MUSIC_FILES: usize = 3;
const AREA_ANALYSIS: usize = 4;
const AREA_ENGINE: usize = 5;
const AREA_TITLES: [&str; 6] = [
    "Older Pioneer players",
    "Newer Pioneer players",
    "Playlists",
    "Music files",
    "Waveforms & beat grids",
    "Denon / Numark players",
];

/// The area a fix belongs to and what it does, in everyday words. Bookkeeping
/// fixes have no line: they are never shown.
fn fix_overview_line(fix_id: &str) -> Option<(usize, &'static str)> {
    let line = match fix_id {
        PDB_TRUNCATED_TABLE_CHAIN_FIX_ID | PDB_TORN_GROWTH_PAGES_FIX_ID => (
            AREA_OLDER_PLAYERS,
            "Repairs damage left in the player database by an interrupted export.",
        ),
        PDB_WRONG_HISTORY_SHAPE_FIX_ID | PDB_EC_CONFLICT_FIX_ID => (
            AREA_OLDER_PLAYERS,
            "Fixes errors that make rekordbox report the USB as corrupted.",
        ),
        PDB_HEADER_COMPATIBILITY_FIX_ID => (
            AREA_OLDER_PLAYERS,
            "Fixes a database setting some players don't accept.",
        ),
        PDB_TRACK_STRING_ALIGNMENT_FIX_ID | PDB_ALBUM_STRING_ALIGNMENT_FIX_ID => (
            AREA_OLDER_PLAYERS,
            "Rewrites track and album names that can freeze players while browsing.",
        ),
        PDB_ARTIST_NAME_LAYOUT_FIX_ID => (
            AREA_OLDER_PLAYERS,
            "Rewrites artist names that can freeze players or go missing on Denon / Numark players.",
        ),
        SYNC_EDB_HISTORY_FROM_PDB_FIX_ID => (
            AREA_NEWER_PLAYERS,
            "Copies the play history so newer players show the same history as older ones.",
        ),
        STRICT_PARITY_UPGRADE_FIX_ID => (
            AREA_PLAYLISTS,
            "Makes every playlist the same on older and newer players.",
        ),
        PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID => (
            AREA_PLAYLISTS,
            "Removes tracks listed twice in a playlist by mistake.",
        ),
        RELINK_MOVED_AUDIO_FIX_ID => (
            AREA_MUSIC_FILES,
            "Reconnects tracks to music files that were renamed or moved on the USB.",
        ),
        UNINDEXED_AUDIO_PLAYLIST_FIX_ID => (
            AREA_MUSIC_FILES,
            "Puts music files that are in no playlist into a playlist called \"Unindexed\".",
        ),
        DUPLICATE_AUDIO_COPIES_FIX_ID => (
            AREA_MUSIC_FILES,
            "Deletes extra copies of music files that are in no playlist.",
        ),
        REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID => (
            AREA_MUSIC_FILES,
            "Removes tracks whose music file is no longer on the USB. Play history keeps them.",
        ),
        EMPTY_ANALYSIS_FILES_FIX_ID => {
            (AREA_ANALYSIS, "Rebuilds missing waveforms and beat grids.")
        }
        SHARED_ANALYSIS_FIX_ID => (
            AREA_ANALYSIS,
            "Gives tracks that show another track's waveform their own.",
        ),
        ANALYSIS_TRACK_PATH_FIX_ID => (
            AREA_ANALYSIS,
            "Labels waveform files so players always pick the right one.",
        ),
        BPM_KEY_MISMATCH_FIX_ID => (
            AREA_ANALYSIS,
            "Makes every player show the same BPM and key.",
        ),
        BEAT_GRID_HEADER_FIX_ID => (
            AREA_ANALYSIS,
            "Updates beat grids from older app versions so Beat Sync works on every player.",
        ),
        MISSING_MP3_SEEK_DATA_FIX_ID => (
            AREA_ANALYSIS,
            "Adds the data players need to jump to any point in MP3 tracks.",
        ),
        MISSING_FLAC_SEEK_DATA_FIX_ID => (
            AREA_ANALYSIS,
            "Adds the data players need to jump to any point in FLAC tracks.",
        ),
        ENGINE_LIBRARY_FIX_ID => (
            AREA_ENGINE,
            "Rebuilds the Denon / Numark library and keeps it up to date from now on. Changes \
             made on those players themselves are replaced.",
        ),
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
        return ("No issues found.".to_string(), Vec::new());
    }

    let mut lines: Vec<Vec<&'static str>> = vec![Vec::new(); AREA_TITLES.len()];
    let mut unlisted = 0usize;
    for fix in &fixable {
        match fix_overview_line(&fix.id) {
            Some((area, line)) if !lines[area].contains(&line) => lines[area].push(line),
            Some(_) => {}
            None => unlisted += 1,
        }
    }
    if unlisted > 0 {
        lines[AREA_OLDER_PLAYERS].push("Fixes other errors in the player database.");
    }
    let mut items: Vec<DiagOverviewItem> = AREA_TITLES
        .iter()
        .zip(lines)
        .filter(|(_, lines)| !lines.is_empty())
        .map(|(title, lines)| DiagOverviewItem {
            title: (*title).to_string(),
            status: Some(DiagStatus::Warn),
            detail: lines.join(" "),
        })
        .collect();
    if not_fixable > 0 {
        items.push(DiagOverviewItem {
            title: "Needs your attention".to_string(),
            status: Some(DiagStatus::Fail),
            detail: format!(
                "{} — Technical details says what to do.",
                counted(
                    not_fixable,
                    "problem can't be fixed automatically",
                    "problems can't be fixed automatically",
                )
            ),
        });
    }

    let summary = if fixable.is_empty() {
        "Nothing here can be fixed automatically.".to_string()
    } else {
        format!(
            "Apply Fixes repairs {} on this USB. The databases are backed up first.",
            counted(fixable.len(), "problem", "problems")
        )
    };
    (summary, items)
}
