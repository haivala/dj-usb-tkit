//! The player background color: one setting the app writes to both
//! databases, byte 9 of the PDB's tt=19 rows (what PDB-reading players such
//! as the CDJ-2000NXS2 show) and the eDB's `property.backGroundColorType`
//! (rekordbox, eDB-reading players). rekordbox lets the two differ; the app
//! only ever sets them together, and export and repair leave both alone
//! (see `crate::pdb_writer::t19_sync_target`).

use super::run::{StagedPdb, write_back_both_or_neither};
use super::*;
use crate::models::{
    GetUsbBackgroundColorRequest, SetUsbBackgroundColorRequest, UsbBackgroundColorChip,
    UsbBackgroundColorData,
};

/// rekordbox's list: 0 = default, then pink, red, orange, yellow, green,
/// aqua, blue and purple.
const MAX_BACKGROUND_COLOR: u8 = 8;

/// The colors in rekordbox's order: the index is the stored value. The dots'
/// colors are in `styles.css`.
const BACKGROUND_COLORS: [&str; MAX_BACKGROUND_COLOR as usize + 1] = [
    "Default", "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple",
];

fn background_color_name(color: u8) -> String {
    BACKGROUND_COLORS
        .get(usize::from(color))
        .map_or_else(|| format!("colour {color}"), |name| (*name).to_string())
}

/// A chip is selected only while both databases hold that color.
fn background_color_chips(pdb: Option<u8>, edb: Option<u8>) -> Vec<UsbBackgroundColorChip> {
    let selected = pdb.filter(|_| pdb == edb);
    (0u8..)
        .zip(BACKGROUND_COLORS)
        .map(|(color, name)| UsbBackgroundColorChip {
            color,
            label: name.to_string(),
            selected: selected == Some(color),
        })
        .collect()
}

/// Why no color can be set, or which two rekordbox left. Empty otherwise.
fn background_color_note(pdb: Option<u8>, edb: Option<u8>, reason: Option<&str>) -> String {
    if let Some(reason) = reason {
        return reason.to_string();
    }
    match (pdb, edb) {
        (Some(pdb), Some(edb)) if pdb != edb => format!(
            "rekordbox set two colours: {} for older players (PDB), {} for rekordbox and the CDJ-3000 (eDB). Choosing a colour sets both.",
            background_color_name(pdb),
            background_color_name(edb),
        ),
        _ => String::new(),
    }
}

fn page_size_of(pdb: &[u8]) -> usize {
    crate::utils::read_u32_le_at(pdb, 4).unwrap_or(0) as usize
}

/// The eDB's color, `None` without a `property` row.
fn edb_background_color(conn: &rusqlite::Connection) -> Option<u8> {
    conn.query_row(
        "SELECT COALESCE(backGroundColorType, 0) FROM property LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .map(|color| u8::try_from(color).unwrap_or(0))
}

/// Both databases' colors for the staged `pdb`, and whether a color can be
/// set: only when both have a place for it, so the app never sets one alone.
fn read_background_colors(
    usb_root: &Path,
    pdb: &[u8],
    warnings: &mut Vec<WarningEntry>,
) -> UsbBackgroundColorData {
    let pdb_color = crate::pdb_writer::t19_background_color(pdb, page_size_of(pdb));
    let edb_color = open_edb_rw(usb_root, warnings).and_then(|conn| edb_background_color(&conn));
    let reason = match (pdb_color, edb_color) {
        (None, _) => Some("The USB's PDB has no history (t19) page to hold the color.".to_string()),
        (_, None) => Some("The USB's eDB has no readable property row.".to_string()),
        _ => None,
    };
    UsbBackgroundColorData {
        chips: background_color_chips(pdb_color, edb_color),
        note_text: background_color_note(pdb_color, edb_color, reason.as_deref()),
        status_text: String::new(),
        pdb_color,
        edb_color,
        writable: reason.is_none(),
        reason,
        updated: false,
        warnings: Vec::new(),
    }
}

impl BackendService {
    pub fn get_usb_background_color(
        &self,
        req: GetUsbBackgroundColorRequest,
    ) -> BackendResult<UsbBackgroundColorData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let pdb = StagedPdb::load(&usb_root)?;
        let mut warnings = Vec::<WarningEntry>::new();
        let mut data = read_background_colors(&usb_root, &pdb.bytes, &mut warnings);
        data.warnings = warnings;
        Ok(data)
    }

    /// Sets `req.color` in both databases, in place, and saves both to the
    /// USB or neither. A no-op when both already hold it.
    pub fn set_usb_background_color(
        &self,
        req: SetUsbBackgroundColorRequest,
    ) -> BackendResult<UsbBackgroundColorData> {
        if req.color > MAX_BACKGROUND_COLOR {
            return Err(BackendError::Validation(format!(
                "background color {} is out of range 0..={MAX_BACKGROUND_COLOR}",
                req.color
            )));
        }
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        let mut pdb = StagedPdb::load(&usb_root)?;
        let before = read_background_colors(&usb_root, &pdb.bytes, &mut warnings);
        if let Some(reason) = before.reason {
            return Err(BackendError::Validation(reason));
        }
        if before.pdb_color == Some(req.color) && before.edb_color == Some(req.color) {
            return Ok(UsbBackgroundColorData {
                status_text: "Background colour unchanged".to_string(),
                warnings,
                ..before
            });
        }

        warnings.extend(
            self.backup_usb_databases_with_retention(
                &usb_root,
                "Before background color change",
                None,
            )
            .into_iter()
            .map(|message| {
                logging::log(
                    Level::Info,
                    "usb-background-color",
                    "usb.background-color.backup",
                    message,
                )
            }),
        );

        // The PDB change stays in memory until the eDB one has succeeded.
        let page_size = page_size_of(&pdb.bytes);
        pdb.patch(|bytes| {
            crate::pdb_writer::set_t19_background_color(bytes, page_size, req.color)
        })?;
        let conn = open_edb_rw(&usb_root, &mut warnings)
            .ok_or_else(|| BackendError::Validation("unable to open the USB's eDB".to_string()))?;
        conn.execute(
            "UPDATE property SET backGroundColorType = ?1",
            params![req.color],
        )?;
        drop(conn);
        write_back_both_or_neither(&usb_root, &mut pdb)?;
        self.invalidate_usb_parse_cache();
        warnings.push(logging::log(
            Level::Info,
            "usb-background-color",
            "usb.background-color.set",
            format!(
                "background color set to {} (PDB was {:?}, eDB was {:?})",
                req.color, before.pdb_color, before.edb_color
            ),
        ));

        let mut data = read_background_colors(&usb_root, &pdb.bytes, &mut warnings);
        data.updated = true;
        data.status_text = format!(
            "Background colour set to {}",
            background_color_name(req.color)
        );
        data.warnings = warnings;
        Ok(data)
    }
}

#[cfg(test)]
mod text_tests {
    use super::{background_color_chips, background_color_note};

    #[test]
    fn a_chip_is_selected_only_while_both_databases_hold_it() {
        let selected = |pdb, edb| -> Vec<u8> {
            background_color_chips(pdb, edb)
                .into_iter()
                .filter(|chip| chip.selected)
                .map(|chip| chip.color)
                .collect()
        };
        assert_eq!(background_color_chips(None, None).len(), 9);
        assert_eq!(selected(Some(6), Some(6)), [6]);
        assert!(selected(Some(6), Some(0)).is_empty());
        assert!(selected(None, Some(0)).is_empty());
    }

    #[test]
    fn the_note_says_why_or_names_both_colors() {
        assert_eq!(background_color_note(Some(6), Some(6), None), "");
        assert_eq!(
            background_color_note(None, Some(0), Some("no page")),
            "no page"
        );
        assert_eq!(
            background_color_note(Some(6), Some(0), None),
            "rekordbox set two colours: Aqua for older players (PDB), Default for rekordbox and the CDJ-3000 (eDB). Choosing a colour sets both."
        );
        assert!(background_color_note(Some(20), Some(0), None).contains("colour 20"));
    }
}
