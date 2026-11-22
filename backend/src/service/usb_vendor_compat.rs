use crate::models::WarningEntry;
use std::path::{Path, PathBuf};

use super::usb_staging::DbKind;
use serde::{Deserialize, Serialize};

pub const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
pub const USB_VENDOR_DB_DIR: &str = "rekordbox";
pub const USB_VENDOR_ROOT_DIR_LOWER: &str = "pioneer";
pub const USB_VENDOR_DB_DIR_LOWER: &str = "rekordbox";
pub(crate) const DESKTOP_VENDOR_DIR: &str = "Pioneer";
pub const USB_CONTENTS_DIR: &str = "Contents";
pub const USB_ANALYSIS_DIR: &str = "USBANLZ";
pub const USB_ARTWORK_DIR: &str = "Artwork";

pub(crate) const USB_VENDOR_ROOT_PREFIX: &str = "/PIONEER/";
pub(crate) const USB_CONTENTS_PREFIX: &str = "/Contents/";
pub(crate) const USB_ARTWORK_PREFIX: &str = "/PIONEER/Artwork/";
pub(crate) const USB_ANALYSIS_PREFIX: &str = "/PIONEER/USBANLZ/";

pub(crate) const REKORDBOX_DB_ENV_KEY: &str = "DJUSBTKIT_REKORDBOX_DB_PATH";
/// Pre-rename name of [`REKORDBOX_DB_ENV_KEY`], still honoured.
pub(crate) const LEGACY_REKORDBOX_DB_ENV_KEY: &str = "DJUSBTKIT_MASTER_DB_PATH";
pub(crate) const USB_ROOT_ENV_KEY: &str = "DJUSBTKIT_USB_ROOT";

pub const DEFAULT_MASTER_DB_KEY: &str =
    "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";
pub const DEFAULT_USB_EDB_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";

pub(crate) fn vendor_db_dir(usb_root: &Path) -> PathBuf {
    usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

pub(crate) fn vendor_pdb_path(usb_root: &Path) -> PathBuf {
    vendor_db_dir(usb_root).join("export.pdb")
}

pub(crate) fn desktop_master_db_rel_path() -> PathBuf {
    PathBuf::from(DESKTOP_VENDOR_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("master.db")
}

pub(crate) fn vendor_edb_path(usb_root: &Path) -> PathBuf {
    vendor_db_dir(usb_root).join("exportLibrary.db")
}

/// Sidecar recording why a backup was taken (e.g. "Before export"), written
/// alongside the PDB/eDB snapshots under the same timestamp. Same
/// one-field-struct shape as `usb_identity::DriveMarker`, for consistency
/// with the app's other on-disk JSON metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BackupReason {
    pub reason: String,
    /// Playlist count on the drive at backup time, or `None` for backups
    /// predating this field. See `usb_backups::backup_usb_databases_with_retention`
    /// for how this is resolved (fresh parse vs. carried forward).
    #[serde(default)]
    pub playlist_count: Option<usize>,
}

pub(crate) fn backup_reason_path(backup_dir: &Path, timestamp: &str) -> PathBuf {
    backup_dir.join(format!("{timestamp}.reason.json"))
}

/// Copies the USB's databases into its backups folder. Each copy and failure
/// is logged (Event Log and file) and returned for the command's warnings.
/// A database file being copied into a backup, as `backup_usb_databases`
/// reports it: a slow USB stick can take minutes on one.
pub(crate) struct BackupCopy<'a> {
    /// The database's file name.
    pub file: &'a str,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub mb_per_s: Option<f64>,
}

pub(crate) fn backup_usb_databases(
    usb_root: &Path,
    reason: &str,
    playlist_count: Option<usize>,
    on_copy: &mut dyn FnMut(BackupCopy),
) -> Vec<WarningEntry> {
    use crate::i18n::tr;
    use crate::logging::{self, Level};
    let warn = |code: &str, text: crate::i18n::Text| {
        logging::log_text(Level::Warn, "usb-backups", code, &text)
    };
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let backup_dir = vendor_db_dir(usb_root).join("backups");
    let mut notes = Vec::new();

    if let Err(e) = std::fs::create_dir_all(&backup_dir) {
        notes.push(warn(
            "usb.backup.folder-failed",
            tr::backups_warning_folder_failed(e),
        ));
        return notes;
    }

    // Each database the stick has, copied from the USB itself.
    let databases = super::usb_staging::UsbDatabases::present(usb_root)
        .ok()
        .flatten();
    let mut any_copied = false;
    for (kind, src, stem, ext) in [
        (DbKind::Pdb, vendor_pdb_path(usb_root), "export", "pdb"),
        (
            DbKind::Edb,
            vendor_edb_path(usb_root),
            "exportLibrary",
            "db",
        ),
    ] {
        if !databases.as_ref().is_some_and(|dbs| dbs.has(kind)) {
            continue;
        }
        let dest = backup_dir.join(format!("{stem}_{timestamp}.{ext}"));
        let file = format!("{stem}.{ext}");
        match copy_reporting(&src, &dest, &mut |done_bytes, total_bytes, mb_per_s| {
            on_copy(BackupCopy {
                file: &file,
                done_bytes,
                total_bytes,
                mb_per_s,
            })
        }) {
            Ok(_) => {
                any_copied = true;
                notes.push(logging::log_text(
                    Level::Info,
                    "usb-backups",
                    "usb.backup.written",
                    &tr::backups_note_written(
                        dest.file_name().unwrap_or_default().to_string_lossy(),
                    ),
                ));
            }
            Err(e) => notes.push(warn(
                "usb.backup.copy-failed",
                tr::backups_warning_copy_failed(format!("{stem}.{ext}"), e),
            )),
        }
    }

    if any_copied {
        let marker = BackupReason {
            reason: reason.to_string(),
            playlist_count,
        };
        if let Ok(encoded) = serde_json::to_string(&marker)
            && let Err(e) = std::fs::write(backup_reason_path(&backup_dir, &timestamp), encoded)
        {
            logging::debug("usb-backups", format!("Backup reason not recorded: {e}"));
        }
    }
    notes
}

/// `std::fs::copy` in 1 MiB chunks, reporting (bytes copied, file size, MB/s
/// once measured) at the start, about twice a second and at the end.
fn copy_reporting(
    src: &Path,
    dest: &Path,
    on_progress: &mut dyn FnMut(u64, u64, Option<f64>),
) -> std::io::Result<u64> {
    let mut input = std::fs::File::open(src)?;
    let total = input.metadata().map(|m| m.len()).unwrap_or(0);
    let mut output = std::fs::File::create(dest)?;
    let mut meter = crate::progress::SpeedMeter::new();
    let mut done = 0u64;
    on_progress(0, total, None);
    crate::progress::copy_counting(&mut input, &mut output, &mut |bytes| {
        done += bytes;
        if meter.bytes(bytes) {
            on_progress(done, total.max(done), meter.mb_per_s());
        }
    })?;
    on_progress(done, total.max(done), meter.mb_per_s());
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn backup_usb_databases_writes_reason_sidecar_alongside_copied_files() {
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        std::fs::create_dir_all(vendor_db_dir(usb_root)).unwrap();
        std::fs::write(vendor_pdb_path(usb_root), b"pdb").unwrap();
        std::fs::write(vendor_edb_path(usb_root), b"edb").unwrap();

        backup_usb_databases(usb_root, "Before export", Some(7), &mut |_| {});

        let backup_dir = vendor_db_dir(usb_root).join("backups");
        let sidecars: Vec<_> = std::fs::read_dir(&backup_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".reason.json"))
            .collect();
        assert_eq!(
            sidecars.len(),
            1,
            "exactly one reason sidecar per backup event"
        );

        let bytes = std::fs::read(sidecars[0].path()).unwrap();
        let marker: BackupReason = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(marker.reason, "Before export");
        assert_eq!(marker.playlist_count, Some(7));
    }

    #[test]
    fn backup_usb_databases_reads_back_a_missing_playlist_count_as_none() {
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        std::fs::create_dir_all(vendor_db_dir(usb_root)).unwrap();
        std::fs::write(vendor_pdb_path(usb_root), b"pdb").unwrap();

        backup_usb_databases(usb_root, "Before playlist reorder", None, &mut |_| {});

        let backup_dir = vendor_db_dir(usb_root).join("backups");
        let sidecar = std::fs::read_dir(&backup_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().ends_with(".reason.json"))
            .expect("sidecar written");
        let bytes = std::fs::read(sidecar.path()).unwrap();
        let marker: BackupReason = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(marker.playlist_count, None);
    }

    #[test]
    fn backup_reason_deserializes_a_legacy_sidecar_missing_the_playlist_count_field() {
        let marker: BackupReason = serde_json::from_str(r#"{"reason":"Before export"}"#).unwrap();
        assert_eq!(marker.reason, "Before export");
        assert_eq!(marker.playlist_count, None);
    }

    #[test]
    fn backup_usb_databases_skips_sidecar_when_nothing_copied() {
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        // No live PDB/eDB present -- nothing to back up, so no sidecar either.
        backup_usb_databases(usb_root, "Before export", Some(3), &mut |_| {});

        let backup_dir = vendor_db_dir(usb_root).join("backups");
        let sidecar_count = std::fs::read_dir(&backup_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".reason.json"))
            .count();
        assert_eq!(sidecar_count, 0);
    }
}
