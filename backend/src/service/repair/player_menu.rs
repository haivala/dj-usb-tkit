//! Player menu config: reading, editing, and eDB↔PDB sync.

use super::*;

/// Player menu kinds that cannot be removed once present in the current menu.
/// TRACK=131, PLAYLIST=132, FOLDER=144, SEARCH=145, HISTORY=149.
pub(super) const REQUIRED_PLAYER_MENU_KINDS: &[u32] = &[131, 132, 144, 145, 149];

/// Whether a player-menu item of this kind may be removed from the current
/// menu. Mirrored onto `UsbPlayerMenuItem.removable` so the frontend never
/// re-derives the set; `update_usb_player_menu_config` is the authority.
pub(super) fn player_menu_kind_removable(kind: u32) -> bool {
    !REQUIRED_PLAYER_MENU_KINDS.contains(&kind)
}

pub(super) fn normalize_player_menu_name(raw: &str) -> String {
    raw.replace(['\u{fffa}', '\u{fffb}'], "").trim().to_string()
}

#[derive(Debug, Clone)]
pub(super) struct EdbMenuRow {
    menu_item_id: u32,
    kind: u32,
    name: String,
    sequence_no: Option<u32>,
    is_visible: bool,
}

pub(super) fn load_edb_menu_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<EdbMenuRow>> {
    let Some(conn) = open_edb_rw(usb_root, warnings) else {
        return Err(crate::error::BackendError::Validation(
            "unable to open eDB for player menu config".to_string(),
        ));
    };
    if !table_exists(&conn, "menuItem") || !table_exists(&conn, "category") {
        return Err(crate::error::BackendError::Validation(
            "eDB missing menuItem/category tables".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        r#"
        SELECT m.menuItem_id,
               COALESCE(m.kind, 0),
               COALESCE(m.name, ''),
               c.sequenceNo,
               COALESCE(c.isVisible, 0)
        FROM menuItem m
        LEFT JOIN category c ON c.menuItem_id = m.menuItem_id
        ORDER BY COALESCE(c.sequenceNo, 2147483647), m.menuItem_id
        "#,
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(EdbMenuRow {
                menu_item_id: u32::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                kind: u32::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                name: normalize_player_menu_name(&row.get::<_, String>(2)?),
                sequence_no: row
                    .get::<_, Option<i64>>(3)?
                    .and_then(|v| u32::try_from(v).ok()),
                is_visible: row.get::<_, i64>(4).unwrap_or(0) == 1,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load the player menu state from a USB, treating eDB.category as the master source.
///
/// eDB is what the player software reads; PDB t16 is a secondary index that
/// must mirror eDB. `current` is the eDB-visible set in sequenceNo order;
/// `available` is the full eDB menuItem catalog minus the visible set.
///
/// `divergence` summarises where PDB t16 diverges from the eDB visible set.
/// Non-empty divergence means the two indexes disagree; Fix PDB sync resolves it.
pub(crate) fn load_usb_player_menu_config_public(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(
    Vec<UsbPlayerMenuItem>,
    Vec<UsbPlayerMenuItem>,
    UsbPlayerMenuDivergence,
)> {
    load_usb_player_menu_config(usb_root, warnings)
}

/// Build PDB t17 8-byte rows from the current eDB.category state.
/// Used to keep PDB t17 in sync with eDB.category after a menu config save.
pub(super) fn load_t17_encoded_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<[u8; 8]>> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT c.category_id, c.menuItem_id, c.sequenceNo, c.isVisible, m.kind
         FROM category c
         JOIN menuItem m ON m.menuItem_id = c.menuItem_id
         ORDER BY c.category_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (cat_id, menu_item_id, seq_no, is_visible, kind) = r?;
        out.push(crate::service::export_helpers::encode_pdb_t17_cat_row(
            u16::try_from(menu_item_id).unwrap_or(0),
            u16::try_from(cat_id).unwrap_or(0),
            is_visible != 0,
            u32::try_from(kind).unwrap_or(0),
            u16::try_from(seq_no).unwrap_or(0),
        ));
    }
    Ok(out)
}

pub(super) fn load_usb_player_menu_config(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(
    Vec<UsbPlayerMenuItem>,
    Vec<UsbPlayerMenuItem>,
    UsbPlayerMenuDivergence,
)> {
    let edb_rows = load_edb_menu_rows(usb_root, warnings)?;
    let pdb_rows = crate::service::export_helpers::load_pdb_t16_decoded(usb_root)?;

    let pdb_kinds: HashSet<u32> = pdb_rows.iter().map(|r| u32::from(r.kind)).collect();

    // current = eDB visible items in eDB sequenceNo order.
    let mut edb_visible: Vec<&EdbMenuRow> = edb_rows.iter().filter(|r| r.is_visible).collect();
    edb_visible.sort_by_key(|r| (r.sequence_no.unwrap_or(u32::MAX), r.menu_item_id));
    let edb_visible_kinds: HashSet<u32> = edb_visible.iter().map(|r| r.kind).collect();

    let mut current = Vec::<UsbPlayerMenuItem>::with_capacity(edb_visible.len());
    for (seq, row) in edb_visible.iter().enumerate() {
        let origin = if pdb_kinds.contains(&row.kind) {
            UsbPlayerMenuItemOrigin::Both
        } else {
            UsbPlayerMenuItemOrigin::EdbOnly
        };
        current.push(UsbPlayerMenuItem {
            menu_item_id: row.menu_item_id,
            kind: row.kind,
            name: row.name.clone(),
            is_visible: true,
            sequence_no: Some(u32::try_from(seq).unwrap_or(u32::MAX)),
            origin,
            removable: player_menu_kind_removable(row.kind),
        });
    }

    // available = all eDB menuItems not currently visible.
    let mut available = Vec::<UsbPlayerMenuItem>::new();
    for row in &edb_rows {
        if row.is_visible {
            continue;
        }
        let origin = if pdb_kinds.contains(&row.kind) {
            UsbPlayerMenuItemOrigin::PdbOnly
        } else {
            UsbPlayerMenuItemOrigin::EdbOnly
        };
        available.push(UsbPlayerMenuItem {
            menu_item_id: row.menu_item_id,
            kind: row.kind,
            name: row.name.clone(),
            is_visible: false,
            sequence_no: None,
            origin,
            removable: player_menu_kind_removable(row.kind),
        });
    }
    available.sort_by_key(|item| item.menu_item_id);

    // divergence = where PDB t16 diverges from the eDB visible set.
    let mut in_pdb_only: Vec<u32> = pdb_kinds
        .iter()
        .copied()
        .filter(|k| !edb_visible_kinds.contains(k))
        .collect();
    let mut in_edb_visible_only: Vec<u32> = edb_visible_kinds
        .iter()
        .copied()
        .filter(|k| !pdb_kinds.contains(k))
        .collect();
    in_pdb_only.sort_unstable();
    in_pdb_only.dedup();
    in_edb_visible_only.sort_unstable();
    in_edb_visible_only.dedup();

    let pdb_order: Vec<u32> = pdb_rows.iter().map(|r| u32::from(r.kind)).collect();
    let common_from_pdb: Vec<u32> = pdb_order
        .iter()
        .copied()
        .filter(|k| edb_visible_kinds.contains(k))
        .collect();
    let common_from_edb: Vec<u32> = edb_visible
        .iter()
        .map(|r| r.kind)
        .filter(|k| pdb_kinds.contains(k))
        .collect();
    let order_mismatch = common_from_pdb != common_from_edb;

    let edb_all_kinds: HashSet<u32> = edb_rows.iter().map(|r| r.kind).collect();
    let mut pdb_missing_kinds: Vec<u32> = edb_all_kinds
        .iter()
        .copied()
        .filter(|k| !pdb_kinds.contains(k))
        .collect();
    pdb_missing_kinds.sort_unstable();

    let can_sync = !in_edb_visible_only.is_empty();
    let can_restore = !pdb_missing_kinds.is_empty();
    let mut summary_parts = Vec::new();
    if can_sync {
        summary_parts.push(format!(
            "{} active menu items missing from PDB",
            in_edb_visible_only.len()
        ));
    }
    if can_restore {
        summary_parts.push(format!(
            "PDB missing {} browse categories",
            pdb_missing_kinds.len()
        ));
    }
    let summary = if summary_parts.is_empty() {
        String::new()
    } else {
        format!("{}.", summary_parts.join("; "))
    };

    Ok((
        current,
        available,
        UsbPlayerMenuDivergence {
            in_edb_visible_only,
            in_pdb_only,
            order_mismatch,
            pdb_missing_kinds,
            summary,
            can_sync,
            can_restore,
        },
    ))
}

/// Mirror eDB `category` rows from an ordered list of PDB t16 kinds.
///
/// PDB is master; eDB must reflect the same kind set in the same order or
/// Newer players may refuse the USB. For each kind in `pdb_kind_order`, the matching
/// eDB `category` row is set `isVisible = 1` with `sequenceNo = i`; if no
/// category row exists for that kind's menuItem, one is inserted. All other
/// eDB `category` rows are set `isVisible = 0`. Kinds in PDB that have no
/// matching eDB `menuItem` row are left unmirrored (eDB cannot show them);
/// this is surfaced via a warning so callers can notice.
pub(super) fn mirror_edb_category_from_pdb_kinds(
    usb_root: &std::path::Path,
    pdb_kind_order: &[u32],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<bool> {
    let Some(mut conn) = open_edb_rw(usb_root, warnings) else {
        return Err(crate::error::BackendError::Validation(
            "unable to open eDB for player menu update".to_string(),
        ));
    };
    if !table_exists(&conn, "menuItem") || !table_exists(&conn, "category") {
        return Err(crate::error::BackendError::Validation(
            "eDB missing menuItem/category tables".to_string(),
        ));
    }

    let mut desired_kinds = Vec::<u32>::new();
    let mut seen = HashSet::<u32>::new();
    for kind in pdb_kind_order {
        if seen.insert(*kind) {
            desired_kinds.push(*kind);
        }
    }

    let menu_item_id_by_kind: HashMap<u32, i64> = {
        let mut stmt = conn.prepare("SELECT menuItem_id, kind FROM menuItem")?;
        stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
            .filter_map(|result| result.ok())
            .filter_map(|(id, kind)| u32::try_from(kind).ok().map(|k| (k, id)))
            .collect()
    };

    {
        let tx = conn.transaction()?;

        let category_rows = {
            let mut stmt = tx.prepare("SELECT category_id, menuItem_id FROM category")?;
            stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut category_by_menu = HashMap::<i64, i64>::new();
        let mut max_category_id = 0i64;
        for (category_id, menu_item_id) in category_rows {
            if category_id > max_category_id {
                max_category_id = category_id;
            }
            category_by_menu.entry(menu_item_id).or_insert(category_id);
        }

        let mut mirrored_menu_ids = HashSet::<i64>::new();
        for (seq, kind) in desired_kinds.iter().enumerate() {
            let Some(menu_id) = menu_item_id_by_kind.get(kind).copied() else {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-player-menu",
                    "usb.player-menu.pdb-kind-unmapped",
                    format!("PDB kind {kind} has no matching eDB menuItem; eDB cannot mirror it"),
                ));
                continue;
            };
            mirrored_menu_ids.insert(menu_id);
            let seq_i64 = i64::try_from(seq + 1).unwrap_or(1);
            if let Some(category_id) = category_by_menu.get(&menu_id).copied() {
                tx.execute(
                    "UPDATE category SET isVisible = 1, sequenceNo = ?2 WHERE category_id = ?1",
                    params![category_id, seq_i64],
                )?;
            } else {
                max_category_id += 1;
                tx.execute(
                    "INSERT INTO category (category_id, menuItem_id, sequenceNo, isVisible) VALUES (?1, ?2, ?3, 1)",
                    params![max_category_id, menu_id, seq_i64],
                )?;
                category_by_menu.insert(menu_id, max_category_id);
            }
        }

        for (menu_id, category_id) in category_by_menu.iter() {
            if mirrored_menu_ids.contains(menu_id) {
                continue;
            }
            tx.execute(
                "UPDATE category SET isVisible = 0, sequenceNo = 0 WHERE category_id = ?1",
                params![category_id],
            )?;
        }

        tx.commit()?;
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(true)
}

/// The menu order after `action` on `kind`. An edit that doesn't apply (adding
/// a shown kind, moving the first one up, a kind not in the menu) leaves the
/// order as it is.
pub(crate) fn apply_player_menu_edit(
    current: &[u32],
    action: UsbPlayerMenuEdit,
    kind: u32,
) -> Vec<u32> {
    let mut kinds = current.to_vec();
    let at = kinds.iter().position(|&k| k == kind);
    match (action, at) {
        (UsbPlayerMenuEdit::Add, None) => kinds.push(kind),
        (UsbPlayerMenuEdit::Remove, Some(i)) => {
            kinds.remove(i);
        }
        (UsbPlayerMenuEdit::MoveUp, Some(i)) if i > 0 => kinds.swap(i, i - 1),
        (UsbPlayerMenuEdit::MoveDown, Some(i)) if i + 1 < kinds.len() => kinds.swap(i, i + 1),
        _ => {}
    }
    kinds
}

impl BackendService {
    pub fn get_usb_player_menu_config(
        &self,
        req: GetUsbPlayerMenuConfigRequest,
    ) -> BackendResult<GetUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        let (current_items, available_items, divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;
        Ok(GetUsbPlayerMenuConfigData {
            current_items,
            available_items,
            divergence,
            warnings,
        })
    }

    /// Restore PDB t16 from the full eDB menuItem catalog.
    ///
    /// Reads ALL eDB menuItem rows (visible and hidden) and writes them to PDB
    /// t16 in menuItem_id order. This repairs USBs where PDB was trimmed by
    /// old code, restoring the full browse category set.
    pub fn sync_usb_player_menu_edb_to_pdb(
        &self,
        req: GetUsbPlayerMenuConfigRequest,
    ) -> BackendResult<UpdateUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        warnings.extend(
            self.backup_usb_databases_with_retention(&usb_root, "Before repair", None)
                .into_iter()
                .map(|message| {
                    logging::log(
                        Level::Info,
                        "usb-player-menu",
                        "usb.player-menu.backup",
                        message,
                    )
                }),
        );

        let mut edb_rows = load_edb_menu_rows(&usb_root, &mut warnings)?;
        edb_rows.sort_by_key(|r| r.menu_item_id);

        let all_rows: Vec<(u16, String)> = edb_rows
            .iter()
            .filter_map(|r| u16::try_from(r.kind).ok().map(|k| (k, r.name.clone())))
            .collect();

        let pdb_updated = crate::service::export_helpers::patch_pdb_columns_menu_set_by_kind(
            &usb_root, &all_rows,
        )?;
        if pdb_updated {
            warnings.push(logging::log(
                Level::Info,
                "usb-player-menu",
                "usb.player-menu.t16-restored",
                "PDB t16 columns restored from eDB menuItem catalog",
            ));
        }

        match load_t17_encoded_rows(&usb_root, &mut warnings) {
            Ok(t17_rows) if !t17_rows.is_empty() => {
                match crate::service::export_helpers::patch_pdb_t17_category_snapshot(
                    &usb_root, &t17_rows,
                ) {
                    Ok(true) => {
                        warnings.push(logging::log(
                            Level::Info,
                            "usb-player-menu",
                            "usb.player-menu.t17-snapshot-updated",
                            "PDB t17 category snapshot updated",
                        ));
                    }
                    Ok(false) => {}
                    Err(e) => {
                        warnings.push(logging::log(
                            Level::Warn,
                            "usb-player-menu",
                            "usb.player-menu.t17-update-skipped",
                            format!("PDB t17 update skipped: {e}"),
                        ));
                    }
                }
            }
            _ => {}
        }

        if let Err(err) = usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Pdb) {
            warnings.push(logging::log(
                Level::Error,
                "usb-player-menu",
                "usb.player-menu.pdb-write-back-failed",
                format!("PDB write-back to USB failed: {err}"),
            ));
        }
        warnings.extend(self.refresh_engine_library(&usb_root));

        let (current_items, available_items, divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;

        Ok(UpdateUsbPlayerMenuConfigData {
            updated: pdb_updated,
            current_items,
            available_items,
            divergence,
            warnings,
        })
    }

    pub fn update_usb_player_menu_config(
        &self,
        req: UpdateUsbPlayerMenuConfigRequest,
    ) -> BackendResult<UpdateUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        let (before_current, before_available, before_divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;

        // Build lookup: (kind -> display name), preferring PDB-sourced names
        // (they came from the stick itself) and falling back to eDB names for
        // kinds only available there.
        let mut name_by_kind = HashMap::<u32, String>::new();
        for item in &before_available {
            name_by_kind.insert(item.kind, item.name.clone());
        }
        for item in &before_current {
            name_by_kind.insert(item.kind, item.name.clone());
        }

        let before_kinds = before_current
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        let desired_kinds = apply_player_menu_edit(&before_kinds, req.action, req.kind);

        if desired_kinds == before_kinds {
            // Nothing to write (and no backup to make).
            return Ok(UpdateUsbPlayerMenuConfigData {
                updated: false,
                current_items: before_current,
                available_items: before_available,
                divergence: before_divergence,
                warnings,
            });
        }

        // Reject removing a browse category the CDJ requires.
        let desired_set: HashSet<u32> = desired_kinds.iter().copied().collect();
        for &kind in REQUIRED_PLAYER_MENU_KINDS {
            if before_current.iter().any(|item| item.kind == kind) && !desired_set.contains(&kind) {
                let name = name_by_kind.get(&kind).map(|s| s.as_str()).unwrap_or("?");
                return Err(crate::error::BackendError::Validation(format!(
                    "Player menu item \"{name}\" (kind {kind}) cannot be removed"
                )));
            }
        }

        // Validate desired kinds: drop any with no known display name.
        let mut final_kinds = Vec::<u32>::with_capacity(desired_kinds.len());
        for kind in &desired_kinds {
            if !name_by_kind.contains_key(kind) {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-player-menu",
                    "usb.player-menu.dropped-unknown-kind",
                    format!("dropped unknown player menu kind {kind}: no display name available"),
                ));
                continue;
            }
            final_kinds.push(*kind);
        }

        warnings.extend(
            self.backup_usb_databases_with_retention(&usb_root, "Before menu update", None)
                .into_iter()
                .map(|message| {
                    logging::log(
                        Level::Info,
                        "usb-player-menu",
                        "usb.player-menu.backup",
                        message,
                    )
                }),
        );

        // Snapshot eDB for rollback if the write fails. Staged path: this must
        // match whatever path `mirror_edb_category_from_pdb_kinds`'s
        // `open_edb_rw` actually mutates (the local staged copy when staging
        // is active), or the snapshot/restore would operate on a file the
        // write never touched.
        let edb_path = usb_staging::stage_edb(&usb_root)?;
        let edb_snapshot = if edb_path.is_file() {
            std::fs::read(&edb_path).ok()
        } else {
            None
        };

        if let Err(err) = mirror_edb_category_from_pdb_kinds(&usb_root, &final_kinds, &mut warnings)
        {
            warnings.push(logging::log(
                Level::Error,
                "usb-player-menu",
                "usb.player-menu.edb-write-failed",
                format!("eDB write failed ({err}); restoring eDB from snapshot"),
            ));
            if let Some(snapshot) = edb_snapshot.as_ref() {
                let _ = std::fs::write(&edb_path, snapshot);
            }
            // Deliberately no write_back_if_changed here: the local copy was
            // just restored to its pre-attempt state, so the real USB drive
            // must never see this failed attempt.
            return Err(err);
        }
        if let Err(err) = usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Edb) {
            warnings.push(logging::log(
                Level::Error,
                "usb-player-menu",
                "usb.player-menu.edb-write-back-failed",
                format!("eDB write-back to USB failed: {err}"),
            ));
        }

        let (current_items, available_items, divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;
        let after_kinds = current_items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        let updated = before_kinds != after_kinds;

        if updated {
            match load_t17_encoded_rows(&usb_root, &mut warnings) {
                Ok(t17_rows) if !t17_rows.is_empty() => {
                    match crate::service::export_helpers::patch_pdb_t17_category_snapshot(
                        &usb_root, &t17_rows,
                    ) {
                        Ok(true) => {
                            warnings.push(logging::log(
                                Level::Info,
                                "usb-player-menu",
                                "usb.player-menu.t17-snapshot-updated",
                                "PDB t17 category snapshot updated",
                            ));
                        }
                        Ok(false) => {}
                        Err(e) => {
                            warnings.push(logging::log(
                                Level::Warn,
                                "usb-player-menu",
                                "usb.player-menu.t17-update-skipped",
                                format!("PDB t17 update skipped: {e}"),
                            ));
                        }
                    }
                }
                _ => {}
            }
            if let Err(err) =
                usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Pdb)
            {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-player-menu",
                    "usb.player-menu.pdb-write-back-failed",
                    format!("PDB write-back to USB failed: {err}"),
                ));
            }
            warnings.extend(self.refresh_engine_library(&usb_root));
        }

        Ok(UpdateUsbPlayerMenuConfigData {
            updated,
            current_items,
            available_items,
            divergence,
            warnings,
        })
    }
}
