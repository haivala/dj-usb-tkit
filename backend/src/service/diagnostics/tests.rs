use std::collections::{HashMap, HashSet};
use std::fs;

use super::{
    DISPLAY_ANALYSIS, DISPLAY_EDB, DISPLAY_ENGINE, DISPLAY_LAYOUT, DISPLAY_LOW_LEVEL, DISPLAY_PDB,
    DISPLAY_PLAYLISTS, ExportDbPlaylist, OverviewFacts, ParityTotals, RatingColourParity,
    build_usb_parity_comparison, compare_rating_colour, count_named_history_rows,
    diagnose_analysis_integrity, diagnose_contents_integrity, diagnose_playlist_resolution_with_db,
    diagnose_playlist_resolution_with_edb_internal, diagnostics_overview,
    evaluate_strict_raw_coverage_parity, normalize_pdb_path_for_edb_lookup,
    normalize_track_path_for_identity, parity_issue_labels, parity_overview, parity_section_groups,
    regroup_sections, track_identity_key,
};
use crate::models::UsbParityPlaylistDetail;

fn diag_section(title: &str, status: &str, checks: &[(&str, &str)]) -> crate::models::DiagSection {
    use crate::models::{DiagCheck, DiagSection, DiagStatus};
    let parse = |s: &str| match s {
        "WARN" => DiagStatus::Warn,
        "FAIL" => DiagStatus::Fail,
        _ => DiagStatus::Pass,
    };
    DiagSection {
        title: title.to_string(),
        status: parse(status),
        checks: checks
            .iter()
            .map(|(label, st)| DiagCheck {
                label: label.to_string(),
                status: parse(st),
                detail: String::new(),
                link: None,
            })
            .collect(),
        counts: None,
    }
}

#[test]
fn display_sections_regroup_checks_by_label_in_layout_order() {
    use crate::models::DiagStatus;
    let pdb = diag_section(
        "PDB Integrity",
        "WARN",
        &[
            ("PDB exists", "PASS"),
            ("PDB playlists", "PASS"),
            ("Orphaned entries", "WARN"),
            ("num_rl=8191 pages", "PASS"),
        ],
    );
    let playlists = diag_section(
        "Playlist Resolution",
        "PASS",
        &[("Overall resolution", "PASS"), ("My Playlist", "PASS")],
    );
    let contents = diag_section(
        "Contents Integrity",
        "PASS",
        &[
            ("PDB/eDB path agreement", "PASS"),
            ("Track/album string alignment", "PASS"),
        ],
    );
    let edb = diag_section("Database Access", "PASS", &[("eDB", "PASS")]);
    // Status carried by the stage, not by any check (player-counter confidence).
    let cdj = diag_section("Player Counter Snapshot", "WARN", &[("Shape mode", "PASS")]);
    let groups = regroup_sections(
        &DISPLAY_LAYOUT,
        &[
            (&pdb, DISPLAY_PDB),
            (&playlists, DISPLAY_PLAYLISTS),
            (&edb, DISPLAY_EDB),
            (&contents, DISPLAY_EDB),
            (&cdj, DISPLAY_LOW_LEVEL),
        ],
    );
    type SectionLayout<'a> = (&'a str, Vec<&'a str>);
    let layout: Vec<(&str, Vec<SectionLayout>)> = groups
        .iter()
        .map(|g| {
            (
                g.title.as_str(),
                g.sections
                    .iter()
                    .map(|s| {
                        (
                            s.title.as_str(),
                            s.checks.iter().map(|c| c.label.as_str()).collect(),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        layout,
        vec![
            (
                "Databases",
                vec![
                    ("PDB", vec!["PDB exists", "Track/album string alignment"]),
                    ("eDB", vec!["eDB", "PDB/eDB path agreement"]),
                ]
            ),
            (
                "Library",
                vec![(
                    "Playlists",
                    vec![
                        "Overall resolution",
                        "PDB playlists",
                        "Orphaned entries",
                        "My Playlist"
                    ]
                )]
            ),
            (
                "Low-level",
                vec![
                    ("Player counters", vec!["Shape mode"]),
                    ("Page format", vec!["num_rl=8191 pages"]),
                ]
            ),
        ]
    );
    // The PDB stage's WARN came from "Orphaned entries", which moved out.
    assert!(matches!(groups[0].sections[0].status, DiagStatus::Pass));
    assert!(matches!(groups[1].sections[0].status, DiagStatus::Warn));
    assert!(matches!(groups[2].sections[0].status, DiagStatus::Warn));
}

#[test]
fn overview_explains_each_area_in_plain_words() {
    let pdb = diag_section(
        "PDB Integrity",
        "WARN",
        &[
            ("PDB exists", "PASS"),
            ("PDB parseable", "PASS"),
            ("PDB page headers", "WARN"),
        ],
    );
    let edb = diag_section("Database Access", "PASS", &[("eDB", "PASS")]);
    let contents = diag_section(
        "Contents Integrity",
        "WARN",
        &[("PDB/eDB path agreement", "WARN")],
    );
    let playlists = diag_section(
        "Playlist Resolution",
        "PASS",
        &[("Overall resolution", "PASS")],
    );
    let analysis = diag_section("Analysis Files", "PASS", &[("PDB analysis refs", "PASS")]);
    let engine = diag_section("Engine DJ Library", "PASS", &[("Not on this USB", "PASS")]);
    let groups = regroup_sections(
        &DISPLAY_LAYOUT,
        &[
            (&pdb, DISPLAY_PDB),
            (&playlists, DISPLAY_PLAYLISTS),
            (&analysis, DISPLAY_ANALYSIS),
            (&edb, DISPLAY_EDB),
            (&contents, DISPLAY_EDB),
            (&engine, DISPLAY_ENGINE),
        ],
    );
    let overview = diagnostics_overview(
        &groups,
        &OverviewFacts {
            edb_file_present: true,
            pdb_tracks: 120,
            edb_tracks: 20,
            pdb_only_tracks: 100,
            edb_only_tracks: 0,
            playlists: 3,
            playlist_entries: 130,
            resolved_playlist_entries: 130,
        },
    );
    let rows: Vec<(&str, Option<&str>, &str)> = overview
        .iter()
        .map(|i| {
            (
                i.title.as_str(),
                i.status.as_ref().map(|s| match s {
                    crate::models::DiagStatus::Pass => "PASS",
                    crate::models::DiagStatus::Warn => "WARN",
                    crate::models::DiagStatus::Fail => "FAIL",
                }),
                i.detail.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "Older Pioneer players",
                Some("WARN"),
                "Readable, but parts of the player database are irregular and desktop DJ \
                 software may complain. Preview Fixes shows what can be repaired."
            ),
            (
                "Newer Pioneer players",
                Some("WARN"),
                "100 tracks are missing from the database newer players read, so \
                 they may not show up there. Preview Fixes shows what can be repaired."
            ),
            (
                "Playlists",
                Some("PASS"),
                "Every track in your 3 playlists is found."
            ),
            (
                "Waveforms & beat grids",
                Some("PASS"),
                "Every track has waveform and beat grid data."
            ),
            (
                "Denon / Numark players",
                None,
                "Not set up. Only needed for Denon Prime / Numark Mixstream players: turn \
                 on Settings → Export → \"Write Engine DJ library\"."
            ),
        ]
    );
}

fn parity_detail() -> UsbParityPlaylistDetail {
    use crate::models::DiagStatus;
    UsbParityPlaylistDetail {
        name: "P".to_string(),
        pdb_tracks: 10,
        edb_tracks: 10,
        matched_tracks: 10,
        only_in_pdb: 0,
        only_in_edb: 0,
        order_mismatch: false,
        path_mismatch_tracks: 0,
        dictionary_id_issue_tracks: 0,
        playlist_id_match: true,
        sort_order_match: true,
        parent_match: None,
        pdb_playlist_id: None,
        edb_playlist_id: None,
        pdb_sort_order: None,
        edb_sort_order: None,
        pdb_duplicate_entries: 0,
        edb_missing_core_metadata: 0,
        pdb_missing_core_metadata: 0,
        artwork_mismatch_tracks: 0,
        rating_colour_mismatch_tracks: 0,
        sample_only_in_pdb: Vec::new(),
        sample_only_in_edb: Vec::new(),
        sample_metadata_mismatches: Vec::new(),
        status: DiagStatus::Pass,
        issue_labels: Vec::new(),
    }
}

#[test]
fn overview_uses_singular_wording_for_one() {
    let pdb = diag_section(
        "PDB Integrity",
        "PASS",
        &[("PDB exists", "PASS"), ("PDB parseable", "PASS")],
    );
    let edb = diag_section("Database Access", "PASS", &[("eDB", "PASS")]);
    let contents = diag_section(
        "Contents Integrity",
        "WARN",
        &[("PDB/eDB path agreement", "WARN")],
    );
    let groups = regroup_sections(
        &DISPLAY_LAYOUT,
        &[
            (&pdb, DISPLAY_PDB),
            (&edb, DISPLAY_EDB),
            (&contents, DISPLAY_EDB),
        ],
    );
    let overview = diagnostics_overview(
        &groups,
        &OverviewFacts {
            edb_file_present: true,
            pdb_tracks: 1,
            edb_tracks: 0,
            pdb_only_tracks: 1,
            edb_only_tracks: 0,
            playlists: 1,
            playlist_entries: 1,
            resolved_playlist_entries: 1,
        },
    );
    assert_eq!(overview[0].detail, "Ready: 1 playlist, 1 track.");
    assert!(overview[1].detail.starts_with(
        "1 track is missing from the database newer players read, so it may not show up there."
    ));
}

#[test]
fn parity_overview_explains_mismatches_in_plain_words() {
    use crate::models::{DiagCheck, DiagStatus};
    let check = |label: &str, status: DiagStatus| DiagCheck {
        label: label.to_string(),
        status,
        detail: String::new(),
        link: None,
    };
    let checks = vec![
        check("Overall player parity status", DiagStatus::Fail),
        check("Playlist membership parity", DiagStatus::Fail),
        check("PDB metadata completeness", DiagStatus::Pass),
        check("Rating and colour parity", DiagStatus::Warn),
        check("Indexed audio file presence", DiagStatus::Pass),
    ];
    let groups = parity_section_groups(&checks);
    let titles: Vec<&str> = groups[0]
        .sections
        .iter()
        .map(|s| s.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Playlists", "Track details", "Music files"]);
    assert!(
        groups[0]
            .sections
            .iter()
            .flat_map(|s| &s.checks)
            .all(|c| c.label != "Overall player parity status")
    );

    let mut detail = parity_detail();
    detail.only_in_pdb = 2;
    let mut rated = parity_detail();
    rated.rating_colour_mismatch_tracks = 3;
    let overview = parity_overview(
        &groups,
        &ParityTotals::from_details(&[detail, rated]),
        2,
        Some(&evaluate_strict_raw_coverage_parity(0, 0, 20)),
    );
    let rows: Vec<(&str, bool, &str)> = overview
        .iter()
        .map(|i| {
            (
                i.title.as_str(),
                matches!(i.status, Some(DiagStatus::Pass)),
                i.detail.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "Playlists",
                false,
                "2 playlist tracks are only in the database older Pioneer players read. \
                 Preview Fixes shows what can be repaired."
            ),
            (
                "Track details",
                false,
                "3 tracks have a different rating or colour on older Pioneer players. \
                 Preview Fixes shows what can be repaired."
            ),
            (
                "Music files",
                true,
                "Every playlist track's audio file is on the USB, and both databases point \
                 to the same files."
            ),
        ]
    );
}

#[test]
fn parity_issue_labels_at_full_parity_is_empty() {
    assert!(parity_issue_labels(&parity_detail()).is_empty());
}

#[test]
fn parity_issue_labels_membership_badge_needs_tracks_on_the_other_side() {
    let mut d = parity_detail();
    d.only_in_pdb = 4;
    d.edb_tracks = 6;
    assert_eq!(parity_issue_labels(&d), vec!["+PDB 4"]);

    // wholly missing on eDB -> no `+PDB` badge (it isn't a partial diff)
    let mut wholly_missing = parity_detail();
    wholly_missing.only_in_pdb = 50;
    wholly_missing.edb_tracks = 0;
    wholly_missing.playlist_id_match = false;
    wholly_missing.sort_order_match = false;
    assert_eq!(
        parity_issue_labels(&wholly_missing),
        vec!["id mismatch", "sort mismatch"]
    );
}

#[test]
fn parity_issue_labels_lists_structural_and_count_issues() {
    let mut d = parity_detail();
    d.pdb_tracks = 10;
    d.edb_tracks = 8;
    d.only_in_pdb = 2;
    d.playlist_id_match = false;
    d.sort_order_match = false;
    d.pdb_missing_core_metadata = 1;
    d.dictionary_id_issue_tracks = 2;
    assert_eq!(
        parity_issue_labels(&d),
        vec![
            "+PDB 2",
            "id mismatch",
            "sort mismatch",
            "PDB gaps 1",
            "dict issues 2"
        ]
    );
}
use crate::models::{DiagStatus, UsbTrack};
use crate::pdb_reader::{
    ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbPlaylistEntryRow, PdbPlaylistTreeRow,
    PdbTrackRow,
};
use tempfile::tempdir;

// --- count_named_history_rows ---

#[test]
fn count_named_history_rows_excludes_seed_only_export() {
    // Shaped like the real byte-level data confirmed on several unrelated
    // fresh/never-played rekordbox exports: blank template playlists
    // paired with trackless entries, no real session.
    let playlists = (0..22)
        .map(|id| PdbHistoryPlaylistRow {
            id,
            name: String::new(),
            source_table: 17,
        })
        .collect::<Vec<_>>();
    let entries = (0..17)
        .map(|id| PdbHistoryEntryRow {
            track_id: None,
            playlist_id: id,
            entry_index: 1,
            source_table: 18,
        })
        .collect::<Vec<_>>();

    assert_eq!(count_named_history_rows(&playlists, &entries), (0, 0));
}

#[test]
fn count_named_history_rows_counts_real_sessions_only() {
    let playlists = vec![
        PdbHistoryPlaylistRow {
            id: 1,
            name: "HISTORY 001".to_string(),
            source_table: 17,
        },
        PdbHistoryPlaylistRow {
            id: 2,
            name: String::new(),
            source_table: 17,
        },
    ];
    let entries = vec![
        PdbHistoryEntryRow {
            track_id: Some(101),
            playlist_id: 1,
            entry_index: 1,
            source_table: 18,
        },
        PdbHistoryEntryRow {
            track_id: None,
            playlist_id: 2,
            entry_index: 1,
            source_table: 18,
        },
    ];

    assert_eq!(count_named_history_rows(&playlists, &entries), (1, 1));
}

#[test]
fn track_identity_prefers_contents_path_for_absolute_usb_path() {
    let key = track_identity_key(
        "/tmp/workspace/USB/Contents/Artist/Track.mp3",
        "Track",
        "Artist",
        Some("2002"),
    );
    let expected = format!(
        "path:{}",
        normalize_track_path_for_identity("/Contents/Artist/Track.mp3")
    );
    assert_eq!(key, expected);
}

#[test]
fn track_identity_path_matches_relative_contents_path() {
    let abs = normalize_track_path_for_identity("/tmp/workspace/USB/Contents/Artist/Track.mp3");
    let rel = normalize_track_path_for_identity("/Contents/Artist/Track.mp3");
    assert_eq!(abs, rel);
}

#[test]
fn normalize_pdb_path_extracts_contents_segment() {
    let value = "/mnt/Somewhere/USB/Contents/Artist/Album/Track.mp3";
    assert_eq!(
        normalize_pdb_path_for_edb_lookup(value),
        "/Contents/Artist/Album/Track.mp3"
    );
}

#[test]
fn contents_integrity_treats_unindexed_audio_as_warning() {
    let section = diagnose_contents_integrity(12, 0);
    let count_match = section
        .checks
        .iter()
        .find(|check| check.label == "Count match")
        .expect("count match check");
    assert!(matches!(count_match.status, DiagStatus::Warn));
    let counts = section.counts.expect("contents counts");
    assert_eq!(counts.contents_count, 12);
    assert_eq!(counts.indexed_count, 0);
    assert_eq!(counts.mismatch_count, 12);
}

fn make_usb_track(id: &str, title: &str, artist: &str, file_path: &str) -> UsbTrack {
    UsbTrack {
        id: id.to_string(),
        local_track_id: None,
        title: title.to_string(),
        artist: artist.to_string(),
        album: None,
        year: None,
        genre: None,
        track_number: None,
        bpm: None,
        key: None,
        key_display: None,
        key_color: None,
        file_path: file_path.to_string(),
        format_ext: crate::audio_format::format_ext_from_path(file_path),
        needs_hydration: false,
        format_compat: Default::default(),
        usb_media_path: None,
        artwork_path: None,
        artwork_data_url: None,
        waveform_peaks_path: None,
        usb_analysis_path: None,
        usb_analysis_path_raw: None,
        waveform_preview: None,
        duration_ms: None,
        file_size_bytes: None,
    }
}

fn make_pdb_track(id: u32, title: &str, artist_id: u32, file_path: &str) -> PdbTrackRow {
    PdbTrackRow {
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        id,
        artist_id,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        color_id: 0,
        rating: 0,
        bitrate_kbps: None,
        track_number: id,
        tempo_x100: 12000,
        release_year: None,
        bit_depth: None,
        duration_seconds: Some(180),
        file_type: None,
        isrc: None,
        date_added: None,
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info: None,
        autoload_hotcues: None,
        title: title.to_string(),
        anlz_path: format!("/PIONEER/USBANLZ/track-{id}.DAT"),
        track_file_path: file_path.to_string(),
    }
}

fn make_single_playlist_parsed(name: &str, playlist_id: u32, sort_order: u32) -> ParsedPdb {
    let mut parsed = ParsedPdb::default();
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: playlist_id,
        parent_id: 0,
        sort_order,
        row_is_folder: false,
        name: name.to_string(),
    });
    parsed
        .tracks
        .push(make_pdb_track(1, "Track", 1, "/Contents/Artist/Track.mp3"));
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id,
    });
    parsed
}

fn edb_playlists_from_tracks(
    playlists: HashMap<String, Vec<UsbTrack>>,
) -> HashMap<String, ExportDbPlaylist> {
    playlists
        .into_iter()
        .enumerate()
        .map(|(idx, (name, tracks))| {
            (
                name,
                ExportDbPlaylist {
                    playlist_id: i64::try_from(idx + 1).expect("playlist id"),
                    sort_order: i64::try_from(idx + 1).expect("sort order"),
                    tracks,
                },
            )
        })
        .collect()
}

/// An eDB at a temp USB root with one track at `/Contents/Artist/Track.mp3`,
/// optionally in a playlist.
fn edb_with_rating_colour(
    root: &std::path::Path,
    rating: u8,
    color_id: u8,
    in_playlist: bool,
) -> rusqlite::Connection {
    let export_db_dir = root.join("PIONEER").join("rekordbox");
    fs::create_dir_all(&export_db_dir).expect("create export db dir");
    let conn =
        rusqlite::Connection::open(export_db_dir.join("exportLibrary.db")).expect("open export db");
    conn.execute_batch(&format!(
        r#"
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY, title TEXT, path TEXT,
              rating INTEGER, color_id INTEGER
            );
            CREATE TABLE playlist_content (
              playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER
            );
            INSERT INTO content VALUES (1, 'Track', '/Contents/Artist/Track.mp3', {rating}, {color_id});
            {}
            "#,
        if in_playlist {
            "INSERT INTO playlist_content VALUES (1, 1, 0);"
        } else {
            ""
        }
    ))
    .expect("seed export db");
    drop(conn);
    let mut warnings = Vec::new();
    crate::edb::open_edb_from_usb_root(root, &mut warnings).expect("reopen export db")
}

#[test]
fn rating_colour_parity_counts_playlist_tracks_whose_pdb_row_differs() {
    let edb_playlists = |in_playlist: bool| {
        edb_playlists_from_tracks(HashMap::from([(
            "Test".to_string(),
            if in_playlist {
                vec![make_usb_track(
                    "usb-track-1",
                    "Track",
                    "Artist",
                    "/Contents/Artist/Track.mp3",
                )]
            } else {
                Vec::new()
            },
        )]))
    };
    // (eDB rating, colour), (PDB rating, colour), in a playlist → (tracks, rating, colour)
    for (edb, pdb, in_playlist, expected) in [
        ((3, 2), (3, 2), true, (0, 0, 0)),
        ((3, 2), (0, 0), true, (1, 1, 1)),
        ((0, 5), (0, 0), true, (1, 0, 1)),
        ((4, 0), (0, 0), true, (1, 1, 0)),
        // Not in any playlist: not compared.
        ((3, 2), (0, 0), false, (0, 0, 0)),
    ] {
        let root = tempdir().expect("tempdir");
        let conn = edb_with_rating_colour(root.path(), edb.0, edb.1, in_playlist);
        let mut parsed = make_single_playlist_parsed("Test", 1, 1);
        parsed.tracks[0].rating = pdb.0;
        parsed.tracks[0].color_id = pdb.1;

        let result = compare_rating_colour(&conn, &parsed, &edb_playlists(in_playlist));
        assert_eq!(
            (
                result.mismatched_tracks,
                result.rating_mismatches,
                result.colour_mismatches
            ),
            expected,
            "eDB {edb:?}, PDB {pdb:?}, in playlist {in_playlist}"
        );
    }
}

#[test]
fn parity_comparison_notes_a_rating_or_colour_difference_without_failing() {
    let mut parsed = make_single_playlist_parsed("Test", 1, 1);
    for track in &mut parsed.tracks {
        track.tempo_x100 = 0;
        track.duration_seconds = None;
        track.anlz_path = String::new();
    }
    let edb_tracks = HashMap::from([(
        "Test".to_string(),
        vec![make_usb_track(
            "usb-track-1",
            "Track",
            "Artist",
            "/Contents/Artist/Track.mp3",
        )],
    )]);
    let rating_colour = RatingColourParity {
        mismatched_tracks: 1,
        rating_mismatches: 1,
        colour_mismatches: 0,
        mismatched_paths: HashSet::from(["/Contents/Artist/Track.mp3".to_string()]),
    };

    let (checks, summary_rows, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks.clone()),
        &rating_colour,
        None,
    );

    let check = checks
        .iter()
        .find(|check| check.label == "Rating and colour parity")
        .expect("rating and colour parity check");
    assert!(matches!(check.status, DiagStatus::Warn));
    assert!(
        check
            .detail
            .contains("rating differs: 1, colour differs: 0")
    );

    let summary = summary_rows
        .iter()
        .find(|row| row.label == "Rating/colour mismatches")
        .expect("rating/colour summary row");
    assert!(matches!(summary.status, DiagStatus::Warn));
    assert_eq!(summary.count, 1);
    // Counted on the playlist too, so the repair offers strict parity for it.
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].rating_colour_mismatch_tracks, 1);
    assert!(
        details[0]
            .issue_labels
            .contains(&"rating/colour mismatch 1".to_string())
    );
    // Minor: it doesn't change the report's overall status.
    let (_, _, _, overall_without) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    assert_eq!(format!("{overall:?}"), format!("{overall_without:?}"));
}

#[test]
fn parity_counts_use_unique_tracks_when_pdb_has_duplicate_entries() {
    let mut parsed = ParsedPdb {
        tracks: vec![
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(1, "A", 1, "/Contents/A.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(2, "B", 1, "/Contents/B.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(3, "C", 1, "/Contents/C.mp3")
            },
        ],
        ..Default::default()
    };
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "Test".to_string(),
    });
    // Duplicate entries inflate raw count to 12, but unique set is 3.
    for i in 0..4u32 {
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i * 3,
            track_id: 1,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i * 3 + 1,
            track_id: 2,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i * 3 + 2,
            track_id: 3,
            playlist_id: 10,
        });
    }

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert(
        "Test".to_string(),
        vec![
            make_usb_track("1", "A", "Artist", "/Contents/A.mp3"),
            make_usb_track("2", "B", "Artist", "/Contents/B.mp3"),
            make_usb_track("3", "C", "Artist", "/Contents/C.mp3"),
        ],
    );

    let (_, _, details, _) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let test = details
        .iter()
        .find(|d| d.name == "Test")
        .expect("test detail");
    assert_eq!(test.pdb_tracks, 3);
    assert_eq!(test.edb_tracks, 3);
    assert_eq!(test.matched_tracks, 3);
    assert_eq!(test.only_in_pdb, 0);
    assert_eq!(test.only_in_edb, 0);
}

#[test]
fn strict_raw_coverage_thresholds_transition_pass_warn_fail() {
    let pass = evaluate_strict_raw_coverage_parity(0, 0, 100);
    assert!(matches!(pass.status, DiagStatus::Pass));
    assert_eq!(pass.missing_count, 0);
    assert_eq!(pass.extra_count, 0);
    assert!(pass.detail.contains("all 100 indexed"));

    let warn_missing = evaluate_strict_raw_coverage_parity(3, 0, 100);
    assert!(matches!(warn_missing.status, DiagStatus::Warn));
    assert_eq!(warn_missing.missing_count, 3);

    let warn_extra = evaluate_strict_raw_coverage_parity(0, 3, 100);
    assert!(matches!(warn_extra.status, DiagStatus::Warn));
    assert_eq!(warn_extra.extra_count, 3);

    let fail_missing = evaluate_strict_raw_coverage_parity(20, 0, 100);
    assert!(matches!(fail_missing.status, DiagStatus::Fail));
    assert_eq!(fail_missing.missing_count, 20);
}

/// Writes `files` under `<root>/Contents` and returns `scan_audio_presence`
/// for PDB tracks at `pdb_paths` and eDB content rows at `edb_paths`.
fn audio_presence_for(
    root: &std::path::Path,
    files: &[&str],
    pdb_paths: &[&str],
    edb_paths: &[&str],
) -> super::AudioPresence {
    for file in files {
        let abs = root.join(file.trim_start_matches('/'));
        fs::create_dir_all(abs.parent().expect("parent")).expect("create contents dir");
        fs::write(&abs, b"fake audio").expect("write audio file");
    }
    let mut parsed = ParsedPdb::default();
    for (i, path) in pdb_paths.iter().enumerate() {
        parsed
            .tracks
            .push(make_pdb_track(i as u32 + 1, "Track", 1, path));
    }
    let edb_paths = edb_paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    super::scan_audio_presence(root, &parsed, &edb_paths)
}

#[test]
fn audio_presence_matches_alias_stripped_on_disk_filename() {
    // A PDB row can keep a trailing-space alias of the file name ("Track - .mp3"
    // for "Track -.mp3"). It is the same file: neither missing nor unindexed.
    let root = tempdir().expect("tempdir");
    let presence = audio_presence_for(
        root.path(),
        &["/Contents/Artist/Track -.mp3"],
        &["/Contents/Artist/Track - .mp3"],
        &[],
    );
    assert!(presence.missing.is_empty(), "{presence:?}");
    assert!(presence.unindexed.is_empty(), "{presence:?}");
    assert_eq!(presence.indexed_on_usb.len(), 1);
}

#[test]
fn audio_presence_keeps_numeric_suffixed_filenames_distinct() {
    // "Best of-2025.mp3" and "Best of.mp3" are two files; neither stands in
    // for the other. Indexing only one leaves the other unindexed.
    let root = tempdir().expect("tempdir");
    let files = [
        "/Contents/Artist/Best of-2025.mp3",
        "/Contents/Artist/Best of.mp3",
    ];
    let both = audio_presence_for(root.path(), &files, &files, &[]);
    assert_eq!(both.indexed_count, 2);
    assert!(
        both.missing.is_empty() && both.unindexed.is_empty(),
        "{both:?}"
    );

    let one = audio_presence_for(
        root.path(),
        &[],
        &["/Contents/Artist/Best of-2025.mp3"],
        &[],
    );
    assert_eq!(
        one.unindexed,
        vec!["/Contents/Artist/Best of.mp3".to_string()]
    );
}

#[test]
fn audio_presence_matches_case_insensitive_on_disk_folder_casing() {
    // FAT32/exFAT keep a folder's first casing after the artist tag is
    // corrected; the file is still reachable.
    let root = tempdir().expect("tempdir");
    let presence = audio_presence_for(
        root.path(),
        &["/Contents/SOME ARTIST/Track.mp3"],
        &["/Contents/Some Artist/Track.mp3"],
        &[],
    );
    assert!(presence.missing.is_empty(), "{presence:?}");
    assert!(presence.unindexed.is_empty(), "{presence:?}");
}

#[test]
fn audio_presence_counts_edb_content_paths_as_indexed() {
    // rekordbox can write a track twice ("Track.mp3" and "Track-1.mp3") with
    // the PDB row naming one copy and the eDB row the other: both are indexed.
    let root = tempdir().expect("tempdir");
    let presence = audio_presence_for(
        root.path(),
        &["/Contents/Artist/Track.mp3", "/Contents/Artist/Track-1.mp3"],
        &["/Contents/Artist/Track-1.mp3"],
        &["/Contents/Artist/Track.mp3"],
    );
    assert_eq!(presence.indexed_count, 2);
    assert!(presence.unindexed.is_empty(), "{presence:?}");
    assert!(presence.missing.is_empty(), "{presence:?}");
}

#[test]
fn audio_presence_reports_missing_from_either_database_once() {
    let root = tempdir().expect("tempdir");
    let presence = audio_presence_for(
        root.path(),
        &["/Contents/Artist/Here.mp3"],
        &["/Contents/Artist/Here.mp3", "/Contents/Artist/Gone.mp3"],
        &["/Contents/artist/gone.mp3", "/Contents/Artist/Edb Only.mp3"],
    );
    assert_eq!(
        presence.missing,
        vec![
            "/Contents/Artist/Edb Only.mp3".to_string(),
            "/Contents/Artist/Gone.mp3".to_string(),
        ]
    );
    assert_eq!(presence.indexed_count, 3);
}

#[test]
fn parity_comparison_reports_strict_raw_coverage_as_separate_check_and_row() {
    let parsed = make_single_playlist_parsed("RawCoverage", 1, 1);
    let edb_tracks = HashMap::from([(
        "RawCoverage".to_string(),
        vec![make_usb_track(
            "1",
            "Track",
            "Artist",
            "/Contents/Artist/Track.mp3",
        )],
    )]);
    let raw_coverage = evaluate_strict_raw_coverage_parity(9, 0, 100);

    let (checks, summary_rows, _details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        Some(raw_coverage),
    );

    let raw_check = checks
        .iter()
        .find(|check| check.label == "Indexed audio file presence")
        .expect("indexed audio file presence check");
    assert!(matches!(raw_check.status, DiagStatus::Fail));
    assert!(raw_check.detail.contains("missing from USB"));
    assert_eq!(raw_check.link.as_deref(), Some("event-log"));

    let raw_summary = summary_rows
        .iter()
        .find(|row| row.label == "Indexed audio file presence")
        .expect("indexed audio file presence summary row");
    assert!(matches!(raw_summary.status, DiagStatus::Fail));
    assert_eq!(raw_summary.count, 9);
    assert!(matches!(overall, DiagStatus::Fail));
}

#[test]
fn parity_report_fails_when_playlist_is_missing_from_one_database() {
    for missing_from_pdb in [true, false] {
        let name = if missing_from_pdb {
            "Missing From PDB"
        } else {
            "Missing From eDB"
        };
        let (parsed, edb_tracks) = if missing_from_pdb {
            let edb_tracks = HashMap::from([(
                name.to_string(),
                vec![make_usb_track(
                    "1",
                    "Track",
                    "Artist",
                    "/Contents/Artist/Track.mp3",
                )],
            )]);
            (ParsedPdb::default(), edb_tracks)
        } else {
            (
                make_single_playlist_parsed(name, 10, 1),
                HashMap::<String, Vec<UsbTrack>>::new(),
            )
        };

        let (checks, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &RatingColourParity::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == name)
            .expect("playlist detail");
        assert!(matches!(overall, DiagStatus::Fail), "{name}");
        assert_eq!(
            playlist.pdb_tracks,
            if missing_from_pdb { 0 } else { 1 },
            "{name}"
        );
        assert_eq!(
            playlist.edb_tracks,
            if missing_from_pdb { 1 } else { 0 },
            "{name}"
        );
        assert_eq!(
            playlist.only_in_pdb,
            if missing_from_pdb { 0 } else { 1 },
            "{name}"
        );
        assert_eq!(
            playlist.only_in_edb,
            if missing_from_pdb { 1 } else { 0 },
            "{name}"
        );
        assert!(matches!(playlist.status, DiagStatus::Fail), "{name}");
        let membership = checks
            .iter()
            .find(|c| c.label == "Playlist membership parity")
            .expect("membership parity check");
        assert!(matches!(membership.status, DiagStatus::Fail), "{name}");
        let expected_detail = if missing_from_pdb {
            "only-in-eDB=1"
        } else {
            "only-in-PDB=1"
        };
        assert!(
            membership.detail.contains(expected_detail),
            "{name}: {:?}",
            membership
        );
    }
}

#[test]
fn parity_report_fails_when_playlist_order_differs() {
    let mut parsed = ParsedPdb::default();
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "Order".to_string(),
    });
    parsed.tracks = vec![
        make_pdb_track(1, "A", 1, "/Contents/Artist/A.mp3"),
        make_pdb_track(2, "B", 1, "/Contents/Artist/B.mp3"),
    ];
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id: 10,
    });
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 1,
        track_id: 2,
        playlist_id: 10,
    });

    let mut first = make_usb_track("1", "A", "Artist", "/Contents/Artist/A.mp3");
    first.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
    first.bpm = Some(120.0);
    first.duration_ms = Some(180_000);
    first.track_number = Some(1);
    let mut second = make_usb_track("2", "B", "Artist", "/Contents/Artist/B.mp3");
    second.usb_analysis_path = Some("/PIONEER/USBANLZ/track-2.DAT".to_string());
    second.bpm = Some(120.0);
    second.duration_ms = Some(180_000);
    second.track_number = Some(2);

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert("Order".to_string(), vec![second, first]);

    let (checks, _, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let playlist = details
        .iter()
        .find(|d| d.name == "Order")
        .expect("order detail");
    assert!(matches!(overall, DiagStatus::Fail));
    assert!(playlist.order_mismatch);
    assert!(matches!(playlist.status, DiagStatus::Fail));
    let ordering = checks
        .iter()
        .find(|c| c.label == "Playlist ordering parity")
        .expect("ordering check");
    assert!(matches!(ordering.status, DiagStatus::Fail));
    assert!(ordering.detail.contains("entry-order mismatches=1"));
}

#[test]
fn parity_report_fails_when_playlist_sort_order_differs() {
    let mut parsed = ParsedPdb::default();
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "First".to_string(),
    });
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 20,
        parent_id: 0,
        sort_order: 2,
        row_is_folder: false,
        name: "Second".to_string(),
    });
    parsed.tracks = vec![
        make_pdb_track(1, "A", 1, "/Contents/Artist/A.mp3"),
        make_pdb_track(2, "B", 1, "/Contents/Artist/B.mp3"),
    ];
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id: 10,
    });
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 2,
        playlist_id: 20,
    });

    let edb_playlists = HashMap::from([
        (
            "First".to_string(),
            ExportDbPlaylist {
                playlist_id: 10,
                sort_order: 2,
                tracks: vec![make_usb_track("1", "A", "Artist", "/Contents/Artist/A.mp3")],
            },
        ),
        (
            "Second".to_string(),
            ExportDbPlaylist {
                playlist_id: 20,
                sort_order: 1,
                tracks: vec![make_usb_track("2", "B", "Artist", "/Contents/Artist/B.mp3")],
            },
        ),
    ]);

    let (checks, _, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists,
        &RatingColourParity::default(),
        None,
    );

    assert!(matches!(overall, DiagStatus::Fail));
    assert_eq!(details.len(), 2);
    assert_eq!(details[0].name, "First");
    assert!(!details[0].sort_order_match);
    assert!(!details[1].sort_order_match);
    let ordering = checks
        .iter()
        .find(|c| c.label == "Playlist ordering parity")
        .expect("ordering check");
    assert!(matches!(ordering.status, DiagStatus::Fail));
    assert!(ordering.detail.contains("entry-order mismatches=0"));
    assert!(ordering.detail.contains("playlist-sort-order mismatches=2"));
}

#[test]
fn parity_playlist_details_follow_playlist_sort_order_not_name() {
    let mut parsed = ParsedPdb::default();
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "Zeta".to_string(),
    });
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 20,
        parent_id: 0,
        sort_order: 2,
        row_is_folder: false,
        name: "Alpha".to_string(),
    });
    parsed.tracks = vec![
        make_pdb_track(1, "Track Z", 1, "/Contents/Artist/Z.mp3"),
        make_pdb_track(2, "Track A", 1, "/Contents/Artist/A.mp3"),
    ];
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id: 10,
    });
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 2,
        playlist_id: 20,
    });

    let mut edb_playlists = HashMap::<String, ExportDbPlaylist>::new();
    edb_playlists.insert(
        "Zeta".to_string(),
        ExportDbPlaylist {
            playlist_id: 10,
            sort_order: 1,
            tracks: vec![make_usb_track(
                "1",
                "Track Z",
                "Artist",
                "/Contents/Artist/Z.mp3",
            )],
        },
    );
    edb_playlists.insert(
        "Alpha".to_string(),
        ExportDbPlaylist {
            playlist_id: 20,
            sort_order: 2,
            tracks: vec![make_usb_track(
                "2",
                "Track A",
                "Artist",
                "/Contents/Artist/A.mp3",
            )],
        },
    );

    let (_checks, _rows, details, _overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists,
        &RatingColourParity::default(),
        None,
    );

    assert_eq!(details.len(), 2);
    assert_eq!(details[0].name, "Zeta");
    assert_eq!(details[1].name, "Alpha");
}

#[test]
fn parity_report_fails_when_media_or_analysis_path_differs_between_edb_and_pdb() {
    for analysis_path_case in [false, true] {
        let name = if analysis_path_case {
            "Analysis Paths"
        } else {
            "Paths"
        };
        let mut parsed = make_single_playlist_parsed(name, 10, 1);
        let mut track = if analysis_path_case {
            make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3")
        } else {
            parsed.tracks[0].track_file_path = "/Contents/Artist/DIFFERENT.mp3".to_string();
            make_usb_track("1", "Track", "Artist", "/contents/artist/track.mp3")
        };
        track.usb_analysis_path = Some(if analysis_path_case {
            "/PIONEER/USBANLZ/track-1-DIFFERENT.DAT".to_string()
        } else {
            "/PIONEER/USBANLZ/track-1.DAT".to_string()
        });
        track.bpm = Some(120.0);
        track.duration_ms = Some(180_000);
        track.track_number = Some(1);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert(name.to_string(), vec![track]);

        let (checks, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &RatingColourParity::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == name)
            .expect("paths detail");
        assert!(matches!(overall, DiagStatus::Fail), "{name}");
        assert_eq!(playlist.path_mismatch_tracks, 1, "{name}");
        assert!(matches!(playlist.status, DiagStatus::Fail), "{name}");
        if analysis_path_case {
            assert!(
                playlist
                    .sample_metadata_mismatches
                    .iter()
                    .any(|m| m.contains("analysisPath")),
                "{playlist:?}"
            );
        }
        let paths = checks
            .iter()
            .find(|c| c.label == "Media and analysis path parity")
            .expect("path parity check");
        assert!(matches!(paths.status, DiagStatus::Fail), "{name}");
    }
}

#[test]
fn parity_report_passes_when_both_have_artwork_with_different_paths() {
    let mut parsed = make_single_playlist_parsed("Artwork", 1, 1);
    parsed.tracks[0].artwork_id = 7;
    parsed
        .artworks
        .insert(7, "/PIONEER/Artwork/cover.jpg".to_string());

    let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
    track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
    track.artwork_path = Some("/PIONEER/Artwork/other.jpg".to_string());
    track.bpm = Some(120.0);
    track.duration_ms = Some(180_000);
    track.track_number = Some(1);

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert("Artwork".to_string(), vec![track]);

    let (checks, _, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let playlist = details
        .iter()
        .find(|d| d.name == "Artwork")
        .expect("artwork detail");
    assert!(matches!(overall, DiagStatus::Pass));
    // Both have artwork (different paths) — no presence mismatch
    assert_eq!(playlist.artwork_mismatch_tracks, 0);
    assert!(matches!(playlist.status, DiagStatus::Pass));
    let artwork = checks
        .iter()
        .find(|c| c.label == "Artwork presence parity")
        .expect("artwork parity check");
    assert!(matches!(artwork.status, DiagStatus::Pass));
}

#[test]
fn parity_report_warns_when_artwork_present_in_one_db_only() {
    // PDB has artwork, eDB does not
    let mut parsed = make_single_playlist_parsed("ArtworkGap", 1, 1);
    parsed.tracks[0].artwork_id = 7;
    parsed
        .artworks
        .insert(7, "/PIONEER/Artwork/cover.jpg".to_string());

    let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
    track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
    track.artwork_path = None;
    track.bpm = Some(120.0);
    track.duration_ms = Some(180_000);
    track.track_number = Some(1);

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert("ArtworkGap".to_string(), vec![track]);

    let (checks, _, details, _overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let playlist = details
        .iter()
        .find(|d| d.name == "ArtworkGap")
        .expect("artwork detail");
    assert_eq!(playlist.artwork_mismatch_tracks, 1);
    let artwork = checks
        .iter()
        .find(|c| c.label == "Artwork presence parity")
        .expect("artwork parity check");
    assert!(matches!(artwork.status, DiagStatus::Warn));
}

#[test]
fn parity_report_fails_when_required_pdb_dictionary_ids_do_not_resolve() {
    let mut parsed = make_single_playlist_parsed("Dictionary IDs", 10, 1);
    parsed.tracks[0].artist_id = 99;

    let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
    track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
    track.bpm = Some(120.0);
    track.duration_ms = Some(180_000);
    track.track_number = Some(1);

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert("Dictionary IDs".to_string(), vec![track]);

    let (checks, _, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let playlist = details
        .iter()
        .find(|d| d.name == "Dictionary IDs")
        .expect("dictionary detail");
    assert!(matches!(overall, DiagStatus::Fail));
    assert_eq!(playlist.dictionary_id_issue_tracks, 1);
    assert!(
        playlist
            .sample_metadata_mismatches
            .iter()
            .any(|m| m.contains("artistDictId"))
    );
    let dictionaries = checks
        .iter()
        .find(|c| c.label == "PDB dictionary id resolution")
        .expect("dictionary id resolution check");
    assert!(matches!(dictionaries.status, DiagStatus::Fail));
}

#[test]
fn parity_report_warns_when_edb_source_metadata_is_incomplete() {
    let mut parsed = make_single_playlist_parsed("Thin eDB", 10, 1);
    parsed.tracks[0].album_id = 7;
    parsed.tracks[0].key_id = 8;
    parsed.albums.insert(7, "Album".to_string());
    parsed.keys.insert(8, "8A".to_string());

    let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
    track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
    track.bpm = Some(120.0);
    track.duration_ms = Some(180_000);
    track.track_number = Some(1);

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert("Thin eDB".to_string(), vec![track]);

    let (checks, _, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let playlist = details
        .iter()
        .find(|d| d.name == "Thin eDB")
        .expect("thin edb detail");
    let edb_source = checks
        .iter()
        .find(|c| c.label == "eDB source completeness")
        .expect("eDB source completeness check");
    assert!(matches!(edb_source.status, DiagStatus::Warn));
    assert!(matches!(overall, DiagStatus::Fail));
    assert_eq!(playlist.edb_missing_core_metadata, 1);
    assert!(matches!(playlist.status, DiagStatus::Fail));
}

#[test]
fn parity_report_treats_empty_artist_as_unknown_artist_for_gate() {
    let mut parsed = ParsedPdb::default();
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "Unknown Artist Gate".to_string(),
    });
    parsed
        .tracks
        .push(make_pdb_track(1, "Track", 0, "/Contents/Unknown/Track.mp3"));
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id: 1,
    });

    let mut track = make_usb_track("1", "Track", "", "/Contents/Unknown/Track.mp3");
    track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
    track.bpm = Some(120.0);
    track.duration_ms = Some(180_000);
    track.track_number = Some(1);

    let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
    edb_tracks.insert("Unknown Artist Gate".to_string(), vec![track]);

    let (_, _, details, overall) = build_usb_parity_comparison(
        &parsed,
        &edb_playlists_from_tracks(edb_tracks),
        &RatingColourParity::default(),
        None,
    );
    let playlist = details
        .iter()
        .find(|d| d.name == "Unknown Artist Gate")
        .expect("unknown artist gate detail");

    assert!(matches!(overall, DiagStatus::Pass));
    assert_eq!(playlist.pdb_missing_core_metadata, 0);
    assert_eq!(playlist.edb_missing_core_metadata, 0);
    assert!(matches!(playlist.status, DiagStatus::Pass));
}

#[test]
fn playlist_resolution_uses_unique_entries_when_duplicates_exist() {
    let mut parsed = ParsedPdb {
        tracks: vec![
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(1, "A", 1, "/Contents/A.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(2, "B", 1, "/Contents/B.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(3, "C", 1, "/Contents/C.mp3")
            },
        ],
        ..Default::default()
    };
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "Test".to_string(),
    });
    for i in 0..4u32 {
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i * 3,
            track_id: 1,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i * 3 + 1,
            track_id: 2,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i * 3 + 2,
            track_id: 3,
            playlist_id: 10,
        });
    }

    let (_section, details) = diagnose_playlist_resolution_with_db(Some(&parsed), None);
    let test = details
        .iter()
        .find(|d| d.name == "Test")
        .expect("test detail");
    assert_eq!(test.total_entries, 3);
    assert_eq!(test.resolved_entries, 3);
    assert!((test.resolution_rate - 1.0).abs() < 0.0001);
}

#[test]
fn playlist_resolution_collapses_duplicate_leaf_names() {
    let mut parsed = ParsedPdb {
        tracks: vec![
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(1, "A", 1, "/Contents/A.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(2, "B", 1, "/Contents/B.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(3, "C", 1, "/Contents/C.mp3")
            },
        ],
        ..Default::default()
    };
    parsed.artists.insert(1, "Artist".to_string());
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        row_is_folder: false,
        name: "Dup".to_string(),
    });
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 11,
        parent_id: 0,
        sort_order: 2,
        row_is_folder: false,
        name: "Dup".to_string(),
    });
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 12,
        parent_id: 0,
        sort_order: 3,
        row_is_folder: false,
        name: "Dup".to_string(),
    });
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id: 10,
    });
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 1,
        track_id: 2,
        playlist_id: 11,
    });
    parsed.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 2,
        track_id: 3,
        playlist_id: 12,
    });

    let (_section, details) = diagnose_playlist_resolution_with_db(Some(&parsed), None);
    let dup_entries = details.iter().filter(|d| d.name == "Dup").count();
    assert_eq!(dup_entries, 1, "duplicate playlist names must collapse");
    let dup = details
        .iter()
        .find(|d| d.name == "Dup")
        .expect("dup detail");
    assert_eq!(dup.total_entries, 3);
    assert_eq!(dup.resolved_entries, 3);
}

#[test]
fn playlist_resolution_passes_for_fully_resolved_playlist_without_edb_match() {
    let parsed = make_single_playlist_parsed("PDB Only", 10, 1);

    let (section, details) =
        diagnose_playlist_resolution_with_db(Some(&parsed), Some(&HashMap::new()));
    let playlist = details
        .iter()
        .find(|d| d.name == "PDB Only")
        .expect("pdb only detail");
    assert!(matches!(section.status, DiagStatus::Pass));
    assert_eq!(playlist.total_entries, 1);
    assert_eq!(playlist.resolved_entries, 1);
    assert_eq!(playlist.pdb_entries, 1);
    assert_eq!(playlist.edb_entries, 0);
    assert_eq!(playlist.matched_entries, 0);
    assert!(matches!(playlist.status, DiagStatus::Pass));
    let overlap = section
        .checks
        .iter()
        .find(|c| c.label == "PDB vs eDB key overlap (informational)")
        .expect("overlap check");
    assert!(overlap.detail.contains("PDB 0.0% (0/1)"));
}

#[test]
fn playlist_resolution_summary_remains_operational_not_player_validation_wording() {
    let parsed = make_single_playlist_parsed("Operational", 10, 1);

    let (section, _details) =
        diagnose_playlist_resolution_with_db(Some(&parsed), Some(&HashMap::new()));
    let overall = section
        .checks
        .iter()
        .find(|c| c.label == "Overall resolution")
        .expect("overall resolution check");
    assert!(overall.detail.contains("entries resolve"));
    assert!(
        !overall
            .detail
            .to_ascii_lowercase()
            .contains("player parity")
    );
    assert!(!overall.detail.to_ascii_lowercase().contains("validation"));
    let interpretation = section
        .checks
        .iter()
        .find(|c| c.label == "Operational interpretation")
        .expect("operational interpretation check");
    assert!(matches!(interpretation.status, DiagStatus::Pass));
    assert!(
        interpretation
            .detail
            .contains("strict parity may still fail")
    );
    assert!(interpretation.detail.contains("operationally usable"));
}

#[test]
fn playlist_resolution_emits_stage_and_per_playlist_progress_messages() {
    let parsed = make_single_playlist_parsed("Progress", 10, 1);
    let mut progress = Vec::<String>::new();

    let (_section, _details) = diagnose_playlist_resolution_with_edb_internal(
        Some(&parsed),
        Some(&HashMap::new()),
        |done, total, message| {
            progress.push(format!("{done}/{total} {message}"));
        },
    );

    assert_eq!(progress.len(), 1);
    assert!(progress[0].contains("1/1"));
    assert!(progress[0].contains("Resolving playlist 1/1: Progress"));
}

// ── Identity-key normalization tests ─────────────────────────────

#[test]
fn normalize_track_path_for_identity_strips_backslash_and_lowercases() {
    let result =
        super::normalize_track_path_for_identity("\\PIONEER\\Contents\\Artist\\Album\\track.mp3");
    // canonicalize_playlist_name strips non-alphanumeric, lowercases
    assert!(
        result.contains("contents"),
        "expected 'contents' segment, got: {result}"
    );
    assert!(
        result.contains("artist"),
        "expected 'artist' segment, got: {result}"
    );
    assert!(!result.contains('\\'), "backslashes should be stripped");
}

#[test]
fn normalize_track_path_for_identity_extracts_contents_segment() {
    let result = super::normalize_track_path_for_identity(
        "/mnt/usb/PIONEER/Contents/Artist/Album/track.mp3",
    );
    assert!(
        result.starts_with("/contents/"),
        "should start with '/contents/', got: {result}"
    );
    assert!(
        result.contains("/artist/album/track.mp3"),
        "should contain path segments, got: {result}"
    );
}

#[test]
fn normalize_track_path_for_identity_handles_empty() {
    assert_eq!(super::normalize_track_path_for_identity(""), "");
    assert_eq!(super::normalize_track_path_for_identity("  "), "");
}

#[test]
fn normalize_pdb_path_for_edb_lookup_preserves_case() {
    let result = super::normalize_pdb_path_for_edb_lookup(
        "/mnt/usb/PIONEER/Contents/Artist/Album/Track.MP3",
    );
    assert!(
        result.starts_with("/Contents/"),
        "expected /Contents/ prefix, got: {result}"
    );
    assert!(
        result.contains("Track.MP3"),
        "should preserve original case"
    );
}

#[test]
fn normalize_pdb_path_for_edb_lookup_strips_backslash() {
    let result = super::normalize_pdb_path_for_edb_lookup("\\PIONEER\\Contents\\Artist\\track.mp3");
    assert!(!result.contains('\\'), "backslashes should be stripped");
    // The rfind looks for /contents/ (lowercased), then slices the original
    assert!(
        result.contains("Contents"),
        "expected Contents segment, got: {result}"
    );
}

#[test]
fn normalize_pdb_path_for_edb_lookup_handles_bare_contents() {
    let result = super::normalize_pdb_path_for_edb_lookup("Contents/Artist/track.mp3");
    assert_eq!(result, "/Contents/Artist/track.mp3");
}

#[test]
fn track_identity_key_matches_for_equivalent_pdb_and_edb_paths() {
    let pdb_key =
        super::track_identity_key("/Contents/Artist/Album/track.mp3", "Title", "Artist", None);
    let edb_key =
        super::track_identity_key("/Contents/Artist/Album/track.mp3", "Title", "Artist", None);
    assert_eq!(
        pdb_key, edb_key,
        "same path should produce same identity key"
    );
}

#[test]
fn track_identity_key_matches_with_backslash_prefix() {
    let pdb_key = super::track_identity_key(
        "\\PIONEER\\Contents\\Artist\\track.mp3",
        "Title",
        "Artist",
        None,
    );
    let edb_key = super::track_identity_key("/Contents/Artist/track.mp3", "Title", "Artist", None);
    assert_eq!(
        pdb_key, edb_key,
        "backslash PDB path should match forward-slash eDB path"
    );
}

#[test]
fn track_identity_key_falls_back_to_metadata() {
    let key = super::track_identity_key("", "My Title", "My Artist", None);
    assert!(
        key.starts_with("meta:"),
        "empty path should fall back to meta key, got: {key}"
    );
}

#[test]
fn track_identity_key_preserves_unicode_path_segments() {
    let pdb_key = super::track_identity_key(
        "/Contents/星空レコード/Album/track.flac",
        "Sample Ascent",
        "星空レコード",
        None,
    );
    let edb_key = super::track_identity_key(
        "/Contents/星空レコード/Album/track.flac",
        "Sample Ascent",
        "星空レコード",
        None,
    );
    assert_eq!(pdb_key, edb_key);
    assert!(pdb_key.contains("星空レコード"));
}

#[test]
fn track_identity_key_falls_back_to_id() {
    let key = super::track_identity_key("", "", "", Some("track-123"));
    assert_eq!(key, "id:track-123");
}

#[test]
fn track_identity_key_returns_unknown_when_all_empty() {
    let key = super::track_identity_key("", "", "", None);
    assert_eq!(key, "unknown");
}

/// A `.DAT` at `<root>/<anlz_path>` whose `PPTH` names `track_path` (none
/// when empty).
fn write_dat_naming(root: &std::path::Path, anlz_path: &str, track_path: &str) {
    let dat = root.join(anlz_path.trim_start_matches('/'));
    fs::create_dir_all(dat.parent().expect("parent")).expect("create bundle dir");
    let waveform = crate::service::anlz::WaveformData::empty();
    fs::write(
        &dat,
        crate::service::anlz::build_anlz_dat_file(&waveform, track_path, None, 1_000, None, &[]),
    )
    .expect("write DAT");
}

#[test]
fn analysis_ownership_picks_the_owner_of_a_shared_bundle() {
    const APP: u32 = 99;
    let shared = "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT";
    let track = |id: u32, path: &str, anlz: &str, master_db_id: u32| PdbTrackRow {
        anlz_path: anlz.to_string(),
        master_db_id: Some(master_db_id),
        ..make_pdb_track(id, "Track", 1, path)
    };
    let parsed = ParsedPdb {
        tracks: vec![
            track(1, "/Contents/A/First.mp3", shared, 7),
            track(2, "/Contents/B/Second.mp3", shared, APP),
        ],
        ..ParsedPdb::default()
    };
    let owner = |root: &std::path::Path, app: Option<u32>| {
        let found = super::scan_analysis_ownership(root, &parsed, app, true);
        assert_eq!(found.shared.len(), 1, "{found:?}");
        assert_eq!(found.shared[0].track_ids, vec![1, 2]);
        found.shared[0].owner
    };

    let root = tempdir().expect("tempdir");
    // No PPTH: the track this app exported wrote the file; without knowing
    // the app's id, nobody owns it.
    write_dat_naming(root.path(), shared, "");
    assert_eq!(owner(root.path(), Some(APP)), Some(2));
    assert_eq!(owner(root.path(), None), None);
    // A PPTH decides, whoever exported.
    write_dat_naming(root.path(), shared, "/Contents/A/First.mp3");
    assert_eq!(owner(root.path(), Some(APP)), Some(1));
    // Database-only: shared, owner unknown.
    let db_only = super::scan_analysis_ownership(root.path(), &parsed, Some(APP), false);
    assert_eq!(db_only.shared.len(), 1);
}

#[test]
fn analysis_ownership_finds_unmarked_and_foreign_single_bundles() {
    let root = tempdir().expect("tempdir");
    let own = "/PIONEER/USBANLZ/P002/00000002/ANLZ0000.DAT";
    let bare = "/PIONEER/USBANLZ/P003/00000003/ANLZ0000.DAT";
    let foreign = "/PIONEER/USBANLZ/P004/00000004/ANLZ0000.DAT";
    write_dat_naming(root.path(), own, "/Contents/A/Own.mp3");
    write_dat_naming(root.path(), bare, "");
    write_dat_naming(root.path(), foreign, "/Contents/A/Own.mp3");
    let track = |id: u32, path: &str, anlz: &str| PdbTrackRow {
        anlz_path: anlz.to_string(),
        ..make_pdb_track(id, "Track", 1, path)
    };
    let parsed = ParsedPdb {
        tracks: vec![
            track(1, "/Contents/A/Own.mp3", own),
            track(2, "/Contents/A/Bare.mp3", bare),
            // Its bundle names track 1: it has none of its own.
            track(3, "/Contents/A/Foreign.mp3", foreign),
        ],
        ..ParsedPdb::default()
    };
    let found = super::scan_analysis_ownership(root.path(), &parsed, None, true);
    assert_eq!(
        found
            .unmarked
            .iter()
            .map(|b| b.track_id)
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(found.shared.len(), 1, "{found:?}");
    assert_eq!(found.shared[0].track_ids, vec![3]);
    assert_eq!(found.shared[0].owner, None);
    assert_eq!(found.non_owner_track_ids(), HashSet::from([3]));
}

/// rekordbox transaction bookkeeping (here: ACTV track pages inside a
/// multi-page chain and a playlist_tree footer shape the writer doesn't use)
/// passes, with the counts named in the detail.
#[test]
fn pdb_integrity_reports_rekordbox_bookkeeping_as_pass() {
    use crate::models::DiagStatus;

    const PAGE: usize = 4096;
    let data_page = |index: u32, table_type: u32, flags: u8, nrs: u8, next: u32| {
        let mut p = vec![0u8; PAGE];
        p[4..8].copy_from_slice(&index.to_le_bytes());
        p[8..12].copy_from_slice(&table_type.to_le_bytes());
        p[0x0c..0x10].copy_from_slice(&next.to_le_bytes());
        p[0x18] = nrs;
        p[0x1b] = flags;
        p[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        p
    };
    let mut header = vec![0u8; PAGE];
    header[4..8].copy_from_slice(&(PAGE as u32).to_le_bytes());
    let mut playlist_tree = data_page(3, 7, 0x24, 6, 0);
    playlist_tree[0x20..0x22].copy_from_slice(&1u16.to_le_bytes());
    playlist_tree[0x22..0x24].copy_from_slice(&1u16.to_le_bytes());
    let pages = [
        header,
        data_page(1, 0, 0x34, 4, 2),
        data_page(2, 0, 0x34, 4, 0),
        playlist_tree,
    ];

    let td = tempfile::tempdir().expect("tempdir");
    let pdb_path = td.path().join("export.pdb");
    fs::write(&pdb_path, pages.concat()).expect("write test pdb");

    let mut warnings = Vec::new();
    let (section, _) = super::pdb_integrity::diagnose_pdb_integrity(&pdb_path, &mut warnings);
    let check = section
        .checks
        .iter()
        .find(|c| c.label == "PDB structural integrity")
        .unwrap_or_else(|| panic!("structural integrity check: {:#?}", section.checks));
    assert!(
        matches!(check.status, DiagStatus::Pass),
        "rekordbox bookkeeping must not warn: {check:?}"
    );
    assert!(
        check.detail.contains("ACTV in multi-page chain") && check.detail.contains("playlist_tree"),
        "detail should name the bookkeeping: {}",
        check.detail
    );
}

#[test]
fn edb_analysis_refs_pass_when_the_edb_has_no_tracks() {
    let edb_check = |content_rows: Option<usize>| {
        diagnose_analysis_integrity(None, None, content_rows)
            .checks
            .into_iter()
            .find(|c| c.label == "eDB analysis refs")
            .expect("eDB analysis refs check")
    };
    let empty = edb_check(Some(0));
    assert!(matches!(empty.status, crate::models::DiagStatus::Pass));
    assert_eq!(empty.detail, "No tracks in the eDB");
    // Tracks but no playlist data, or an eDB that couldn't be read, still warn.
    for rows in [Some(3), None] {
        assert!(matches!(
            edb_check(rows).status,
            crate::models::DiagStatus::Warn
        ));
    }
}
