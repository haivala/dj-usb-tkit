use std::path::{Path, PathBuf};

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

pub(crate) fn backup_usb_databases(usb_root: &Path) -> Vec<String> {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let backup_dir = vendor_db_dir(usb_root).join("backups");
    let mut notes = Vec::new();

    if let Err(e) = std::fs::create_dir_all(&backup_dir) {
        notes.push(format!("Backup skipped: could not create backups dir: {e}"));
        return notes;
    }

    for (src, stem, ext) in [
        (vendor_pdb_path(usb_root), "export", "pdb"),
        (vendor_edb_path(usb_root), "exportLibrary", "db"),
    ] {
        if !src.is_file() {
            continue;
        }
        let dest = backup_dir.join(format!("{stem}_{timestamp}.{ext}"));
        match std::fs::copy(&src, &dest) {
            Ok(_) => notes.push(format!(
                "Backup: {}",
                dest.file_name().unwrap_or_default().to_string_lossy()
            )),
            Err(e) => notes.push(format!("Backup failed for {stem}.{ext}: {e}")),
        }
    }
    notes
}
