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
    use super::run::FixOutcomes;

    let mut outcomes = FixOutcomes::default();
    let ok = || Ok::<_, String>("fixed 2 rows".to_string());
    outcomes.record("Fix A", true, true, ok);
    outcomes.record("Fix B", true, true, || Err::<String, _>("disk full"));
    outcomes.record("Fix C", true, false, ok);
    outcomes.record("Fix D", false, true, ok);
    outcomes.record("Fix E", false, false, ok);

    assert_eq!(outcomes.applied, vec!["Fix A: fixed 2 rows"]);
    assert_eq!(outcomes.failed, vec!["Fix B failed: disk full"]);
    assert_eq!(
        outcomes.skipped,
        vec!["Fix C: nothing to apply", "Fix D: not selected"]
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
        playlist_id_match: true,
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

    let clean = make_strict_repair_detail("Clean");
    assert!(!playlist_requires_strict_upgrade(&clean));
}

#[test]
fn strict_repair_playlist_names_follow_pdb_order_with_edb_only_tail() {
    let mut parsed = ParsedPdb::default();
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

    let mut edb_playlists = HashMap::<String, ExportDbPlaylist>::new();
    edb_playlists.insert(
        "Alpha".to_string(),
        ExportDbPlaylist {
            playlist_id: 20,
            sort_order: 0,
            tracks: Vec::new(),
        },
    );
    edb_playlists.insert(
        "Zeta".to_string(),
        ExportDbPlaylist {
            playlist_id: 10,
            sort_order: 1,
            tracks: Vec::new(),
        },
    );
    edb_playlists.insert(
        "Beta".to_string(),
        ExportDbPlaylist {
            playlist_id: 30,
            sort_order: 0,
            tracks: Vec::new(),
        },
    );

    let names = strict_repair_ordered_playlist_names(&parsed, Some(&edb_playlists));

    assert_eq!(names, vec!["Zeta", "Alpha", "Beta"]);
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

    let (history_rows, history_content_rows) = derive_history_sync_payload(&parsed);
    assert_eq!(history_rows, vec![(10, 0, "HISTORY 001".to_string(), 0, 0)]);
    assert_eq!(history_content_rows, vec![(10, 101, 1)]);
}

fn test_usb_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let td = tempdir().expect("tempdir");
    let usb_root = td.path().join("USB_TEST");
    std::fs::create_dir_all(&usb_root).expect("create usb root");
    crate::service::initialize_usb(&usb_root).expect("initialize usb");
    (td, usb_root)
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
    assert_eq!((before.pdb_color, before.edb_color), (Some(0), Some(0)));
    assert!(before.writable);

    let set_green = set(5).expect("set green");
    assert!(set_green.updated);
    assert_eq!((set_green.pdb_color, set_green.edb_color), (Some(5), Some(5)));
    let after = get();
    assert_eq!((after.pdb_color, after.edb_color), (Some(5), Some(5)));

    // On the USB itself, not just the staged copies.
    let pdb = std::fs::read(vendor_pdb_path(&usb_root)).expect("read pdb");
    assert_eq!(crate::pdb_writer::t19_background_color(&pdb, 4096), Some(5));
    let conn = crate::edb::open_edb(
        &super::super::usb_vendor_compat::vendor_edb_path(&usb_root),
        &mut Vec::new(),
    )
    .expect("open eDB");
    let edb_color: i64 = conn
        .query_row("SELECT backGroundColorType FROM property", [], |row| row.get(0))
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
    assert_eq!(apply_player_menu_edit(&menu, Add, 4), vec![1, 2, 3, 4]);
    assert_eq!(apply_player_menu_edit(&menu, Add, 2), vec![1, 2, 3]);
    assert_eq!(apply_player_menu_edit(&menu, Remove, 2), vec![1, 3]);
    assert_eq!(apply_player_menu_edit(&menu, Remove, 9), vec![1, 2, 3]);
    assert_eq!(apply_player_menu_edit(&menu, MoveUp, 2), vec![2, 1, 3]);
    assert_eq!(apply_player_menu_edit(&menu, MoveUp, 1), vec![1, 2, 3]);
    assert_eq!(apply_player_menu_edit(&menu, MoveDown, 2), vec![1, 3, 2]);
    assert_eq!(apply_player_menu_edit(&menu, MoveDown, 3), vec![1, 2, 3]);
    assert_eq!(apply_player_menu_edit(&menu, MoveDown, 9), vec![1, 2, 3]);
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
    let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));

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
        Err(BackendError::Validation("failed halfway".into()))
    });

    assert!(result.is_err());
    assert_eq!(pdb.bytes, before);
}

/// Stages both databases of `usb_root` and changes the staged PDB's header
/// compatibility field, as a repair run would before write-back.
fn staged_repair_with_changed_pdb(usb_root: &Path) -> super::run::StagedPdb {
    let mut pdb = super::run::StagedPdb::load(usb_root).expect("load staged pdb");
    usb_staging::stage_edb(usb_root).expect("stage edb");
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
    let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));
    let (_td, usb_root) = test_usb_root();
    let raw_pdb_path = vendor_pdb_path(&usb_root);
    let raw_pdb_before = std::fs::read(&raw_pdb_path).unwrap();
    let mut pdb = staged_repair_with_changed_pdb(&usb_root);

    // Something else writes the USB's eDB after it was staged.
    append_byte(&super::super::usb_vendor_compat::vendor_edb_path(&usb_root));

    let err = super::run::write_back_both_or_neither(&usb_root, &mut pdb)
        .expect_err("write-back must refuse an outside change");
    assert!(
        err.to_string().contains("changed since it was last read"),
        "{err}"
    );
    assert_eq!(std::fs::read(&raw_pdb_path).unwrap(), raw_pdb_before);
    assert_eq!(
        std::fs::read(usb_staging::stage_pdb(&usb_root).unwrap()).unwrap(),
        raw_pdb_before,
        "the staged PDB is staged again from the USB"
    );
}

#[test]
fn repair_write_back_puts_the_previous_pdb_back_when_the_edb_write_fails() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));
    let (_td, usb_root) = test_usb_root();
    let raw_pdb_path = vendor_pdb_path(&usb_root);
    let raw_edb_path = super::super::usb_vendor_compat::vendor_edb_path(&usb_root);
    let raw_pdb_before = std::fs::read(&raw_pdb_path).unwrap();
    let raw_edb_before = std::fs::read(&raw_edb_path).unwrap();
    let mut pdb = staged_repair_with_changed_pdb(&usb_root);
    append_byte(&usb_staging::stage_edb(&usb_root).unwrap());

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
        std::fs::read(usb_staging::stage_edb(&usb_root).unwrap()).unwrap(),
        raw_edb_before,
        "the staged eDB is staged again from the USB"
    );
}

#[test]
fn apply_fix_sync_edb_history_from_pdb_flushes_to_usb_when_staging_enabled() {
    let cache_dir = tempdir().expect("cache dir").keep();
    let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));

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

    let mut warnings = Vec::new();
    let (history_written, _history_content_written) = service
        .apply_fix_sync_edb_history_from_pdb(&usb_root, &parsed, None, &mut warnings)
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
    let raw_edb_path = super::super::usb_vendor_compat::vendor_edb_path(&usb_root);
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

    let committed =
        usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Edb).unwrap();
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
fn sentinel_u5_repair_detects_and_fixes_sentinel_footer() {
    let mut p1 = data_page(1, 7); // tt=7 (playlist_tree): correct is (nrs, 0)
    set_nrs(&mut p1, 3);
    set_u5_num_rl(&mut p1, 0x1FFF, 0x1FFF);
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_sentinel_u5_on_data_pages(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1, "expected one sentinel-u5 page detected");
    assert_eq!(pages[0].page_index, 1);

    // No-op on empty input.
    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_sentinel_u5_repair(bytes, &[])).unwrap(),
        0
    );

    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_sentinel_u5_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE;
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
        3
    );
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
        0
    );

    assert!(
        detect_pdb_sentinel_u5_on_data_pages(&read_pdb_file(&pdb_path)).is_empty(),
        "repair should be idempotent"
    );
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

#[test]
fn wrong_page_flags_repair_detects_and_fixes_invalid_flags() {
    let mut p1 = data_page(1, 7); // tt=7: only 0x24 is valid
    set_flags(&mut p1, 0x99);
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_wrong_page_flags(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_index, 1);
    assert_eq!(pages[0].correct_flags, 0x24);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_wrong_page_flags_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );

    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_wrong_page_flags_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    assert_eq!(bytes[TEST_PAGE_SIZE + 0x1b], 0x24);
    assert!(detect_pdb_wrong_page_flags(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn wrong_page_flags_repair_allows_active_flag_on_track_table() {
    // tt=0 (tracks) accepts both 0x24 and 0x34; only a value outside that
    // set should be flagged, and the correction target is 0x34 (active).
    let mut p1 = data_page(1, 0);
    set_flags(&mut p1, 0x11);
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_wrong_page_flags(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].correct_flags, 0x34);
}

#[test]
fn zero_tranrf_repair_detects_and_fixes_missing_transaction_flags() {
    // tt=0, u5=2 (non-dict shape): tranrf must mirror rowpf in every group.
    let mut p1 = data_page(1, 0);
    set_nrs(&mut p1, 1);
    set_u5_num_rl(&mut p1, 2, 0);
    // Single footer group at the tail of the page: [row-offset(u16)] [rowpf(u16)] [tranrf(u16)]
    // laid out backwards from the end: cursor -= 4 (rowpf|tranrf), then -= glen*2 (offsets).
    let rowpf_off = TEST_PAGE_SIZE - 4;
    let tranrf_off = TEST_PAGE_SIZE - 2;
    p1[rowpf_off..rowpf_off + 2].copy_from_slice(&1u16.to_le_bytes()); // rowpf != 0
    p1[tranrf_off..tranrf_off + 2].copy_from_slice(&0u16.to_le_bytes()); // tranrf == 0 (bug)
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_zero_tranrf_all_tables(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1, "expected one zero-tranrf page detected");
    assert_eq!(pages[0].page_index, 1);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_zero_tranrf_repair(bytes, &[])).unwrap(),
        0
    );

    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_zero_tranrf_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE + tranrf_off;
    assert_eq!(
        u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap()),
        1,
        "tranrf should now mirror rowpf"
    );
    assert!(detect_pdb_zero_tranrf_all_tables(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn zero_tranrf_repair_detects_corruption_on_a_wrapped_257_row_page() {
    // tt=8 (playlist_entries), u5=1: only the last footer group carries
    // tranrf. 257 rows is the smallest row count that both wraps `nrs`
    // (257 & 0xFF = 1) and crosses a 16-row group boundary (257 rows =
    // 17 groups), which used to make `nrs.max(num_rl)` undercount the
    // group total by one and silently skip inspecting the true last
    // group entirely -- see docs/PDB.md "Page Footer Conventions".
    let mut p1 = data_page(1, 8);
    p1[0x18] = 1; // packed row count low byte: 257 & 0xFF
    p1[0x19] = 1; // packed row count mid byte: (257 >> 8) & 0xFF
    p1[0x1a] = 0;
    set_u5_num_rl(&mut p1, 1, 256); // (u5, num_rl) = (1, trc-1) for trc=257

    // Footer groups are written backward from the page end. 16 full
    // groups (rows 0..255) each consume 4 (rowpf|tranrf) + 16*2 (offsets)
    // = 36 bytes; the 17th group (row 256 only) starts right after them.
    let last_group_rowpf_off = TEST_PAGE_SIZE - 16 * 36 - 4;
    let last_group_tranrf_off = last_group_rowpf_off + 2;
    p1[last_group_rowpf_off..last_group_rowpf_off + 2].copy_from_slice(&1u16.to_le_bytes()); // rowpf != 0
    // tranrf left at 0 (page starts zero-filled) -- the corrupt shape.

    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_zero_tranrf_all_tables(&read_pdb_file(&pdb_path));
    assert_eq!(
        pages.len(),
        1,
        "the 257th row's group must be inspected and flagged, not skipped"
    );
    assert_eq!(pages[0].page_index, 1);

    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_zero_tranrf_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE + last_group_tranrf_off;
    assert_eq!(
        u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap()),
        1,
        "tranrf should now mirror rowpf"
    );
    assert!(detect_pdb_zero_tranrf_all_tables(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn wrong_history_page_shape_repair_detects_and_fixes_old_bug_pattern() {
    // tt=17 (columns/history-family): correct convention is (nrs, 0); the
    // old-bug pattern (u5=1, num_rl=nrs-1) must be flagged and corrected.
    let mut p1 = data_page(1, 17);
    set_nrs(&mut p1, 4);
    set_u5_num_rl(&mut p1, 1, 3); // nrs-1 == 3
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_wrong_history_page_shape(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].nrs, 4);

    assert_eq!(
        patch_pdb_file(
            &usb_root,
            |bytes| apply_pdb_wrong_history_page_shape_repair(bytes, &[])
        )
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_wrong_history_page_shape_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE;
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
        4
    );
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
        0
    );
    assert!(detect_pdb_wrong_history_page_shape(&read_pdb_file(&pdb_path)).is_empty());
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
    use crate::pdb_writer::{read_present_page_rows, rewrite_variable_page_rows_in_place};
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
            let mut rows = read_present_page_rows(&page, PAGE_SIZE);
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
    use crate::pdb_writer::{read_present_page_rows, rewrite_variable_page_rows_in_place};
    use crate::utils::{
        collect_chain, page_offset, read_u16_le_at, read_u32_le_at, table_ptr_fields,
    };
    const PAGE_SIZE: usize = 4096;
    let (_ec, first, last) = table_ptr_fields(bytes, 0).expect("t00 pointer");
    let chain = collect_chain(bytes, PAGE_SIZE, first, last).expect("t00 chain");
    for page_idx in chain.into_iter().skip(1) {
        let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
        let mut rows = read_present_page_rows(&bytes[off..off + PAGE_SIZE], PAGE_SIZE);
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
    use crate::pdb_writer::{read_present_page_rows, rewrite_variable_page_rows_in_place};
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
        let mut rows = read_present_page_rows(&page, PAGE_SIZE);
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
fn wrong_track_u5_repair_detects_and_fixes_active_page_footer() {
    // tt=0, flags=0x34 (active): only (u5=2, num_rl=0) or (u5=1, num_rl=nrs-1)
    // are valid; anything else must be flagged and normalised to the
    // writer's own current convention, (u5=1, num_rl=nrs-1) -- not the
    // older (2, 0) shape, which the detector still tolerates as valid on
    // pages that already have it but is no longer the repair's target.
    let mut p1 = data_page(1, 0);
    set_flags(&mut p1, 0x34);
    set_nrs(&mut p1, 5);
    set_u5_num_rl(&mut p1, 99, 99);
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_wrong_track_u5(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_index, 1);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_wrong_track_u5_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_wrong_track_u5_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE;
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
        1
    );
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
        4
    );
    assert!(detect_pdb_wrong_track_u5(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn t00_multipage_active_repair_flags_only_predecessor_pages() {
    // Two chained tt=0 pages, both ACTV (0x34). Page 1 -> Page 2 makes
    // page 1 a predecessor (must become SEAL); page 2 is the terminal
    // active page (next_page not in the data-page set) and stays ACTV.
    let mut p1 = data_page(1, 0);
    set_flags(&mut p1, 0x34);
    set_nrs(&mut p1, 4);
    set_next_page(&mut p1, 2);
    let mut p2 = data_page(2, 0);
    set_flags(&mut p2, 0x34);
    set_nrs(&mut p2, 4);
    set_next_page(&mut p2, 0); // terminal: doesn't point at another data page
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1, p2]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_t00_multipage_active_pages(&read_pdb_file(&pdb_path));
    assert_eq!(
        pages.len(),
        1,
        "only the predecessor page should be flagged"
    );
    assert_eq!(pages[0].page_index, 1);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| apply_pdb_t00_multipage_active_repair(
            bytes,
            &[]
        ))
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_t00_multipage_active_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE; // page 1
    assert_eq!(bytes[off + 0x1b], 0x24, "predecessor page must become SEAL");
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
        1
    );
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
        3
    );
    // Terminal page 2 must be left untouched.
    let off2 = TEST_PAGE_SIZE * 2;
    assert_eq!(bytes[off2 + 0x1b], 0x34);
    assert!(detect_pdb_t00_multipage_active_pages(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn wrong_playlist_tree_shape_repair_detects_and_fixes_footer() {
    // tt=7 (playlist_tree): correct convention is (nrs, 0).
    let mut p1 = data_page(1, 7);
    set_nrs(&mut p1, 6);
    set_u5_num_rl(&mut p1, 1, 1); // wrong: neither num_rl==0 nor u5==nrs
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let pages = detect_pdb_wrong_playlist_tree_shape(&read_pdb_file(&pdb_path));
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].nrs, 6);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| {
            apply_pdb_wrong_playlist_tree_shape_repair(bytes, &[])
        })
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_wrong_playlist_tree_shape_repair(bytes, &pages)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let off = TEST_PAGE_SIZE;
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
        6
    );
    assert_eq!(
        u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
        0
    );
    assert!(detect_pdb_wrong_playlist_tree_shape(&read_pdb_file(&pdb_path)).is_empty());
}

#[test]
fn tombstoned_playlist_tree_id_repair_zeroes_duplicate_dead_id() {
    // tt=7 (playlist_tree), id field at row byte 12 (TOMBSTONE_ID_TABLES).
    // Two row slots: row0 (active, id=42) and row1 (tombstoned, id=42 —
    // a stale duplicate left behind by an earlier repair).
    let mut p1 = data_page(1, 7);
    p1[0x18] = 2; // packed row count low byte: num_row_offsets = 2
    // rowpf group header at the tail: cursor -= 4 for (rowpf, tranrf).
    let rowpf_off = TEST_PAGE_SIZE - 4;
    // bit0 (row index 0, first row) = active; bit1 (row index 1, last row) = tombstoned.
    p1[rowpf_off..rowpf_off + 2].copy_from_slice(&0b01u16.to_le_bytes());
    // Row-offset table (2 entries, u16 each), stored in reversed row order:
    // position0 -> LAST row (index1), position1 -> FIRST row (index0).
    let offsets_off = rowpf_off - 4;
    p1[offsets_off..offsets_off + 2].copy_from_slice(&0x20u16.to_le_bytes()); // row1 heap offset
    p1[offsets_off + 2..offsets_off + 4].copy_from_slice(&0x00u16.to_le_bytes()); // row0 heap offset
    // Row bytes on the heap (heap_start = 0x28): id field at row_byte_off + 12.
    let heap_start = 0x28usize;
    p1[heap_start + 12..heap_start + 16].copy_from_slice(&42u32.to_le_bytes()); // row0 id (active)
    p1[heap_start + 0x20 + 12..heap_start + 0x20 + 16].copy_from_slice(&42u32.to_le_bytes()); // row1 id (tombstoned dup)
    let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
    let pdb_path = vendor_pdb_path(&usb_root);

    let items = detect_pdb_tombstoned_playlist_tree_ids(&read_pdb_file(&pdb_path));
    assert_eq!(
        items.len(),
        1,
        "expected the duplicate tombstoned id to be flagged"
    );
    assert_eq!(items[0].page_index, 1);
    assert_eq!(items[0].id_field_offset, heap_start + 0x20 + 12);

    assert_eq!(
        patch_pdb_file(&usb_root, |bytes| {
            apply_pdb_tombstoned_playlist_tree_id_repair(bytes, &[])
        })
        .unwrap(),
        0
    );
    let patched = patch_pdb_file(&usb_root, |bytes| {
        apply_pdb_tombstoned_playlist_tree_id_repair(bytes, &items)
    })
    .unwrap();
    assert_eq!(patched, 1);

    let bytes = std::fs::read(&pdb_path).unwrap();
    let id_off = TEST_PAGE_SIZE + heap_start + 0x20 + 12;
    assert_eq!(
        u32::from_le_bytes(bytes[id_off..id_off + 4].try_into().unwrap()),
        0
    );
    assert!(detect_pdb_tombstoned_playlist_tree_ids(&read_pdb_file(&pdb_path)).is_empty());
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
        let mut warnings = Vec::new();
        let mut conn = open_edb_rw(&usb_root, &mut warnings).expect("open eDB rw");
        let tx = conn.transaction().expect("begin tx");
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer); \
             CREATE TABLE IF NOT EXISTS history_content(history_id integer, content_id integer, sequenceNo integer); \
             DELETE FROM history; DELETE FROM history_content; \
             INSERT INTO history(history_id, sequenceNo, name, attribute, history_id_parent) VALUES (999, 5, 'Stale', 0, 0); \
             INSERT INTO history_content(history_id, content_id, sequenceNo) VALUES (999, 555, 1), (999, 556, 2);",
        )
        .expect("seed stale history rows");
        tx.commit().expect("commit seed");
    }

    let mut warnings = Vec::new();
    let (history_written, history_content_written) = service
        .apply_fix_sync_edb_history_from_pdb(&usb_root, &parsed, None, &mut warnings)
        .expect("sync edb history");
    assert_eq!(history_written, 1);
    assert_eq!(
        history_content_written, 1,
        "the track_id=0 entry must be dropped"
    );

    let conn = open_edb_rw(&usb_root, &mut Vec::new()).expect("re-open eDB rw");
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
    assert_eq!(content_id, 101);
}

#[test]
fn apply_fix_empty_analysis_files_bakes_real_pdb_tempo_not_120_default() {
    let (_td, usb_root) = test_usb_root();
    let service_data_dir = tempdir().expect("service data dir");
    let service = BackendService::new(service_data_dir.path()).expect("backend service");

    let audio_path = usb_root.join("Contents/track.wav");
    std::fs::create_dir_all(audio_path.parent().unwrap()).unwrap();
    write_sine_wav(&audio_path, 1_000);

    let anlz_dir = usb_root.join("PIONEER/USBANLZ/P001/00000001");
    std::fs::create_dir_all(&anlz_dir).unwrap();
    let dat_path = anlz_dir.join("ANLZ0000.DAT");
    std::fs::write(&dat_path, b"").unwrap(); // the "empty analysis file" needing repair

    let parsed = ParsedPdb {
        tracks: vec![PdbTrackRow {
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
        proposal(PDB_WRONG_PAGE_FLAGS_FIX_ID, true),
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
