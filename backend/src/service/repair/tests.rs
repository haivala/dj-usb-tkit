use super::*;

fn fp(size: u32, title: &str) -> Option<ContentFingerprint> {
    Some((size, title.to_string(), "artist".to_string()))
}

#[test]
fn match_moved_audio_pairs_by_fingerprint_then_file_name() {
    let missing = vec![
        (1, "/Contents/A/old name.mp3".to_string(), fp(100, "one")),
        (
            2,
            "/Contents/Old Folder/Two.mp3".to_string(),
            fp(200, "two"),
        ),
    ];
    let unindexed = vec![
        ("/Contents/A/new name.mp3".to_string(), fp(100, "one")),
        (
            "/Contents/New Folder/two.MP3".to_string(),
            fp(201, "re-encoded"),
        ),
    ];
    let pairs = match_moved_audio(&missing, &unindexed);
    assert_eq!(
        pairs,
        vec![
            MovedAudioRelink {
                track_id: 1,
                old_path: "/Contents/A/old name.mp3".to_string(),
                new_path: "/Contents/A/new name.mp3".to_string(),
            },
            MovedAudioRelink {
                track_id: 2,
                old_path: "/Contents/Old Folder/Two.mp3".to_string(),
                new_path: "/Contents/New Folder/two.MP3".to_string(),
            },
        ]
    );
}

#[test]
fn fix_outcomes_record_covers_every_selected_found_combination() {
    use super::run::{Detail, FixOutcomes, OutcomeKind};
    use crate::i18n::tr;

    let mut outcomes = FixOutcomes::default();
    let ok = || Ok(Detail::File("fixed 2 rows".to_string()));
    outcomes.record(tr::repair_fix_header(), true, true, ok);
    outcomes.record(tr::repair_fix_bpm_key(), true, true, || {
        Err(BackendError::Internal("disk full".to_string()))
    });
    outcomes.record(tr::repair_fix_relink(), true, false, ok);
    outcomes.record(tr::repair_fix_unindexed(), false, true, ok);
    outcomes.record(tr::repair_fix_engine(), false, false, ok);

    assert_eq!(
        outcomes.lines(OutcomeKind::Applied),
        vec!["Repair PDB Header Compatibility Field: fixed 2 rows"]
    );
    assert_eq!(
        outcomes.lines(OutcomeKind::Failed),
        vec!["Fix BPM/Key Mismatch failed: internal error: disk full"]
    );
    assert_eq!(
        outcomes.lines(OutcomeKind::Skipped),
        vec![
            "Relink Moved Audio Files: nothing to apply",
            "Add Unindexed Audio to a Playlist: not selected"
        ]
    );
}

#[test]
fn fix_outcomes_event_log_names_a_fix_without_its_file_format_detail() {
    use super::run::{Detail, FixOutcomes, OutcomeKind};
    use crate::i18n::tr;

    let mut outcomes = FixOutcomes::default();
    outcomes.record(tr::repair_fix_page_headers(), true, true, || {
        Ok(Detail::File("patched 3 pages".to_string()))
    });
    outcomes.record(tr::repair_fix_relink(), true, true, || {
        Ok(Detail::Text(tr::repair_outcome_detail_relinked(1)))
    });
    let mut warnings = Vec::new();
    outcomes.log(&mut warnings);

    let shown: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(
        shown,
        vec![
            "repair applied: Repair Damaged PDB Page Headers",
            "repair applied: Relink Moved Audio Files: relinked 1 track"
        ]
    );
    // The response keeps the detail.
    assert_eq!(
        outcomes.lines(OutcomeKind::Applied)[0],
        "Repair Damaged PDB Page Headers: patched 3 pages"
    );
}

#[test]
fn match_moved_audio_leaves_ambiguous_pairs_unmatched() {
    // Two candidate files for one reference.
    let missing = vec![(1, "/Contents/x.mp3".to_string(), fp(100, "one"))];
    let unindexed = vec![
        ("/Contents/a.mp3".to_string(), fp(100, "one")),
        ("/Contents/b.mp3".to_string(), fp(100, "one")),
    ];
    assert!(match_moved_audio(&missing, &unindexed).is_empty());

    // Two references claiming one file.
    let missing = vec![
        (1, "/Contents/A/x.mp3".to_string(), fp(100, "one")),
        (2, "/Contents/B/x.mp3".to_string(), fp(100, "one")),
    ];
    let unindexed = vec![("/Contents/C/y.mp3".to_string(), fp(100, "one"))];
    assert!(match_moved_audio(&missing, &unindexed).is_empty());

    // Nothing in common.
    let missing = vec![(1, "/Contents/x.mp3".to_string(), fp(100, "one"))];
    let unindexed = vec![("/Contents/y.mp3".to_string(), fp(300, "other"))];
    assert!(match_moved_audio(&missing, &unindexed).is_empty());
}

#[test]
fn match_moved_audio_does_not_relink_across_encodings() {
    let missing = vec![(1, "/Contents/Track.mp3".to_string(), fp(100, "one"))];
    let unindexed = vec![("/Contents/Track.flac".to_string(), fp(100, "one"))];
    assert!(match_moved_audio(&missing, &unindexed).is_empty());
}
use crate::edb::ExportDbPlaylist;
use crate::edb::open_edb_rw;
use crate::models::{DiagStatus, UsbParityPlaylistDetail};
use crate::pdb_reader::{
    ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbPlaylistTreeRow, PdbTrackRow,
};
use crate::service::anlz::read_beatgrid_tempo_from_anlz;
use crate::service::export_helpers::inspect_pdb_columns_playlist_order;
use tempfile::tempdir;

fn make_strict_repair_detail(name: &str) -> UsbParityPlaylistDetail {
    UsbParityPlaylistDetail {
        name: name.to_string(),
        pdb_tracks: 1,
        edb_tracks: 1,
        matched_tracks: 1,
        only_in_pdb: 0,
        only_in_edb: 0,
        order_mismatch: false,
        path_mismatch_tracks: 0,
        dictionary_id_issue_tracks: 0,
        playlist_in_both: true,
        sort_order_match: true,
        parent_match: Some(true),
        pdb_playlist_id: Some(1),
        edb_playlist_id: Some(1),
        pdb_sort_order: Some(1),
        edb_sort_order: Some(1),
        pdb_duplicate_entries: 0,
        edb_missing_core_metadata: 0,
        pdb_missing_core_metadata: 0,
        artwork_mismatch_tracks: 0,
        rating_colour_mismatch_tracks: 0,
        field_note_tracks: 0,
        sample_only_in_pdb: Vec::new(),
        sample_only_in_edb: Vec::new(),
        sample_metadata_mismatches: Vec::new(),
        status: DiagStatus::Pass,
        issue_labels: Vec::new(),
    }
}

#[test]
fn playlist_requires_strict_upgrade_detects_failure_cases() {
    let mut failing = make_strict_repair_detail("Failing");
    failing.status = DiagStatus::Fail;
    failing.pdb_missing_core_metadata = 1;
    assert!(playlist_requires_strict_upgrade(&failing));

    let mut pdb_only = make_strict_repair_detail("PDB Only");
    pdb_only.status = DiagStatus::Fail;
    pdb_only.only_in_pdb = 1;
    assert!(playlist_requires_strict_upgrade(&pdb_only));

    let mut noted = make_strict_repair_detail("Field Notes");
    noted.field_note_tracks = 1;
    assert!(playlist_requires_strict_upgrade(&noted));

    let clean = make_strict_repair_detail("Clean");
    assert!(!playlist_requires_strict_upgrade(&clean));
}

#[test]
fn strict_repair_playlist_keys_follow_pdb_order_with_edb_only_tail() {
    let mut parsed = ParsedPdb::default();
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 1,
        is_folder: false,
        name: "Zeta".to_string(),
    });
    parsed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 20,
        parent_id: 0,
        sort_order: 2,
        is_folder: false,
        name: "Alpha".to_string(),
    });

    let mut edb_playlists = HashMap::<String, ExportDbPlaylist>::new();
    edb_playlists.insert(
        "Alpha".to_string(),
        ExportDbPlaylist {
            playlist_id: 20,
            sort_order: 0,
            path: crate::pdb_reader::PlaylistPath::top_level("Alpha"),
            tracks: Vec::new(),
        },
    );
    edb_playlists.insert(
        "Zeta".to_string(),
        ExportDbPlaylist {
            playlist_id: 10,
            sort_order: 1,
            path: crate::pdb_reader::PlaylistPath::top_level("Zeta"),
            tracks: Vec::new(),
        },
    );
    edb_playlists.insert(
        "Beta".to_string(),
        ExportDbPlaylist {
            playlist_id: 30,
            sort_order: 0,
            path: crate::pdb_reader::PlaylistPath::top_level("Beta"),
            tracks: Vec::new(),
        },
    );

    let keys = strict_repair_ordered_playlist_keys(&parsed, Some(&edb_playlists));

    assert_eq!(keys, vec!["zeta", "alpha", "beta"]);
}

#[test]
fn derive_history_sync_payload_uses_named_histories_and_valid_entries() {
    let parsed = ParsedPdb {
        history_playlists: vec![
            PdbHistoryPlaylistRow {
                id: 10,
                name: "HISTORY 001".to_string(),
                source_table: 11,
            },
            PdbHistoryPlaylistRow {
                id: 11,
                name: "Not History".to_string(),
                source_table: 11,
            },
        ],
        history_entries: vec![
            PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 10,
                entry_index: 1,
                source_table: 12,
            },
            PdbHistoryEntryRow {
                track_id: Some(0),
                playlist_id: 10,
                entry_index: 2,
                source_table: 12,
            },
            PdbHistoryEntryRow {
                track_id: Some(202),
                playlist_id: 11,
                entry_index: 1,
                source_table: 12,
            },
        ],
        ..ParsedPdb::default()
    };

    // The eDB numbers its content rows on its own: PDB track 101 is eDB
    // content 7, and a track with no eDB row stays in the PDB's history only.
    let content_ids = HashMap::from([(101, 7)]);
    let (history_rows, history_content_rows) = derive_history_sync_payload(&parsed, &content_ids);
    assert_eq!(history_rows, vec![(10, 0, "HISTORY 001".to_string(), 0, 0)]);
    assert_eq!(history_content_rows, vec![(10, 7, 1)]);
    let (_, unmatched) = derive_history_sync_payload(&parsed, &HashMap::new());
    assert!(unmatched.is_empty());
}

fn test_usb_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let td = tempdir().expect("tempdir");
    let usb_root = td.path().join("USB_TEST");
    std::fs::create_dir_all(&usb_root).expect("create usb root");
    crate::service::initialize_usb(&usb_root).expect("initialize usb");
    (td, usb_root)
}

#[test]
fn menu_and_color_failures_leave_both_usb_databases_unchanged() {
    for menu in [true, false] {
        let cache = tempdir().unwrap();
        let _staging = staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let (_usb, usb_root) = test_usb_root();
        let data = tempdir().unwrap();
        let service = BackendService::new(data.path()).unwrap();
        let pdb = vendor_pdb_path(&usb_root);
        let edb = super::super::usb::vendor_compat::vendor_edb_path(&usb_root);
        let before = (std::fs::read(&pdb).unwrap(), std::fs::read(&edb).unwrap());
        let blocker = staging::write_back_temp_path_for_test(&usb_root, staging::DbKind::Edb);
        std::fs::create_dir_all(blocker).unwrap();
        if menu {
            let before_menu = service
                .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
                    usb_root: Some(usb_root.to_string_lossy().into_owned()),
                })
                .unwrap();
            let kind = before_menu
                .current_items
                .iter()
                .find(|item| item.menu_item_id == 17)
                .unwrap()
                .kind;
            service
                .update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
                    usb_root: Some(usb_root.to_string_lossy().into_owned()),
                    action: UsbPlayerMenuEdit::MoveUp,
                    kind,
                })
                .expect_err("menu save must fail");
        } else {
            service
                .set_usb_background_color(crate::models::SetUsbBackgroundColorRequest {
                    usb_root: Some(usb_root.to_string_lossy().into_owned()),
                    color: 5,
                })
                .expect_err("color save must fail");
        }
        assert_eq!(
            (std::fs::read(pdb).unwrap(), std::fs::read(edb).unwrap()),
            before
        );
    }
}

#[test]
fn background_color_sets_both_databases_on_the_usb() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");
    let root = Some(usb_root.to_string_lossy().to_string());
    let get = || {
        service
            .get_usb_background_color(crate::models::GetUsbBackgroundColorRequest {
                usb_root: root.clone(),
            })
            .expect("get color")
    };
    let set = |color| {
        service.set_usb_background_color(crate::models::SetUsbBackgroundColorRequest {
            usb_root: root.clone(),
            color,
        })
    };

    let before = get();
    let purple = Some(crate::pdb_writer::INIT_BACKGROUND_COLOR);
    assert_eq!((before.pdb_color, before.edb_color), (purple, purple));
    assert!(before.writable);

    let set_green = set(5).expect("set green");
    assert!(set_green.updated);
    assert_eq!(
        (set_green.pdb_color, set_green.edb_color),
        (Some(5), Some(5))
    );
    let after = get();
    assert_eq!((after.pdb_color, after.edb_color), (Some(5), Some(5)));

    // On the USB itself, not just the staged copies.
    let pdb = std::fs::read(vendor_pdb_path(&usb_root)).expect("read pdb");
    assert_eq!(crate::pdb_writer::t19_background_color(&pdb, 4096), Some(5));
    let conn = crate::edb::open_edb(
        &super::super::usb::vendor_compat::vendor_edb_path(&usb_root),
        &mut Vec::new(),
    )
    .expect("open eDB");
    let edb_color: i64 = conn
        .query_row("SELECT backGroundColorType FROM property", [], |row| {
            row.get(0)
        })
        .expect("property");
    assert_eq!(edb_color, 5);

    assert!(!set(5).expect("same color").updated);
    assert!(set(9).is_err(), "out of range");
    assert_eq!(get().pdb_color, Some(5));
}

#[test]
fn player_menu_config_roundtrip_updates_edb_not_pdb() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    let before = service
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("get cdj menu config");
    assert!(
        !before.current_items.is_empty(),
        "expected initialized USB to have visible player menu items"
    );
    // Playlist (menuItem_id=17) is at sequenceNo=5 by default — not first.
    assert_ne!(
        before.current_items.first().map(|i| i.menu_item_id),
        Some(17),
        "playlist should not be first in default seeded state"
    );
    let pdb_row_count_before = inspect_pdb_columns_playlist_order(&usb_root)
        .expect("inspect pdb before")
        .expect("pdb t16 rows available")
        .1;

    // Move PLAYLIST up to the top, one step at a time, as the editor's Up
    // button does.
    let playlist_kind = before
        .current_items
        .iter()
        .find(|item| item.menu_item_id == 17)
        .expect("playlist in the menu")
        .kind;
    let steps = before
        .current_items
        .iter()
        .position(|item| item.kind == playlist_kind)
        .expect("playlist position");
    let mut updated = None;
    for _ in 0..steps {
        updated = Some(
            service
                .update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                    action: UsbPlayerMenuEdit::MoveUp,
                    kind: playlist_kind,
                })
                .expect("update cdj menu config"),
        );
    }
    let updated = updated.expect("at least one move");
    assert!(updated.updated, "expected player menu update to be applied");
    assert_eq!(
        updated.current_items.first().map(|item| item.menu_item_id),
        Some(17)
    );
    assert_eq!(
        updated.current_items.len(),
        before.current_items.len(),
        "moving changes the order only"
    );

    // PDB t16 must NOT be modified — row count stays the same.
    let pdb_after = inspect_pdb_columns_playlist_order(&usb_root)
        .expect("inspect pdb after update")
        .expect("pdb t16 rows available");
    assert_eq!(
        pdb_after.1, pdb_row_count_before,
        "PDB t16 row count must not change: category edit is eDB-only"
    );

    // Re-read via get_usb_player_menu_config to confirm the change persisted in eDB.
    let after = service
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("re-read cdj menu config after update");
    let after_ids = after
        .current_items
        .iter()
        .map(|i| i.menu_item_id)
        .collect::<Vec<_>>();
    assert_eq!(
        after_ids.first(),
        Some(&17),
        "menu order must persist across a re-read"
    );
}

#[test]
fn player_menu_edits_apply_to_the_current_order() {
    use UsbPlayerMenuEdit::*;
    let menu = [1, 2, 3];
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, Add, 4),
        vec![1, 2, 3, 4]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, Add, 2),
        vec![1, 2, 3]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, Remove, 2),
        vec![1, 3]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, Remove, 9),
        vec![1, 2, 3]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, MoveUp, 2),
        vec![2, 1, 3]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, MoveUp, 1),
        vec![1, 2, 3]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, MoveDown, 2),
        vec![1, 3, 2]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, MoveDown, 3),
        vec![1, 2, 3]
    );
    assert_eq!(
        player_menu::apply_player_menu_edit(&menu, MoveDown, 9),
        vec![1, 2, 3]
    );
}

// ── Local HDD staging integration ─────────────────────────────────
//
// These follow the same pattern edb.rs's staging tests already
// established: `set_cache_root_for_test` overrides `cache_root()` only
// for the calling thread, and `cargo test` never runs two tests
// concurrently on the same thread, so this needs no lock and can't
// race with unrelated tests on other threads. The returned guard resets
// the override on drop (including on panic), and the cache tempdir is
// leaked via `.keep()` as harmless extra insurance.

#[test]
fn repair_apply_batches_local_writes_and_flushes_pdb_to_usb_once() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = staging::set_cache_root_for_test(Some(cache_dir));

    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    // Corrupt the header-compatibility field (offset 0x10) on the real,
    // initialized USB's export.pdb to an unknown value, so the repair
    // detects and proposes fixing it.
    let raw_pdb_path = vendor_pdb_path(&usb_root);
    {
        let mut bytes = std::fs::read(&raw_pdb_path).expect("read seed pdb");
        bytes[0x10..0x14].copy_from_slice(&99u32.to_le_bytes());
        std::fs::write(&raw_pdb_path, &bytes).expect("write corrupted header value");
    }
    let raw_bytes_before_apply = std::fs::read(&raw_pdb_path).unwrap();
    let mut progress_messages = Vec::new();

    let result = service
        .repair_usb_diagnostics_with_progress(
            RepairUsbDiagnosticsRequest {
                fix_names: Default::default(),
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                apply: true,
                selected_fix_ids: vec![PDB_HEADER_COMPATIBILITY_FIX_ID.to_string()],
            },
            |current, _, message| progress_messages.push((current, message.to_string())),
        )
        .expect("repair apply");

    assert!(
        !result.applied_fixes.is_empty() && result.failed_fixes.is_empty(),
        "expected header-compatibility fix to apply cleanly: {result:?}"
    );
    assert!(
        progress_messages
            .iter()
            .any(|(percent, message)| *percent == 70
                && message == "USB: Applying Repair PDB Header Compatibility Field")
    );
    assert_eq!(
        progress_messages
            .iter()
            .filter(|(_, message)| message.starts_with("USB: Applying "))
            .count(),
        2, // The generic apply stage and the one selected fix.
    );
    assert!(
        progress_messages
            .iter()
            .any(|(percent, message)| *percent == 95 && message == "USB: Saving repaired databases")
    );

    // The batched end-of-apply write-back must have reached the *raw*
    // USB path, not just the local cache -- this is what the
    // `?`-early-return fix and the write-back call placement exist to
    // guarantee.
    let raw_bytes_after_apply = std::fs::read(&raw_pdb_path).unwrap();
    assert_ne!(
        raw_bytes_before_apply, raw_bytes_after_apply,
        "USB file must reflect the fix after the batched write-back"
    );
    let value_after = u32::from_le_bytes(raw_bytes_after_apply[0x10..0x14].try_into().unwrap());
    assert_eq!(value_after, PDB_HEADER_COMPATIBILITY_FALLBACK_VALUE);
}

#[test]
fn a_page_fix_that_fails_halfway_leaves_the_staged_pdb_unchanged() {
    let (_td, usb_root) = test_usb_root();
    let mut pdb = super::run::StagedPdb::load(&usb_root).expect("load staged pdb");
    let before = pdb.bytes.clone();

    let result: BackendResult<()> = pdb.patch(|bytes| {
        bytes[0x10] = 0xAA;
        Err(BackendError::Internal("failed halfway".into()))
    });

    assert!(result.is_err());
    assert_eq!(pdb.bytes, before);
}

/// Stages both databases of `usb_root` and changes the staged PDB's header
/// compatibility field, as a repair run would before write-back.
fn staged_repair_with_changed_pdb(usb_root: &Path) -> super::run::StagedPdb {
    let mut pdb = super::run::StagedPdb::load(usb_root).expect("load staged pdb");
    staging::stage_edb(usb_root).expect("stage edb");
    pdb.patch(|bytes| {
        bytes[0x10..0x14].copy_from_slice(&99u32.to_le_bytes());
        Ok(())
    })
    .expect("patch staged pdb");
    pdb
}

fn append_byte(path: &Path) {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open for append")
        .write_all(b"x")
        .expect("append");
}

#[test]
fn repair_write_back_writes_neither_database_when_the_usb_changed_since_staging() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = staging::set_cache_root_for_test(Some(cache_dir));
    let (_td, usb_root) = test_usb_root();
    let raw_pdb_path = vendor_pdb_path(&usb_root);
    let raw_pdb_before = std::fs::read(&raw_pdb_path).unwrap();
    let mut pdb = staged_repair_with_changed_pdb(&usb_root);

    // Something else writes the USB's eDB after it was staged.
    append_byte(&super::super::usb::vendor_compat::vendor_edb_path(
        &usb_root,
    ));

    let err = super::run::write_back_both_or_neither(&usb_root, &mut pdb)
        .expect_err("write-back must refuse an outside change");
    assert!(
        err.english().contains("changed since it was last read"),
        "{err:?}"
    );
    assert_eq!(std::fs::read(&raw_pdb_path).unwrap(), raw_pdb_before);
    assert_eq!(
        std::fs::read(staging::stage_pdb(&usb_root).unwrap()).unwrap(),
        raw_pdb_before,
        "the staged PDB is staged again from the USB"
    );
}

#[test]
fn repair_write_back_puts_the_previous_pdb_back_when_the_edb_write_fails() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = staging::set_cache_root_for_test(Some(cache_dir));
    let (_td, usb_root) = test_usb_root();
    let raw_pdb_path = vendor_pdb_path(&usb_root);
    let raw_edb_path = super::super::usb::vendor_compat::vendor_edb_path(&usb_root);
    let raw_pdb_before = std::fs::read(&raw_pdb_path).unwrap();
    let raw_edb_before = std::fs::read(&raw_edb_path).unwrap();
    let mut pdb = staged_repair_with_changed_pdb(&usb_root);
    append_byte(&staging::stage_edb(&usb_root).unwrap());

    // The eDB's write-back copies through this temp path; a directory
    // there makes that copy fail after the PDB was written.
    let edb_tmp = raw_edb_path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::create_dir_all(edb_tmp.join("blocked")).unwrap();

    super::run::write_back_both_or_neither(&usb_root, &mut pdb)
        .expect_err("the eDB write-back must fail");
    assert_eq!(
        std::fs::read(&raw_pdb_path).unwrap(),
        raw_pdb_before,
        "the PDB the USB held before is put back"
    );
    assert_eq!(std::fs::read(&raw_edb_path).unwrap(), raw_edb_before);
    assert_eq!(
        std::fs::read(staging::stage_edb(&usb_root).unwrap()).unwrap(),
        raw_edb_before,
        "the staged eDB is staged again from the USB"
    );
}

#[test]
fn repair_write_back_removes_a_rebuilt_pdb_when_the_edb_write_fails() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = staging::set_cache_root_for_test(Some(cache_dir));
    let (_td, usb_root) = test_usb_root();
    let raw_pdb_path = vendor_pdb_path(&usb_root);
    let raw_edb_path = super::super::usb::vendor_compat::vendor_edb_path(&usb_root);
    let rebuilt = std::fs::read(&raw_pdb_path).unwrap();
    std::fs::remove_file(&raw_pdb_path).unwrap();
    let mut pdb = super::run::StagedPdb::load(&usb_root).expect("load staged pdb");
    append_byte(&staging::stage_edb(&usb_root).unwrap());
    pdb.patch(|bytes| {
        bytes.clone_from(&rebuilt);
        Ok(())
    })
    .expect("stage rebuilt pdb");

    let edb_tmp = raw_edb_path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::create_dir_all(edb_tmp.join("blocked")).unwrap();

    super::run::write_back_both_or_neither(&usb_root, &mut pdb)
        .expect_err("the eDB write-back must fail");
    assert!(
        !raw_pdb_path.exists(),
        "a USB that had no PDB is left without one"
    );
}

#[test]
fn apply_fix_sync_edb_history_from_pdb_flushes_to_usb_when_staging_enabled() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = staging::set_cache_root_for_test(Some(cache_dir));

    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    let parsed = ParsedPdb {
        history_playlists: vec![PdbHistoryPlaylistRow {
            id: 10,
            name: "HISTORY 001".to_string(),
            source_table: 17,
        }],
        history_entries: vec![PdbHistoryEntryRow {
            track_id: Some(101),
            playlist_id: 10,
            entry_index: 1,
            source_table: 18,
        }],
        ..ParsedPdb::default()
    };

    let (history_written, _history_content_written) = service
        .apply_fix_sync_edb_history_from_pdb(&usb_root, &parsed, None)
        .expect("sync edb history");
    assert_eq!(history_written, 1);

    // Before any write-back: the write went through `open_edb_rw`, which
    // is staging-transparent, so it must have landed in the local
    // staged copy only -- the raw USB eDB file must not have the new
    // row yet. This is the eDB flush gap this whole change exists to
    // close: proving it *doesn't* reach USB here is as important as
    // proving it *does* after the flush below.
    // The real initialize_usb-seeded eDB is SQLCipher-encrypted, so
    // `crate::edb::open_edb` (which tries the known keys) is used
    // instead of a plain `rusqlite::Connection::open` -- applied
    // directly to the raw path to bypass staging entirely.
    let raw_edb_path = super::super::usb::vendor_compat::vendor_edb_path(&usb_root);
    {
        let mut open_warnings = Vec::new();
        let conn =
            crate::edb::open_edb(&raw_edb_path, &mut open_warnings).expect("open raw usb edb");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM history WHERE name = 'HISTORY 001'",
                [],
                |r| r.get(0),
            )
            .expect("query raw usb edb before write-back");
        assert_eq!(
            count, 0,
            "raw USB eDB must not see the change before write-back"
        );
    }

    let committed = staging::write_back_if_changed(&usb_root, staging::DbKind::Edb).unwrap();
    assert!(committed, "expected the eDB write-back to actually commit");

    let mut open_warnings = Vec::new();
    let conn =
        crate::edb::open_edb(&raw_edb_path, &mut open_warnings).expect("re-open raw usb edb");
    let (hid, name): (i64, String) = conn
        .query_row("SELECT history_id, name FROM history", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("query raw usb edb after write-back");
    assert_eq!(hid, 10);
    assert_eq!(name, "HISTORY 001");
}

// ── Hand-built minimal PDB fixtures for the byte-level repair pairs ──
//
// These construct a minimal but structurally valid PDB file (a header
// page plus one or more data pages) directly in memory rather than
// corrupting a real export, since these detect_pdb_*/apply_pdb_*_repair
// functions are pure `&[u8]` byte-format functions. Field offsets below
// mirror what each detect_pdb_* function reads: idx@+4, table_type@+8,
// next_page@+0xc, nrs@+0x18 (single byte) or packed 3-byte row count
// @0x18..0x1b, page_flags@+0x1b, used_s@+0x1e, u5@+0x20, num_rl@+0x22.

const TEST_PAGE_SIZE: usize = 4096;

fn empty_page() -> Vec<u8> {
    vec![0u8; TEST_PAGE_SIZE]
}

fn header_page() -> Vec<u8> {
    let mut p = empty_page();
    p[4..8].copy_from_slice(&(TEST_PAGE_SIZE as u32).to_le_bytes());
    p
}

/// A minimal non-empty, non-sentinel data page: idx set, used_s nonzero,
/// flags/u5/num_rl/nrs zeroed (callers overwrite what they need).
fn data_page(page_index: u32, table_type: u32) -> Vec<u8> {
    let mut p = empty_page();
    p[4..8].copy_from_slice(&page_index.to_le_bytes());
    p[8..12].copy_from_slice(&table_type.to_le_bytes());
    p[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes()); // used_s: nonzero placeholder
    p
}

fn set_flags(page: &mut [u8], flags: u8) {
    page[0x1b] = flags;
}

fn set_next_page(page: &mut [u8], next: u32) {
    page[0x0c..0x10].copy_from_slice(&next.to_le_bytes());
}

fn set_nrs(page: &mut [u8], nrs: u8) {
    page[0x18] = nrs;
}

fn set_u5_num_rl(page: &mut [u8], u5: u16, num_rl: u16) {
    page[0x20..0x22].copy_from_slice(&u5.to_le_bytes());
    page[0x22..0x24].copy_from_slice(&num_rl.to_le_bytes());
}

/// USB-root-shaped wrapper so `apply_*` functions (which build their own
/// `vendor_pdb_path(usb_root)`) find the fixture PDB.
fn write_pdb_pages_as_usb_root(pages: Vec<Vec<u8>>) -> (tempfile::TempDir, std::path::PathBuf) {
    let td = tempdir().expect("tempdir");
    let usb_root = td.path().to_path_buf();
    let pdb_path = vendor_pdb_path(&usb_root);
    std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
    let mut bytes = Vec::with_capacity(pages.len() * TEST_PAGE_SIZE);
    for p in &pages {
        assert_eq!(
            p.len(),
            TEST_PAGE_SIZE,
            "page must be exactly one page long"
        );
        bytes.extend_from_slice(p);
    }
    std::fs::write(&pdb_path, &bytes).expect("write test pdb");
    (td, usb_root)
}

#[test]
fn duplicate_playlist_entries_repair_removes_stale_rows_and_is_idempotent() {
    use crate::pdb_writer::writer_tests::build_tt8_test_page;

    let bytes = build_tt8_test_page(&[
        (1, 100, 1), // stale duplicate, must be removed
        (2, 200, 1), // untouched: distinct track
        (5, 100, 1), // kept: highest entry_index for (playlist=1, track=100)
    ]);
    let pages: Vec<Vec<u8>> = bytes.chunks(TEST_PAGE_SIZE).map(|c| c.to_vec()).collect();
    let (_td, usb_root) = write_pdb_pages_as_usb_root(pages);

    let removed = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_duplicate_playlist_entries_repair(bytes)
    })
    .unwrap();
    assert_eq!(removed, 1);

    let removed_again = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_duplicate_playlist_entries_repair(bytes)
    })
    .unwrap();
    assert_eq!(removed_again, 0, "repair should be idempotent");
}

#[test]
fn duplicate_playlist_entries_repair_is_noop_on_clean_pdb() {
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), data_page(1, 8)]);
    let removed = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_duplicate_playlist_entries_repair(bytes)
    })
    .unwrap();
    assert_eq!(removed, 0);
}

/// A one-group tt=7 page with `slots` row slots and the given footer bits.
fn playlist_tree_page(slots: u8, u5: u16, num_rl: u16, rowpf: u16, tranrf: u16) -> Vec<u8> {
    let mut p = data_page(1, 7);
    set_flags(&mut p, 0x34);
    set_nrs(&mut p, slots);
    set_u5_num_rl(&mut p, u5, num_rl);
    let end = TEST_PAGE_SIZE;
    p[end - 4..end - 2].copy_from_slice(&rowpf.to_le_bytes());
    p[end - 2..end].copy_from_slice(&tranrf.to_le_bytes());
    p
}

fn page_u5_num_rl_tranrf(bytes: &[u8]) -> (u16, u16, u16) {
    let off = TEST_PAGE_SIZE;
    let end = off + TEST_PAGE_SIZE;
    (
        u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
        u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
        u16::from_le_bytes(bytes[end - 2..end].try_into().unwrap()),
    )
}

/// Detects the page, applies the fix and returns the page's
/// `(u5, num_rl, tranrf)` afterwards.
fn fix_playlist_tree_page(page: Vec<u8>) -> (u16, u16, u16) {
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), page]);
    let pdb_path = vendor_pdb_path(&usb_root);
    let pages = detect_pdb_inconsistent_page_footers(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_inconsistent_page_footer_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);
    let bytes = std::fs::read(&pdb_path).unwrap();
    assert!(detect_pdb_inconsistent_page_footers(&bytes).is_empty());
    page_u5_num_rl_tranrf(&bytes)
}

fn playlist_tree_page_flagged(page: Vec<u8>) -> bool {
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), page]);
    !detect_pdb_inconsistent_page_footers(&read_pdb_file(&vendor_pdb_path(&usb_root))).is_empty()
}

#[test]
fn damaged_page_header_repair_restores_flags_and_live_count() {
    // What a crashed rekordbox write leaves: flags 0x10 and a live-row count
    // that isn't the number of rowpf bits (3 live of 4 slots, header says 0).
    let mut p1 = playlist_tree_page(4, 1, 3, 0b1011, 0b1000);
    set_flags(&mut p1, 0x10);
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_damaged_page_headers(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].flags, Some(0x24));
    assert_eq!(pages[0].slot_counts, Some((4, 3)));
    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_damaged_page_header_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_damaged_page_header_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let page = &bytes[TEST_PAGE_SIZE..2 * TEST_PAGE_SIZE];
    assert_eq!(page[0x1b], 0x24);
    assert_eq!(pdb_page::read_slot_counts(page), Some((4, 3)));
    assert!(detect_pdb_damaged_page_headers(&bytes).is_empty());
}

#[test]
fn damaged_page_headers_accept_active_flags_on_every_table() {
    // rekordbox writes 0x34 on any table, not only tracks and history.
    for table_type in [0u32, 2, 7, 8, 13] {
        let mut p1 = playlist_tree_page(4, 1, 3, 0b1111, 0b1000);
        p1[8..12].copy_from_slice(&table_type.to_le_bytes());
        p1[0x19] = 0x80; // live-row count 4 in bits 13-23
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        assert!(
            detect_pdb_damaged_page_headers(&read_pdb_file(&vendor_pdb_path(&usb_root))).is_empty(),
            "table {table_type}"
        );
    }
}

#[test]
fn footer_transaction_reads_the_slot_count_past_255() {
    // tt=8, 257 slots: the count wraps a u8 (257 & 0xFF = 1) and spans 17
    // footer groups. (1, 256) with tranrf on slot 256 follows the rule; the
    // same fields with no tranrf bit do not, and the fix marks slot 256.
    let page = |tranrf: u16| {
        let mut p1 = data_page(1, 8);
        p1[0x18] = 1; // packed slot count low byte: 257 & 0xFF
        p1[0x19] = 1; // packed slot count mid byte: (257 >> 8) & 0xFF
        set_u5_num_rl(&mut p1, 1, 256);
        let last_group_rowpf_off = TEST_PAGE_SIZE - 16 * 36 - 4;
        p1[last_group_rowpf_off..last_group_rowpf_off + 2].copy_from_slice(&1u16.to_le_bytes());
        p1[last_group_rowpf_off + 2..last_group_rowpf_off + 4]
            .copy_from_slice(&tranrf.to_le_bytes());
        p1
    };
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), page(1)]);
    assert!(
        detect_pdb_inconsistent_page_footers(&read_pdb_file(&vendor_pdb_path(&usb_root)))
            .is_empty()
    );

    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), page(0)]);
    let pdb_path = vendor_pdb_path(&usb_root);
    let pages = detect_pdb_inconsistent_page_footers(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1, "the 257th slot's group must be read");
    assert_eq!((pages[0].u5, pages[0].num_rl), (1, 256));
    patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_inconsistent_page_footer_repair(bytes, &pages)
    })
    .unwrap();
    assert!(detect_pdb_inconsistent_page_footers(&read_pdb_file(&pdb_path)).is_empty());
}
/// Build a small realistic PDB (via the real writer) with one track and one album, both
/// carrying non-ASCII names, so their string slots are UTF-16 encoded.
fn build_seed_pdb_bytes_for_alignment_test() -> Vec<u8> {
    let mut data = crate::pdb_writer::PdbData::empty();
    data.artists.push(crate::pdb_writer::PdbArtistRow {
        id: 1,
        name: "Existing Artist".into(),
    });
    data.albums.push(crate::pdb_writer::PdbAlbumRow {
        id: 1,
        name: "Café Album".into(),
        artist_id: 1,
    });
    data.genres.push(crate::pdb_writer::PdbDictRow {
        id: 1,
        name: "Existing Genre".into(),
    });
    data.keys.push(crate::pdb_writer::PdbKeyRow {
        id: 1,
        name: "Am".into(),
    });
    data.tracks.push(PdbTrackRowData {
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 0,
        key_id: 1,
        genre_id: 0,
        title: "Café Track".into(),
        anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
        file_path: "/Contents/existing.flac".into(),
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        bitrate_kbps: None,
        track_number: None,
        bpm: None,
        release_year: None,
        bit_depth: None,
        duration_seconds: Some(180),
        file_type: None,
        isrc: None,
        date_added: None,
        analyze_date: None,
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    });
    data.colors = crate::pdb_writer::standard_colors();
    data.columns_raw_rows = crate::pdb_writer::standard_columns_raw();
    crate::pdb_writer::write_pdb(&data).expect("write seed")
}

/// Corrupt the on-disk track row (id 1) and album row (id 1) to simulate a misaligned export:
/// insert one byte before the UTF-16 string content and bump the recorded offset, breaking
/// 4-byte alignment without changing the decoded content.
fn corrupt_alignment_in_place(bytes: &mut [u8]) {
    use crate::pdb_writer::rewrite_variable_page_rows_in_place;
    use crate::utils::{
        collect_chain, page_offset, read_u16_le_at, read_u32_le_at, table_ptr_fields,
    };
    const PAGE_SIZE: usize = 4096;

    for (table_type, id_offset) in [(0u32, 72usize), (3u32, 12usize)] {
        let (_ec, first, last) =
            table_ptr_fields(bytes, table_type).expect("table pointer present");
        let chain = collect_chain(bytes, PAGE_SIZE, first, last).expect("chain present");
        for page_idx in chain.into_iter().skip(1) {
            let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
            let Some(page_view) = bytes.get(off..off + PAGE_SIZE) else {
                continue;
            };
            let used_s = read_u16_le_at(page_view, 30).unwrap_or(0);
            if used_s == 0 {
                continue;
            }
            let page = page_view.to_vec();
            let mut rows = pdb_page::present_rows(&page);
            let mut any_changed = false;
            for row in rows.iter_mut() {
                if read_u32_le_at(row, id_offset) != Some(1) {
                    continue;
                }
                if table_type == 0 {
                    let off17 = read_u16_le_at(row, 94 + 2 * 17).unwrap() as usize;
                    assert_eq!(row[off17], 0x90);
                    row.insert(off17, 0u8);
                    for index in 17..21usize {
                        let off_pos = 94 + 2 * index;
                        let cur = read_u16_le_at(row, off_pos).unwrap();
                        row[off_pos..off_pos + 2].copy_from_slice(&(cur + 1).to_le_bytes());
                    }
                } else {
                    let name_offset = row[21] as usize;
                    assert_eq!(row[name_offset], 0x90);
                    row.insert(name_offset, 0u8);
                    row[21] = (name_offset + 1) as u8;
                }
                any_changed = true;
            }
            if any_changed {
                assert!(rewrite_variable_page_rows_in_place(
                    bytes, page_idx, &rows, PAGE_SIZE
                ));
            }
        }
    }
}

#[test]
fn detect_pdb_string_misalignment_finds_corrupted_track_and_album_rows() {
    let mut bytes = build_seed_pdb_bytes_for_alignment_test();
    assert!(
        detect_pdb_string_misalignment(&bytes).is_empty(),
        "current writer must produce aligned rows"
    );
    corrupt_alignment_in_place(&mut bytes);

    let td = tempdir().expect("tempdir");
    let usb_root = td.path().to_path_buf();
    let pdb_path = vendor_pdb_path(&usb_root);
    std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
    std::fs::write(&pdb_path, &bytes).expect("write corrupted pdb");

    let found = detect_pdb_string_misalignment(&read_pdb_file(&pdb_path));
    assert!(
        found.iter().any(|m| m.table_type == 0 && m.id == 1),
        "must detect the misaligned track row"
    );
    assert!(
        found.iter().any(|m| m.table_type == 3 && m.id == 1),
        "must detect the misaligned album row"
    );
}

#[test]
fn repair_usb_diagnostics_proposes_and_applies_string_alignment_fixes() {
    let mut bytes = build_seed_pdb_bytes_for_alignment_test();
    corrupt_alignment_in_place(&mut bytes);

    let td = tempdir().expect("tempdir");
    let usb_root = td.path().to_path_buf();
    let pdb_path = vendor_pdb_path(&usb_root);
    std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
    std::fs::write(&pdb_path, &bytes).expect("write corrupted pdb");

    let found_before = detect_pdb_string_misalignment(&read_pdb_file(&pdb_path));
    assert!(!found_before.is_empty());

    let track_ids: Vec<u32> = found_before
        .iter()
        .filter(|m| m.table_type == 0)
        .map(|m| m.id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let album_ids: Vec<u32> = found_before
        .iter()
        .filter(|m| m.table_type == 3)
        .map(|m| m.id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(track_ids, vec![1]);
    assert_eq!(album_ids, vec![1]);

    let track_patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_track_string_alignment_repair(bytes, &track_ids)
    })
    .expect("apply track repair");
    assert_eq!(track_patched, 1);
    let album_patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_album_string_alignment_repair(bytes, &album_ids)
    })
    .expect("apply album repair");
    assert_eq!(album_patched, 1);

    assert!(
        detect_pdb_string_misalignment(&read_pdb_file(&pdb_path)).is_empty(),
        "no misaligned rows should remain after both repairs"
    );

    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse repaired pdb");
    assert_eq!(
        parsed
            .tracks
            .iter()
            .find(|t| t.id == 1)
            .map(|t| t.title.as_str()),
        Some("Café Track")
    );
    assert_eq!(
        parsed.albums.get(&1).map(String::as_str),
        Some("Café Album")
    );
}

/// Replace track 1's comment (string slot 16) with `comment` stored as long ASCII (`0x40`) at a
/// misaligned row offset, as older builds wrote it, shifting the later slot offsets.
fn store_long_ascii_comment_in_place(bytes: &mut [u8], comment: &str) {
    use crate::pdb_writer::rewrite_variable_page_rows_in_place;
    use crate::utils::{
        collect_chain, page_offset, read_u16_le_at, read_u32_le_at, table_ptr_fields,
    };
    const PAGE_SIZE: usize = 4096;
    let (_ec, first, last) = table_ptr_fields(bytes, 0).expect("t00 pointer");
    let chain = collect_chain(bytes, PAGE_SIZE, first, last).expect("t00 chain");
    for page_idx in chain.into_iter().skip(1) {
        let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
        let mut rows = pdb_page::present_rows(&bytes[off..off + PAGE_SIZE]);
        let Some(row) = rows
            .iter_mut()
            .find(|row| read_u32_le_at(row, 72) == Some(1))
        else {
            continue;
        };
        let start = read_u16_le_at(row, 94 + 2 * 16).unwrap() as usize;
        let end = read_u16_le_at(row, 94 + 2 * 17).unwrap() as usize;
        // Misalign the comment (one byte past a 4-byte boundary), as older builds left it.
        let mut encoded = vec![0u8; (5 - start % 4) % 4];
        let comment_at = start + encoded.len();
        assert_eq!(comment_at % 4, 1);
        encoded.push(0x40);
        encoded.extend_from_slice(&((4 + comment.len()) as u16).to_le_bytes());
        encoded.push(0);
        encoded.extend_from_slice(comment.as_bytes());
        // Keep the later (UTF-16) slots 4-byte aligned.
        while (encoded.len() as isize - (end - start) as isize) % 4 != 0 {
            encoded.push(0);
        }
        let delta = encoded.len() as isize - (end - start) as isize;
        row.splice(start..end, encoded);
        row[94 + 2 * 16..94 + 2 * 16 + 2].copy_from_slice(&(comment_at as u16).to_le_bytes());
        for index in 17..21usize {
            let pos = 94 + 2 * index;
            let cur = read_u16_le_at(row, pos).unwrap() as isize;
            row[pos..pos + 2].copy_from_slice(&((cur + delta) as u16).to_le_bytes());
        }
        assert!(rewrite_variable_page_rows_in_place(
            bytes, page_idx, &rows, PAGE_SIZE
        ));
        return;
    }
    panic!("track 1 not found");
}

#[test]
fn repair_realigns_misaligned_long_ascii_track_strings() {
    // A misaligned long ASCII string makes older CDJs give a communication error as soon as the
    // track list opens; the track string repair re-encodes the row with it 4-byte aligned.
    let comment = "x".repeat(200);
    let mut bytes = build_seed_pdb_bytes_for_alignment_test();
    store_long_ascii_comment_in_place(&mut bytes, &comment);

    let td = tempdir().expect("tempdir");
    let usb_root = td.path().to_path_buf();
    let pdb_path = vendor_pdb_path(&usb_root);
    std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
    std::fs::write(&pdb_path, &bytes).expect("write pdb");

    let found = detect_pdb_string_misalignment(&read_pdb_file(&pdb_path));
    assert!(
        found
            .iter()
            .any(|m| m.table_type == 0 && m.id == 1 && m.slot_label == "comment"),
        "{found:?}"
    );
    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_track_string_alignment_repair(
            bytes,
            &[1]
        ))
        .expect("apply track repair"),
        1
    );
    assert!(detect_pdb_string_misalignment(&read_pdb_file(&pdb_path)).is_empty());

    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse repaired pdb");
    let track = parsed.tracks.iter().find(|t| t.id == 1).expect("track 1");
    assert_eq!(track.dj_comment.as_deref(), Some(comment.as_str()));
    assert_eq!(track.title, "Café Track");
}

#[test]
fn diagnostics_fail_misaligned_long_ascii_until_the_user_repairs() {
    // Nothing rewrites misaligned rows on its own: diagnostics shows them as failing, under
    // "Older Pioneer players", so the user knows to run Preview Fixes.
    let (_td, usb_root) = test_usb_root();
    let mut bytes = build_seed_pdb_bytes_for_alignment_test();
    store_long_ascii_comment_in_place(&mut bytes, &"x".repeat(200));
    std::fs::write(vendor_pdb_path(&usb_root), &bytes).expect("write pdb");

    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");
    let report = service
        .run_usb_diagnostics(crate::models::RunUsbDiagnosticsRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("diagnostics");
    let check = report
        .section_groups
        .iter()
        .flat_map(|group| &group.sections)
        .flat_map(|section| &section.checks)
        .find(|check| check.label == "Track/album string alignment")
        .expect("string alignment check");
    assert!(
        matches!(check.status, crate::models::DiagStatus::Fail),
        "{check:?}"
    );
    assert!(check.detail.contains("long ASCII"), "{}", check.detail);
    let older = report
        .overview
        .iter()
        .find(|item| item.title == "Older Pioneer players")
        .expect("older players line");
    assert!(
        matches!(older.status, Some(crate::models::DiagStatus::Fail)),
        "{older:?}"
    );
    assert!(older.detail.contains("Preview Fixes"), "{}", older.detail);
    assert_eq!(std::fs::read(vendor_pdb_path(&usb_root)).unwrap(), bytes);
}

/// Reproduces a real-world report: a track exported by an old (pre-alignment-fix) build was
/// played on a CDJ, which recorded it into a PDB history playlist; the user then removed the
/// track's regular playlist. A later library rewrite orphaned the album's original data page
/// from the live t03 chain (the table's first/last pointers moved on, e.g. because a fresh
/// growth page took over) without touching that page's still-`present` misaligned row.
/// `detect_pdb_string_misalignment` still finds it (it scans every physical page, not just
/// the live chain) and CDJ hardware can still choke on it, but the repair used to walk only
/// the live chain and would silently report success without ever touching the orphaned row.
/// Older writers put a short ASCII artist name at offset 12; Engine DJ's own `export.pdb`
/// import reads it only at 10 (where rekordbox puts it) and shows no artist. The diagnostics
/// flag such rows and the repair moves the name to 10 without changing it.
#[test]
fn artist_name_layout_repair_moves_ascii_name_to_rekordbox_offset() {
    use crate::pdb_writer::rewrite_variable_page_rows_in_place;
    use crate::utils::{collect_chain, read_u32_le_at, table_ptr_fields};
    const PAGE_SIZE: usize = 4096;
    let mut bytes = build_seed_pdb_bytes_for_alignment_test();
    assert!(
        detect_pdb_string_misalignment(&bytes).is_empty(),
        "current writer must put an ASCII artist name at 10"
    );

    // Rewrite the artist row (id 1) the old way: 2 zero bytes before the name, offset 12.
    let (_ec, first, last) = table_ptr_fields(&bytes, 2).expect("t02 pointer present");
    let chain = collect_chain(&bytes, PAGE_SIZE, first, last).expect("t02 chain");
    for page_idx in chain.into_iter().skip(1) {
        let off = page_idx as usize * PAGE_SIZE;
        let page = bytes[off..off + PAGE_SIZE].to_vec();
        let mut rows = pdb_page::present_rows(&page);
        let mut changed = false;
        for row in rows.iter_mut() {
            if read_u32_le_at(row, 4) == Some(1) {
                assert_eq!(row[9], 10);
                row.splice(10..10, [0u8, 0u8]);
                row[9] = 12;
                changed = true;
            }
        }
        if changed {
            assert!(rewrite_variable_page_rows_in_place(
                &mut bytes, page_idx, &rows, PAGE_SIZE
            ));
        }
    }

    let td = tempdir().expect("tempdir");
    let usb_root = td.path().to_path_buf();
    let pdb_path = vendor_pdb_path(&usb_root);
    std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
    std::fs::write(&pdb_path, &bytes).expect("write old-layout pdb");

    let found = detect_pdb_string_misalignment(&read_pdb_file(&pdb_path));
    assert!(
        found
            .iter()
            .any(|m| m.table_type == 2 && m.id == 1 && m.slot_label == "ascii_name_offset"),
        "must flag the ASCII artist name at 12, got {found:?}"
    );

    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_artist_name_layout_repair(bytes, &[1])
    })
    .expect("apply artist repair");
    assert_eq!(patched, 1);
    assert!(
        detect_pdb_string_misalignment(&read_pdb_file(&pdb_path)).is_empty(),
        "no artist row should be flagged after one repair pass"
    );
    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse repaired pdb");
    assert_eq!(
        parsed.artists.get(&1).map(String::as_str),
        Some("Existing Artist")
    );
}

#[test]
fn album_string_alignment_repair_fixes_row_orphaned_from_live_t03_chain() {
    let mut bytes = build_seed_pdb_bytes_for_alignment_test();
    corrupt_alignment_in_place(&mut bytes);
    const PAGE_SIZE: usize = 4096;

    // Orphan the t03 data page (still holding the misaligned album id=1 row) from the live
    // chain by repointing the table's first/last pointers at a freshly-appended, empty page
    // instead -- simulating a later rewrite that grew the table onto a new page without
    // revisiting the old page's content.
    let (ec, first, last) = crate::utils::table_ptr_fields(&bytes, 3).expect("t03 pointer present");
    let new_page = (bytes.len() / PAGE_SIZE) as u32;
    bytes.extend(std::iter::repeat_n(0u8, PAGE_SIZE));
    assert!(
        crate::utils::set_table_ptr_fields(&mut bytes, 3, ec, new_page, new_page),
        "repoint t03 first/last at the fresh empty page"
    );
    // Sanity: the orphaned page is unreachable from the new chain, but the row is still on
    // disk and still flagged by the raw scan.
    assert!(crate::utils::collect_chain(&bytes, PAGE_SIZE, new_page, new_page).is_some());
    assert_ne!(first, new_page);
    assert_ne!(last, new_page);

    let td = tempdir().expect("tempdir");
    let usb_root = td.path().to_path_buf();
    let pdb_path = vendor_pdb_path(&usb_root);
    std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
    std::fs::write(&pdb_path, &bytes).expect("write corrupted pdb");

    let found_before = detect_pdb_string_misalignment(&read_pdb_file(&pdb_path));
    assert!(
        found_before.iter().any(|m| m.table_type == 3 && m.id == 1),
        "raw scan must still find the orphaned misaligned album row"
    );

    let album_patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_album_string_alignment_repair(bytes, &[1])
    })
    .expect("apply album repair");
    assert_eq!(
        album_patched, 1,
        "repair must find and fix the orphaned row, not silently report 0"
    );

    assert!(
        !detect_pdb_string_misalignment(&read_pdb_file(&pdb_path))
            .iter()
            .any(|m| m.table_type == 3),
        "no misaligned album rows should remain after repair, including orphaned ones \
         (the seed's track row was left deliberately uncorrected -- only album alignment \
         is under test here)"
    );
}

#[test]
fn footer_transaction_accepts_rekordbox_transaction_footer() {
    // rekordbox: 7 slots, 2 live, last transaction touched slots 3-6.
    assert!(!playlist_tree_page_flagged(playlist_tree_page(
        7, 4, 3, 0b011_0000, 0b111_1000
    )));
    // rekordbox: transaction bits on deleted slots {0, 3, 5}.
    assert!(!playlist_tree_page_flagged(playlist_tree_page(
        16, 3, 0, 0xfff0, 0b10_1001
    )));
}

#[test]
fn footer_transaction_accepts_fresh_page() {
    // Our fresh page: (trc, 0), every slot live and in the transaction.
    assert!(!playlist_tree_page_flagged(playlist_tree_page(
        6, 6, 0, 0b11_1111, 0b11_1111
    )));
}

#[test]
fn footer_transaction_fixes_forced_trc_footer_from_tranrf() {
    // Old repair forced (trc, 0) onto a rekordbox page with deleted rows.
    assert_eq!(
        fix_playlist_tree_page(playlist_tree_page(7, 7, 0, 0b011_0000, 0b111_1000)),
        (4, 3, 0b111_1000)
    );
}

#[test]
fn footer_transaction_drops_tranrf_copied_from_rowpf() {
    // Old repair: (trc, 0) plus the zero-tranrf fill, which copied rowpf into
    // the groups whose tranrf was 0. Group 0 keeps rekordbox's {0, 3, 5};
    // group 1's copy goes, giving back rekordbox's (3, 0). A row deleted
    // after the fill (slot 16) leaves the copy a superset of rowpf.
    let mut page = playlist_tree_page(20, 20, 0, 0xfff0, 0b10_1001);
    let g1 = TEST_PAGE_SIZE - 36;
    page[g1 - 4..g1 - 2].copy_from_slice(&0b1110u16.to_le_bytes());
    page[g1 - 2..g1].copy_from_slice(&0b1111u16.to_le_bytes());
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), page]);
    let pdb_path = vendor_pdb_path(&usb_root);
    let pages = detect_pdb_inconsistent_page_footers(&read_pdb_file(&pdb_path));
    patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_inconsistent_page_footer_repair(bytes, &pages)
    })
    .unwrap();
    let bytes = std::fs::read(&pdb_path).unwrap();
    assert_eq!(page_u5_num_rl_tranrf(&bytes), (3, 0, 0b10_1001));
    let g1_abs = TEST_PAGE_SIZE + g1;
    assert_eq!(
        u16::from_le_bytes(bytes[g1_abs - 2..g1_abs].try_into().unwrap()),
        0
    );
    assert!(detect_pdb_inconsistent_page_footers(&bytes).is_empty());
}

#[test]
fn footer_transaction_fixes_old_writer_footer_without_tranrf() {
    // The original broken export: (1, 1), no tranrf at all. Every slot
    // becomes the transaction, the fresh-page shape.
    assert_eq!(
        fix_playlist_tree_page(playlist_tree_page(6, 1, 1, 0b11_1111, 0)),
        (6, 0, 0b11_1111)
    );
    assert_eq!(
        patch_pdb_file(
            &write_pdb_pages_as_usb_root(vec![header_page(), data_page(1, 7)]).1,
            |bytes| apply_pdb_inconsistent_page_footer_repair(bytes, &[])
        )
        .unwrap(),
        0
    );
}

#[test]
fn stale_sentinel_btree_repair_rebuilds_missing_index() {
    // One active tt=9 data page plus a sentinel (flags=0x64) page for the
    // same table with an empty (ne=0) B-tree despite the active page
    // existing — the "missing_entries" stale condition.
    let mut data = data_page(1, 9);
    set_flags(&mut data, 0x34);
    let sentinel = data_page(2, 9); // flags default 0 below, override to 0x64
    let mut sentinel = sentinel;
    set_flags(&mut sentinel, 0x64);
    // ne (num_entries) @ +0x38 and u7 (write pointer) @ +0x26 both default 0.
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), data, sentinel]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_stale_sentinel_btree(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_index, 2);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_stale_sentinel_btree_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_stale_sentinel_btree_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let soff = TEST_PAGE_SIZE * 2;
    assert_eq!(
        u16::from_le_bytes(bytes[soff + 0x38..soff + 0x3a].try_into().unwrap()),
        1
    );
    assert_eq!(
        u32::from_le_bytes(bytes[soff + 0x3c..soff + 0x40].try_into().unwrap()),
        8, // page_index(1) * 8
    );
    assert!(detect_pdb_stale_sentinel_btree(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn ec_data_page_conflict_repair_relocates_empty_candidate() {
    // Table B's empty_candidate (ec) points at page 1, which is actually
    // owned by table A — a stale pointer left by an old additive writer.
    let mut header = header_page();
    header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
    let toff = 0x1c;
    header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9 (table B)
    header[toff + 4..toff + 8].copy_from_slice(&1u32.to_le_bytes()); // ec = page 1 (conflict)
    header[toff + 8..toff + 12].copy_from_slice(&10u32.to_le_bytes()); // fp
    header[toff + 12..toff + 16].copy_from_slice(&1u32.to_le_bytes()); // lp = page 1 (fp != lp)
    let mut owned_by_a = data_page(1, 5); // table A's real data page
    set_flags(&mut owned_by_a, 0x34);
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header, owned_by_a]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let conflicts = detect_pdb_ec_data_page_conflicts(&read_pdb_file(&pdb_path));
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].table_type, 9);
    assert_eq!(conflicts[0].last_page, 1);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_ec_data_page_conflict_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_ec_data_page_conflict_repair(bytes, &conflicts)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let new_ec = u32::from_le_bytes(bytes[toff + 4..toff + 8].try_into().unwrap());
    assert_eq!(
        new_ec, 2,
        "ec should be relocated past the physical end of the file"
    );
    // The (former) conflicting page's next_page must be repointed too.
    let p1_next = u32::from_le_bytes(
        bytes[TEST_PAGE_SIZE + 0x0c..TEST_PAGE_SIZE + 0x10]
            .try_into()
            .unwrap(),
    );
    assert_eq!(p1_next, new_ec);
    assert!(detect_pdb_ec_data_page_conflicts(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn truncated_table_chain_repair_restores_last_and_ec_to_physical_reality() {
    // Mirrors an additive-growth write interrupted after the table
    // pointer was advanced to claim page 3 as the new last page, but
    // before page 3 was ever flushed to disk: the real last page (2)
    // already has its `next` field pointing at 3, anticipating the
    // growth that never landed.
    let mut header = header_page();
    header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
    let toff = 0x1c;
    header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9
    header[toff + 4..toff + 8].copy_from_slice(&4u32.to_le_bytes()); // ec = 4 (nonexistent)
    header[toff + 8..toff + 12].copy_from_slice(&1u32.to_le_bytes()); // fp = 1
    header[toff + 12..toff + 16].copy_from_slice(&3u32.to_le_bytes()); // lp = 3 (nonexistent)

    let mut p1 = data_page(1, 9);
    set_flags(&mut p1, 0x34);
    set_next_page(&mut p1, 2);
    let mut p2 = data_page(2, 9); // real last page; already points at the growth that never landed
    set_flags(&mut p2, 0x34);
    set_next_page(&mut p2, 3);

    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header, p1, p2]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let fixes = detect_pdb_truncated_table_chains(&read_pdb_file(&pdb_path));
    assert_eq!(fixes.len(), 1);
    assert_eq!(fixes[0].table_type, 9);
    assert_eq!(fixes[0].corrected_last, 2);
    assert_eq!(fixes[0].corrected_ec, 3);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_truncated_table_chain_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_truncated_table_chain_repair(bytes, &fixes)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let new_ec = u32::from_le_bytes(bytes[toff + 4..toff + 8].try_into().unwrap());
    let new_last = u32::from_le_bytes(bytes[toff + 12..toff + 16].try_into().unwrap());
    assert_eq!(new_ec, 3);
    assert_eq!(new_last, 2);
    // Page 2's own content is untouched — its `next` already matched.
    let p2_next = u32::from_le_bytes(
        bytes[2 * TEST_PAGE_SIZE + 0x0c..2 * TEST_PAGE_SIZE + 0x10]
            .try_into()
            .unwrap(),
    );
    assert_eq!(p2_next, 3);
    assert!(
        crate::utils::collect_chain(&bytes, TEST_PAGE_SIZE, 1, 2).is_some(),
        "chain must now walk cleanly to the corrected last page"
    );
    assert!(detect_pdb_truncated_table_chains(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn truncated_table_chain_detected_past_trailing_blank_reserve_page() {
    // An interrupted additive-growth write linked an all-zero reserve page
    // (3) into the chain tail *and* advanced the header's
    // last/empty_candidate pointers even further ahead (past EOF). The real
    // last live page (2) already points its `next` at page 3.
    let mut header = header_page();
    header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
    let toff = 0x1c;
    header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9
    header[toff + 4..toff + 8].copy_from_slice(&5u32.to_le_bytes()); // ec = 5 (nonexistent)
    header[toff + 8..toff + 12].copy_from_slice(&1u32.to_le_bytes()); // fp = 1
    header[toff + 12..toff + 16].copy_from_slice(&4u32.to_le_bytes()); // lp = 4 (nonexistent)

    let mut p1 = data_page(1, 9);
    set_flags(&mut p1, 0x34);
    set_next_page(&mut p1, 2);
    let mut p2 = data_page(2, 9); // real last live page
    set_flags(&mut p2, 0x34);
    set_next_page(&mut p2, 3);
    // page 3: blank in-file reserve page linked into the chain tail
    let mut p3 = empty_page();
    p3[4..8].copy_from_slice(&3u32.to_le_bytes());
    p3[8..12].copy_from_slice(&9u32.to_le_bytes());
    // used_s stays 0; next stays 0

    // 4 physical pages (0..3): page 3 is in-bounds, page 4 (declared last) is not.
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header, p1, p2, p3]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let fixes = detect_pdb_truncated_table_chains(&read_pdb_file(&pdb_path));
    assert_eq!(
        fixes.len(),
        1,
        "trailing blank page must not defeat detection"
    );
    assert_eq!(fixes[0].table_type, 9);
    assert_eq!(
        fixes[0].corrected_last, 2,
        "anchor on the last live page, not the trailing blank reserve page"
    );
    assert_eq!(
        fixes[0].corrected_ec, 3,
        "empty_candidate == real last page's own next (the blank reserve page)"
    );

    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_truncated_table_chain_repair(bytes, &fixes)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let new_ec = u32::from_le_bytes(bytes[toff + 4..toff + 8].try_into().unwrap());
    let new_last = u32::from_le_bytes(bytes[toff + 12..toff + 16].try_into().unwrap());
    assert_eq!(new_ec, 3);
    assert_eq!(new_last, 2);
    assert!(
        crate::utils::collect_chain(&bytes, TEST_PAGE_SIZE, 1, 2).is_some(),
        "chain must now walk cleanly to the corrected last page"
    );
    assert!(detect_pdb_truncated_table_chains(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn torn_growth_repair_zeroes_garbage_ec_page_and_truncates_tail() {
    // Mirrors a USB disconnected mid-additive-growth-write: table tt=9's
    // empty_candidate (page 2) was earmarked as the next blank slot but
    // never got zeroed/written, and the file was already extended past
    // next_unused_page (3) with an unpopulated, non-zero tail (page 3).
    let mut header = header_page();
    header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
    header[0x0c..0x10].copy_from_slice(&3u32.to_le_bytes()); // next_unused_page = 3
    let toff = 0x1c;
    header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9
    header[toff + 4..toff + 8].copy_from_slice(&2u32.to_le_bytes()); // ec = page 2 (garbage)
    header[toff + 8..toff + 12].copy_from_slice(&1u32.to_le_bytes()); // fp = 1
    header[toff + 12..toff + 16].copy_from_slice(&1u32.to_le_bytes()); // lp = 1

    let mut p1 = data_page(1, 9); // table tt=9's only real data page
    set_flags(&mut p1, 0x34);
    let garbage_ec = vec![0xABu8; TEST_PAGE_SIZE]; // page 2: torn ec target
    let garbage_tail = vec![0xCDu8; TEST_PAGE_SIZE]; // page 3: never-populated tail

    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header, p1, garbage_ec, garbage_tail]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let report = detect_pdb_torn_growth_pages(&read_pdb_file(&pdb_path));
    assert_eq!(report.garbage_ec_pages, vec![2]);
    assert_eq!(report.truncate_to_pages, Some(3));

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_torn_growth_pages_repair(
            bytes,
            &PdbTornGrowthReport::default()
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_torn_growth_pages_repair(bytes, &report)
    })
    .unwrap();
    assert_eq!(patched, 1, "one garbage ec page zeroed");

    let bytes = std::fs::read(&pdb_path).unwrap();
    assert_eq!(
        bytes.len(),
        3 * TEST_PAGE_SIZE,
        "never-populated tail must be truncated away"
    );
    let ec_off = 2 * TEST_PAGE_SIZE;
    assert!(
        bytes[ec_off..ec_off + TEST_PAGE_SIZE]
            .iter()
            .all(|b| *b == 0),
        "the reclaimed ec page must be all-zero"
    );
    let seqdb = u32::from_le_bytes(bytes[0x14..0x18].try_into().unwrap());
    let max_seq = crate::pdb_writer::max_seqpage_in_file(&bytes, TEST_PAGE_SIZE);
    assert!(
        seqdb > max_seq,
        "seqdb must stay strictly greater than max seqpage"
    );
    assert!(detect_pdb_torn_growth_pages(&read_pdb_file(&pdb_path)).is_empty());

    // The reclaimed ec page must now satisfy the additive writer's own
    // "all zero" reuse precondition (pdb_writer's
    // append_rows_to_chain_in_place), proving a follow-up rekordbox
    // export would succeed instead of hitting the same rejection again.
    let page = &bytes[ec_off..ec_off + TEST_PAGE_SIZE];
    assert!(page.iter().all(|b| *b == 0));
}

#[test]
fn apply_fix_sync_edb_history_from_pdb_overwrites_stale_edb_rows() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    // A named history playlist with one valid (non-zero track_id) entry
    // and one invalid entry (track_id=0, must be dropped) — mirrors
    // derive_history_sync_payload's own filtering rules.
    let parsed = ParsedPdb {
        tracks: vec![crate::pdb_reader::PdbTrackRow {
            id: 101,
            track_file_path: "/Contents/Artist/Track.mp3".to_string(),
            ..Default::default()
        }],
        history_playlists: vec![PdbHistoryPlaylistRow {
            id: 10,
            name: "HISTORY 001".to_string(),
            source_table: 17,
        }],
        history_entries: vec![
            PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 10,
                entry_index: 1,
                source_table: 18,
            },
            PdbHistoryEntryRow {
                track_id: Some(0),
                playlist_id: 10,
                entry_index: 2,
                source_table: 18,
            },
        ],
        ..ParsedPdb::default()
    };

    // Seed the eDB with stale history rows that must be replaced.
    {
        let mut conn = open_edb_rw(&usb_root).expect("open eDB rw");
        let tx = conn.transaction().expect("begin tx");
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer); \
             CREATE TABLE IF NOT EXISTS history_content(history_id integer, content_id integer, sequenceNo integer); \
             DELETE FROM history; DELETE FROM history_content; \
             INSERT INTO history(history_id, sequenceNo, name, attribute, history_id_parent) VALUES (999, 5, 'Stale', 0, 0); \
             INSERT INTO history_content(history_id, content_id, sequenceNo) VALUES (999, 555, 1), (999, 556, 2); \
             INSERT INTO content(content_id, path) VALUES (7, '/Contents/Artist/Track.mp3');",
        )
        .expect("seed stale history rows");
        tx.commit().expect("commit seed");
    }

    let (history_written, history_content_written) = service
        .apply_fix_sync_edb_history_from_pdb(&usb_root, &parsed, None)
        .expect("sync edb history");
    assert_eq!(history_written, 1);
    assert_eq!(
        history_content_written, 1,
        "the track_id=0 entry must be dropped"
    );

    let conn = open_edb_rw(&usb_root).expect("re-open eDB rw");
    let (hid, name): (i64, String) = conn
        .query_row("SELECT history_id, name FROM history", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("history row");
    assert_eq!(hid, 10);
    assert_eq!(name, "HISTORY 001");
    let (content_hid, content_id): (i64, i64) = conn
        .query_row(
            "SELECT history_id, content_id FROM history_content",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("history_content row");
    assert_eq!(content_hid, 10);
    assert_eq!(
        content_id, 7,
        "the eDB row holds the eDB's id for the track"
    );
}

#[test]
fn apply_fix_empty_analysis_files_bakes_real_pdb_tempo_not_120_default() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    let audio_path = usb_root.join("Contents/track.wav");
    std::fs::create_dir_all(audio_path.parent().unwrap()).unwrap();
    write_sine_wav(&audio_path, 20_000);

    let anlz_dir = usb_root.join("PIONEER/USBANLZ/P001/00000001");
    std::fs::create_dir_all(&anlz_dir).unwrap();
    let dat_path = anlz_dir.join("ANLZ0000.DAT");
    std::fs::write(&dat_path, b"").unwrap(); // the "empty analysis file" needing repair

    let parsed = ParsedPdb {
        tracks: vec![PdbTrackRow {
            header_flags: 0,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id: 1,
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
            tempo_x100: 14_000, // the track's real, already-known tempo
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(222),
            file_type: None,
            isrc: None,
            date_added: None,
            analyze_date: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: "Track".to_string(),
            anlz_path: "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".to_string(),
            track_file_path: "/Contents/track.wav".to_string(),
        }],
        ..ParsedPdb::default()
    };

    let empty_path =
        resolve_usb_side_path(&usb_root, "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT")
            .expect("resolve anlz path");

    let mut warnings = Vec::new();
    let (fixed, skipped, failed, _writes) = service
        .apply_fix_empty_analysis_files(
            &usb_root,
            &[empty_path],
            None,
            Some(&parsed),
            &mut warnings,
        )
        .expect("apply fix_empty_analysis_files");
    assert_eq!((fixed, skipped, failed), (1, 0, 0));

    let dat_bytes = std::fs::read(&dat_path).expect("regenerated .DAT");
    assert_eq!(
        read_beatgrid_tempo_from_anlz(&dat_bytes),
        Some(14_000),
        "regenerated bundle must bake the track's real PDB tempo"
    );
    // Drawn by the analysis pipeline: the first beat placed from the audio,
    // the detail waveform one entry per 1/150 s of the whole track.
    assert!(read_first_beat_from_anlz(&dat_bytes).is_some());
    let ext = std::fs::read(dat_path.with_extension("EXT")).expect("regenerated .EXT");
    let pwv3 = crate::service::anlz::anlz_chunk_payload(&ext, b"PWV3").expect("PWV3");
    assert_eq!(
        pwv3.len(),
        crate::service::analysis::waveform_detail_entries_for_duration(20_000)
    );
}
#[test]
fn apply_fix_empty_analysis_files_keeps_folders_with_the_same_letters_apart() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    // `P001/00000001` and `P0010/0000001` differ only in where the `/` is.
    let track = |id: u32, tempo_x100: u32, anlz_dir: &str, audio: &str| {
        let audio_path = usb_root.join("Contents").join(audio);
        std::fs::create_dir_all(audio_path.parent().unwrap()).unwrap();
        write_sine_wav(&audio_path, 1_000);
        std::fs::create_dir_all(usb_root.join("PIONEER/USBANLZ").join(anlz_dir)).unwrap();
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
            tempo_x100,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(222),
            file_type: None,
            isrc: None,
            date_added: None,
            analyze_date: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: audio.to_string(),
            anlz_path: format!("/PIONEER/USBANLZ/{anlz_dir}/ANLZ0000.DAT"),
            track_file_path: format!("/Contents/{audio}"),
        }
    };
    let parsed = ParsedPdb {
        tracks: vec![
            track(1, 14_000, "P001/00000001", "a.wav"),
            track(2, 9_000, "P0010/0000001", "b.wav"),
        ],
        ..ParsedPdb::default()
    };
    let dat_path = usb_root.join("PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT");
    std::fs::write(&dat_path, b"").unwrap();
    let empty_path =
        resolve_usb_side_path(&usb_root, "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT")
            .expect("resolve anlz path");

    let mut warnings = Vec::new();
    let (fixed, skipped, failed, _writes) = service
        .apply_fix_empty_analysis_files(
            &usb_root,
            &[empty_path],
            None,
            Some(&parsed),
            &mut warnings,
        )
        .expect("apply fix_empty_analysis_files");
    assert_eq!((fixed, skipped, failed), (1, 0, 0));

    let dat_bytes = std::fs::read(&dat_path).expect("regenerated .DAT");
    assert_eq!(
        read_beatgrid_tempo_from_anlz(&dat_bytes),
        Some(14_000),
        "the empty file's own track must be used, not the other folder's"
    );
    assert!(
        !usb_root
            .join("PIONEER/USBANLZ/P0010/0000001/ANLZ0000.DAT")
            .exists(),
        "nothing may be written into the other track's folder"
    );
}

#[test]
fn apply_fix_empty_analysis_files_fails_when_source_audio_cannot_be_decoded() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    let audio_path = usb_root.join("Contents/track.mp3");
    std::fs::create_dir_all(audio_path.parent().unwrap()).unwrap();
    std::fs::write(&audio_path, vec![7u8; 2000]).unwrap(); // not audio

    let anlz_dir = usb_root.join("PIONEER/USBANLZ/P001/00000001");
    std::fs::create_dir_all(&anlz_dir).unwrap();
    let dat_path = anlz_dir.join("ANLZ0000.DAT");
    std::fs::write(&dat_path, b"").unwrap();

    let parsed = ParsedPdb {
        tracks: vec![PdbTrackRow {
            header_flags: 0,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id: 1,
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
            tempo_x100: 14_000,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(222),
            file_type: None,
            isrc: None,
            date_added: None,
            analyze_date: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: "Track".to_string(),
            anlz_path: "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".to_string(),
            track_file_path: "/Contents/track.mp3".to_string(),
        }],
        ..ParsedPdb::default()
    };

    let empty_path =
        resolve_usb_side_path(&usb_root, "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT")
            .expect("resolve anlz path");

    let mut warnings = Vec::new();
    let (fixed, skipped, failed, _writes) = service
        .apply_fix_empty_analysis_files(
            &usb_root,
            &[empty_path],
            None,
            Some(&parsed),
            &mut warnings,
        )
        .expect("apply fix_empty_analysis_files");
    assert_eq!((fixed, skipped, failed), (0, 0, 1));
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "usb.repair.empty-analysis.analyze-failed"),
        "undecodable source must be reported as failed: {warnings:?}"
    );
    assert!(
        std::fs::read(&dat_path).expect("dat").is_empty(),
        "no made-up waveform may be written"
    );
}

/// A mono 16-bit 44.1 kHz WAV of a 440 Hz sine, `duration_ms` long.
fn write_sine_wav(path: &Path, duration_ms: u32) {
    let sample_rate: u32 = 44_100;
    let samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let data_len = (samples * 2) as u32;
    let mut out = Vec::<u8>::with_capacity(44 + samples * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..samples {
        let t = i as f32 / sample_rate as f32;
        let v = ((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.25 * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out).expect("write wav");
}

fn history_test_track(id: u32, path: &str) -> PdbTrackRow {
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
        date_added: None,
        analyze_date: None,
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info: None,
        autoload_hotcues: None,
        title: "Track".to_string(),
        anlz_path: String::new(),
        track_file_path: path.to_string(),
    }
}

#[test]
fn missing_audio_history_keys_cover_pdb_history_tracks_only() {
    let parsed = ParsedPdb {
        tracks: vec![
            history_test_track(1, "/Contents/Artist/Played.mp3"),
            history_test_track(2, "/Contents/Artist/Never Played.mp3"),
        ],
        history_entries: vec![PdbHistoryEntryRow {
            track_id: Some(1),
            playlist_id: 10,
            entry_index: 1,
            source_table: 18,
        }],
        ..ParsedPdb::default()
    };
    let keys = missing_audio_history_keys(&parsed, None);
    assert!(keys.contains(&audio_path_key("/Contents/ARTIST/Played.mp3")));
    assert!(!keys.contains(&audio_path_key("/Contents/Artist/Never Played.mp3")));
}

fn proposal(id: &str, supported: bool) -> RepairFixProposal {
    RepairFixProposal {
        id: id.to_string(),
        title: id.to_string(),
        description: String::new(),
        supported,
        destructive: false,
        estimated_writes: 0,
        estimated_deletes: 0,
        always_applied: fix_always_applied(id),
        bookkeeping: fix_is_bookkeeping(id),
        link: None,
        name_input: None,
    }
}

#[test]
fn every_shown_fix_has_a_plain_language_overview_line() {
    for id in REPAIR_FIX_DISPLAY_ORDER {
        if !fix_is_bookkeeping(id) {
            let (summary, items) = repair_overview(&[proposal(id, true)], &[]);
            assert!(summary.contains("1 problem "), "{id}: {summary}");
            assert!(
                !items[0].detail.contains("other errors"),
                "{id} has no overview line"
            );
        }
    }
}

#[test]
fn repair_overview_groups_fixes_by_area_and_flags_what_it_cant_fix() {
    let (summary, items) = repair_overview(&[], &[]);
    assert_eq!(summary, "No issues found.");
    assert!(items.is_empty());

    let fixes = [
        proposal(PDB_TRUNCATED_TABLE_CHAIN_FIX_ID, true),
        proposal(PDB_TORN_GROWTH_PAGES_FIX_ID, true),
        proposal(PDB_STALE_SENTINEL_BTREE_FIX_ID, true),
        proposal(BPM_KEY_MISMATCH_FIX_ID, true),
        proposal(ENGINE_LIBRARY_FIX_ID, true),
    ];
    let unsupported = [RepairUnsupportedItem {
        issue: "2 malformed USBANLZ entries".to_string(),
        reason: "Inspect the Event Log.".to_string(),
        link: None,
    }];
    let (summary, items) = repair_overview(&fixes, &unsupported);
    // Bookkeeping is neither counted nor shown.
    assert_eq!(
        summary,
        "Apply Fixes repairs 4 problems on this USB. The databases are backed up first."
    );
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Older Pioneer players",
            "Waveforms & beat grids",
            "Denon / Numark players",
            "Needs your attention",
        ]
    );
    // Two fixes with the same plain-language line show it once.
    assert_eq!(
        items[0].detail,
        "Repairs damage left in the player database by an interrupted export."
    );
    assert!(matches!(items[3].status, Some(DiagStatus::Fail)));
    assert!(items[3].detail.starts_with("1 problem can't be fixed"));

    let (summary, _) = repair_overview(&[], &unsupported);
    assert_eq!(summary, "Nothing here can be fixed automatically.");
}

#[test]
fn repair_rows_are_ready_to_show() {
    let fixes = [
        proposal(BPM_KEY_MISMATCH_FIX_ID, true),
        RepairFixProposal {
            always_applied: true,
            ..proposal("always", true)
        },
        RepairFixProposal {
            bookkeeping: true,
            ..proposal("bookkeeping", true)
        },
        proposal("manual", false),
    ];
    let unsupported = [RepairUnsupportedItem {
        issue: "2 malformed entries".to_string(),
        reason: "Inspect the Event Log.".to_string(),
        link: Some("event-log".to_string()),
    }];

    let rows = run::repair_rows(&fixes, &unsupported);
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            BPM_KEY_MISMATCH_FIX_ID,
            "always",
            "manual",
            "unsupported:2 malformed entries"
        ]
    );

    assert!(rows[0].selectable && rows[0].checked && !rows[0].locked);
    assert!(rows[0].locked_tooltip.is_none());
    assert!(rows[1].selectable && rows[1].checked && rows[1].locked);
    assert!(rows[1].locked_tooltip.is_some());
    assert!(!rows[2].selectable && !rows[2].checked && !rows[2].locked);
    assert!(!rows[3].selectable);
    assert_eq!(rows[3].meta_text, "✗ preview-only · safe");
    assert_eq!(rows[3].link.as_deref(), Some("event-log"));
}

#[test]
fn damaged_track_rows_pair_only_with_one_intact_twin_and_keep_the_played_or_earlier_row() {
    let track = |id: u32, path: &str, size: u32| crate::pdb_reader::PdbTrackRow {
        id,
        track_file_path: path.to_string(),
        title: "Some Title".to_string(),
        file_size_bytes: Some(size),
        duration_seconds: Some(300),
        ..Default::default()
    };
    let parsed = ParsedPdb {
        tracks: vec![
            // NUL runs in the path; one intact row for the same file. The
            // damaged row is the earlier one, so it stays.
            track(1, "/Contents/Artist A/Song\0\0ne.mp3", 1000),
            track(2, "/Contents/Artist A/Song One.mp3", 1000),
            // Damaged, but two intact rows fit: no merge.
            track(3, "/Contents/Artist B/Tw\0\0.mp3", 2000),
            track(4, "/Contents/Artist B/Two.mp3", 2000),
            track(5, "/Contents/Artist B/Twa.mp3", 2000),
            // A plain missing file without damage is not this fix's business.
            track(6, "/Contents/Artist C/Gone.mp3", 1000),
            // The intact row is the earlier one: the damaged one goes.
            track(7, "/Contents/Artist D/Four.mp3", 3000),
            track(8, "/Contents/Artist D/F\0\0r.mp3", 3000),
            // Same size and duration, but the name doesn't fit: no twin.
            track(9, "/Contents/Artist E/Fiv\0\0mp3", 4000),
            track(10, "/Contents/Artist E/Other.mp3", 4000),
            // The newer row is in a play history: it stays, though the
            // damaged one is earlier.
            track(11, "/Contents/Artist F/S\0\0.mp3", 5000),
            track(12, "/Contents/Artist F/Six.mp3", 5000),
        ],
        history_entries: vec![crate::pdb_reader::PdbHistoryEntryRow {
            track_id: Some(12),
            playlist_id: 1,
            entry_index: 1,
            source_table: 12,
        }],
        ..ParsedPdb::default()
    };
    let missing_keys: HashSet<String> = [
        "/Contents/Artist A/Song\0\0ne.mp3",
        "/Contents/Artist B/Tw\0\0.mp3",
        "/Contents/Artist C/Gone.mp3",
        "/Contents/Artist D/F\0\0r.mp3",
        "/Contents/Artist E/Fiv\0\0mp3",
        "/Contents/Artist F/S\0\0.mp3",
    ]
    .iter()
    .map(|p| audio_path_key(p))
    .collect();

    let found = detect_nul_damaged_tracks(&parsed, None, &missing_keys);
    assert_eq!(found.pdb_twins, HashMap::from([(2, 1), (8, 7), (11, 12)]));
    assert!(found.edb_twins.is_empty());
    assert_eq!(
        found.merged_paths,
        vec![
            "/Contents/Artist A/Song\0\0ne.mp3".to_string(),
            "/Contents/Artist D/F\0\0r.mp3".to_string(),
            "/Contents/Artist F/S\0\0.mp3".to_string(),
        ]
    );
    assert_eq!(
        found.without_twin,
        vec![
            "/Contents/Artist B/Tw\0\0.mp3".to_string(),
            "/Contents/Artist E/Fiv\0\0mp3".to_string(),
        ]
    );
}

#[test]
fn unindexed_playlist_name_is_the_typed_name_or_the_default() {
    let name =
        |typed: Option<&str>| run::unindexed_playlist_name(typed.map(str::to_string).as_ref());
    assert_eq!(name(None), "Unindexed");
    assert_eq!(name(Some("")), "Unindexed");
    assert_eq!(name(Some("   ")), "Unindexed");
    assert_eq!(name(Some("\0")), "Unindexed");
    assert_eq!(name(Some("  My Strays ")), "My Strays");
}

/// A PDB with folders in rekordbox's shape: two style folders, each with an
/// "Opening" folder holding a "Warmup" playlist of its own track (same
/// names, different tracks), an empty folder and a top-level playlist.
fn write_folder_tree_pdb(usb_root: &Path) {
    use crate::pdb_writer::{PdbArtistRow, PdbData, PdbPlaylistEntryRow, PdbPlaylistTreeRow};
    let mut data = PdbData::empty();
    data.colors = crate::pdb_writer::standard_colors();
    data.columns_raw_rows = crate::pdb_writer::standard_columns_raw();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Fixture Artist".into(),
    });
    for id in 1..=3 {
        data.tracks.push(PdbTrackRowData {
            composer_id: 0,
            label_id: 0,
            remixer_id: 0,
            header_flags_u32: None,
            id,
            artist_id: 1,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            title: format!("Track {id}"),
            anlz_path: format!("/PIONEER/USBANLZ/P000/0000000{id}/ANLZ0000.DAT"),
            file_path: format!("/Contents/Fixture Artist/Track {id}.mp3"),
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            bitrate_kbps: None,
            track_number: None,
            bpm: None,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(180),
            file_type: None,
            isrc: None,
            date_added: None,
            analyze_date: None,
            release_date: None,
            dj_comment: None,
            file_name: Some(format!("Track {id}.mp3")),
            publish_track_info_on: None,
            autoload_hotcues_on: None,
            color_id: None,
            rating: None,
        });
    }
    let row = |id, parent_id, sort_order, is_folder, name: &str| PdbPlaylistTreeRow {
        id,
        parent_id,
        sort_order,
        is_folder,
        name: name.into(),
    };
    // Sort orders with gaps, as rekordbox leaves them in the PDB.
    data.playlist_tree = vec![
        row(1, 0, 0, true, "Style A"),
        row(2, 1, 0, true, "Opening"),
        row(3, 2, 0, false, "Warmup"),
        row(4, 0, 2, true, "Style B"),
        row(5, 4, 0, true, "Opening"),
        row(6, 5, 0, false, "Warmup"),
        row(7, 0, 4, true, "Empty"),
        row(8, 0, 6, false, "Top"),
    ];
    data.playlist_entries = [(3, 1), (6, 2), (8, 3)]
        .map(|(playlist_id, track_id)| PdbPlaylistEntryRow {
            entry_index: 1,
            track_id,
            playlist_id,
        })
        .to_vec();
    std::fs::write(
        vendor_pdb_path(usb_root),
        crate::pdb_writer::write_pdb(&data).expect("write PDB"),
    )
    .expect("store PDB");
}

/// Every folder and playlist of a playlist tree, by path.
fn tree_paths(
    rows: &[crate::pdb_reader::PdbPlaylistTreeRow],
) -> std::collections::BTreeSet<(String, bool)> {
    let tree = crate::pdb_reader::PlaylistTree::new(rows);
    rows.iter()
        .map(|row| (tree.path(row).display(), row.is_folder))
        .collect()
}

fn edb_tree_of(usb_root: &Path) -> Vec<crate::pdb_reader::PdbPlaylistTreeRow> {
    let conn = staging::UsbDatabases::of(usb_root)
        .expect("databases")
        .open_edb_writable()
        .expect("open eDB");
    crate::edb::read_edb_playlist_tree(&conn).expect("eDB tree")
}

fn pdb_tree_of(usb_root: &Path) -> Vec<crate::pdb_reader::PdbPlaylistTreeRow> {
    crate::pdb_reader::parse_pdb(&vendor_pdb_path(usb_root))
        .expect("parse PDB")
        .playlist_tree
}

/// The eDB playlist at `path` ("Folder / Name"): its tracks' file paths.
fn edb_playlist_tracks(usb_root: &Path, path: &str) -> Vec<String> {
    let conn = staging::UsbDatabases::of(usb_root)
        .expect("databases")
        .open_edb_writable()
        .expect("open eDB");
    let library = crate::edb::read_edb(
        &conn,
        usb_root,
        crate::edb::EdbRead {
            playlists: true,
            ..crate::edb::EdbRead::default()
        },
    )
    .expect("read eDB");
    library
        .playlists
        .get(path)
        .unwrap_or_else(|| panic!("no eDB playlist {path}: {:?}", library.playlists.keys()))
        .tracks
        .iter()
        .map(|track| track.usb_media_path.clone().unwrap_or_default())
        .collect()
}

fn apply_strict_parity(service: &BackendService, usb_root: &Path) {
    let data = service
        .repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
            fix_names: Default::default(),
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec![STRICT_PARITY_UPGRADE_FIX_ID.to_string()],
        })
        .expect("repair apply");
    assert!(
        data.applied_fixes
            .iter()
            .any(|line| line.contains("Strict Parity")),
        "{data:?}"
    );
    assert!(data.failed_fixes.is_empty(), "{data:?}");
}

/// The folder-tree stick with its eDB rebuilt from the PDB.
fn folder_tree_stick_with_rebuilt_edb() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    tempfile::TempDir,
    BackendService,
) {
    let (td, usb_root) = test_usb_root();
    write_folder_tree_pdb(&usb_root);
    let edb = crate::service::usb::vendor_compat::vendor_edb_path(&usb_root);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", edb.display()));
    }
    let data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(data_dir.path()).expect("backend service");
    apply_strict_parity(&service, &usb_root);
    (td, usb_root, data_dir, service)
}

#[test]
fn rebuilt_edb_has_the_pdb_folders_and_same_named_playlists() {
    let (_td, usb_root, _data_dir, _service) = folder_tree_stick_with_rebuilt_edb();

    let edb = tree_paths(&edb_tree_of(&usb_root));
    assert_eq!(edb, tree_paths(&pdb_tree_of(&usb_root)));
    for expected in [
        ("Style A / Opening / Warmup", false),
        ("Style B / Opening / Warmup", false),
        ("Empty", true),
        ("Top", false),
    ] {
        assert!(
            edb.contains(&(expected.0.to_string(), expected.1)),
            "{expected:?} missing from {edb:?}"
        );
    }
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Style A / Opening / Warmup"),
        ["/Contents/Fixture Artist/Track 1.mp3"]
    );
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Style B / Opening / Warmup"),
        ["/Contents/Fixture Artist/Track 2.mp3"]
    );
    // Each folder numbered 0..n, folders and playlists together, in the
    // PDB's order.
    let mut top = edb_tree_of(&usb_root)
        .into_iter()
        .filter(|row| row.parent_id == 0)
        .map(|row| (row.sort_order, row.name))
        .collect::<Vec<_>>();
    top.sort();
    assert_eq!(
        top,
        [
            (0, "Style A".to_string()),
            (1, "Style B".to_string()),
            (2, "Empty".to_string()),
            (3, "Top".to_string()),
        ]
    );
}

#[test]
fn strict_parity_keeps_same_named_playlists_in_other_folders() {
    let (_td, usb_root, _data_dir, service) = folder_tree_stick_with_rebuilt_edb();
    let tree_before = tree_paths(&edb_tree_of(&usb_root));
    // Style B's Warmup loses its track in the eDB: the repair puts it back
    // without touching Style A's.
    {
        let edb_rows = edb_tree_of(&usb_root);
        let tree = crate::pdb_reader::PlaylistTree::new(&edb_rows);
        let style_b = edb_rows
            .iter()
            .find(|row| tree.path(row).display() == "Style B / Opening / Warmup")
            .expect("Style B Warmup");
        let conn = staging::UsbDatabases::of(&usb_root)
            .expect("databases")
            .open_edb_writable()
            .expect("open eDB");
        conn.execute(
            "DELETE FROM playlist_content WHERE playlist_id = ?1",
            rusqlite::params![style_b.id],
        )
        .expect("empty Style B Warmup");
    }

    apply_strict_parity(&service, &usb_root);

    assert_eq!(tree_paths(&edb_tree_of(&usb_root)), tree_before);
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Style A / Opening / Warmup"),
        ["/Contents/Fixture Artist/Track 1.mp3"]
    );
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Style B / Opening / Warmup"),
        ["/Contents/Fixture Artist/Track 2.mp3"]
    );
}

#[test]
fn rebuilt_pdb_has_the_edb_folders() {
    let (_td, usb_root, _data_dir, service) = folder_tree_stick_with_rebuilt_edb();
    let edb = tree_paths(&edb_tree_of(&usb_root));
    std::fs::remove_file(vendor_pdb_path(&usb_root)).expect("delete PDB");

    apply_strict_parity(&service, &usb_root);

    assert_eq!(tree_paths(&pdb_tree_of(&usb_root)), edb);
}

#[test]
fn strict_parity_replaces_flattened_edb_copies_with_the_pdb_folders() {
    let (_td, usb_root, _data_dir, service) = folder_tree_stick_with_rebuilt_edb();
    let pdb = tree_paths(&pdb_tree_of(&usb_root));
    // The eDB as earlier versions' repair left it: no folders, and the two
    // Warmups merged into one top-level playlist holding both tracks.
    {
        let edb_rows = edb_tree_of(&usb_root);
        let tree = crate::pdb_reader::PlaylistTree::new(&edb_rows);
        let id_of = |path: &str| {
            edb_rows
                .iter()
                .find(|row| tree.path(row).display() == path)
                .map(|row| row.id)
                .expect("eDB playlist")
        };
        let (a, b) = (
            id_of("Style A / Opening / Warmup"),
            id_of("Style B / Opening / Warmup"),
        );
        let conn = staging::UsbDatabases::of(&usb_root)
            .expect("databases")
            .open_edb_writable()
            .expect("open eDB");
        conn.execute_batch(&format!(
            "UPDATE playlist_content SET playlist_id = {a}, sequenceNo = 2 WHERE playlist_id = {b};
             DELETE FROM playlist WHERE playlist_id = {b} OR attribute = 1;
             UPDATE playlist SET playlist_id_parent = 0;"
        ))
        .expect("flatten the eDB");
    }
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Warmup").len(),
        2,
        "the flattened copy holds both Warmups' tracks"
    );

    apply_strict_parity(&service, &usb_root);

    assert_eq!(tree_paths(&edb_tree_of(&usb_root)), pdb);
    assert_eq!(
        tree_paths(&pdb_tree_of(&usb_root)),
        pdb,
        "the PDB gains no copies"
    );
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Style A / Opening / Warmup"),
        ["/Contents/Fixture Artist/Track 1.mp3"]
    );
    assert_eq!(
        edb_playlist_tracks(&usb_root, "Style B / Opening / Warmup"),
        ["/Contents/Fixture Artist/Track 2.mp3"]
    );
}

// ── update_app_analysis_files: what it finds ───────────────────────────────

const APP_DB_ID: u32 = 4_242;

/// A waveform of `bins` bins that changes from bin to bin.
fn varied_waveform(bins: usize, seed: usize) -> crate::service::anlz::WaveformData {
    let energy = (0..bins)
        .map(|i| ((i + seed) * 13 % 127) as u8)
        .collect::<Vec<_>>();
    crate::service::anlz::WaveformData {
        peaks: (0..bins)
            .map(|i| ((i + seed) * 37 % 90) as u8 + 5)
            .collect(),
        bands: (0..bins).map(|i| ((i + seed) % 6) as u8).collect(),
        low_energy: energy.clone(),
        mid_energy: energy.clone(),
        high_energy: energy.clone(),
        low_energy_full: energy.clone(),
        mid_energy_full: energy.clone(),
        high_energy_full: energy,
        peak_level: 1.0,
    }
}

/// Writes a whole bundle at `dat` drawn from `waveform`.
fn write_bundle(
    dat: &Path,
    waveform: &crate::service::anlz::WaveformData,
    track_path: &str,
    duration_ms: u64,
) {
    use crate::service::anlz::{build_anlz_2ex_file, build_anlz_dat_file, build_anlz_ext_file};
    std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
    let bpm = Some(128.0);
    std::fs::write(
        dat,
        build_anlz_dat_file(waveform, track_path, bpm, duration_ms, Some(100), &[]),
    )
    .unwrap();
    std::fs::write(
        dat.with_extension("EXT"),
        build_anlz_ext_file(waveform, track_path, bpm, duration_ms, Some(100), &[]),
    )
    .unwrap();
    std::fs::write(
        dat.with_extension("2EX"),
        build_anlz_2ex_file(waveform, track_path, duration_ms),
    )
    .unwrap();
}

/// A USB track `n` with its audio and a bundle drawn from `waveform`.
fn stick_track(
    usb_root: &Path,
    n: u32,
    seconds: u32,
    waveform: &crate::service::anlz::WaveformData,
    master_content_id: Option<u32>,
) -> PdbTrackRow {
    let track_path = format!("/Contents/track{n}.wav");
    let audio = usb_root.join(track_path.trim_start_matches('/'));
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    std::fs::write(&audio, b"audio").unwrap();
    let anlz_path = format!("/PIONEER/USBANLZ/P00{n}/0000000{n}/ANLZ0000.DAT");
    write_bundle(
        &usb_root.join(anlz_path.trim_start_matches('/')),
        waveform,
        &track_path,
        u64::from(seconds) * 1000,
    );
    PdbTrackRow {
        title: format!("Track {n}"),
        anlz_path,
        duration_seconds: Some(seconds),
        sample_rate_hz: Some(44_100),
        file_size_bytes: Some(1000 + n),
        master_db_id: master_content_id.map(|_| APP_DB_ID),
        master_content_id,
        tempo_x100: 12_800,
        ..history_test_track(n, &track_path)
    }
}

/// A library track `id` at `file_path`, its file there, exported as
/// `master_content_id` when given, with a bundle drawn from `waveform` when
/// given.
fn library_track(
    service: &BackendService,
    id: &str,
    file_path: &Path,
    seconds: u32,
    waveform: Option<&crate::service::anlz::WaveformData>,
    master_content_id: Option<u32>,
) {
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(file_path, b"audio").unwrap();
    let bundle = waveform.map(|waveform| {
        let dat = service
            .db
            .data_dir()
            .join("analysis/waveforms")
            .join(format!("{id}.DAT"));
        write_bundle(&dat, waveform, "", u64::from(seconds) * 1000);
        dat.to_string_lossy().to_string()
    });
    let conn = service.db.connect().unwrap();
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, waveform_peaks_path, \
         created_at, updated_at) VALUES (?1, ?1, '', ?2, ?3, ?4, 'now', 'now')",
        params![
            id,
            file_path.to_string_lossy(),
            i64::from(seconds) * 1000,
            bundle
        ],
    )
    .unwrap();
    if let Some(master_content_id) = master_content_id {
        BackendService::ensure_track_export_identity_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO track_export_identity (track_id, master_content_id, created_at, \
             updated_at) VALUES (?1, ?2, 'now', 'now')",
            params![id, master_content_id],
        )
        .unwrap();
    }
}

fn app_bundle_updates(
    service: &BackendService,
    usb_root: &Path,
    parsed: &ParsedPdb,
) -> HashMap<u32, AppBundleUpdate> {
    let ownership = scan_analysis_ownership(usb_root, parsed, Some(APP_DB_ID), true);
    service
        .detect_app_bundle_updates(
            usb_root,
            parsed,
            Some(APP_DB_ID),
            &AppBundleFindings {
                ownership: &ownership,
                outdated_grids: &[],
                mp3_missing_seek: &[],
            },
        )
        .expect("detect")
        .into_iter()
        .map(|update| (update.track_id, update))
        .collect()
}

#[test]
fn app_analysis_check_finds_repair_drawn_waveforms_and_where_to_take_new_ones() {
    let (_td, usb_root) = test_usb_root();
    let data = tempdir().expect("data dir");
    let service = BackendService::new(data.path()).expect("service");
    let library = data.path().join("music");
    let seconds = 222;
    let drawn = varied_waveform(PRE_0_6_7_REPAIR_WAVEFORM_BINS, 0);
    let detail = varied_waveform(
        crate::service::analysis::waveform_detail_entries_for_duration(222_000),
        5,
    );

    // 1: not in the library -> drawn from the USB's audio.
    let t1 = stick_track(&usb_root, 1, seconds, &drawn, None);
    // 2: exported from a library track with its waveform -> copied.
    let t2 = stick_track(&usb_root, 2, seconds, &drawn, Some(77));
    library_track(
        &service,
        "L2",
        &library.join("2.wav"),
        seconds,
        Some(&detail),
        Some(77),
    );
    // 3: exported from a library track not analyzed yet -> analyzed first.
    let t3 = stick_track(&usb_root, 3, seconds, &drawn, Some(78));
    library_track(
        &service,
        "L3",
        &library.join("3.wav"),
        seconds,
        None,
        Some(78),
    );
    // 4: rekordbox's own analysis, one value per entry -> never touched.
    let t4 = stick_track(&usb_root, 4, seconds, &detail, None);
    // 5: the library track is the USB file itself.
    let t5 = stick_track(&usb_root, 5, seconds, &drawn, None);
    library_track(
        &service,
        "L5",
        &usb_root.join("Contents/track5.wav"),
        seconds,
        Some(&detail),
        None,
    );
    // 6: same content as a library track but another length -> not it.
    let mut t6 = stick_track(&usb_root, 6, seconds, &drawn, None);
    t6.title = "L6".to_string();
    library_track(
        &service,
        "L6",
        &library.join("6.wav"),
        100,
        Some(&detail),
        None,
    );
    service
        .db
        .connect()
        .unwrap()
        .execute(
            "UPDATE tracks SET file_size_bytes = 1006 WHERE id = 'L6'",
            [],
        )
        .unwrap();

    let parsed = ParsedPdb {
        tracks: vec![t1, t2, t3, t4, t5, t6],
        ..ParsedPdb::default()
    };
    let found = app_bundle_updates(&service, &usb_root, &parsed);
    let source = |id: u32| found.get(&id).and_then(|u| u.waveform.clone());
    assert_eq!(
        source(1),
        Some((WaveformFault::RepairDrawn, WaveformSource::Draw))
    );
    let library_dat = |id: &str| data.path().join(format!("analysis/waveforms/{id}.DAT"));
    assert_eq!(
        source(2),
        Some((
            WaveformFault::RepairDrawn,
            WaveformSource::Library(library_dat("L2"))
        ))
    );
    assert_eq!(
        source(3),
        Some((
            WaveformFault::RepairDrawn,
            WaveformSource::AnalyzeLibrary("L3".to_string())
        ))
    );
    assert!(!found.contains_key(&4), "{:?}", found.get(&4));
    assert_eq!(
        source(5),
        Some((
            WaveformFault::RepairDrawn,
            WaveformSource::Library(library_dat("L5"))
        ))
    );
    assert_eq!(
        source(6),
        Some((WaveformFault::RepairDrawn, WaveformSource::Draw))
    );
}

#[test]
fn app_analysis_check_finds_stretched_exports_by_the_library_waveform() {
    let (_td, usb_root) = test_usb_root();
    let data = tempdir().expect("data dir");
    let service = BackendService::new(data.path()).expect("service");
    let library = data.path().join("music");
    // Over the old decode cap (24M samples, about 9 minutes at 44.1 kHz).
    let long = 600;
    let entries = |seconds: u32| {
        crate::service::analysis::waveform_detail_entries_for_duration(u64::from(seconds) * 1000)
    };
    let stretched = varied_waveform(entries(long), 0);
    let current = varied_waveform(entries(long), 9);

    // 1: the library draws it differently -> copied.
    let t1 = stick_track(&usb_root, 1, long, &stretched, Some(81));
    library_track(
        &service,
        "L1",
        &library.join("1.wav"),
        long,
        Some(&current),
        Some(81),
    );
    // 2: the same as the library's -> nothing to do.
    let t2 = stick_track(&usb_root, 2, long, &current, Some(82));
    library_track(
        &service,
        "L2",
        &library.join("2.wav"),
        long,
        Some(&current),
        Some(82),
    );
    // 3: the library hasn't drawn it again yet -> analyzed first.
    let t3 = stick_track(&usb_root, 3, long, &stretched, Some(83));
    library_track(&service, "L3", &library.join("3.wav"), long, None, Some(83));
    // 4: no longer in the library: nothing to compare with.
    let t4 = stick_track(&usb_root, 4, long, &stretched, Some(84));
    // 5: shorter than the cap: never stretched.
    let t5 = stick_track(
        &usb_root,
        5,
        200,
        &varied_waveform(entries(200), 0),
        Some(85),
    );
    library_track(
        &service,
        "L5",
        &library.join("5.wav"),
        200,
        Some(&varied_waveform(entries(200), 9)),
        Some(85),
    );

    let parsed = ParsedPdb {
        tracks: vec![t1, t2, t3, t4, t5],
        ..ParsedPdb::default()
    };
    let found = app_bundle_updates(&service, &usb_root, &parsed);
    assert_eq!(
        found.get(&1).and_then(|u| u.waveform.clone()),
        Some((
            WaveformFault::Stretched,
            WaveformSource::Library(data.path().join("analysis/waveforms/L1.DAT"))
        ))
    );
    assert_eq!(
        found.get(&3).and_then(|u| u.waveform.clone()),
        Some((
            WaveformFault::Stretched,
            WaveformSource::AnalyzeLibrary("L3".to_string())
        ))
    );
    let mut rest = found
        .keys()
        .copied()
        .filter(|id| ![1, 3].contains(id))
        .collect::<Vec<_>>();
    rest.sort_unstable();
    assert!(rest.is_empty(), "{rest:?}");
}
