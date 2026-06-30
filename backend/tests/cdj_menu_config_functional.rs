use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    GetUsbPlayerMenuConfigRequest, InitializeUsbRequest, RunUsbDiagnosticsRequest,
    UpdateUsbPlayerMenuConfigRequest, UsbPlayerMenuItemOrigin,
};
use backend::service::export_helpers::{
    encode_pdb_t16_row, load_pdb_t16_decoded, load_pdb_t16_raw,
};
use tempfile::tempdir;

const BPM_KIND: u32 = 133;

#[test]
fn pdb_extra_rows_not_divergence() {
    // PDB having MORE rows than eDB-visible is normal: older players read all
    // PDB rows as browse categories; eDB.category.isVisible controls the active
    // set. This must NOT be treated as divergence or trigger the fix banner.
    // A freshly-initialized USB already demonstrates this: PDB=27 rows, 10 visible.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let load = backend.get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    assert!(load.ok, "load failed: {load:?}");
    let data = load.data.expect("data");

    // eDB visible set drives current_items (10 items on fresh init).
    assert_eq!(
        data.current_items.len(),
        10,
        "10 visible items on fresh init"
    );
    assert!(
        data.current_items.iter().all(|item| item.is_visible),
        "every current item must be is_visible=true"
    );
    // Current items are visible in eDB AND in PDB → Both.
    for item in &data.current_items {
        assert!(
            item.origin != UsbPlayerMenuItemOrigin::PdbOnly,
            "PdbOnly item cannot be in current list: kind {} name {}",
            item.kind,
            item.name
        );
    }

    // PDB has 17 extra rows beyond the 10 visible — informational, not divergence.
    assert!(
        !data.divergence.in_pdb_only.is_empty(),
        "PDB has extra rows — in_pdb_only should be non-empty (informational)"
    );
    assert!(
        data.divergence.in_pdb_only.contains(&BPM_KIND),
        "BPM (kind {BPM_KIND}) is in PDB but not eDB-visible"
    );
    assert!(
        data.divergence.in_edb_visible_only.is_empty(),
        "all eDB-visible items must be present in PDB: {:?}",
        data.divergence.in_edb_visible_only
    );

    // No real divergence — the fix banner must not appear.
    assert!(
        data.divergence.is_empty(),
        "USB with in_pdb_only only must NOT be considered divergent: {:?}",
        data.divergence
    );
}

#[test]
fn encode_pdb_t16_row_roundtrips_initialized_usb() {
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let decoded = load_pdb_t16_decoded(&usb_root).expect("load t16");
    let raw = load_pdb_t16_raw(&usb_root).expect("load t16 raw");
    assert_eq!(decoded.len(), raw.len(), "row counts must agree");
    assert!(!decoded.is_empty(), "initialized USB should have t16 rows");

    for (decoded_row, raw_row) in decoded.iter().zip(raw.iter()) {
        let encoded = encode_pdb_t16_row(decoded_row.id, decoded_row.kind, &decoded_row.name);
        assert_eq!(
            encoded, *raw_row,
            "byte-equal round-trip failed for kind {} name {:?}",
            decoded_row.kind, decoded_row.name
        );
    }
}

#[test]
fn save_updates_edb_only_not_pdb() {
    // Vendor software changes Active Category by updating eDB.category only.
    // PDB t16 stays as the full 27-row catalog even when a visible category is
    // removed.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let load = backend.get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    assert!(load.ok, "initial load failed: {load:?}");
    let before = load.data.expect("initial load data");
    assert_eq!(
        before.current_items.len(),
        10,
        "fresh init: 10 visible items"
    );
    let pdb_before = load_pdb_t16_decoded(&usb_root).expect("load PDB before save");
    assert_eq!(
        pdb_before.len(),
        27,
        "fresh init seeds the full PDB catalog"
    );

    // Remove KEY (kind 139) -> submit 9 visible kinds.
    const KEY_KIND: u32 = 139;
    let mut desired_kinds: Vec<u32> = before.current_items.iter().map(|i| i.kind).collect();
    assert!(
        desired_kinds.contains(&KEY_KIND),
        "fresh visible set should contain KEY"
    );
    desired_kinds.retain(|kind| *kind != KEY_KIND);

    let save = backend.update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
        current_menu_item_ids: vec![],
        current_kinds: desired_kinds.clone(),
    });
    assert!(save.ok, "save failed: {save:?}");
    let after = save.data.expect("save data");
    assert!(after.updated, "eDB was changed so updated must be true");
    assert_eq!(after.current_items.len(), 9, "9 items now visible");
    assert!(
        after.divergence.is_empty(),
        "no real divergence: all visible kinds are in PDB"
    );
    assert!(
        after.divergence.pdb_missing_kinds.is_empty(),
        "PDB must still have all 27 catalog rows after eDB-only update"
    );
    // The removed kind is still in PDB (not trimmed) so it appears in in_pdb_only.
    assert!(
        after.divergence.in_pdb_only.contains(&KEY_KIND),
        "removed kind {KEY_KIND} must still be in PDB (not trimmed)"
    );
    let pdb_after = load_pdb_t16_decoded(&usb_root).expect("load PDB after save");
    let pdb_after_kinds: Vec<u32> = pdb_after.iter().map(|row| u32::from(row.kind)).collect();
    assert_eq!(pdb_after_kinds.len(), 27, "PDB t16 keeps the full catalog");
    assert!(
        pdb_after_kinds.contains(&KEY_KIND),
        "PDB t16 must still include KEY after eDB-only removal"
    );
    let after_kinds: Vec<u32> = after.current_items.iter().map(|i| i.kind).collect();
    assert_eq!(
        after_kinds, desired_kinds,
        "visible order must match request"
    );

    // Verify PDB t17 was rewritten with the new eDB.category state.
    // t17 data page is page 36 (0-indexed); offset 24 within the page = nrs.
    let pdb_path = usb_root.join("PIONEER/rekordbox/export.pdb");
    let pdb_bytes = std::fs::read(&pdb_path).expect("read PDB after save");
    assert!(
        pdb_bytes.len() > 37 * 4096,
        "PDB must have at least 37 pages (t17 data at page 36)"
    );
    assert_eq!(
        pdb_bytes[36 * 4096 + 24],
        22,
        "t17 page 36 must still hold 22 rows after CDJ menu update"
    );
    // Rows start at payload offset 40; each row is 8 bytes:
    //   [menuItemId:2LE][categoryId:2LE][0x63][flags][seqNo:2LE]
    // flags: 0=visible, 1=hidden, 2=MATCHING (kind=170) visible.
    // After removing KEY from 10 visible: 9 visible (8 flags=0 + 1 flags=2), 13 hidden (flags=1).
    let page36 = &pdb_bytes[36 * 4096..37 * 4096];
    let mut flags_counts = [0u32; 3]; // [visible, hidden, matching]
    for i in 0..22_usize {
        let off = 40 + i * 8;
        let menu_item_id = u16::from_le_bytes([page36[off], page36[off + 1]]);
        let flags = page36[off + 5];
        assert_ne!(menu_item_id, 0, "t17 row {i}: menuItemId must be non-zero");
        assert!(flags <= 2, "t17 row {i}: unexpected flags value {flags}");
        flags_counts[flags as usize] += 1;
    }
    assert_eq!(
        flags_counts[0], 8,
        "t17 must have 8 visible rows (flags=0) after removing KEY"
    );
    assert_eq!(
        flags_counts[1], 13,
        "t17 must have 13 hidden rows (flags=1) after removing KEY"
    );
    assert_eq!(flags_counts[2], 1, "t17 must have 1 MATCHING row (flags=2)");

    let reload = backend.get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    let reload_data = reload.data.expect("reload data");
    assert!(
        reload_data.divergence.is_empty(),
        "reload: divergence still empty"
    );
    assert!(
        reload_data.divergence.pdb_missing_kinds.is_empty(),
        "reload: PDB still has all 27 rows"
    );
}

#[test]
fn freshly_initialized_usb_has_no_menu_divergence() {
    // initialize_usb seeds PDB t16 with all 27 player browse categories
    // and marks 10 of them visible in eDB.
    // No real divergence: all eDB-visible items are present in PDB.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let load = backend.get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    assert!(load.ok, "get menu config failed: {load:?}");
    let data = load.data.expect("data");
    assert!(
        data.divergence.is_empty(),
        "fresh init must produce zero divergence: {:?}",
        data.divergence
    );
    assert!(
        data.divergence.pdb_missing_kinds.is_empty(),
        "PDB must have all 27 catalog rows on fresh init: missing={:?}",
        data.divergence.pdb_missing_kinds
    );
    // Default: 10 visible kinds in eDB; remaining 17 are available to add.
    assert_eq!(
        data.current_items.len(),
        10,
        "fresh init should default to the visible set (10 items)",
    );
    assert_eq!(
        data.available_items.len(),
        17,
        "remaining 17 menuItems should be available to add",
    );
    let kinds: Vec<u32> = data.current_items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        vec![129, 130, 131, 139, 132, 149, 145, 170, 144, 140],
        "default visible kinds must match vendor order"
    );
    for item in &data.current_items {
        assert_eq!(
            item.origin,
            UsbPlayerMenuItemOrigin::Both,
            "every visible item should be present in both DBs on fresh init"
        );
    }
    for item in &data.available_items {
        assert_eq!(
            item.origin,
            UsbPlayerMenuItemOrigin::PdbOnly,
            "available items are PDB-backed catalog entries not yet visible in eDB"
        );
    }
}

#[test]
fn diagnostics_no_false_divergence_for_vendor_usb() {
    // A USB where PDB has more rows than eDB visible (in_pdb_only non-empty)
    // must NOT produce a player-menu-divergence warning — that state is normal
    // and does not require user action.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let response = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    assert!(response.ok, "diagnostics failed: {response:?}");
    let data = response.data.expect("diagnostics data");
    let menu_warning = data
        .warnings
        .iter()
        .find(|w| w.code == "usb.diagnostics.player-menu-divergence");
    assert!(
        menu_warning.is_none(),
        "USB with in_pdb_only only must NOT produce player-menu-divergence warning; got: {:?}",
        menu_warning.map(|w| w.message.as_str())
    );
}

#[test]
fn sync_edb_to_pdb_restores_trimmed_pdb() {
    // If PDB t16 was trimmed by old code, sync_usb_player_menu_edb_to_pdb must
    // restore it to the full 27-row catalog from eDB menuItems.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    // Simulate old-code damage: trim PDB t16 to just 5 kinds.
    let trim_kinds: &[(u16, String)] = &[
        (129, "ARTIST".to_string()),
        (130, "ALBUM".to_string()),
        (131, "TRACK".to_string()),
        (132, "PLAYLIST".to_string()),
        (149, "HISTORY".to_string()),
    ];
    backend::service::export_helpers::patch_pdb_columns_menu_set_by_kind(&usb_root, trim_kinds)
        .expect("trim PDB for test setup");

    let before = backend
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: Some(usb_root.to_string_lossy().into_owned()),
        })
        .data
        .expect("before load");
    assert!(
        !before.divergence.pdb_missing_kinds.is_empty(),
        "after trimming PDB, pdb_missing_kinds must be non-empty: {:?}",
        before.divergence
    );

    let response = backend.sync_usb_player_menu_edb_to_pdb(GetUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    assert!(response.ok, "sync failed: {response:?}");
    let data = response.data.expect("sync data");
    assert!(data.updated, "PDB was restored so updated must be true");
    assert!(
        data.divergence.pdb_missing_kinds.is_empty(),
        "after sync, PDB must have all 27 rows: {:?}",
        data.divergence
    );
    assert!(
        data.divergence.is_empty(),
        "after sync, no real divergence: {:?}",
        data.divergence
    );

    let reload = backend
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: Some(usb_root.to_string_lossy().into_owned()),
        })
        .data
        .expect("reload data");
    assert!(
        reload.divergence.pdb_missing_kinds.is_empty(),
        "reload: PDB still complete after sync"
    );
    assert!(
        reload.divergence.is_empty(),
        "reload: no divergence after sync"
    );
}

#[test]
fn save_idempotent_when_already_in_sync() {
    // Re-saving the current kind set must leave PDB bytes unchanged and report
    // updated=false.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    backend
        .initialize_usb(InitializeUsbRequest {
            usb_root: usb_root.to_string_lossy().into_owned(),
        })
        .data
        .expect("initialize_usb");

    let load = backend.get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
    });
    let before = load.data.expect("data");
    let kinds: Vec<u32> = before.current_items.iter().map(|i| i.kind).collect();

    let pdb_path = usb_root.join("PIONEER/rekordbox/export.pdb");
    let pdb_before = fs::read(&pdb_path).expect("read pdb");

    let save = backend.update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
        current_menu_item_ids: vec![],
        current_kinds: kinds.clone(),
    });
    assert!(save.ok, "save failed: {save:?}");
    let after = save.data.expect("data");
    let pdb_after = fs::read(&pdb_path).expect("read pdb");
    assert_eq!(
        pdb_before, pdb_after,
        "saving the same kind set must not modify PDB bytes"
    );
    assert!(
        !after.updated,
        "saving the same kind set must report updated=false"
    );
}

#[test]
fn update_player_menu_rejects_removing_protected_kind() {
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    backend
        .initialize_usb(InitializeUsbRequest {
            usb_root: usb_root.to_string_lossy().into_owned(),
        })
        .data
        .expect("initialize_usb");

    let config = backend
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: Some(usb_root.to_string_lossy().into_owned()),
        })
        .data
        .expect("config data");

    // The default initialized menu always includes TRACK (kind=131).
    // Build a kinds list with TRACK removed to trigger the protection.
    const TRACK: u32 = 131;
    let kinds_without_track: Vec<u32> = config
        .current_items
        .iter()
        .map(|i| i.kind)
        .filter(|&k| k != TRACK)
        .collect();

    let resp = backend.update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
        current_kinds: kinds_without_track,
        current_menu_item_ids: vec![],
    });
    assert!(!resp.ok, "expected error when removing TRACK, got ok");
    let err_msg = resp
        .error
        .as_ref()
        .map(|e| e.message.as_str())
        .unwrap_or("");
    assert!(
        err_msg.to_lowercase().contains("cannot be removed")
            || err_msg.to_lowercase().contains("track"),
        "expected descriptive error message, got: {err_msg}"
    );
}

#[test]
fn update_player_menu_vendor_seven_item_set_keeps_pdb_catalog() {
    // Reproduces the vendor 7-item menu (ARTIST, ALBUM, TRACK,
    // PLAYLIST, HISTORY, SEARCH, FOLDER) and verifies our app mirrors the
    // vendor behavior: eDB.category changes, PDB t16 keeps the full catalog.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");

    backend
        .initialize_usb(InitializeUsbRequest {
            usb_root: usb_root.to_string_lossy().into_owned(),
        })
        .data
        .expect("initialize_usb");

    // ARTIST=129, ALBUM=130, TRACK=131, PLAYLIST=132, HISTORY=149, SEARCH=145, FOLDER=144
    let vendor_kinds: Vec<u32> = vec![129, 130, 131, 132, 149, 145, 144];

    let resp = backend.update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
        current_kinds: vendor_kinds.clone(),
        current_menu_item_ids: vec![],
    });
    assert!(resp.ok, "update failed: {resp:?}");
    let data = resp.data.expect("data");

    let after_kinds: Vec<u32> = data.current_items.iter().map(|i| i.kind).collect();
    assert_eq!(
        after_kinds, vendor_kinds,
        "eDB active order must match request"
    );
    let pdb_rows = load_pdb_t16_decoded(&usb_root).expect("load PDB after update");
    let pdb_kinds: Vec<u32> = pdb_rows.iter().map(|row| u32::from(row.kind)).collect();
    assert_eq!(pdb_kinds.len(), 27, "PDB t16 must keep the full catalog");
    assert!(pdb_kinds.contains(&139), "PDB t16 must still include KEY");

    assert!(
        data.divergence.is_empty(),
        "expected zero divergence after 7-item set; got: {:?}",
        data.divergence
    );

    for item in &data.current_items {
        assert_eq!(
            item.origin,
            UsbPlayerMenuItemOrigin::Both,
            "item kind {} should be Both, got {:?}",
            item.kind,
            item.origin
        );
    }

    let reload = backend
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: Some(usb_root.to_string_lossy().into_owned()),
        })
        .data
        .expect("reload data");
    let reload_kinds: Vec<u32> = reload.current_items.iter().map(|i| i.kind).collect();
    assert_eq!(
        reload_kinds, vendor_kinds,
        "on-disk kinds must match after reload"
    );
    assert!(
        reload.divergence.is_empty(),
        "reload divergence must be empty: {:?}",
        reload.divergence
    );
}
