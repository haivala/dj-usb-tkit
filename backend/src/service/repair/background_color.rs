//! The player background color: one setting the app writes to both
//! databases, byte 9 of the PDB's tt=19 rows (what PDB-reading players such
//! as the CDJ-2000NXS2 show) and the eDB's `property.backGroundColorType`
//! (rekordbox, eDB-reading players). rekordbox lets the two differ; the app
//! only ever sets them together, and export and repair leave both alone
//! (see `crate::pdb_writer::t19_sync_target`).

use super::run::StagedPdb;
use super::*;
use crate::i18n::{Text, tr};
use crate::models::{
    GetUsbBackgroundColorRequest, SetUsbBackgroundColorRequest, UsbBackgroundColorChip,
    UsbBackgroundColorData,
};

/// rekordbox's list: 0 = default, then pink, red, orange, yellow, green,
/// aqua, blue and purple.
const MAX_BACKGROUND_COLOR: u8 = 8;

/// The colors in rekordbox's order: the index is the stored value. The dots'
/// colors are in `styles.css`.
const BACKGROUND_COLORS: [fn() -> Text; MAX_BACKGROUND_COLOR as usize + 1] = [
    tr::usb_background_color_default,
    tr::usb_background_color_pink,
    tr::usb_background_color_red,
    tr::usb_background_color_orange,
    tr::usb_background_color_yellow,
    tr::usb_background_color_green,
    tr::usb_background_color_aqua,
    tr::usb_background_color_blue,
    tr::usb_background_color_purple,
];

fn background_color_name(color: u8) -> String {
    BACKGROUND_COLORS
        .get(usize::from(color))
        .map_or_else(|| tr::usb_background_color_numbered(color), |name| name())
        .render()
}

/// A chip is selected only while both databases hold that color. With one
/// database, that one decides.
fn background_color_chips(pdb: Option<u8>, edb: Option<u8>) -> Vec<UsbBackgroundColorChip> {
    let selected = match (pdb, edb) {
        (Some(pdb), Some(edb)) => (pdb == edb).then_some(pdb),
        (one, other) => one.or(other),
    };
    (0u8..)
        .zip(BACKGROUND_COLORS)
        .map(|(color, name)| UsbBackgroundColorChip {
            color,
            label: name().render(),
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
        (Some(pdb), Some(edb)) if pdb != edb => tr::usb_background_color_two_colors(
            background_color_name(pdb),
            background_color_name(edb),
        )
        .render(),
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

/// Why a color can't be set, if it can't.
fn background_color_refusal(has_a_place: bool, writable: bool) -> Option<Text> {
    if !has_a_place {
        Some(tr::usb_background_color_no_place())
    } else if !writable {
        Some(tr::usb_background_color_read_only())
    } else {
        None
    }
}

/// Both databases' colors for the staged `pdb`, and whether a color can be
/// set: either database having a place for it is enough, and the app writes to
/// whichever have one.
fn read_background_colors(usb_root: &Path, pdb: &[u8]) -> UsbBackgroundColorData {
    let pdb_color = (!pdb.is_empty())
        .then(|| crate::pdb_writer::t19_background_color(pdb, page_size_of(pdb)))
        .flatten();
    let edb_color = usb_staging::UsbDatabases::present(usb_root)
        .ok()
        .flatten()
        .and_then(|databases| databases.open_edb_writable())
        .and_then(|conn| edb_background_color(&conn));
    let reason = background_color_refusal(
        pdb_color.is_some() || edb_color.is_some(),
        crate::service::usb_utils::has_write_access(usb_root),
    );
    UsbBackgroundColorData {
        chips: background_color_chips(pdb_color, edb_color),
        note_text: background_color_note(
            pdb_color,
            edb_color,
            reason.as_ref().map(Text::render).as_deref(),
        ),
        status_text: String::new(),
        pdb_color,
        edb_color,
        writable: reason.is_none(),
        reason: reason.as_ref().map(Text::render),
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
        Ok(read_background_colors(&usb_root, &pdb.bytes))
    }

    /// Sets `req.color` in both databases, in place, and saves both to the
    /// USB or neither. A no-op when both already hold it.
    pub fn set_usb_background_color(
        &self,
        req: SetUsbBackgroundColorRequest,
    ) -> BackendResult<UsbBackgroundColorData> {
        if req.color > MAX_BACKGROUND_COLOR {
            return Err(BackendError::Internal(format!(
                "background color {} is out of range 0..={MAX_BACKGROUND_COLOR}",
                req.color
            )));
        }
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        let mut pdb = StagedPdb::load(&usb_root)?;
        let before = read_background_colors(&usb_root, &pdb.bytes);
        if !before.writable {
            return Err(BackendError::Validation(
                background_color_refusal(
                    before.pdb_color.is_some() || before.edb_color.is_some(),
                    false,
                )
                .unwrap_or_else(tr::usb_background_color_read_only),
            ));
        }
        // The read above only looks; this one writes, so a read-only mount is
        // caught here instead of failing halfway through with its own error.
        if !crate::service::usb_utils::can_write_file_at(&usb_root) {
            return Err(BackendError::Validation(
                tr::usb_background_color_read_only(),
            ));
        }
        let holds = |color: Option<u8>| color.is_none_or(|color| color == req.color);
        if holds(before.pdb_color) && holds(before.edb_color) {
            return Ok(UsbBackgroundColorData {
                status_text: tr::usb_background_color_unchanged().render(),
                warnings,
                ..before
            });
        }

        let mut session = self.usb_write_session(
            &usb_root,
            usb_staging::SessionBackup::WithRetention {
                reason: "Before background color change",
                playlist_count: None,
            },
            &mut warnings,
        );

        // The PDB change stays in memory until the eDB one has succeeded.
        if before.pdb_color.is_some() {
            session.will_write(usb_staging::DbKind::Pdb)?;
            let page_size = page_size_of(&pdb.bytes);
            pdb.patch(|bytes| {
                crate::pdb_writer::set_t19_background_color(bytes, page_size, req.color)
            })?;
        }
        if before.edb_color.is_some() {
            let conn = session
                .edb()?
                .ok_or_else(|| BackendError::Validation(tr::usb_background_color_error_no_edb()))?;
            conn.execute(
                "UPDATE property SET backGroundColorType = ?1",
                params![req.color],
            )?;
        }
        pdb.save()?;
        session.commit()?;
        logging::debug(
            "usb-background-color",
            format!(
                "background color set to {} (PDB was {:?}, eDB was {:?})",
                req.color, before.pdb_color, before.edb_color
            ),
        );

        let mut data = read_background_colors(&usb_root, &pdb.bytes);
        data.updated = true;
        data.status_text = tr::usb_background_color_set(background_color_name(req.color)).render();
        data.warnings = warnings;
        Ok(data)
    }
}

#[cfg(test)]
mod text_tests {
    use super::{background_color_chips, background_color_note, background_color_refusal};

    #[test]
    fn a_color_can_be_set_with_a_place_for_it_on_a_writable_usb() {
        assert_eq!(background_color_refusal(true, true), None);
        assert!(
            background_color_refusal(false, true)
                .unwrap()
                .render_english()
                .contains("no place")
        );
        assert!(
            background_color_refusal(true, false)
                .unwrap()
                .render_english()
                .contains("read-only")
        );
    }

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
        // One database: that one decides.
        assert_eq!(selected(None, Some(0)), [0]);
        assert_eq!(selected(Some(4), None), [4]);
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
            "rekordbox set two colours: Aqua for older players (PDB), Default for newer players and rekordbox (eDB). Choosing a colour sets both."
        );
        assert!(background_color_note(Some(20), Some(0), None).contains("colour 20"));
    }
}
