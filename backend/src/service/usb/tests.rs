use super::display_files::UsbDisplaySource;
use super::history::{
    apply_history_dates_from_track_date_created, build_history_track_date_index,
    filter_named_history_playlists, merge_history_sessions, normalize_date_created,
    resolve_history_sessions, select_history_rows,
};
use super::identity::UsbAudioFormats;
use super::playlists::cleanup_empty_dirs_recursive;
use super::staging::UsbWriteSession;
use super::tracks::{
    build_usb_track_index, hydrate_usb_page, hydrate_usb_track_in_place, paginate_usb_tracks,
};
use super::{
    SLOW_USB_STAGE_MS, now, push_usb_stage_timing_with_threshold, slow_stage_threshold_ms,
};
use crate::models::{UsbHistory, UsbTrack};
use crate::pdb_reader::{PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbTrackRow};
use crate::service::build_track_match_fingerprint;
use crate::service::export_helpers::{EdbPlaylistWrite, PlaylistPlacement, write_edb_playlists_in};
use std::collections::HashMap;
use std::path::Path;

#[test]
fn hydrate_usb_page_stops_reading_files_once_superseded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let art = dir.path().join("art.jpg");
    std::fs::write(&art, [0xFF, 0xD8, 0xFF, 0xD9]).expect("write art");
    let tracks: Vec<UsbTrack> = (1..=3)
        .map(|i| {
            let mut track = make_track(&i.to_string(), &format!("/USB/Contents/{i}.mp3"));
            track.artwork_path = Some(art.to_string_lossy().into_owned());
            track.artwork_data_url = None;
            track
        })
        .collect();
    let req = crate::models::FetchUsbTracksRequest::default();
    let files = UsbDisplaySource::direct(dir.path());

    // A newer load starts after the first row: later rows skip the reads
    // but keep their cheap fields.
    let mut page = paginate_usb_tracks(
        "fetch_usb_playlist_tracks",
        "pl",
        tracks.clone(),
        &req,
        Vec::new(),
    )
    .expect("page");
    let checks = std::cell::Cell::new(0);
    hydrate_usb_page(
        "fetch_usb_playlist_tracks",
        &mut page.items,
        crate::service::key_notation::KeyNotation::Classic,
        &files,
        || {
            checks.set(checks.get() + 1);
            checks.get() > 1
        },
    );
    assert!(page.items[0].artwork_data_url.is_some());
    assert!(page.items[1..].iter().all(|t| t.artwork_data_url.is_none()));
    assert_eq!(page.items.len(), 3);

    let mut page = paginate_usb_tracks("fetch_usb_playlist_tracks", "pl", tracks, &req, Vec::new())
        .expect("page");
    hydrate_usb_page(
        "fetch_usb_playlist_tracks",
        &mut page.items,
        crate::service::key_notation::KeyNotation::Classic,
        &files,
        || false,
    );
    assert!(page.items.iter().all(|t| t.artwork_data_url.is_some()));
}

#[test]
fn hydrated_usb_rows_follow_the_sticks_usb_audio_formats() {
    let dir = tempfile::tempdir().expect("tempdir");
    let items: Vec<(String, bool)> = crate::audio_format::UsbAudioFormat::DEFAULT_ORDER
        .into_iter()
        .map(|format| (format.id().to_string(), format.id() != "flac"))
        .collect();
    crate::service::usb::identity::write_usb_audio_formats(
        dir.path(),
        &UsbAudioFormats::from_request(&items).expect("formats"),
    )
    .expect("write formats");
    let files = UsbDisplaySource::direct(dir.path());
    let mut page = vec![
        make_track("1", "/USB/Contents/a.flac"),
        make_track("2", "/USB/Contents/b.mp3"),
    ];
    hydrate_usb_page(
        "fetch_usb_playlist_tracks",
        &mut page,
        crate::service::key_notation::KeyNotation::Classic,
        &files,
        || false,
    );
    assert_eq!(
        page[0].format_compat.warning.as_deref(),
        Some("FLAC is not allowed on this USB (USB audio formats)")
    );
    assert_eq!(page[1].format_compat.warning, None);
}

fn make_track(id: &str, file_path: &str) -> UsbTrack {
    UsbTrack {
        id: id.to_string(),
        local_track_id: None,
        title: "T".to_string(),
        artist: "A".to_string(),
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

fn make_pdb_track(id: u32, date_added: Option<&str>) -> PdbTrackRow {
    PdbTrackRow {
        header_flags: 0,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        id,
        artist_id: 0,
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
        track_number: 0,
        tempo_x100: 0,
        release_year: None,
        bit_depth: None,
        duration_seconds: None,
        file_type: None,
        isrc: None,
        date_added: date_added.map(str::to_string),
        analyze_date: None,
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info: None,
        autoload_hotcues: None,
        title: String::new(),
        anlz_path: String::new(),
        track_file_path: String::new(),
    }
}

#[test]
fn hydrate_usb_track_in_place_sets_needs_hydration_from_missing_display_data() {
    // Everything present -> no further hydration needed.
    let mut complete = make_track("1", "/USB/Contents/a.mp3");
    complete.waveform_preview = Some(vec![1, 2, 3]);
    complete.artwork_path = Some("/USB/art/a.jpg".to_string());
    complete.bpm = Some(128.0);
    complete.key = Some("8A".to_string());
    hydrate_usb_track_in_place(
        &mut complete,
        crate::service::key_notation::KeyNotation::Classic,
        &UsbAudioFormats::default(),
        None,
    );
    assert!(!complete.needs_hydration);

    // Any missing piece -> needs hydration.
    for tweak in [
        |t: &mut UsbTrack| t.bpm = None,
        |t: &mut UsbTrack| t.key = None,
        |t: &mut UsbTrack| t.waveform_preview = None,
        |t: &mut UsbTrack| {
            t.artwork_path = None;
            t.artwork_data_url = None;
        },
    ] {
        let mut track = make_track("2", "/USB/Contents/b.mp3");
        track.waveform_preview = Some(vec![1, 2, 3]);
        track.artwork_path = Some("/USB/art/b.jpg".to_string());
        track.bpm = Some(128.0);
        track.key = Some("8A".to_string());
        tweak(&mut track);
        hydrate_usb_track_in_place(
            &mut track,
            crate::service::key_notation::KeyNotation::Classic,
            &UsbAudioFormats::default(),
            None,
        );
        assert!(
            track.needs_hydration,
            "expected needs_hydration after tweak"
        );
    }
}

#[test]
fn normalize_date_created_accepts_iso_datetime_prefix() {
    assert_eq!(
        normalize_date_created("2024-10-15T09:10:11Z").map(|d| d.format("%Y-%m-%d").to_string()),
        Some("2024-10-15".to_string())
    );
}

#[test]
fn build_history_track_date_index_uses_pdb_track_ids() {
    let index = build_history_track_date_index(&[
        make_pdb_track(1674, Some("2020-09-18")),
        make_pdb_track(917, Some("2025-12-19")),
        make_pdb_track(42, Some("")),
        make_pdb_track(43, None),
    ]);

    assert_eq!(index.get(&1674).map(String::as_str), Some("2020-09-18"));
    assert_eq!(index.get(&917).map(String::as_str), Some("2025-12-19"));
    assert!(!index.contains_key(&42));
    assert!(!index.contains_key(&43));
}

#[test]
fn apply_history_dates_carries_latest_track_date_forward_in_playlist_order() {
    let mut histories = vec![
        UsbHistory {
            id: "usb-h-1".to_string(),
            name: "HISTORY 001".to_string(),
            created_at: Some("2021-09".to_string()),
            can_export_tracklist: true,
            track_count: 2,
            tracks: vec![
                make_track("10", "/USB/Contents/A/1.mp3"),
                make_track("20", "/USB/Contents/A/2.mp3"),
            ],
            total_duration_ms: 0,
            duration_unknown_count: 0,
            source: "PDB".to_string(),
            in_both: false,
            heading_text: String::new(),
        },
        UsbHistory {
            id: "usb-h-2".to_string(),
            name: "HISTORY 002".to_string(),
            created_at: Some("2021-09".to_string()),
            can_export_tracklist: true,
            track_count: 1,
            tracks: vec![make_track("30", "/USB/Contents/A/3.mp3")],
            total_duration_ms: 0,
            duration_unknown_count: 0,
            source: "PDB".to_string(),
            in_both: false,
            heading_text: String::new(),
        },
        UsbHistory {
            id: "usb-h-3".to_string(),
            name: "HISTORY 003".to_string(),
            created_at: Some("2021-09".to_string()),
            can_export_tracklist: true,
            track_count: 1,
            tracks: vec![make_track("40", "/USB/Contents/A/4.mp3")],
            total_duration_ms: 0,
            duration_unknown_count: 0,
            source: "PDB".to_string(),
            in_both: false,
            heading_text: String::new(),
        },
    ];
    let date_created_by_track_id = HashMap::from([
        (10u32, "2024-10-15".to_string()),
        (20u32, "2024-10-14".to_string()),
        (30u32, "2024-10-10".to_string()),
        (40u32, "2024-10-20".to_string()),
    ]);

    apply_history_dates_from_track_date_created(
        &mut histories,
        &date_created_by_track_id,
        &HashMap::new(),
    );

    assert_eq!(histories[0].created_at.as_deref(), Some("2024-10-15"));
    assert_eq!(histories[1].created_at.as_deref(), Some("2024-10-15"));
    assert_eq!(histories[2].created_at.as_deref(), Some("2024-10-20"));
}

#[test]
fn apply_history_dates_from_track_date_created_preserves_existing_valid_date() {
    let mut histories = vec![UsbHistory {
        id: "usb-h-1".to_string(),
        name: "HISTORY 001".to_string(),
        created_at: Some("2026-04-03".to_string()),
        can_export_tracklist: true,
        track_count: 1,
        tracks: vec![make_track("10", "/USB/Contents/A/1.mp3")],
        total_duration_ms: 0,
        duration_unknown_count: 0,
        source: "PDB".to_string(),
        in_both: false,
        heading_text: String::new(),
    }];
    let date_created_by_track_id = HashMap::from([(10u32, "2024-10-15".to_string())]);

    apply_history_dates_from_track_date_created(
        &mut histories,
        &date_created_by_track_id,
        &HashMap::new(),
    );

    assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-03"));
}

#[test]
fn apply_history_dates_from_track_date_created_carries_existing_valid_date_forward() {
    let mut histories = vec![
        UsbHistory {
            id: "usb-h-1".to_string(),
            name: "HISTORY 001".to_string(),
            created_at: Some("2026-04-03".to_string()),
            can_export_tracklist: true,
            track_count: 1,
            tracks: vec![make_track("10", "/USB/Contents/A/1.mp3")],
            total_duration_ms: 0,
            duration_unknown_count: 0,
            source: "PDB".to_string(),
            in_both: false,
            heading_text: String::new(),
        },
        UsbHistory {
            id: "usb-h-2".to_string(),
            name: "HISTORY 002".to_string(),
            created_at: Some("2021-09".to_string()),
            can_export_tracklist: true,
            track_count: 1,
            tracks: vec![make_track("20", "/USB/Contents/A/2.mp3")],
            total_duration_ms: 0,
            duration_unknown_count: 0,
            source: "PDB".to_string(),
            in_both: false,
            heading_text: String::new(),
        },
    ];
    let date_created_by_track_id = HashMap::from([(20u32, "2024-10-15".to_string())]);

    apply_history_dates_from_track_date_created(
        &mut histories,
        &date_created_by_track_id,
        &HashMap::new(),
    );

    assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-03"));
    assert_eq!(histories[1].created_at.as_deref(), Some("2026-04-03"));
}

#[test]
fn select_history_rows_prefers_more_populated_t11_t12_family() {
    let playlists = vec![
        PdbHistoryPlaylistRow {
            id: 1,
            name: "HISTORY 001".to_string(),
            source_table: 17,
        },
        PdbHistoryPlaylistRow {
            id: 2,
            name: "HISTORY 002".to_string(),
            source_table: 11,
        },
        PdbHistoryPlaylistRow {
            id: 3,
            name: "".to_string(),
            source_table: 11,
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
            track_id: Some(201),
            playlist_id: 2,
            entry_index: 1,
            source_table: 12,
        },
        PdbHistoryEntryRow {
            track_id: Some(202),
            playlist_id: 3,
            entry_index: 2,
            source_table: 12,
        },
    ];

    let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
    assert_eq!(history_playlists.len(), 2);
    assert!(history_playlists.iter().all(|row| row.source_table == 11));
    assert_eq!(history_entries.len(), 2);
    assert!(history_entries.iter().all(|row| row.source_table == 12));
}

fn history_rows(
    sessions: &[(u32, &str, &[u32])],
) -> (Vec<PdbHistoryPlaylistRow>, Vec<PdbHistoryEntryRow>) {
    let playlists = sessions
        .iter()
        .map(|(id, name, _)| PdbHistoryPlaylistRow {
            id: *id,
            name: name.to_string(),
            source_table: 11,
        })
        .collect();
    let entries = sessions
        .iter()
        .flat_map(|(id, _, tracks)| {
            tracks
                .iter()
                .enumerate()
                .map(|(index, track_id)| PdbHistoryEntryRow {
                    track_id: Some(*track_id),
                    playlist_id: *id,
                    entry_index: index as u32 + 1,
                    source_table: 12,
                })
        })
        .collect();
    (playlists, entries)
}

fn track_index(ids: &[(u32, &str)]) -> HashMap<u32, UsbTrack> {
    ids.iter()
        .map(|(id, path)| (*id, make_track(&id.to_string(), path)))
        .collect()
}

#[test]
fn history_sessions_merge_both_databases_by_number() {
    // PDB has 1 and 2; the eDB has 2 and 3.
    let (pdb_playlists, pdb_entries) =
        history_rows(&[(1, "HISTORY 001", &[10]), (2, "HISTORY 002", &[20, 21])]);
    let (edb_playlists, edb_entries) =
        history_rows(&[(2, "HISTORY 002", &[500]), (3, "HISTORY 003", &[501])]);
    let pdb_tracks = track_index(&[(10, "/p/10.mp3"), (20, "/p/20.mp3"), (21, "/p/21.mp3")]);
    // eDB 500 is the PDB's 20 under the eDB's own ID; 20 means another
    // track there.
    let edb_tracks = track_index(&[
        (20, "/e/other.mp3"),
        (500, "/e/500.mp3"),
        (501, "/e/501.mp3"),
    ]);

    let merged = merge_history_sessions(
        resolve_history_sessions(&pdb_playlists, &pdb_entries, &pdb_tracks),
        resolve_history_sessions(&edb_playlists, &edb_entries, &edb_tracks),
    );

    let summary = merged
        .iter()
        .map(|(number, (session, source, in_both))| {
            let paths = session
                .tracks
                .iter()
                .map(|t| t.file_path.as_str())
                .collect::<Vec<_>>();
            (*number, *source, *in_both, paths)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            (1, "PDB", false, vec!["/p/10.mp3"]),
            (2, "PDB", true, vec!["/p/20.mp3", "/p/21.mp3"]),
            (3, "eDB", false, vec!["/e/501.mp3"]),
        ]
    );
}

#[test]
fn history_session_empty_in_pdb_takes_the_edb_tracks() {
    let (pdb_playlists, pdb_entries) = history_rows(&[(5, "HISTORY 005", &[])]);
    let (edb_playlists, edb_entries) = history_rows(&[(5, "HISTORY 005", &[7, 8])]);
    let edb_tracks = track_index(&[(7, "/e/7.mp3")]);

    let merged = merge_history_sessions(
        resolve_history_sessions(&pdb_playlists, &pdb_entries, &HashMap::new()),
        resolve_history_sessions(&edb_playlists, &edb_entries, &edb_tracks),
    );

    let (session, source, in_both) = &merged[&5];
    assert_eq!(*source, "eDB");
    assert!(*in_both);
    // A track missing from its own database is unknown, never borrowed
    // from the other one.
    assert_eq!(
        session
            .tracks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["T", "Unknown Track #8"]
    );
}

#[test]
fn select_history_rows_falls_back_to_t17_t18_when_t11_t12_absent() {
    let playlists = vec![PdbHistoryPlaylistRow {
        id: 7,
        name: "HISTORY 007".to_string(),
        source_table: 17,
    }];
    let entries = vec![
        PdbHistoryEntryRow {
            track_id: Some(101),
            playlist_id: 7,
            entry_index: 1,
            source_table: 18,
        },
        PdbHistoryEntryRow {
            track_id: Some(102),
            playlist_id: 7,
            entry_index: 2,
            source_table: 18,
        },
    ];

    let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
    assert_eq!(history_playlists.len(), 1);
    assert_eq!(history_playlists[0].source_table, 17);
    assert_eq!(history_entries.len(), 2);
    assert!(history_entries.iter().all(|row| row.source_table == 18));
}

#[test]
fn select_history_rows_prefers_runtime_t11_t12_even_when_t17_t18_have_seed_volume() {
    let mut playlists = Vec::new();
    for id in 1..=27u32 {
        playlists.push(PdbHistoryPlaylistRow {
            id,
            name: format!("HISTORY {id:03}"),
            source_table: 17,
        });
    }
    playlists.push(PdbHistoryPlaylistRow {
        id: 100,
        name: "HISTORY 100".to_string(),
        source_table: 11,
    });

    let mut entries = Vec::new();
    for id in 1..=27u32 {
        entries.push(PdbHistoryEntryRow {
            track_id: Some(1000 + id),
            playlist_id: id,
            entry_index: 1,
            source_table: 18,
        });
    }
    entries.push(PdbHistoryEntryRow {
        track_id: Some(2000),
        playlist_id: 100,
        entry_index: 1,
        source_table: 12,
    });

    let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
    assert_eq!(history_playlists.len(), 1);
    assert_eq!(history_playlists[0].source_table, 11);
    assert_eq!(history_entries.len(), 1);
    assert_eq!(history_entries[0].source_table, 12);
}

#[test]
fn select_history_rows_uses_t11_t12_when_t17_t18_are_empty() {
    let playlists = vec![PdbHistoryPlaylistRow {
        id: 7,
        name: "HISTORY 007".to_string(),
        source_table: 11,
    }];
    let entries = vec![PdbHistoryEntryRow {
        track_id: Some(101),
        playlist_id: 7,
        entry_index: 1,
        source_table: 12,
    }];

    let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
    assert_eq!(history_playlists.len(), 1);
    assert_eq!(history_playlists[0].source_table, 11);
    assert_eq!(history_entries.len(), 1);
    assert_eq!(history_entries[0].source_table, 12);
}

// --- filter_named_history_playlists ---

#[test]
fn filter_named_history_playlists_drops_blank_seed_rows() {
    // Shaped like the real byte-level data confirmed on several unrelated
    // fresh/never-played rekordbox exports: a block of blank-name
    // template rows shipped alongside one real session.
    let playlists = vec![
        PdbHistoryPlaylistRow {
            id: 65537,
            name: String::new(),
            source_table: 17,
        },
        PdbHistoryPlaylistRow {
            id: 393221,
            name: "\u{1}".to_string(),
            source_table: 17,
        },
        PdbHistoryPlaylistRow {
            id: 10,
            name: "HISTORY 001".to_string(),
            source_table: 17,
        },
    ];

    let filtered = filter_named_history_playlists(playlists);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, 10);
}

#[test]
fn filter_named_history_playlists_drops_all_seed_only_export() {
    // A fresh/never-played export: every t17 row is a blank template
    // slot, no real session recorded yet.
    let playlists = (0..22)
        .map(|id| PdbHistoryPlaylistRow {
            id,
            name: String::new(),
            source_table: 17,
        })
        .collect::<Vec<_>>();

    assert!(filter_named_history_playlists(playlists).is_empty());
}

#[test]
fn filter_named_history_playlists_keeps_all_real_sessions() {
    let playlists = vec![
        PdbHistoryPlaylistRow {
            id: 1,
            name: "HISTORY 001".to_string(),
            source_table: 17,
        },
        PdbHistoryPlaylistRow {
            id: 2,
            name: "HISTORY 002".to_string(),
            source_table: 17,
        },
    ];

    assert_eq!(filter_named_history_playlists(playlists).len(), 2);
}

#[test]
fn slow_stage_threshold_ms_equals_baseline_at_zero_items() {
    assert_eq!(slow_stage_threshold_ms(0), SLOW_USB_STAGE_MS);
}

#[test]
fn slow_stage_threshold_ms_scales_linearly_above_baseline() {
    let baseline = slow_stage_threshold_ms(0);
    let at_1000 = slow_stage_threshold_ms(1_000);
    let at_2000 = slow_stage_threshold_ms(2_000);
    assert!(at_1000 > baseline, "threshold should grow with item count");
    // Linear: doubling the item count should double the allowance added on top of baseline.
    assert_eq!((at_2000 - baseline), (at_1000 - baseline) * 2);
}

// --- build_usb_track_index ---

#[allow(clippy::too_many_arguments)]
fn make_indexed_pdb_track(
    id: u32,
    title: &str,
    artist_id: u32,
    album_id: u32,
    key_id: u32,
    artwork_id: u32,
    track_number: u32,
    tempo_x100: u32,
    track_file_path: &str,
    anlz_path: &str,
) -> PdbTrackRow {
    PdbTrackRow {
        header_flags: 0,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: Some(4096),
        master_content_id: None,
        master_db_id: None,
        id,
        artist_id,
        album_id,
        artwork_id,
        key_id,
        genre_id: 0,
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        color_id: 0,
        rating: 0,
        bitrate_kbps: None,
        track_number,
        tempo_x100,
        release_year: None,
        bit_depth: None,
        duration_seconds: Some(200),
        file_type: None,
        isrc: None,
        date_added: None,
        analyze_date: None,
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info: None,
        autoload_hotcues: None,
        title: title.to_string(),
        anlz_path: anlz_path.to_string(),
        track_file_path: track_file_path.to_string(),
    }
}

#[test]
fn build_usb_track_index_resolves_metadata_and_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let usb_root = tmp.path();
    let parsed = crate::pdb_reader::ParsedPdb {
        tracks: vec![make_indexed_pdb_track(
            1,
            "Song One",
            10,
            20,
            30,
            40,
            5,
            12500,
            "/Contents/Artist/Song One.mp3",
            "/PIONEER/USBANLZ/P001/ANLZ0000.DAT",
        )],
        artists: HashMap::from([(10u32, "DJ Test".to_string())]),
        albums: HashMap::from([(20u32, "Test Album".to_string())]),
        keys: HashMap::from([(30u32, "8A".to_string())]),
        artworks: HashMap::from([(40u32, "/PIONEER/Artwork/a.jpg".to_string())]),
        ..Default::default()
    };

    let index = build_usb_track_index(&parsed, usb_root);
    let track = index.get(&1).expect("track present");
    assert_eq!(track.title, "Song One");
    assert_eq!(track.artist, "DJ Test");
    assert_eq!(track.album.as_deref(), Some("Test Album"));
    assert_eq!(track.key.as_deref(), Some("8A"));
    assert_eq!(track.track_number, Some(5));
    assert_eq!(track.bpm, Some(125.0));
    assert!(track.artwork_path.is_some());
    assert!(track.file_path.ends_with("Song One.mp3"));
    assert_eq!(track.format_ext.as_deref(), Some("mp3"));
    assert!(track.usb_analysis_path.is_some());
    assert_eq!(
        track.usb_analysis_path_raw.as_deref(),
        Some("/PIONEER/USBANLZ/P001/ANLZ0000.DAT")
    );
    assert_eq!(track.duration_ms, Some(200_000));
}

#[test]
fn build_usb_track_index_falls_back_when_title_artist_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let usb_root = tmp.path();
    let parsed = crate::pdb_reader::ParsedPdb {
        tracks: vec![make_indexed_pdb_track(
            2,
            "",
            999,
            0,
            0,
            0,
            0,
            0,
            "relative/song.mp3",
            "",
        )],
        ..Default::default()
    };

    let index = build_usb_track_index(&parsed, usb_root);
    let track = index.get(&2).expect("track present");
    assert_eq!(track.title, "Unknown Title");
    assert_eq!(track.artist, "Unknown Artist");
    assert_eq!(track.album, None);
    assert_eq!(track.track_number, None);
    assert_eq!(track.bpm, None);
    assert!(track.usb_analysis_path.is_none());
}

// --- cleanup_empty_dirs_recursive ---

#[test]
fn cleanup_empty_dirs_recursive_removes_empty_but_keeps_non_empty() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("empty/nested")).unwrap();
    std::fs::create_dir_all(root.join("kept")).unwrap();
    std::fs::write(root.join("kept/file.txt"), b"data").unwrap();

    cleanup_empty_dirs_recursive(root);

    assert!(!root.join("empty").exists());
    assert!(root.join("kept").exists());
    assert!(root.join("kept/file.txt").exists());
}

// --- push_usb_stage_timing / push_usb_stage_timing_with_threshold ---

#[test]
fn push_usb_stage_timing_with_threshold_flags_slow_stage_at_zero_threshold() {
    let mut started = std::time::Instant::now();
    assert!(
        push_usb_stage_timing_with_threshold("test-stage", &mut started, 0),
        "elapsed time always exceeds a zero threshold"
    );
}

#[test]
fn push_usb_stage_timing_with_threshold_not_slow_below_threshold() {
    let mut started = std::time::Instant::now();
    assert!(!push_usb_stage_timing_with_threshold(
        "test-stage",
        &mut started,
        60_000
    ));
}

// --- BackendService USB integration tests ---

fn test_service() -> (tempfile::TempDir, crate::service::BackendService) {
    let dir = tempfile::tempdir().expect("service data dir");
    let service = crate::service::BackendService::new(dir.path()).expect("backend service");
    (dir, service)
}

fn test_usb_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let usb_root = td.path().join("USB_TEST");
    std::fs::create_dir_all(&usb_root).expect("create usb root");
    crate::service::initialize_usb(&usb_root).expect("initialize usb");
    (td, usb_root)
}

#[test]
fn selecting_a_usb_its_diagnostics_and_playlists_share_one_pdb_parse_and_edb_read() {
    let (_data, service) = test_service();
    let (_usb, usb_root) = test_usb_root();
    let root_text = usb_root.to_string_lossy().to_string();
    let cached = |service: &crate::service::BackendService| {
        service
            .parsed_pdb_cache
            .lock()
            .expect("cache lock")
            .as_ref()
            .map(|(_, _, staged)| std::sync::Arc::clone(staged))
            .expect("a cached parse")
    };
    let cached_edb = |service: &crate::service::BackendService| {
        service
            .edb_library_cache
            .lock()
            .expect("cache lock")
            .as_ref()
            .map(|(_, _, library)| std::sync::Arc::clone(library))
            .expect("a cached eDB read")
    };

    let selected = service
        .select_usb(crate::models::SelectUsbRequest {
            path: root_text.clone(),
        })
        .expect("select usb");
    assert!(selected.usb.valid);
    // The playlist state reads the names from the parse.
    service.playlists_state().expect("playlists state");
    let first = cached(&service);
    let first_edb = cached_edb(&service);

    service
        .run_usb_diagnostics(crate::models::RunUsbDiagnosticsRequest {
            usb_root: Some(root_text.clone()),
        })
        .expect("diagnostics");
    service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(root_text.clone()),
        })
        .expect("fetch playlists");
    assert!(std::sync::Arc::ptr_eq(&first, &cached(&service)));
    assert!(std::sync::Arc::ptr_eq(&first_edb, &cached_edb(&service)));

    // A write to the stick drops it: the next reader parses the new file.
    service.invalidate_usb_parse_cache();
    service
        .run_usb_diagnostics(crate::models::RunUsbDiagnosticsRequest {
            usb_root: Some(root_text),
        })
        .expect("diagnostics again");
    assert!(!std::sync::Arc::ptr_eq(&first, &cached(&service)));
    assert!(!std::sync::Arc::ptr_eq(&first_edb, &cached_edb(&service)));
}

fn make_export_track(id: &str, title: &str, filename: &str) -> crate::edb::ExportTrackData {
    crate::edb::ExportTrackData {
        cues: Vec::new(),
        id: id.to_string(),
        title: title.to_string(),
        artist: "Artist".to_string(),
        album: Some("Album".to_string()),
        track_number: Some(1),
        bpm: Some(120.0),
        key: Some("8A".to_string()),
        file_path: format!("/source/{filename}"),
        file_name: filename.to_string(),
        file_timestamp_at: None,
        file_size_bytes: Some(4_000_000),
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
        disc_number: None,
        subtitle: None,
        comment: None,
        title_for_search: None,
        kuvo_delivery_comment: None,
        dj_play_count: None,
        rating: None,
        color_id: None,
        artist_id_lyricist: None,
        artist_id_original_artist: None,
        artist_id_remixer: None,
        artist_id_composer: None,
        genre_id: None,
        genre: None,
        label_id: None,
        isrc: None,
        release_year: None,
        release_date: None,
        recorded_date: None,
        file_type: None,
        artwork_path: None,
        waveform_peaks_path: None,
        duration_ms: Some(200_000),
        first_beat_ms: None,
        beatgrid_from_rekordbox: false,
        position: 0,
        transcode_format: None,
    }
}

fn make_export_manifest(
    playlist_id: &str,
    playlist_name: &str,
    usb_root: &Path,
    tracks: &[(&str, &str, &str)],
) -> crate::service::export_helpers::ExportManifest {
    crate::service::export_helpers::ExportManifest {
        generated_at: "2024-01-01".to_string(),
        playlist_id: playlist_id.to_string(),
        playlist_name: playlist_name.to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: tracks.len(),
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: tracks
            .iter()
            .enumerate()
            .map(
                |(i, (id, title, filename))| crate::edb::ExportManifestTrack {
                    date_added: None,
                    date_created: None,
                    label: None,
                    remixer: None,
                    composer: None,
                    from_library: true,
                    pdb_track_id: None,
                    edb_content_id: None,
                    first_beat_ms: None,
                    cues: Vec::new(),
                    id: id.to_string(),
                    master_db_id: None,
                    master_content_id: None,
                    content_link: None,
                    position: i + 1,
                    track_number: Some(1),
                    title: title.to_string(),
                    artist: "Artist".to_string(),
                    album: Some("Album".to_string()),
                    bpm: Some(120.0),
                    key: Some("8A".to_string()),
                    source_path: format!("/source/{filename}"),
                    exported_path: format!("/Contents/Artist/Album/{filename}"),
                    file_timestamp_at: None,
                    file_size_bytes: Some(4_000_000),
                    sample_rate_hz: None,
                    bit_depth: None,
                    bitrate_kbps: None,
                    disc_number: None,
                    subtitle: None,
                    comment: None,
                    title_for_search: None,
                    kuvo_delivery_comment: None,
                    dj_play_count: None,
                    rating: None,
                    color_id: None,
                    artist_id_lyricist: None,
                    artist_id_original_artist: None,
                    artist_id_remixer: None,
                    artist_id_composer: None,
                    genre_id: None,
                    genre: None,
                    label_id: None,
                    isrc: None,
                    release_year: None,
                    release_date: None,
                    recorded_date: None,
                    file_type: None,
                    owns_exported_media: true,
                    owns_artwork: false,
                    owns_waveform: false,
                    artwork_path: None,
                    waveform_path: None,
                    duration_ms: Some(200_000),
                },
            )
            .collect(),
    }
}

fn seeded_playlist_usb_with_tracks(
    tracks: &[(&str, &str, &str)],
) -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, usb_root) = test_usb_root();
    let playlist = crate::edb::ExportPlaylistData {
        id: "pl-1".to_string(),
        name: "My Playlist".to_string(),
        tracks: tracks
            .iter()
            .map(|(id, title, filename)| make_export_track(id, title, filename))
            .collect(),
    };
    let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, tracks);
    crate::service::export_helpers::write_pdb(&usb_root, &playlist, &manifest, true, None, None)
        .expect("write pdb");
    (dir, usb_root)
}

fn seeded_playlist_usb() -> (tempfile::TempDir, std::path::PathBuf) {
    seeded_playlist_usb_with_tracks(&[("t1", "Song A", "a.mp3")])
}

#[test]
fn validate_usb_root_empty_path_is_invalid() {
    let (_dir, service) = test_service();
    let result = service
        .validate_usb_root(crate::models::ValidateUsbRootRequest {
            path: "   ".to_string(),
        })
        .expect("validate");
    assert!(!result.valid);
    assert!(!result.has_write_access);
    assert!(result.normalized_root.is_none());
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.code == "usb.import.path-empty")
    );
}

#[test]
fn validate_usb_root_initialized_usb_is_valid_and_registers_device() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let result = service
        .validate_usb_root(crate::models::ValidateUsbRootRequest {
            path: usb_root.to_string_lossy().to_string(),
        })
        .expect("validate");
    assert!(result.valid);
    assert!(result.has_vendor_root);
    assert!(result.has_contents);
    assert!(result.has_pdb);
    assert!(result.has_edb);
    assert!(result.normalized_root.is_some());

    let devices = service.list_usb_devices().expect("list devices");
    assert_eq!(devices.items.len(), 1);
    assert!(devices.items[0].mounted);
}

#[test]
fn validate_usb_root_stages_edb_and_verifies_it_for_the_connect() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));

    let result = service
        .validate_usb_root(crate::models::ValidateUsbRootRequest {
            path: usb_root.to_string_lossy().to_string(),
        })
        .expect("validate");
    assert!(result.valid);

    // Mutate the eDB directly on the "USB" after connecting. A later
    // passive `stage_edb` call must trust the connect-time staging
    // `validate_usb_root` already did and not pick up the change --
    // proving staging happened once, at connect, rather than being
    // deferred to (and re-checked by) the next read.
    let edb_path = crate::service::usb::vendor_compat::vendor_edb_path(&usb_root);
    let original = std::fs::read(&edb_path).expect("read seeded edb");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&edb_path, b"changed after connect").expect("mutate edb");

    let staged = crate::service::usb::staging::stage_edb(&usb_root).expect("stage edb");
    assert_eq!(
        std::fs::read(&staged).expect("read staged edb"),
        original,
        "validate_usb_root should already have staged/verified the eDB, so a later \
         passive stage call must not pick up an out-of-band USB change"
    );
}

#[test]
fn prune_usb_device_soft_deletes_and_removes_from_listing() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    service
        .validate_usb_root(crate::models::ValidateUsbRootRequest {
            path: usb_root.to_string_lossy().to_string(),
        })
        .expect("validate");
    let devices = service.list_usb_devices().expect("list devices");
    let id = devices.items[0].id.clone();

    let pruned = service
        .prune_usb_device(crate::models::PruneUsbDeviceRequest { id: id.clone() })
        .expect("prune");
    assert!(pruned.pruned);
    assert!(pruned.devices.is_empty(), "the list comes back without it");

    let pruned_again = service
        .prune_usb_device(crate::models::PruneUsbDeviceRequest { id })
        .expect("prune again");
    assert!(
        !pruned_again.pruned,
        "already-pruned device should be a no-op"
    );
}

#[test]
fn set_usb_device_name_round_trips_through_marker_and_label() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let path = usb_root.to_string_lossy().to_string();

    let before = service
        .get_usb_device_name(crate::models::GetUsbDeviceNameRequest {
            usb_root: path.clone(),
        })
        .expect("get before naming");
    assert_eq!(before.name, None);

    let saved = service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: path.clone(),
            name: "  Club Stick  ".to_string(),
        })
        .expect("set name");
    assert_eq!(saved.name, "Club Stick");
    assert!(saved.updated);
    assert_eq!(saved.status_text, "Drive renamed to Club Stick.");

    let same = service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: path.clone(),
            name: "Club Stick".to_string(),
        })
        .expect("save the same name");
    assert!(!same.updated);
    assert_eq!(same.status_text, "Drive name unchanged.");

    let after = service
        .get_usb_device_name(crate::models::GetUsbDeviceNameRequest { usb_root: path })
        .expect("get after naming");
    assert_eq!(after.name, Some("Club Stick".to_string()));

    let devices = service.list_usb_devices().expect("list devices");
    assert_eq!(devices.items[0].label.as_deref(), Some("Club Stick"));
}

#[test]
fn get_usb_device_name_handles_an_already_known_but_unnamed_device() {
    // Regression test: unlike a device get_usb_device_name has never
    // seen before (no usb_devices row at all, so the SQL row-mapping
    // closure never runs), a device already tracked via validate_usb_root
    // (the normal "connect a USB" flow) has a real row with label = NULL.
    // Reading that NULL column as a plain `String` instead of
    // `Option<String>` errors ("Invalid column type Null"), which is
    // exactly the real-world case a brand-new device never hits.
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let path = usb_root.to_string_lossy().to_string();

    service
        .validate_usb_root(crate::models::ValidateUsbRootRequest { path: path.clone() })
        .expect("validate creates the usb_devices row");
    let devices = service.list_usb_devices().expect("list devices");
    assert_eq!(devices.items.len(), 1);
    assert_eq!(
        devices.items[0].label, None,
        "freshly seen device must be unnamed"
    );

    let result = service
        .get_usb_device_name(crate::models::GetUsbDeviceNameRequest { usb_root: path })
        .expect("must not error on a NULL label column");
    assert_eq!(result.name, None);
}

#[test]
fn set_usb_device_name_rejects_empty_name() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let err = service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: usb_root.to_string_lossy().to_string(),
            name: "   ".to_string(),
        })
        .expect_err("empty name must be rejected");
    assert!(
        matches!(err, crate::error::BackendError::Validation(message) if message.render_english() == "Enter a name for this drive.")
    );
}

#[test]
fn set_usb_device_name_rejects_name_already_used_by_another_drive() {
    let (_dir, service) = test_service();
    let (_usb_dir_a, usb_root_a) = test_usb_root();
    let (_usb_dir_b, usb_root_b) = test_usb_root();

    service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: usb_root_a.to_string_lossy().to_string(),
            name: "Club Stick".to_string(),
        })
        .expect("name first drive");

    let err = service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: usb_root_b.to_string_lossy().to_string(),
            name: "Club Stick".to_string(),
        })
        .expect_err("duplicate name must be rejected");
    assert!(matches!(err, crate::error::BackendError::Validation(_)));

    // Renaming the same drive to the name it already holds is not a collision.
    service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: usb_root_a.to_string_lossy().to_string(),
            name: "Club Stick".to_string(),
        })
        .expect("re-setting own name must succeed");
}

fn get_name(
    service: &crate::service::BackendService,
    usb_root: &Path,
) -> crate::models::GetUsbDeviceNameData {
    service
        .get_usb_device_name(crate::models::GetUsbDeviceNameRequest {
            usb_root: usb_root.to_string_lossy().to_string(),
        })
        .expect("get name")
}

fn set_name(
    service: &crate::service::BackendService,
    usb_root: &Path,
    name: &str,
) -> crate::error::BackendResult<()> {
    service
        .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
            usb_root: usb_root.to_string_lossy().to_string(),
            name: name.to_string(),
        })
        .map(|_| ())
}

/// Another stick with the same volume label: same mount path, no marker.
fn swap_in_unnamed_drive_at(usb_root: &Path) {
    std::fs::remove_dir_all(crate::service::usb::identity::app_marker_dir(usb_root))
        .expect("remove marker");
}

#[test]
fn second_drive_at_the_same_mount_path_is_unnamed_and_cannot_take_the_first_ones_name() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    set_name(&service, &usb_root, "Club Stick").expect("name first drive");

    swap_in_unnamed_drive_at(&usb_root);
    let second = get_name(&service, &usb_root);
    assert_eq!(second.name, None, "must not inherit the first drive's name");
    assert!(
        crate::service::usb::identity::read_drive_name(&usb_root).is_none(),
        "reading the name must not write one onto the drive"
    );

    let err =
        set_name(&service, &usb_root, "Club Stick").expect_err("the first drive's name is taken");
    assert!(matches!(err, crate::error::BackendError::Validation(_)));
    set_name(&service, &usb_root, "Booth Stick").expect("a new name is fine");
    assert_eq!(
        get_name(&service, &usb_root).name.as_deref(),
        Some("Booth Stick")
    );
}

#[test]
fn set_usb_device_name_rejects_a_name_that_only_differs_in_case_or_punctuation() {
    let (_dir, service) = test_service();
    let (_usb_dir_a, usb_root_a) = test_usb_root();
    let (_usb_dir_b, usb_root_b) = test_usb_root();
    set_name(&service, &usb_root_a, "Club Stick").expect("name first drive");

    let err =
        set_name(&service, &usb_root_b, "club-stick").expect_err("same cache key as Club Stick");
    assert!(matches!(err, crate::error::BackendError::Validation(_)));

    set_name(&service, &usb_root_a, "CLUB STICK").expect("own name in another case");
    assert_eq!(
        get_name(&service, &usb_root_a).name.as_deref(),
        Some("CLUB STICK")
    );
}

#[test]
fn renaming_a_drive_moves_its_cache_and_frees_the_old_name() {
    let (_dir, service) = test_service();
    let cache = tempfile::tempdir().expect("cache dir");
    let _cache_guard =
        crate::service::usb::staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
    let (_usb_dir_a, usb_root_a) = test_usb_root();
    let (_usb_dir_b, usb_root_b) = test_usb_root();
    set_name(&service, &usb_root_a, "Club Stick").expect("name drive");
    let archived = cache.path().join("club-stick/backups/export_old.pdb");
    std::fs::create_dir_all(archived.parent().unwrap()).unwrap();
    std::fs::write(&archived, b"pdb").unwrap();

    set_name(&service, &usb_root_a, "Booth Stick").expect("rename");

    assert!(!cache.path().join("club-stick").exists());
    assert_eq!(
        std::fs::read(cache.path().join("booth-stick/backups/export_old.pdb")).unwrap(),
        b"pdb"
    );
    set_name(&service, &usb_root_b, "Club Stick").expect("the old name is free again");
    let err = set_name(&service, &usb_root_b, "Booth Stick").expect_err("the new name is taken");
    assert!(matches!(err, crate::error::BackendError::Validation(_)));
}

#[test]
fn a_name_read_from_the_drive_counts_as_taken() {
    // Named on another computer: only the marker knows the name.
    let (_dir, service) = test_service();
    let (_usb_dir_a, usb_root_a) = test_usb_root();
    let (_usb_dir_b, usb_root_b) = test_usb_root();
    crate::service::usb::identity::write_drive_name(&usb_root_a, "Club Stick").unwrap();

    assert_eq!(
        get_name(&service, &usb_root_a).name.as_deref(),
        Some("Club Stick")
    );
    let err = set_name(&service, &usb_root_b, "Club Stick").expect_err("name registered when read");
    assert!(matches!(err, crate::error::BackendError::Validation(_)));
}

/// What this computer knows about the drive named `name` no longer
/// matches the drive at hand: as if it were a copy on a different-size
/// stick.
fn remember_other_size_for(service: &crate::service::BackendService, name: &str) {
    let conn = service.db.connect().expect("connect");
    crate::service::usb::identity::set_registered_drive(&conn, name, None, Some(1)).expect("size");
}

#[test]
fn a_drive_with_another_drives_name_and_a_different_size_counts_as_unnamed() {
    let (_dir, service) = test_service();
    let cache = tempfile::tempdir().expect("cache dir");
    let _cache_guard =
        crate::service::usb::staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
    let (_usb_dir, usb_root) = test_usb_root();
    set_name(&service, &usb_root, "Club Stick").expect("name drive");
    assert!(crate::service::usb::staging::backups_dir_for(&usb_root).is_some());
    remember_other_size_for(&service, "Club Stick");

    let data = get_name(&service, &usb_root);
    assert_eq!(data.name, None);
    assert_eq!(data.name_taken_by.as_deref(), Some("Club Stick"));
    assert!(
        crate::service::usb::staging::backups_dir_for(&usb_root).is_none(),
        "no backups are archived under the other drive's name"
    );
    let devices_conn = service.db.connect().expect("connect");
    assert_eq!(
        crate::service::usb_drive_name(&devices_conn, &usb_root),
        None
    );
    let devices = service.list_usb_devices().expect("list devices");
    assert_eq!(
        devices.items[0].label.as_deref(),
        Some("Club Stick"),
        "a carried name is not written to the path's label"
    );
}

#[test]
fn a_drive_with_another_drives_name_gets_its_own_without_touching_the_others() {
    let (_dir, service) = test_service();
    let cache = tempfile::tempdir().expect("cache dir");
    let _cache_guard =
        crate::service::usb::staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
    let (_usb_dir, usb_root) = test_usb_root();
    set_name(&service, &usb_root, "Club Stick").expect("name drive");
    std::fs::create_dir_all(cache.path().join("club-stick/backups")).unwrap();
    remember_other_size_for(&service, "Club Stick");
    get_name(&service, &usb_root);

    set_name(&service, &usb_root, "Booth Stick").expect("own name");

    assert!(
        cache.path().join("club-stick/backups").is_dir(),
        "the other drive keeps its cache"
    );
    assert_eq!(
        get_name(&service, &usb_root).name.as_deref(),
        Some("Booth Stick")
    );
    let (_other_dir, other_root) = test_usb_root();
    let err =
        set_name(&service, &other_root, "Club Stick").expect_err("the other drive keeps its name");
    assert!(matches!(err, crate::error::BackendError::Validation(_)));
}

#[test]
fn a_drive_with_another_drives_name_can_take_it_back() {
    // Not a copy after all (say, a resized partition): naming it the
    // same again makes it the drive that name is checked against.
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    set_name(&service, &usb_root, "Club Stick").expect("name drive");
    remember_other_size_for(&service, "Club Stick");
    assert_eq!(get_name(&service, &usb_root).name, None);

    set_name(&service, &usb_root, "Club Stick").expect("take the name back");

    let data = get_name(&service, &usb_root);
    assert_eq!(data.name.as_deref(), Some("Club Stick"));
    assert_eq!(data.name_taken_by, None);
}

#[test]
fn renaming_a_drive_renames_it_on_its_playlists_last_export_lines() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    set_name(&service, &usb_root, "Club Stick").expect("name drive");
    let conn = service.db.connect().expect("db");
    conn.execute_batch(
        "INSERT INTO playlists (id, name, source, last_exported_at, last_exported_usb_root, last_exported_usb_name, created_at, updated_at)
           VALUES ('p1', 'Set', 'local', 'x', '/run/media/dj/STICK', 'Club Stick', '1', '1'),
                  ('p2', 'Other', 'local', 'x', '/run/media/dj/STICK', 'Booth Stick', '2', '2');",
    )
    .expect("seed");

    set_name(&service, &usb_root, "Main Stick").expect("rename drive");

    let names: Vec<(String, Option<String>)> = conn
        .prepare("SELECT id, last_exported_usb_name FROM playlists ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        names,
        vec![
            ("p1".to_string(), Some("Main Stick".to_string())),
            ("p2".to_string(), Some("Booth Stick".to_string())),
        ],
        "only exports to the renamed drive follow its name"
    );
}

#[test]
fn the_mount_path_label_follows_a_name_changed_outside_the_app() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    set_name(&service, &usb_root, "Club Stick").expect("name drive");
    crate::service::usb::identity::write_drive_name(&usb_root, "Booth Stick").unwrap();

    assert_eq!(
        get_name(&service, &usb_root).name.as_deref(),
        Some("Booth Stick")
    );

    let devices = service.list_usb_devices().expect("list devices");
    assert_eq!(devices.items[0].label.as_deref(), Some("Booth Stick"));
}

#[test]
fn pruning_a_drive_frees_its_name() {
    let (_dir, service) = test_service();
    let (_usb_dir_a, usb_root_a) = test_usb_root();
    let (_usb_dir_b, usb_root_b) = test_usb_root();
    set_name(&service, &usb_root_a, "Club Stick").expect("name drive");
    let id = service.list_usb_devices().expect("list").items[0]
        .id
        .clone();

    service
        .prune_usb_device(crate::models::PruneUsbDeviceRequest { id })
        .expect("prune");

    set_name(&service, &usb_root_b, "Club Stick").expect("name is free after pruning");
}

#[test]
fn fetch_usb_playlists_returns_playlist_with_track() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = seeded_playlist_usb();

    let result = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("fetch usb playlists");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].name, "My Playlist");
    assert_eq!(result.items[0].track_count, 1);
    assert_eq!(result.items[0].tracks[0].title, "Song A");
    assert_eq!(result.stats.indexed_tracks, 1);
}

#[test]
fn fetch_usb_playlists_computes_total_duration_server_side() {
    let (_dir, usb_root) =
        seeded_playlist_usb_with_tracks(&[("t1", "Song A", "a.mp3"), ("t2", "Song B", "b.mp3")]);
    let (_dir2, service) = test_service();

    let result = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("fetch usb playlists");

    assert_eq!(result.items.len(), 1);
    // make_export_track() gives every seeded track a fixed 200_000ms
    // duration, so two tracks should sum to 400_000ms with both known.
    assert_eq!(result.items[0].total_duration_ms, 400_000);
    assert_eq!(result.items[0].duration_unknown_count, 0);
}

#[test]
fn fetch_usb_playlist_tracks_paginates_filters_sorts_and_hydrates_the_page() {
    let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
        ("t1", "Charlie", "c.mp3"),
        ("t2", "Alpha", "a.flac"),
        ("t3", "Bravo", "b.wav"),
    ]);
    let (_dir2, service) = test_service();
    let root = usb_root.to_string_lossy().to_string();
    let list = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(root.clone()),
        })
        .expect("fetch playlists");
    let playlist_id = list.items[0].id.clone();

    let req = |over: crate::models::FetchUsbTracksRequest| crate::models::FetchUsbTracksRequest {
        usb_root: Some(root.clone()),
        id: playlist_id.clone(),
        ..over
    };

    // Page 1 of 2 in PDB entry order; whole-list totals.
    let page1 = service
        .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
            limit: 2,
            ..Default::default()
        }))
        .expect("page 1");
    assert_eq!(page1.total, 3);
    assert_eq!(page1.items.len(), 2);
    assert!(page1.has_more);
    assert_eq!(page1.total_duration_ms, 600_000);
    // The page is hydrated: format is populated (from Stage A) ...
    assert_eq!(page1.items[0].format_ext.as_deref(), Some("mp3"));
    // ... and browsing creates no local rows, so none has a local id yet.
    assert!(page1.items.iter().all(|t| t.local_track_id.is_none()));

    let page2 = service
        .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
            limit: 2,
            cursor: page1.next_cursor.clone(),
            ..Default::default()
        }))
        .expect("page 2");
    assert_eq!(page2.items.len(), 1);
    assert!(!page2.has_more);

    // An offset jumps straight to a position, with no cursor needed.
    let jumped = service
        .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
            limit: 1,
            offset: Some(1),
            ..Default::default()
        }))
        .expect("offset page");
    assert_eq!(jumped.items.len(), 1);
    assert_eq!(jumped.items[0].id, page1.items[1].id);
    assert!(jumped.has_more);

    // Server-side title sort spans the whole playlist.
    let sorted = service
        .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
            sort_by: Some("title".to_string()),
            ..Default::default()
        }))
        .expect("sorted");
    assert_eq!(
        sorted
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Alpha", "Bravo", "Charlie"]
    );

    // A cursor is rejected once the sort changes.
    let err = service
        .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
            cursor: page1.next_cursor,
            sort_by: Some("title".to_string()),
            ..Default::default()
        }))
        .unwrap_err();
    assert!(matches!(err, crate::error::BackendError::Internal(_)));

    // Query filters server-side.
    let filtered = service
        .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
            query: "alph".to_string(),
            ..Default::default()
        }))
        .expect("filtered");
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].title, "Alpha");
}

#[test]
fn fetch_usb_playlist_tracks_is_read_only_and_finds_tracks_added_since() {
    let (_dir, usb_root) =
        seeded_playlist_usb_with_tracks(&[("t1", "Charlie", "c.mp3"), ("t2", "Alpha", "a.flac")]);
    let (_dir2, service) = test_service();
    let root = usb_root.to_string_lossy().to_string();
    let list = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(root.clone()),
        })
        .expect("fetch playlists");
    let fetch = || {
        service
            .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                usb_root: Some(root.clone()),
                id: list.items[0].id.clone(),
                ..Default::default()
            })
            .expect("fetch tracks")
    };
    let count = |table: &str| -> i64 {
        let conn = service.db.connect().expect("connect");
        conn.query_row(&format!("SELECT COUNT(1) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count")
    };

    let page = fetch();
    assert_eq!(count("tracks"), 0, "browsing must not create track rows");
    assert_eq!(
        count("track_usb_links"),
        0,
        "browsing must not create USB links"
    );
    assert!(page.items.iter().all(|t| t.local_track_id.is_none()));

    // Adding one to a playlist creates its local row; the next browse
    // finds it through the USB link, still without writing.
    let added = &page.items[0];
    let local_id = service
        .materialize_usb_add_candidate(
            &crate::models::AddTrackCandidate {
                title: added.title.clone(),
                artist: added.artist.clone(),
                album: added.album.clone(),
                file_path: Some(added.file_path.clone()),
                usb_root: Some(root.clone()),
                ..Default::default()
            },
            None,
        )
        .expect("materialize")
        .expect("local id");
    let tracks_after_add = count("tracks");

    let page = fetch();
    assert_eq!(count("tracks"), tracks_after_add);
    let found = page
        .items
        .iter()
        .find(|t| t.file_path == added.file_path)
        .expect("row");
    assert_eq!(found.local_track_id.as_deref(), Some(local_id.as_str()));
    assert!(
        page.items
            .iter()
            .filter(|t| t.file_path != added.file_path)
            .all(|t| t.local_track_id.is_none())
    );
}

/// A row matched to a local track shows that track's own artwork:
/// local ids are resolved before the page is hydrated.
#[test]
fn adding_a_usb_track_keeps_what_the_stick_knows_about_it() {
    let (_dir, usb_root) = test_usb_root();
    let tracks = [("t1", "Song", "song.mp3")];
    let playlist = crate::edb::ExportPlaylistData {
        id: "pl-1".to_string(),
        name: "My Playlist".to_string(),
        tracks: vec![make_export_track("t1", "Song", "song.mp3")],
    };
    let mut manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, &tracks);
    manifest.tracks[0].release_year = Some(2021);
    manifest.tracks[0].genre_id = Some(1);
    manifest.tracks[0].genre = Some("Techno".to_string());
    manifest.tracks[0].owns_artwork = true;
    manifest.tracks[0].artwork_path = Some("/PIONEER/Artwork/00001/b0000001.jpg".to_string());
    crate::service::export_helpers::write_pdb(&usb_root, &playlist, &manifest, true, None, None)
        .expect("write pdb");

    let (_dir2, service) = test_service();
    let root = usb_root.to_string_lossy().to_string();
    let list = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(root.clone()),
        })
        .expect("fetch playlists");
    let row = service
        .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
            usb_root: Some(root.clone()),
            id: list.items[0].id.clone(),
            ..Default::default()
        })
        .expect("fetch tracks")
        .items
        .remove(0);
    assert!(row.duration_ms.is_some(), "the stick must know the length");
    assert_eq!(row.year, Some(2021));

    // As the row's "+" sends it: the candidate has none of these fields.
    let add = || {
        service
            .materialize_usb_add_candidate(
                &crate::models::AddTrackCandidate {
                    title: row.title.clone(),
                    artist: row.artist.clone(),
                    album: row.album.clone(),
                    bpm: row.bpm,
                    file_path: Some(row.file_path.clone()),
                    usb_root: Some(root.clone()),
                    usb_analysis_path: Some("/stick/PIONEER/USBANLZ/ANLZ0000.DAT".to_string()),
                    ..Default::default()
                },
                None,
            )
            .expect("materialize")
            .expect("local id")
    };
    let assert_complete = |local_id: String| {
        let local = service
            .get_tracks_by_ids_with_previews(crate::models::GetTracksByIdsRequest {
                track_ids: vec![local_id],
            })
            .expect("local row")
            .items
            .remove(0);
        assert_eq!(local.duration_ms, row.duration_ms);
        assert_eq!(local.year, row.year);
        assert_eq!(local.genre, row.genre);
        assert_eq!(local.artwork_path, row.artwork_path);
        assert!(
            local.analysis_ready,
            "BPM, waveform and length are all there"
        );
    };
    let local_id = add();
    assert_complete(local_id.clone());

    // A row an earlier version added without them is completed by adding
    // the track again.
    service
        .db
        .connect()
        .expect("connect")
        .execute(
            "UPDATE tracks SET duration_ms = NULL, release_year = NULL, genre = NULL, artwork_path = NULL WHERE id = ?1",
            rusqlite::params![local_id],
        )
        .expect("strip row");
    assert_eq!(add(), local_id);
    assert_complete(local_id);
}

#[test]
fn fetch_usb_playlist_tracks_takes_matched_tracks_artwork_from_the_library() {
    let (_dir, usb_root) =
        seeded_playlist_usb_with_tracks(&[("t1", "Charlie", "c.mp3"), ("t2", "Alpha", "a.flac")]);
    let (data_dir, service) = test_service();
    let root = usb_root.to_string_lossy().to_string();
    let list = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(root.clone()),
        })
        .expect("fetch playlists");
    let fetch = || {
        service
            .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                usb_root: Some(root.clone()),
                id: list.items[0].id.clone(),
                ..Default::default()
            })
            .expect("fetch tracks")
    };

    let page = fetch();
    assert!(page.items.iter().all(|t| t.artwork_data_url.is_none()));

    let added = &page.items[0];
    let local_id = service
        .materialize_usb_add_candidate(
            &crate::models::AddTrackCandidate {
                title: added.title.clone(),
                artist: added.artist.clone(),
                album: added.album.clone(),
                file_path: Some(added.file_path.clone()),
                usb_root: Some(root.clone()),
                ..Default::default()
            },
            None,
        )
        .expect("materialize")
        .expect("local id");
    let local_art = data_dir.path().join("local-art.png");
    std::fs::write(&local_art, b"\x89PNG local").expect("write local art");
    service
        .db
        .connect()
        .expect("connect")
        .execute(
            "UPDATE tracks SET artwork_path = ?1 WHERE id = ?2",
            rusqlite::params![local_art.to_string_lossy(), local_id],
        )
        .expect("set local artwork");

    let page = fetch();
    let matched = page
        .items
        .iter()
        .find(|t| t.file_path == added.file_path)
        .expect("matched row");
    assert_eq!(matched.local_track_id.as_deref(), Some(local_id.as_str()));
    assert!(
        matched
            .artwork_data_url
            .as_deref()
            .is_some_and(|url| url.starts_with("data:image/png;base64,"))
    );
    assert!(
        page.items
            .iter()
            .filter(|t| t.file_path != added.file_path)
            .all(|t| t.artwork_data_url.is_none())
    );
}

/// Regression: overlapping page loads (the previous playlist's still
/// running when the next is selected) failed with "database is locked".
#[test]
fn fetch_usb_playlist_tracks_survives_concurrent_calls() {
    let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
        ("t1", "Charlie", "c.mp3"),
        ("t2", "Alpha", "a.flac"),
        ("t3", "Bravo", "b.wav"),
    ]);
    let (_dir2, service) = test_service();
    let root = usb_root.to_string_lossy().to_string();
    let list = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(root.clone()),
        })
        .expect("fetch playlists");
    let playlist_id = list.items[0].id.clone();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let service = service.clone();
        let root = root.clone();
        let playlist_id = playlist_id.clone();
        handles.push(std::thread::spawn(move || {
            let mut errs = Vec::new();
            for _ in 0..50 {
                if let Err(e) =
                    service.fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                        usb_root: Some(root.clone()),
                        id: playlist_id.clone(),
                        ..Default::default()
                    })
                {
                    errs.push(e.english());
                }
            }
            errs
        }));
    }
    let errs: Vec<String> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    assert!(
        errs.is_empty(),
        "{} errors, first: {:?}",
        errs.len(),
        errs.first()
    );
}

#[test]
fn fetch_usb_playlist_tracks_rejects_unknown_playlist_id() {
    let (_dir, usb_root) = seeded_playlist_usb();
    let (_dir2, service) = test_service();
    let err = service
        .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            id: "usb-pl-does-not-exist".to_string(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(err, crate::error::BackendError::NotFound(_)));
}

// --- USB order edit (`usb::order_edit`) ---

fn root_text(usb_root: &Path) -> Option<String> {
    Some(usb_root.to_string_lossy().to_string())
}

/// A stick with one playlist, "My Playlist", in both databases, holding
/// a.mp3, b.mp3 and c.mp3 in that order.
fn order_edit_usb() -> (tempfile::TempDir, std::path::PathBuf) {
    let tracks = [
        ("t1", "Song A", "a.mp3"),
        ("t2", "Song B", "b.mp3"),
        ("t3", "Song C", "c.mp3"),
    ];
    let (dir, usb_root) = seeded_playlist_usb_with_tracks(&tracks);
    let playlist = crate::edb::ExportPlaylistData {
        id: "pl-1".to_string(),
        name: "My Playlist".to_string(),
        tracks: tracks
            .iter()
            .map(|(id, title, filename)| make_export_track(id, title, filename))
            .collect(),
    };
    let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, &tracks);
    let mut session = UsbWriteSession::without_service(&usb_root);
    write_edb_playlists_in(
        &mut session,
        &[EdbPlaylistWrite {
            playlist: &playlist,
            manifest: &manifest,
            mirror_playlist_entries: true,
            identity: None,
            placement: PlaylistPlacement::MoveToFront,
        }],
    )
    .expect("write edb playlist");
    session.commit().expect("write edb playlist");
    // That writes the staged copy only (a no-op without staging).
    crate::service::usb::staging::write_back_if_changed(
        &usb_root,
        crate::service::usb::staging::DbKind::Edb,
    )
    .expect("write the edb back");
    (dir, usb_root)
}

fn file_name_of(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The playlist's files in the order the USB view shows them.
fn view_files(
    service: &crate::service::BackendService,
    usb_root: &Path,
    playlist_id: &str,
) -> Vec<(String, String)> {
    service
        .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
            usb_root: root_text(usb_root),
            id: playlist_id.to_string(),
            ..Default::default()
        })
        .expect("fetch tracks")
        .items
        .into_iter()
        .map(|track| (file_name_of(track.identity_path()), track.id))
        .collect()
}

fn view_names(view: &[(String, String)]) -> Vec<&str> {
    view.iter().map(|(name, _)| name.as_str()).collect()
}

/// The files of the USB's PDB playlist `name`, by `entry_index`, read from
/// the stick's own file.
fn usb_pdb_files(usb_root: &Path, name: &str) -> Vec<String> {
    let parsed = crate::pdb_reader::parse_pdb(
        &crate::service::usb::vendor_compat::vendor_pdb_path(usb_root),
    )
    .expect("parse usb pdb");
    let playlist = parsed
        .playlist_tree
        .iter()
        .find(|row| row.name == name && !row.row_is_folder)
        .expect("pdb playlist");
    let mut entries: Vec<_> = parsed
        .playlist_entries
        .iter()
        .filter(|entry| entry.playlist_id == playlist.id)
        .collect();
    entries.sort_by_key(|entry| entry.entry_index);
    entries
        .iter()
        .map(|entry| {
            let track = parsed
                .tracks
                .iter()
                .find(|track| track.id == entry.track_id)
                .expect("pdb track");
            file_name_of(&track.track_file_path)
        })
        .collect()
}

/// The files of the USB's eDB playlist `name`, by `sequenceNo`, read from
/// the stick's own file alone.
fn usb_edb_files(usb_root: &Path, name: &str) -> Vec<String> {
    let conn = crate::edb::open_edb(
        &crate::service::usb::vendor_compat::vendor_edb_path(usb_root),
        &mut Vec::new(),
    )
    .expect("open usb edb");
    let mut stmt = conn
        .prepare(
            "SELECT c.path FROM playlist_content pc
             JOIN playlist p ON p.playlist_id = pc.playlist_id
             JOIN content c ON c.content_id = pc.content_id
             WHERE p.name = ?1 AND p.attribute = 0 ORDER BY pc.sequenceNo",
        )
        .expect("prepare");
    stmt.query_map([name], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|path| file_name_of(&path.expect("path")))
        .collect()
}

fn begin_order_edit(service: &crate::service::BackendService, usb_root: &Path) {
    service
        .begin_usb_order_edit(crate::models::UsbOrderEditRequest {
            usb_root: root_text(usb_root),
        })
        .expect("begin order edit");
}

fn commit_order_edit(
    service: &crate::service::BackendService,
    usb_root: &Path,
) -> crate::error::BackendResult<crate::models::UsbOrderEditData> {
    service.commit_usb_order_edit(crate::models::UsbOrderEditRequest {
        usb_root: root_text(usb_root),
    })
}

/// Moves the row of `file` to before the row of `before` (the end: `None`).
fn move_file(
    service: &crate::service::BackendService,
    usb_root: &Path,
    playlist_id: &str,
    file: &str,
    before: Option<&str>,
) -> crate::error::BackendResult<crate::models::UsbOrderEditData> {
    let view = view_files(service, usb_root, playlist_id);
    let id_of = |name: &str| {
        view.iter()
            .find(|(file, _)| file == name)
            .map(|(_, id)| id.clone())
            .expect("row of file")
    };
    service.move_usb_playlist_track(crate::models::MoveUsbPlaylistTrackRequest {
        usb_root: root_text(usb_root),
        playlist_id: playlist_id.to_string(),
        move_track_id: id_of(file),
        before_track_id: before.map(id_of),
    })
}

fn only_playlist_id(service: &crate::service::BackendService, usb_root: &Path) -> String {
    service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: root_text(usb_root),
        })
        .expect("fetch playlists")
        .items[0]
        .id
        .clone()
}

/// Puts the playlists in `ids`' order with one move each, in an order edit.
fn edit_playlist_order(service: &crate::service::BackendService, usb_root: &Path, ids: &[String]) {
    begin_order_edit(service, usb_root);
    for id in ids {
        service
            .move_usb_playlist(crate::models::MoveUsbPlaylistRequest {
                usb_root: root_text(usb_root),
                playlist_id: id.clone(),
                before_playlist_id: None,
            })
            .expect("move playlist");
    }
    commit_order_edit(service, usb_root).expect("commit order edit");
}

/// A staged stick, connected (`validate_usb_root`) as the app connects one.
fn connected_order_edit_usb(
    service: &crate::service::BackendService,
) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let (dir, usb_root) = order_edit_usb();
    service
        .validate_usb_root(crate::models::ValidateUsbRootRequest {
            path: usb_root.to_string_lossy().to_string(),
        })
        .expect("validate usb");
    let playlist_id = only_playlist_id(service, &usb_root);
    (dir, usb_root, playlist_id)
}

fn usb_db_bytes(usb_root: &Path) -> (Vec<u8>, Vec<u8>) {
    (
        std::fs::read(crate::service::usb::vendor_compat::vendor_pdb_path(
            usb_root,
        ))
        .expect("usb pdb"),
        std::fs::read(crate::service::usb::vendor_compat::vendor_edb_path(
            usb_root,
        ))
        .expect("usb edb"),
    )
}

#[test]
fn usb_order_edit_writes_back_to_usb_when_staging_enabled() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);

    begin_order_edit(&service, &usb_root);
    move_file(&service, &usb_root, &playlist_id, "c.mp3", Some("a.mp3")).expect("move");
    assert_eq!(
        view_names(&view_files(&service, &usb_root, &playlist_id)),
        ["c.mp3", "a.mp3", "b.mp3"],
        "the view shows the pending order"
    );
    assert_eq!(
        usb_pdb_files(&usb_root, "My Playlist"),
        ["a.mp3", "b.mp3", "c.mp3"]
    );
    commit_order_edit(&service, &usb_root).expect("commit");

    // Read straight from the stick's files: the staged copies don't count,
    // and the eDB must hold the order without its write-ahead log.
    let expected = ["c.mp3", "a.mp3", "b.mp3"];
    assert_eq!(usb_pdb_files(&usb_root, "My Playlist"), expected);
    assert_eq!(usb_edb_files(&usb_root, "My Playlist"), expected);
    assert_eq!(
        view_names(&view_files(&service, &usb_root, &playlist_id)),
        expected
    );
}

#[test]
fn usb_order_edit_writes_one_playlist_order_to_both_databases() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let root = root_text(&usb_root);
    let names_in_order = |service: &crate::service::BackendService| {
        service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: root.clone(),
            })
            .expect("fetch playlists")
            .items
    };

    // "A" and "B" are in both databases, "C" only in the eDB.
    for (id, name, in_pdb) in [
        ("pl-a", "A", true),
        ("pl-b", "B", true),
        ("pl-c", "C", false),
    ] {
        let tracks = [("t1", "Song", "a.mp3")];
        let playlist = crate::edb::ExportPlaylistData {
            id: id.to_string(),
            name: name.to_string(),
            tracks: vec![make_export_track("t1", "Song", "a.mp3")],
        };
        let manifest = make_export_manifest(id, name, &usb_root, &tracks);
        if in_pdb {
            crate::service::export_helpers::write_pdb(
                &usb_root, &playlist, &manifest, true, None, None,
            )
            .expect("write pdb playlist");
        }
        let mut session = UsbWriteSession::without_service(&usb_root);
        write_edb_playlists_in(
            &mut session,
            &[EdbPlaylistWrite {
                playlist: &playlist,
                manifest: &manifest,
                mirror_playlist_entries: true,
                identity: None,
                placement: PlaylistPlacement::MoveToFront,
            }],
        )
        .expect("write edb playlist");
        session.commit().expect("write edb playlist");
    }

    let items = names_in_order(&service);
    let id_of = |name: &str| {
        items
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.id.clone())
            .expect("playlist id")
    };
    assert!(id_of("C").starts_with("usb-pl-name-"));

    for order in [["C", "B", "A"], ["B", "C", "A"], ["A", "B", "C"]] {
        let ids: Vec<String> = order.iter().map(|name| id_of(name)).collect();
        edit_playlist_order(&service, &usb_root, &ids);

        let read: Vec<String> = names_in_order(&service)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(read, order);

        let conn = crate::edb::open_edb_rw(&usb_root).expect("open edb");
        for (position, name) in order.iter().enumerate() {
            let sequence: i64 = conn
                .query_row(
                    "SELECT sequenceNo FROM playlist WHERE name = ?1 AND attribute = 0",
                    [name],
                    |row| row.get(0),
                )
                .expect("edb row");
            assert_eq!(sequence, position as i64, "eDB place of {name}");
        }
    }
}

#[test]
fn usb_order_edit_saves_order_with_only_edb_playlists() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let root = root_text(&usb_root);
    for (id, name) in [("pl-a", "A"), ("pl-b", "B")] {
        let tracks = [("t1", "Song", "a.mp3")];
        let playlist = crate::edb::ExportPlaylistData {
            id: id.to_string(),
            name: name.to_string(),
            tracks: vec![make_export_track("t1", "Song", "a.mp3")],
        };
        let manifest = make_export_manifest(id, name, &usb_root, &tracks);
        let mut session = UsbWriteSession::without_service(&usb_root);
        write_edb_playlists_in(
            &mut session,
            &[EdbPlaylistWrite {
                playlist: &playlist,
                manifest: &manifest,
                mirror_playlist_entries: true,
                identity: None,
                placement: PlaylistPlacement::MoveToFront,
            }],
        )
        .expect("write edb playlist");
        session.commit().expect("write edb playlist");
    }
    let fetch = || {
        service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: root.clone(),
            })
            .expect("fetch playlists")
            .items
    };
    let mut ids: Vec<String> = fetch().iter().map(|p| p.id.clone()).collect();
    ids.reverse();
    edit_playlist_order(&service, &usb_root, &ids);

    let read: Vec<String> = fetch().iter().map(|p| p.id.clone()).collect();
    assert_eq!(read, ids);
}

#[test]
fn usb_order_edit_conflict_in_either_database_stops_the_save_before_writing() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    for changed in [
        crate::service::usb::vendor_compat::vendor_pdb_path,
        crate::service::usb::vendor_compat::vendor_edb_path,
    ] {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "c.mp3", Some("a.mp3")).expect("move");

        // Something else writes the stick after it was staged.
        let path = changed(&usb_root);
        let bytes = std::fs::read(&path).expect("usb file");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, &bytes).expect("rewrite usb file");
        let before = usb_db_bytes(&usb_root);

        let err = commit_order_edit(&service, &usb_root).expect_err("conflict");
        assert!(
            err.english().contains("changed since it was last read"),
            "{}",
            err.english()
        );
        assert_eq!(usb_db_bytes(&usb_root), before, "neither database written");
        let state = service.usb_order_edit_state(&usb_root);
        assert!(state.active && state.changed, "the pending order stays");
    }
}

#[test]
fn usb_order_edit_failed_write_back_keeps_the_order_for_a_retry() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
    begin_order_edit(&service, &usb_root);
    move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

    // The PDB, written first, can't be written: nothing is.
    let blocker = crate::service::usb::staging::write_back_temp_path_for_test(
        &usb_root,
        crate::service::usb::staging::DbKind::Pdb,
    );
    std::fs::create_dir_all(&blocker).expect("block the pdb write-back");
    let before = usb_db_bytes(&usb_root);
    commit_order_edit(&service, &usb_root).expect_err("write-back fails");
    assert_eq!(usb_db_bytes(&usb_root), before);
    assert!(service.usb_order_edit_state(&usb_root).changed);
    assert_eq!(
        view_names(&view_files(&service, &usb_root, &playlist_id)),
        ["b.mp3", "c.mp3", "a.mp3"],
        "the view still shows the pending order"
    );

    std::fs::remove_dir(&blocker).expect("unblock");
    commit_order_edit(&service, &usb_root).expect("retry");
    let expected = ["b.mp3", "c.mp3", "a.mp3"];
    assert_eq!(usb_pdb_files(&usb_root, "My Playlist"), expected);
    assert_eq!(usb_edb_files(&usb_root, "My Playlist"), expected);
    assert!(!service.usb_order_edit_state(&usb_root).active);
}

#[test]
fn usb_order_edit_failed_edb_write_restores_pdb_and_cancel_leaves_usb_unchanged() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
    begin_order_edit(&service, &usb_root);
    move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

    // A failed second write must restore the first database too.
    let before = usb_db_bytes(&usb_root);
    let blocker = crate::service::usb::staging::write_back_temp_path_for_test(
        &usb_root,
        crate::service::usb::staging::DbKind::Edb,
    );
    std::fs::create_dir_all(&blocker).expect("block the edb write-back");
    commit_order_edit(&service, &usb_root).expect_err("save fails");
    assert_eq!(
        usb_db_bytes(&usb_root),
        before,
        "both databases rolled back"
    );
    assert!(service.usb_order_edit_state(&usb_root).changed);
    let cancelled = service
        .cancel_usb_order_edit(crate::models::UsbOrderEditRequest {
            usb_root: root_text(&usb_root),
        })
        .expect("cancel");
    assert!(!cancelled.status_text.contains("stays on the USB"));
    std::fs::remove_dir(&blocker).expect("unblock");
    assert!(
        !crate::service::usb::staging::write_back_if_changed(
            &usb_root,
            crate::service::usb::staging::DbKind::Edb
        )
        .expect("write back")
    );
    assert_eq!(usb_db_bytes(&usb_root), before);
}

#[test]
fn usb_order_edit_failure_after_a_disconnect_drops_the_staged_copies() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
    let staged = [
        crate::service::usb::staging::stage_pdb(&usb_root).expect("staged pdb"),
        crate::service::usb::staging::stage_edb(&usb_root).expect("staged edb"),
    ];
    begin_order_edit(&service, &usb_root);
    move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

    // The stick goes away once the staged copies are patched.
    let unplug = |_: usize, _: usize, message: &str| {
        if message == crate::i18n::tr::usb_progress_order_edit_write_back().render() {
            std::fs::remove_dir_all(&usb_root).expect("unplug");
            std::fs::write(&usb_root, b"").expect("nothing to write to");
        }
    };
    service
        .commit_usb_order_edit_with_progress(
            crate::models::UsbOrderEditRequest {
                usb_root: root_text(&usb_root),
            },
            unplug,
        )
        .expect_err("the stick is gone");
    for path in &staged {
        assert!(!path.exists(), "{} dropped", path.display());
    }
    service
        .cancel_usb_order_edit(crate::models::UsbOrderEditRequest {
            usb_root: root_text(&usb_root),
        })
        .expect("cancel");
    assert!(!service.usb_order_edit_state(&usb_root).active);
    std::fs::remove_file(&usb_root).expect("tidy up");
}

#[test]
fn usb_order_edit_incomplete_edb_checkpoint_stops_the_save() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
    begin_order_edit(&service, &usb_root);
    move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

    // A reader holding a snapshot of the staged eDB keeps its log from
    // being checkpointed.
    let reader = crate::edb::open_edb_rw(&usb_root).expect("open staged edb");
    reader
        .execute_batch("PRAGMA journal_mode=WAL;")
        .expect("wal");
    reader
        .busy_timeout(std::time::Duration::from_millis(50))
        .expect("timeout");
    reader.execute_batch("BEGIN").expect("begin");
    let _: i64 = reader
        .query_row("SELECT count(*) FROM playlist_content", [], |row| {
            row.get(0)
        })
        .expect("read");
    let before = usb_db_bytes(&usb_root);

    let err = commit_order_edit(&service, &usb_root).expect_err("checkpoint incomplete");
    assert!(
        err.english().contains("couldn't finish writing"),
        "{}",
        err.english()
    );
    assert_eq!(usb_db_bytes(&usb_root), before, "nothing written back");
    assert!(service.usb_order_edit_state(&usb_root).changed);
    drop(reader);
}

#[test]
fn usb_order_edit_that_comes_out_different_in_the_two_databases_is_not_saved() {
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = order_edit_usb();
    // The eDB files "Song A" under another path than the PDB does (what the
    // strict parity repair fixes): the edit's order, keyed by path, can't
    // place it in the PDB, where it ends up last.
    let conn = crate::edb::open_edb_rw(&usb_root).expect("open edb");
    conn.execute(
        "UPDATE content SET path = replace(path, 'a.mp3', 'a-moved.mp3') WHERE path LIKE '%a.mp3'",
        [],
    )
    .expect("move a.mp3 in the edb");
    drop(conn);
    crate::service::usb::staging::write_back_if_changed(
        &usb_root,
        crate::service::usb::staging::DbKind::Edb,
    )
    .expect("write the edb back");
    service
        .validate_usb_root(crate::models::ValidateUsbRootRequest {
            path: usb_root.to_string_lossy().to_string(),
        })
        .expect("validate usb");
    let playlist_id = only_playlist_id(&service, &usb_root);

    begin_order_edit(&service, &usb_root);
    move_file(
        &service,
        &usb_root,
        &playlist_id,
        "c.mp3",
        Some("a-moved.mp3"),
    )
    .expect("move");
    let before = usb_db_bytes(&usb_root);
    let err = commit_order_edit(&service, &usb_root).expect_err("orders differ");
    let message = err.english();
    assert!(
        message.contains("My Playlist") && message.contains("Upgrade Export Data To Strict Parity"),
        "{message}"
    );
    assert_eq!(usb_db_bytes(&usb_root), before, "nothing written back");
    assert!(
        service.usb_order_edit_state(&usb_root).changed,
        "the edit stays on"
    );
}

#[test]
fn usb_order_edit_refuses_changes_while_it_saves() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = order_edit_usb();
    let (_other_dir, other_root) = test_usb_root();
    let playlist_id = only_playlist_id(&service, &usb_root);
    begin_order_edit(&service, &usb_root);
    move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

    let during = service.clone();
    let root = root_text(&usb_root);
    let mut checked = false;
    let check = |_: usize, _: usize, message: &str| {
        if message != crate::i18n::tr::usb_progress_backing_up_usb_databases().render() {
            return;
        }
        checked = true;
        let edit = crate::models::UsbOrderEditRequest {
            usb_root: root.clone(),
        };
        assert!(during.begin_usb_order_edit(edit.clone()).is_err());
        assert!(
            during
                .move_usb_playlist(crate::models::MoveUsbPlaylistRequest {
                    usb_root: root.clone(),
                    playlist_id: playlist_id.clone(),
                    before_playlist_id: None,
                })
                .is_err()
        );
        assert!(
            during
                .sort_usb_playlist_tracks(crate::models::SortUsbPlaylistTracksRequest {
                    usb_root: root.clone(),
                    playlist_id: playlist_id.clone(),
                    sort_by: Some("title".to_string()),
                    sort_dir: None,
                })
                .is_err()
        );
        assert!(during.cancel_usb_order_edit(edit.clone()).is_err());
        assert!(during.commit_usb_order_edit(edit).is_err());
        assert!(
            during
                .select_usb(crate::models::SelectUsbRequest {
                    path: other_root.to_string_lossy().to_string(),
                })
                .is_err(),
            "the selected USB can't change"
        );
        assert!(
            during.try_lock_usb_writes().is_none(),
            "the save holds the USB write lock"
        );
    };
    service
        .commit_usb_order_edit_with_progress(
            crate::models::UsbOrderEditRequest {
                usb_root: root_text(&usb_root),
            },
            check,
        )
        .expect("commit");
    assert!(checked);
    assert_eq!(
        usb_pdb_files(&usb_root, "My Playlist"),
        ["b.mp3", "c.mp3", "a.mp3"]
    );
}

#[test]
fn failed_playlist_removal_keeps_both_databases_and_audio_for_retry() {
    let cache = tempfile::tempdir().unwrap();
    let _staging =
        crate::service::usb::staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
    let (_dir, service) = test_service();
    let (_usb, root, _) = connected_order_edit_usb(&service);
    let parsed =
        crate::pdb_reader::parse_pdb(&crate::service::usb::vendor_compat::vendor_pdb_path(&root))
            .unwrap();
    let audio: Vec<_> = parsed
        .tracks
        .iter()
        .map(|track| {
            let path = root.join(track.track_file_path.trim_start_matches('/'));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"audio to preserve").unwrap();
            path
        })
        .collect();
    let before = usb_db_bytes(&root);
    let blocker = crate::service::usb::staging::write_back_temp_path_for_test(
        &root,
        crate::service::usb::staging::DbKind::Edb,
    );
    std::fs::create_dir_all(&blocker).unwrap();
    let request = || crate::models::RemoveUsbPlaylistRequest {
        usb_root: root_text(&root),
        playlist_id: None,
        playlist_name: "My Playlist".into(),
    };
    service
        .remove_usb_playlist(request())
        .expect_err("second database write fails");
    assert_eq!(usb_db_bytes(&root), before);
    for path in &audio {
        assert_eq!(std::fs::read(path).unwrap(), b"audio to preserve");
    }
    std::fs::remove_dir(blocker).unwrap();
    service
        .remove_usb_playlist(request())
        .expect("retry the removal");
    assert!(audio.iter().all(|path| !path.exists()));
}

#[test]
fn remove_usb_playlist_removes_from_pdb_and_edb() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = seeded_playlist_usb();

    let result = service
        .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            playlist_id: None,
            playlist_name: "My Playlist".to_string(),
        })
        .expect("remove usb playlist");

    assert_eq!(result.removed_from_pdb, 1);

    let after = service
        .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("fetch after remove");
    assert!(after.items.iter().all(|p| p.name != "My Playlist"));
}

#[test]
fn remove_usb_playlist_writes_back_edb_to_usb_when_staging_enabled() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = seeded_playlist_usb();

    // Seed the eDB counterpart directly, before staging is enabled, so
    // it lands on the raw USB path (not a staged cache copy).
    let tracks: [(&str, &str, &str); 1] = [("t1", "Song A", "a.mp3")];
    let playlist = crate::edb::ExportPlaylistData {
        id: "pl-1".to_string(),
        name: "My Playlist".to_string(),
        tracks: tracks
            .iter()
            .map(|(id, title, filename)| make_export_track(id, title, filename))
            .collect(),
    };
    let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, &tracks);
    let mut session = UsbWriteSession::without_service(&usb_root);
    write_edb_playlists_in(
        &mut session,
        &[EdbPlaylistWrite {
            playlist: &playlist,
            manifest: &manifest,
            mirror_playlist_entries: true,
            identity: None,
            placement: PlaylistPlacement::MoveToFront,
        }],
    )
    .expect("seed edb playlist");
    session.commit().expect("seed edb playlist");

    // Enable staging now: remove_usb_playlist will operate on a local
    // HDD cache copy of the eDB, not the USB-mounted file directly.
    let cache_dir = tempfile::tempdir().expect("cache dir").keep();
    let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));

    let result = service
        .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            playlist_id: None,
            playlist_name: "My Playlist".to_string(),
        })
        .expect("remove usb playlist");
    assert_eq!(result.removed_from_edb, 1);

    // Bypass staging entirely and open the raw USB-mounted eDB file --
    // the file the physical drive actually holds. Before the write-back
    // fix, the eDB deletion only ever landed in the local staged cache
    // copy and never reached this path.
    let raw_edb_path = crate::service::usb::vendor_compat::vendor_edb_path(&usb_root);
    let mut open_warnings = Vec::new();
    let conn = crate::edb::open_edb(&raw_edb_path, &mut open_warnings).expect("open raw usb edb");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playlist WHERE name = ?1",
            rusqlite::params!["My Playlist"],
            |row| row.get(0),
        )
        .expect("count playlists");
    assert_eq!(
        count, 0,
        "removed playlist must be written back to the raw USB eDB, not just the staged cache copy"
    );
}

#[test]
fn remove_usb_playlist_rejects_empty_name() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let err = service
        .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            playlist_id: None,
            playlist_name: "   ".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, crate::error::BackendError::Internal(_)));
}

#[test]
fn remove_usb_playlist_not_found_returns_error() {
    let (_dir, service) = test_service();
    let (_usb_dir, usb_root) = test_usb_root();
    let err = service
        .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            playlist_id: None,
            playlist_name: "Does Not Exist".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, crate::error::BackendError::NotFound(_)));
}

#[test]
fn merge_orphaned_usb_placeholder_tracks_reassigns_playlist_and_links() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    let usb_root_path = "/mnt/usb-device";
    crate::service::usb::identity::upsert_usb_device(
        &conn,
        Path::new(usb_root_path),
        false,
        &now(),
    )
    .expect("seed usb device");

    let fingerprint = build_track_match_fingerprint("Song", "Artist", None);
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
         VALUES ('local-1', 'Song', 'Artist', '/library/song.mp3', 200000, 5000, ?1, datetime('now'), datetime('now'))",
        rusqlite::params![fingerprint],
    )
    .expect("insert local track");
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
         VALUES ('placeholder-1', 'Song', 'Artist', '/mnt/usb-device/Contents/song.mp3', 200000, 5000, ?1, datetime('now'), datetime('now'))",
        rusqlite::params![fingerprint],
    )
    .expect("insert placeholder track");
    drop(conn);

    let playlist = service
        .create_playlist(crate::models::CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    let conn = service.db.connect().expect("connect");
    conn.execute(
        "INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
         VALUES ('pt-1', ?1, 'placeholder-1', 0, datetime('now'))",
        rusqlite::params![playlist.playlist_id],
    )
    .expect("link playlist track");
    drop(conn);

    let result = service
        .merge_orphaned_usb_placeholder_tracks()
        .expect("merge placeholders");
    assert_eq!(result.merged, 1);

    let conn = service.db.connect().expect("connect");
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracks WHERE id = 'placeholder-1'",
            [],
            |r| r.get(0),
        )
        .expect("count placeholder");
    assert_eq!(remaining, 0);
    let playlist_track_id: String = conn
        .query_row(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1",
            rusqlite::params![playlist.playlist_id],
            |r| r.get(0),
        )
        .expect("playlist track id");
    assert_eq!(playlist_track_id, "local-1");
}
