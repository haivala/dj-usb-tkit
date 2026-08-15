use std::path::{Path, PathBuf};

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

pub(crate) const MASTER_DB_ENV_KEY: &str = "DJUSBTKIT_MASTER_DB_PATH";
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
}

pub(crate) fn backup_reason_path(backup_dir: &Path, timestamp: &str) -> PathBuf {
    backup_dir.join(format!("{timestamp}.reason.json"))
}

pub(crate) fn backup_usb_databases(usb_root: &Path, reason: &str) -> Vec<String> {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let backup_dir = vendor_db_dir(usb_root).join("backups");
    let mut notes = Vec::new();

    if let Err(e) = std::fs::create_dir_all(&backup_dir) {
        notes.push(format!("Backup skipped: could not create backups dir: {e}"));
        return notes;
    }

    let mut any_copied = false;
    for (src, stem, ext) in [
        (vendor_pdb_path(usb_root), "export", "pdb"),
        (vendor_edb_path(usb_root), "exportLibrary", "db"),
    ] {
        if !src.is_file() {
            continue;
        }
        let dest = backup_dir.join(format!("{stem}_{timestamp}.{ext}"));
        match std::fs::copy(&src, &dest) {
            Ok(_) => {
                any_copied = true;
                notes.push(format!(
                    "Backup: {}",
                    dest.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(e) => notes.push(format!("Backup failed for {stem}.{ext}: {e}")),
        }
    }

    if any_copied {
        let marker = BackupReason { reason: reason.to_string() };
        if let Ok(encoded) = serde_json::to_string(&marker)
            && let Err(e) = std::fs::write(backup_reason_path(&backup_dir, &timestamp), encoded)
        {
            notes.push(format!("Backup reason not recorded: {e}"));
        }
    }
    notes
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

        backup_usb_databases(usb_root, "Before export");

        let backup_dir = vendor_db_dir(usb_root).join("backups");
        let sidecars: Vec<_> = std::fs::read_dir(&backup_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".reason.json"))
            .collect();
        assert_eq!(sidecars.len(), 1, "exactly one reason sidecar per backup event");

        let bytes = std::fs::read(sidecars[0].path()).unwrap();
        let marker: BackupReason = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(marker.reason, "Before export");
    }

    #[test]
    fn backup_usb_databases_skips_sidecar_when_nothing_copied() {
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        // No live PDB/eDB present -- nothing to back up, so no sidecar either.
        backup_usb_databases(usb_root, "Before export");

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
