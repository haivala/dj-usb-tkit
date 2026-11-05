use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    GetUsbPlayerMenuConfigRequest, InitializeUsbRequest, UpdateUsbPlayerMenuConfigData,
    UpdateUsbPlayerMenuConfigRequest, UsbPlayerMenuEdit,
};
use backend::service::export_helpers::{
    encode_pdb_t16_row, load_pdb_t16_decoded, load_pdb_t16_raw,
};
use tempfile::tempdir;

/// One player menu editor edit, as the editor's buttons send it.
fn edit_menu(
    backend: &BackendCommands,
    usb_root: &std::path::Path,
    action: UsbPlayerMenuEdit,
    kind: u32,
) -> backend::models::ApiResponse<UpdateUsbPlayerMenuConfigData> {
    backend.update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
        usb_root: Some(usb_root.to_string_lossy().into_owned()),
        action,
        kind,
    })
}

/// Brings the shown menu to `target` with the editor's edits: Remove what
/// isn't wanted, Add what is missing, then Up until each sits in its place.
fn set_menu_with_edits(
    backend: &BackendCommands,
    usb_root: &std::path::Path,
    target: &[u32],
) -> UpdateUsbPlayerMenuConfigData {
    let load = |backend: &BackendCommands| {
        backend
            .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
                usb_root: Some(usb_root.to_string_lossy().into_owned()),
            })
            .data
            .expect("menu config")
            .current_items
            .iter()
            .map(|i| i.kind)
            .collect::<Vec<u32>>()
    };
    let mut last = None;
    let mut apply = |action, kind| {
        let resp = edit_menu(backend, usb_root, action, kind);
        assert!(resp.ok, "{action:?} {kind} failed: {resp:?}");
        last = resp.data;
    };
    for kind in load(backend) {
        if !target.contains(&kind) {
            apply(UsbPlayerMenuEdit::Remove, kind);
        }
    }
    for &kind in target {
        if !load(backend).contains(&kind) {
            apply(UsbPlayerMenuEdit::Add, kind);
        }
    }
    for (place, &kind) in target.iter().enumerate() {
        let at = load(backend)
            .iter()
            .position(|&k| k == kind)
            .expect("added");
        for _ in place..at {
            apply(UsbPlayerMenuEdit::MoveUp, kind);
        }
    }
    last.expect("at least one edit")
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

    // Remove KEY (kind 139).
    const KEY_KIND: u32 = 139;
    assert!(
        before.current_items.iter().any(|i| i.kind == KEY_KIND),
        "fresh visible set should contain KEY"
    );

    let save = edit_menu(&backend, &usb_root, UsbPlayerMenuEdit::Remove, KEY_KIND);
    assert!(save.ok, "save failed: {save:?}");
    let after = save.data.expect("save data");
    assert!(after.updated, "eDB was changed so updated must be true");
    assert_eq!(after.current_items.len(), 9, "9 items now visible");
    // The removed kind is still in PDB (not trimmed) so it appears in in_pdb_only.
    let pdb_after = load_pdb_t16_decoded(&usb_root).expect("load PDB after save");
    let pdb_after_kinds: Vec<u32> = pdb_after.iter().map(|row| u32::from(row.kind)).collect();
    assert_eq!(pdb_after_kinds.len(), 27, "PDB t16 keeps the full catalog");
    assert!(
        pdb_after_kinds.contains(&KEY_KIND),
        "PDB t16 must still include KEY after eDB-only removal"
    );
    let after_kinds: Vec<u32> = after.current_items.iter().map(|i| i.kind).collect();
    let expected_kinds: Vec<u32> = before
        .current_items
        .iter()
        .map(|i| i.kind)
        .filter(|&kind| kind != KEY_KIND)
        .collect();
    assert_eq!(after_kinds, expected_kinds, "the others keep their order");

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
        reload_data
            .current_items
            .iter()
            .all(|item| item.kind != KEY_KIND),
        "the removed kind stays removed after a reload"
    );
}

#[test]
fn freshly_initialized_usb_shows_the_default_menu() {
    // initialize_usb seeds PDB t16 with all 27 player browse categories
    // and marks 10 of them visible in eDB.
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
    // The first and last rows can only move one way, and the labels are ready to show.
    let last = data.current_items.len() - 1;
    assert!(!data.current_items[0].can_move_up && data.current_items[0].can_move_down);
    assert!(data.current_items[last].can_move_up && !data.current_items[last].can_move_down);
    assert!(data.current_items[1].can_move_up && data.current_items[1].can_move_down);
    assert!(data.current_items.iter().all(|item| !item.label.is_empty()));
    assert!(
        data.available_items
            .iter()
            .all(|item| !item.can_move_up && !item.can_move_down)
    );
    assert_eq!(data.status_text, "Player menu loaded");
}

#[test]
fn menu_works_with_only_the_edb() {
    // A missing database is fine: the edit goes to the one that exists.
    let usb_dir = tempdir().expect("usb tempdir");
    let usb_root = usb_dir.path().to_path_buf();
    let data_dir = tempdir().expect("data tempdir");
    let backend = BackendCommands::new(data_dir.path()).expect("create backend");
    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().into_owned(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");
    let pdb = usb_root.join("PIONEER/rekordbox/export.pdb");
    fs::remove_file(&pdb).expect("remove PDB");

    let root = Some(usb_root.to_string_lossy().into_owned());
    let before = backend
        .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
            usb_root: root.clone(),
        })
        .data
        .expect("menu without a PDB");
    let first = before.current_items[0].kind;

    let edit = edit_menu(&backend, &usb_root, UsbPlayerMenuEdit::MoveDown, first);
    assert!(edit.ok, "edit without a PDB failed: {edit:?}");
    let edit = edit.data.expect("edit data");
    assert!(edit.updated);
    assert_eq!(edit.current_items[1].kind, first);
    assert!(!pdb.exists(), "the edit must not create a PDB");
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
    let first = before.current_items.first().expect("a shown item").kind;

    let pdb_path = usb_root.join("PIONEER/rekordbox/export.pdb");
    let pdb_before = fs::read(&pdb_path).expect("read pdb");

    // Up on the first item changes nothing.
    let save = edit_menu(&backend, &usb_root, UsbPlayerMenuEdit::MoveUp, first);
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
    const TRACK: u32 = 131;
    assert!(config.current_items.iter().any(|i| i.kind == TRACK));

    let resp = edit_menu(&backend, &usb_root, UsbPlayerMenuEdit::Remove, TRACK);
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

    let data = set_menu_with_edits(&backend, &usb_root, &vendor_kinds);

    let after_kinds: Vec<u32> = data.current_items.iter().map(|i| i.kind).collect();
    assert_eq!(
        after_kinds, vendor_kinds,
        "eDB active order must match request"
    );
    let pdb_rows = load_pdb_t16_decoded(&usb_root).expect("load PDB after update");
    let pdb_kinds: Vec<u32> = pdb_rows.iter().map(|row| u32::from(row.kind)).collect();
    assert_eq!(pdb_kinds.len(), 27, "PDB t16 must keep the full catalog");
    assert!(pdb_kinds.contains(&139), "PDB t16 must still include KEY");

    // An edit that applies says so, and one that doesn't says that.
    let root = Some(usb_root.to_string_lossy().into_owned());
    let edit = |action| {
        backend
            .update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
                usb_root: root.clone(),
                action,
                kind: vendor_kinds[0],
            })
            .data
            .expect("edit data")
            .status_text
    };
    assert_eq!(edit(UsbPlayerMenuEdit::MoveUp), "Player menu unchanged");
    assert_eq!(edit(UsbPlayerMenuEdit::MoveDown), "Player menu updated");
    assert_eq!(edit(UsbPlayerMenuEdit::MoveUp), "Player menu updated");

    // The background colour says what it did too, and its chips follow.
    let set_color = |color| {
        backend
            .set_usb_background_color(backend::models::SetUsbBackgroundColorRequest {
                usb_root: root.clone(),
                color,
            })
            .data
            .expect("colour data")
    };
    let set = set_color(5);
    assert_eq!(set.status_text, "Background colour set to Green");
    assert_eq!(
        set.chips
            .iter()
            .filter(|chip| chip.selected)
            .map(|chip| chip.color)
            .collect::<Vec<_>>(),
        [5]
    );
    assert_eq!(set.note_text, "");
    assert_eq!(set_color(5).status_text, "Background colour unchanged");

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
}
