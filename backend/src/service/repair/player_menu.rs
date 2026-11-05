//! Player menu config: reading and editing it in both databases.
//!
//! The menu is held twice: eDB `category`, and PDB `t17`, one 8-byte row per
//! category with the item, the category, a byte rekordbox keeps per row, a flag
//! (1 = hidden) and the order. PDB `t16` is the catalog that names the items and
//! is never changed here. A stick with only one database uses that one.

use super::*;

/// Player menu kinds that cannot be removed once present in the current menu.
/// TRACK=131, PLAYLIST=132, FOLDER=144, SEARCH=145, HISTORY=149, as rekordbox
/// locks them, and DATE ADDED.
pub(super) const REQUIRED_PLAYER_MENU_KINDS: &[u32] = &[131, 132, 140, 144, 145, 149];

/// DATE ADDED looks forced on: both databases list it on an untouched stick while
/// rekordbox's editor doesn't count it, and rekordbox's own edits leave its row
/// out of step between them. So it always shows, can't be moved or removed, and
/// its rows are never written.
const DATE_ADDED_KIND: u32 = 140;
const T17_FLAG_HIDDEN: u8 = 1;

/// A menu item's id and its encoded t17 row.
type T17RowByItem = (u32, [u8; 8]);

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
/// Build PDB t17 8-byte rows from the current eDB.category state, by item, and
/// DATE ADDED's item.
/// Used to keep PDB t17 in sync with eDB.category after a menu config save.
pub(super) fn load_t17_encoded_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(Vec<T17RowByItem>, Option<u32>)> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return Ok((Vec::new(), None));
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
    let mut date_added_item = None;
    for r in rows {
        let (cat_id, menu_item_id, seq_no, is_visible, kind) = r?;
        let kind = u32::try_from(kind).unwrap_or(0);
        let item = u32::try_from(menu_item_id).unwrap_or(0);
        if kind == DATE_ADDED_KIND {
            date_added_item = Some(item);
        }
        out.push((
            item,
            crate::service::export_helpers::encode_pdb_t17_cat_row(
                u16::try_from(menu_item_id).unwrap_or(0),
                u16::try_from(cat_id).unwrap_or(0),
                is_visible != 0,
                kind,
                u16::try_from(seq_no).unwrap_or(0),
            ),
        ));
    }
    Ok((out, date_added_item))
}

/// A menu row's text.
fn player_menu_label(name: &str, kind: u32) -> String {
    let name = name.trim();
    if name.is_empty() {
        format!("Menu {kind}")
    } else {
        name.to_string()
    }
}

/// One menu row as a database holds it, whichever database that is.
#[derive(Debug, Clone)]
struct MenuRow {
    menu_item_id: u32,
    kind: u32,
    name: String,
    visible: bool,
    sequence_no: u32,
}

fn t17_row_item(row: &[u8; 8]) -> u32 {
    u32::from(u16::from_le_bytes([row[0], row[1]]))
}

fn t17_row_sequence(row: &[u8; 8]) -> u16 {
    u16::from_le_bytes([row[6], row[7]])
}

/// The eDB's menu. An item with no `category` row isn't a browse category:
/// COMMENTS and the like are `sort` items, and CUE is in neither.
fn edb_menu_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<MenuRow>> {
    Ok(load_edb_menu_rows(usb_root, warnings)?
        .into_iter()
        .filter_map(|row| {
            Some(MenuRow {
                sequence_no: row.sequence_no?,
                menu_item_id: row.menu_item_id,
                kind: row.kind,
                name: row.name,
                visible: row.is_visible,
            })
        })
        .collect())
}

/// The PDB's menu: `t17` says what shows and in what order, `t16` names it.
fn pdb_menu_rows(usb_root: &std::path::Path) -> BackendResult<Vec<MenuRow>> {
    let catalog = crate::service::export_helpers::load_pdb_t16_decoded(usb_root)?;
    let rows = crate::service::export_helpers::load_pdb_t17_live_rows(usb_root)?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let menu_item_id = t17_row_item(row);
            let entry = catalog
                .iter()
                .find(|entry| u32::from(entry.id) == menu_item_id)?;
            Some(MenuRow {
                menu_item_id,
                kind: u32::from(entry.kind),
                name: normalize_player_menu_name(&entry.name),
                visible: row[5] != T17_FLAG_HIDDEN,
                sequence_no: u32::from(t17_row_sequence(row)),
            })
        })
        .collect())
}

fn stick_has_edb(usb_root: &std::path::Path) -> BackendResult<bool> {
    Ok(usb_staging::stage_edb(usb_root)?.is_file())
}

/// The menu rows, from the eDB when the stick has one, else from the PDB.
fn menu_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<MenuRow>> {
    if stick_has_edb(usb_root)? {
        return edb_menu_rows(usb_root, warnings);
    }
    let rows = pdb_menu_rows(usb_root)?;
    if !rows.is_empty() {
        return Ok(rows);
    }
    // No t17 rows: older players show the whole catalog.
    let catalog = crate::service::export_helpers::load_pdb_t16_decoded(usb_root)?;
    if catalog.is_empty() {
        return Err(crate::error::BackendError::Validation(
            "this USB has no player menu in its eDB or PDB".to_string(),
        ));
    }
    Ok((1u32..)
        .zip(catalog)
        .map(|(sequence_no, entry)| MenuRow {
            menu_item_id: u32::from(entry.id),
            kind: u32::from(entry.kind),
            name: normalize_player_menu_name(&entry.name),
            visible: true,
            sequence_no,
        })
        .collect())
}

/// Whether edits can be written: with no eDB and no t17 rows there's nothing to
/// update, and t16 is never changed.
fn menu_is_editable(usb_root: &std::path::Path) -> BackendResult<bool> {
    Ok(stick_has_edb(usb_root)?
        || !crate::service::export_helpers::load_pdb_t17_live_rows(usb_root)?.is_empty())
}

/// What shows, in order, leaving DATE ADDED out: its rows are out of step on
/// rekordbox-written sticks, and it isn't edited.
fn compared_menu_order(rows: &[MenuRow]) -> Vec<u32> {
    let mut shown: Vec<&MenuRow> = rows
        .iter()
        .filter(|row| row.visible && row.kind != DATE_ADDED_KIND)
        .collect();
    shown.sort_by_key(|row| (row.sequence_no, row.menu_item_id));
    shown.iter().map(|row| row.menu_item_id).collect()
}

/// Whether both databases hold a menu and they show different ones.
fn menu_databases_differ(usb_root: &std::path::Path, warnings: &mut Vec<WarningEntry>) -> bool {
    if !stick_has_edb(usb_root).unwrap_or(false) {
        return false;
    }
    let (Ok(edb), Ok(pdb)) = (edb_menu_rows(usb_root, warnings), pdb_menu_rows(usb_root)) else {
        return false;
    };
    !pdb.is_empty() && compared_menu_order(&edb) != compared_menu_order(&pdb)
}

pub(super) fn load_usb_player_menu_config(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(Vec<UsbPlayerMenuItem>, Vec<UsbPlayerMenuItem>)> {
    let editable = menu_is_editable(usb_root)?;
    let mut rows = menu_rows(usb_root, warnings)?;
    rows.sort_by_key(|row| (row.sequence_no, row.menu_item_id));
    // DATE ADDED always shows. Rekordbox can leave the eDB saying it's hidden.
    for row in &mut rows {
        if row.kind == DATE_ADDED_KIND && !row.visible {
            row.visible = true;
            row.sequence_no = u32::MAX;
        }
    }
    rows.sort_by_key(|row| (row.sequence_no, row.menu_item_id));

    let movable: Vec<u32> = rows
        .iter()
        .filter(|row| row.visible && row.kind != DATE_ADDED_KIND)
        .map(|row| row.kind)
        .collect();
    let item =
        |row: &MenuRow, sequence_no: Option<u32>, position: Option<usize>| UsbPlayerMenuItem {
            menu_item_id: row.menu_item_id,
            kind: row.kind,
            name: row.name.clone(),
            is_visible: row.visible,
            sequence_no,
            removable: editable && player_menu_kind_removable(row.kind),
            label: player_menu_label(&row.name, row.kind),
            can_move_up: editable && position.is_some_and(|at| at > 0),
            can_move_down: editable && position.is_some_and(|at| at + 1 < movable.len()),
        };
    let current = rows
        .iter()
        .filter(|row| row.visible)
        .enumerate()
        .map(|(seq, row)| {
            let position = movable.iter().position(|&kind| kind == row.kind);
            item(row, Some(u32::try_from(seq).unwrap_or(u32::MAX)), position)
        })
        .collect();
    let mut available: Vec<UsbPlayerMenuItem> = rows
        .iter()
        .filter(|row| !row.visible)
        .map(|row| item(row, None, None))
        .collect();
    available.sort_by_key(|item| item.menu_item_id);
    Ok((current, available))
}

/// The order values for `kinds`, counting from 1 and skipping `reserved`: DATE
/// ADDED's value, which stays as it is.
fn menu_sequence_values(kinds: &[u32], reserved: Option<u32>) -> Vec<(u32, u32)> {
    let mut next = 0u32;
    kinds
        .iter()
        .map(|&kind| {
            next += 1;
            if Some(next) == reserved {
                next += 1;
            }
            (kind, next)
        })
        .collect()
}

/// `live` with each row's flag and order taken from `wanted`, a menu state by
/// item. Everything else in a row stays, and rows stay in place. DATE ADDED's row
/// isn't touched. A row the table lacks is added after the others.
fn updated_t17_rows(
    live: &[[u8; 8]],
    wanted: &[T17RowByItem],
    date_added_item: Option<u32>,
) -> Vec<[u8; 8]> {
    let mut rows: Vec<[u8; 8]> = live
        .iter()
        .map(|row| {
            let mut row = *row;
            if Some(t17_row_item(&row)) == date_added_item {
                return row;
            }
            if let Some((_, new)) = wanted.iter().find(|(item, _)| *item == t17_row_item(&row)) {
                row[5] = new[5];
                row[6] = new[6];
                row[7] = new[7];
            }
            row
        })
        .collect();
    for (item, new) in wanted {
        let known = live.iter().any(|row| t17_row_item(row) == *item);
        if !known && Some(*item) != date_added_item {
            rows.push(*new);
        }
    }
    rows
}

/// The t17 rows for the eDB's menu, on top of the rows the PDB already has.
fn t17_rows_from_edb(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<[u8; 8]>> {
    let (encoded, date_added_item) = load_t17_encoded_rows(usb_root, warnings)?;
    let live = crate::service::export_helpers::load_pdb_t17_live_rows(usb_root)?;
    Ok(updated_t17_rows(&live, &encoded, date_added_item))
}

/// The t17 rows for a stick with no eDB: `kinds` show, in order, the rest hide.
fn t17_rows_for_kinds(usb_root: &std::path::Path, kinds: &[u32]) -> BackendResult<Vec<[u8; 8]>> {
    let rows = pdb_menu_rows(usb_root)?;
    let live = crate::service::export_helpers::load_pdb_t17_live_rows(usb_root)?;
    let date_added = rows.iter().find(|row| row.kind == DATE_ADDED_KIND);
    let values = menu_sequence_values(kinds, date_added.map(|row| row.sequence_no));
    let wanted: Vec<T17RowByItem> = rows
        .iter()
        .map(|row| {
            let value = values.iter().find(|(kind, _)| *kind == row.kind);
            let sequence_no = value.map_or(0, |(_, seq)| *seq);
            let new = crate::service::export_helpers::encode_pdb_t17_cat_row(
                u16::try_from(row.menu_item_id).unwrap_or(0),
                0,
                value.is_some(),
                row.kind,
                u16::try_from(sequence_no).unwrap_or(0),
            );
            (row.menu_item_id, new)
        })
        .collect();
    Ok(updated_t17_rows(
        &live,
        &wanted,
        date_added.map(|row| row.menu_item_id),
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
        if *kind != DATE_ADDED_KIND && seen.insert(*kind) {
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

        // DATE ADDED's row stays exactly as it is.
        let date_added_menu_id = menu_item_id_by_kind.get(&DATE_ADDED_KIND).copied();
        let date_added_sequence: Option<u32> = match date_added_menu_id {
            Some(menu_id) => tx
                .query_row(
                    "SELECT sequenceNo FROM category WHERE menuItem_id = ?1 AND isVisible = 1",
                    params![menu_id],
                    |row| row.get::<_, i64>(0),
                )
                .ok()
                .and_then(|value| u32::try_from(value).ok()),
            None => None,
        };
        let sequence_values = menu_sequence_values(&desired_kinds, date_added_sequence);

        let mut mirrored_menu_ids = HashSet::<i64>::new();
        mirrored_menu_ids.extend(date_added_menu_id);
        for (kind, seq) in &sequence_values {
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
            let seq_i64 = i64::from(*seq);
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
        let (current_items, available_items) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;
        if menu_databases_differ(&usb_root, &mut warnings) {
            warnings.push(logging::log(
                Level::Warn,
                "usb-player-menu",
                "usb.player-menu.databases-differ",
                "The PDB and eDB show different player menus. Showing the eDB's: the next menu change writes it to both.",
            ));
        }
        Ok(GetUsbPlayerMenuConfigData {
            status_text: "Player menu loaded".to_string(),
            current_items,
            available_items,
            warnings,
        })
    }

    pub fn update_usb_player_menu_config(
        &self,
        req: UpdateUsbPlayerMenuConfigRequest,
    ) -> BackendResult<UpdateUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        let (before_current, before_available) =
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

        if !menu_is_editable(&usb_root)? {
            return Err(crate::error::BackendError::Validation(
                "this USB's player menu can't be changed: it has no eDB and no menu rows in its PDB"
                    .to_string(),
            ));
        }
        let has_edb = stick_has_edb(&usb_root)?;
        let databases_differ = menu_databases_differ(&usb_root, &mut warnings);

        // Reject removing a browse category the CDJ requires.
        if req.action == UsbPlayerMenuEdit::Remove
            && !player_menu_kind_removable(req.kind)
            && before_current.iter().any(|item| item.kind == req.kind)
        {
            let name = name_by_kind
                .get(&req.kind)
                .map(|s| s.as_str())
                .unwrap_or("?");
            return Err(crate::error::BackendError::Validation(format!(
                "Player menu item \"{name}\" (kind {}) cannot be removed",
                req.kind
            )));
        }

        let before_kinds = before_current
            .iter()
            .map(|item| item.kind)
            .filter(|&kind| kind != DATE_ADDED_KIND)
            .collect::<Vec<_>>();
        let desired_kinds = if req.kind == DATE_ADDED_KIND {
            before_kinds.clone()
        } else {
            apply_player_menu_edit(&before_kinds, req.action, req.kind)
        };

        // An edit that changes nothing still writes both when they differ.
        if desired_kinds == before_kinds && !databases_differ {
            // Nothing to write (and no backup to make).
            return Ok(UpdateUsbPlayerMenuConfigData {
                updated: false,
                status_text: "Player menu unchanged".to_string(),
                current_items: before_current,
                available_items: before_available,
                warnings,
            });
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

        if has_edb {
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

            if let Err(err) =
                mirror_edb_category_from_pdb_kinds(&usb_root, &final_kinds, &mut warnings)
            {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-player-menu",
                    "usb.player-menu.edb-write-failed",
                    format!("eDB write failed ({err}); restoring eDB from snapshot"),
                ));
                if let Some(snapshot) = edb_snapshot.as_ref()
                    && let Err(restore_err) = std::fs::write(&edb_path, snapshot)
                {
                    warnings.push(logging::log(
                        Level::Error,
                        "usb-player-menu",
                        "usb.player-menu.edb-restore-failed",
                        format!(
                            "eDB snapshot could not be put back ({restore_err}): {}",
                            edb_path.display()
                        ),
                    ));
                }
                // Deliberately no write_back_if_changed here: the local copy was
                // just restored to its pre-attempt state, so the real USB drive
                // must never see this failed attempt.
                return Err(err);
            }
            if let Err(err) =
                usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Edb)
            {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-player-menu",
                    "usb.player-menu.edb-write-back-failed",
                    format!("eDB write-back to USB failed: {err}"),
                ));
            }
        } else {
            let rows = t17_rows_for_kinds(&usb_root, &final_kinds)?;
            crate::service::export_helpers::patch_pdb_t17_category_snapshot(&usb_root, &rows)?;
        }

        let (current_items, available_items) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;
        let after_kinds = current_items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        let before_all_kinds = before_current
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        let updated = before_all_kinds != after_kinds || databases_differ;

        if updated {
            if has_edb {
                let patched = t17_rows_from_edb(&usb_root, &mut warnings).and_then(|rows| {
                    crate::service::export_helpers::patch_pdb_t17_category_snapshot(
                        &usb_root, &rows,
                    )
                });
                match patched {
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
            status_text: if updated {
                "Player menu updated"
            } else {
                "Player menu unchanged"
            }
            .to_string(),
            current_items,
            available_items,
            warnings,
        })
    }
}

#[cfg(test)]
mod t17_tests {
    use super::*;

    // Rows as rekordbox wrote them on a real stick: item, category, its own byte,
    // flag, order.
    const BPM_HIDDEN: [u8; 8] = [5, 0, 6, 0, 0x05, 1, 0, 0];
    const ARTIST_SHOWN: [u8; 8] = [2, 0, 2, 0, 0x02, 0, 1, 0];
    const DATE_ADDED_SHOWN: [u8; 8] = [22, 0, 27, 0, 0x63, 0, 10, 0];

    #[test]
    fn updating_t17_keeps_each_rows_place_and_byte_and_leaves_date_added() {
        let live = [BPM_HIDDEN, ARTIST_SHOWN, DATE_ADDED_SHOWN];
        let encode = |item: u16, category, visible, kind, seq| {
            (
                u32::from(item),
                crate::service::export_helpers::encode_pdb_t17_cat_row(
                    item, category, visible, kind, seq,
                ),
            )
        };
        let wanted = [
            encode(2, 2, false, 129, 0),
            encode(5, 6, true, 133, 11),
            encode(22, 27, false, 140, 0),
        ];
        assert_eq!(
            updated_t17_rows(&live, &wanted, Some(22)),
            [
                [5, 0, 6, 0, 0x05, 0, 11, 0],
                [2, 0, 2, 0, 0x02, 1, 0, 0],
                DATE_ADDED_SHOWN,
            ]
        );
    }

    #[test]
    fn a_row_the_table_lacks_goes_after_the_others() {
        let new = crate::service::export_helpers::encode_pdb_t17_cat_row(5, 6, true, 133, 2);
        assert_eq!(
            updated_t17_rows(&[ARTIST_SHOWN], &[(5, new)], None),
            [ARTIST_SHOWN, new]
        );
    }

    #[test]
    fn sequence_values_skip_the_reserved_one() {
        assert_eq!(
            menu_sequence_values(&[129, 130, 133], Some(2)),
            [(129, 1), (130, 3), (133, 4)]
        );
        assert_eq!(menu_sequence_values(&[129], None), [(129, 1)]);
    }

    #[test]
    fn the_comparison_ignores_date_added_and_gaps() {
        let row = |menu_item_id, kind, visible, sequence_no| MenuRow {
            menu_item_id,
            kind,
            name: String::new(),
            visible,
            sequence_no,
        };
        // As rekordbox leaves them: the order values differ and DATE ADDED is out of
        // step, but the same menu shows.
        let edb = [
            row(5, 133, true, 9),
            row(22, DATE_ADDED_KIND, true, 10),
            row(18, 152, true, 11),
        ];
        let pdb = [
            row(5, 133, true, 9),
            row(22, DATE_ADDED_KIND, false, 0),
            row(18, 152, true, 10),
        ];
        assert_eq!(compared_menu_order(&edb), compared_menu_order(&pdb));
        let moved = [row(18, 152, true, 1), row(5, 133, true, 2)];
        assert_ne!(compared_menu_order(&edb), compared_menu_order(&moved));
    }
}
