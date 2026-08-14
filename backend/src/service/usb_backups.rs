//! Split storage + retention for USB DB backup snapshots created by
//! `usb_vendor_compat::backup_usb_databases`: the single newest snapshot per
//! file always stays on the USB (so at least one restore point travels with
//! the physical drive to any computer), while older snapshots are archived
//! off to the HDD-side staging cache (`usb_staging::backups_dir_for`) so
//! they stop consuming space on the USB stick. A user-configurable
//! retention count (`SETTING_UI_BACKUP_RETENTION_COUNT`) then caps the
//! combined USB+cache count, pruning the oldest cache-side snapshots first.
//!
//! Archiving to the cache requires a named drive (`usb_identity`) -- an
//! unnamed drive has no stable cache directory to archive into, so
//! reconciliation just leaves everything on the USB in that case.
//!
//! Also backs the "Backups" UI panel: list / restore / delete.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};

use crate::error::{BackendError, BackendResult};
use crate::models::{
    DeleteUsbBackupData, DeleteUsbBackupRequest, ListUsbBackupsData, ListUsbBackupsRequest,
    RestoreUsbBackupData, RestoreUsbBackupRequest, UsbBackupEntry,
};

use super::usb_utils::resolve_usb_root;
use super::usb_vendor_compat::{vendor_db_dir, vendor_edb_path, vendor_pdb_path};
use super::{BackendService, DEFAULT_BACKUP_RETENTION_COUNT, SETTING_UI_BACKUP_RETENTION_COUNT};

const STEMS: [(&str, &str); 2] = [("export", "pdb"), ("exportLibrary", "db")];

fn usb_backups_dir(usb_root: &Path) -> PathBuf {
    vendor_db_dir(usb_root).join("backups")
}

fn cache_backups_dir(usb_root: &Path) -> Option<PathBuf> {
    super::usb_staging::backups_dir_for(usb_root)
}

/// Snapshot filenames are `{stem}_{timestamp}.{ext}` with a fixed-width
/// `%Y-%m-%d_%H-%M-%S` timestamp, so plain filename sort order is also
/// chronological order -- no need to parse the timestamp back out.
fn list_stem_files(dir: &Path, stem: &str, ext: &str) -> Vec<PathBuf> {
    let prefix = format!("{stem}_");
    let suffix = format!(".{ext}");
    let mut files = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.starts_with(&prefix) && name.ends_with(&suffix)).then_some(e.path())
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn move_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    // rename fails across filesystems (USB -> internal HDD): fall back to copy + remove.
    std::fs::copy(src, dest)?;
    std::fs::remove_file(src)
}

/// Splits USB-side snapshots (keep newest, archive the rest to the HDD
/// cache when a cache dir is available) then prunes the combined USB+cache
/// count for each stem down to `retention_count`, oldest cache-side entries
/// first. Returns human-readable notes for the caller's warning log, same
/// shape as `backup_usb_databases`'s own return value. Best-effort
/// throughout: a stat/move/delete failure is noted, never propagated as a
/// hard error -- this runs after the backup copy itself already succeeded
/// or failed, and must never turn a successful backup into a failed one.
fn reconcile_backups(usb_root: &Path, retention_count: u32) -> Vec<String> {
    let retention_count = retention_count.max(1) as usize;
    let usb_dir = usb_backups_dir(usb_root);
    let cache_dir = cache_backups_dir(usb_root);
    let mut notes = Vec::new();

    for (stem, ext) in STEMS {
        let mut usb_files = list_stem_files(&usb_dir, stem, ext);
        let newest = usb_files.pop();

        if let Some(cache_dir) = &cache_dir {
            let mut archived = 0usize;
            for old in usb_files {
                let dest = cache_dir.join(old.file_name().unwrap_or_default());
                match move_file(&old, &dest) {
                    Ok(()) => archived += 1,
                    Err(e) => notes.push(format!(
                        "Backup archive failed for {}: {e}",
                        old.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
            if archived > 0 {
                notes.push(format!("Archived {archived} older {stem} backup(s) to local cache"));
            }
        }
        // No cache dir (drive not yet named): leave older USB snapshots in
        // place untouched -- nothing to prune against yet either.
        let Some(cache_dir) = &cache_dir else {
            continue;
        };

        let mut combined = list_stem_files(cache_dir, stem, ext);
        // The single USB-side survivor always counts as the newest and is
        // never pruned; only cache-side entries are candidates for deletion.
        let total = combined.len() + newest.is_some() as usize;
        if total > retention_count {
            let overflow = total - retention_count;
            combined.sort();
            let mut pruned = 0usize;
            for path in combined.into_iter().take(overflow) {
                match std::fs::remove_file(&path) {
                    Ok(()) => pruned += 1,
                    Err(e) => notes.push(format!(
                        "Backup prune failed for {}: {e}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
            if pruned > 0 {
                notes.push(format!("Pruned {pruned} old {stem} backup(s) beyond retention limit"));
            }
        }
    }

    notes
}

fn read_backup_retention_count(conn: &rusqlite::Connection) -> u32 {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![SETTING_UI_BACKUP_RETENTION_COUNT],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|v| v.trim().parse::<u32>().ok())
    .filter(|n| *n >= 1)
    .unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT)
}

fn entries_in(dir: &Path, location: &str) -> Vec<UsbBackupEntry> {
    let mut out = Vec::new();
    for (stem, ext) in STEMS {
        for path in list_stem_files(dir, stem, ext) {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let timestamp = filename
                .strip_prefix(&format!("{stem}_"))
                .and_then(|s| s.strip_suffix(&format!(".{ext}")))
                .unwrap_or(&filename)
                .to_string();
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            out.push(UsbBackupEntry {
                stem: stem.to_string(),
                filename,
                timestamp,
                size_bytes,
                location: location.to_string(),
            });
        }
    }
    out
}

/// Finds a backup snapshot by stem+filename, checking the USB backups dir
/// first, then the HDD cache backups dir.
fn find_backup(usb_root: &Path, stem: &str, filename: &str) -> BackendResult<PathBuf> {
    let candidate = usb_backups_dir(usb_root).join(filename);
    if candidate.is_file() {
        return Ok(candidate);
    }
    if let Some(cache_dir) = cache_backups_dir(usb_root) {
        let candidate = cache_dir.join(filename);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(BackendError::NotFound(format!(
        "backup not found: {stem}/{filename}"
    )))
}

fn live_path_for_stem(usb_root: &Path, stem: &str) -> Option<PathBuf> {
    match stem {
        "export" => Some(vendor_pdb_path(usb_root)),
        "exportLibrary" => Some(vendor_edb_path(usb_root)),
        _ => None,
    }
}

impl BackendService {
    /// Takes a fresh timestamped backup of `usb_root`'s live DB files (same
    /// raw copy `usb_vendor_compat::backup_usb_databases` always did), then
    /// reconciles storage per this module's split/retention policy. This is
    /// the function every mutating USB command should call instead of
    /// `usb_vendor_compat::backup_usb_databases` directly.
    pub(crate) fn backup_usb_databases_with_retention(&self, usb_root: &Path) -> Vec<String> {
        let mut notes = super::usb_vendor_compat::backup_usb_databases(usb_root);
        let retention_count = self
            .db
            .connect()
            .ok()
            .map(|conn| read_backup_retention_count(&conn))
            .unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT);
        notes.extend(reconcile_backups(usb_root, retention_count));
        notes
    }

    pub fn list_usb_backups(&self, req: ListUsbBackupsRequest) -> BackendResult<ListUsbBackupsData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let mut items = entries_in(&usb_backups_dir(&usb_root), "usb");
        if let Some(cache_dir) = cache_backups_dir(&usb_root) {
            items.extend(entries_in(&cache_dir, "cache"));
        }
        items.sort_by(|a, b| a.stem.cmp(&b.stem).then(b.timestamp.cmp(&a.timestamp)));
        Ok(ListUsbBackupsData { items })
    }

    pub fn restore_usb_backup(
        &self,
        req: RestoreUsbBackupRequest,
    ) -> BackendResult<RestoreUsbBackupData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let snapshot = find_backup(&usb_root, &req.stem, &req.filename)?;
        let live_path = live_path_for_stem(&usb_root, &req.stem).ok_or_else(|| {
            BackendError::Validation(format!("unknown backup stem: {}", req.stem))
        })?;

        // Preserve the current live state as its own backup before
        // overwriting it, same as every other mutating USB command.
        self.backup_usb_databases_with_retention(&usb_root);

        std::fs::copy(&snapshot, &live_path)?;
        Ok(RestoreUsbBackupData { restored: true })
    }

    pub fn delete_usb_backup(&self, req: DeleteUsbBackupRequest) -> BackendResult<DeleteUsbBackupData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let snapshot = find_backup(&usb_root, &req.stem, &req.filename)?;
        std::fs::remove_file(&snapshot)?;
        Ok(DeleteUsbBackupData { deleted: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_service() -> (tempfile::TempDir, BackendService) {
        let dir = tempdir().expect("service data dir");
        let service = BackendService::new(dir.path()).expect("backend service");
        (dir, service)
    }

    fn touch_backup(dir: &Path, stem: &str, ext: &str, ts: &str, content: &[u8]) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{stem}_{ts}.{ext}"));
        std::fs::write(&path, content).unwrap();
        path
    }

    fn name_drive(usb_root: &Path, name: &str) {
        crate::service::usb_identity::write_drive_name(usb_root, name).unwrap();
    }

    #[test]
    fn reconcile_keeps_newest_on_usb_and_archives_older_when_named() {
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        name_drive(usb_root, "Club Stick");
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"old1");
        touch_backup(&backups_dir, "export", "pdb", "2020-01-02_00-00-00", b"old2");
        touch_backup(&backups_dir, "export", "pdb", "2020-01-03_00-00-00", b"newest");

        reconcile_backups(usb_root, 10);

        let remaining_usb = list_stem_files(&backups_dir, "export", "pdb");
        assert_eq!(remaining_usb.len(), 1);
        assert_eq!(std::fs::read(&remaining_usb[0]).unwrap(), b"newest");

        let cache_dir = cache_backups_dir(usb_root).expect("cache dir for named drive");
        let archived = list_stem_files(&cache_dir, "export", "pdb");
        assert_eq!(archived.len(), 2);
    }

    #[test]
    fn reconcile_leaves_everything_on_usb_when_unnamed() {
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"old1");
        touch_backup(&backups_dir, "export", "pdb", "2020-01-02_00-00-00", b"newest");

        reconcile_backups(usb_root, 10);

        let remaining = list_stem_files(&backups_dir, "export", "pdb");
        assert_eq!(remaining.len(), 2, "unnamed drive must not archive anything");
    }

    #[test]
    fn reconcile_prunes_oldest_beyond_retention() {
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        name_drive(usb_root, "Club Stick");
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        let backups_dir = usb_backups_dir(usb_root);
        for i in 1..=5 {
            touch_backup(
                &backups_dir,
                "export",
                "pdb",
                &format!("2020-01-0{i}_00-00-00"),
                format!("v{i}").as_bytes(),
            );
        }

        reconcile_backups(usb_root, 2);

        let remaining_usb = list_stem_files(&backups_dir, "export", "pdb");
        assert_eq!(remaining_usb.len(), 1);
        assert_eq!(std::fs::read(&remaining_usb[0]).unwrap(), b"v5");

        let cache_dir = cache_backups_dir(usb_root).unwrap();
        let archived = list_stem_files(&cache_dir, "export", "pdb");
        assert_eq!(
            archived.len(),
            1,
            "combined USB(1)+cache should cap at retention_count=2"
        );
        assert_eq!(
            std::fs::read(&archived[0]).unwrap(),
            b"v4",
            "must keep the second-newest, prune the rest"
        );
    }

    #[test]
    fn list_usb_backups_merges_usb_and_cache_locations() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        name_drive(usb_root, "Club Stick");
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        touch_backup(
            &usb_backups_dir(usb_root),
            "export",
            "pdb",
            "2020-01-02_00-00-00",
            b"newest",
        );
        let cache_dir = cache_backups_dir(usb_root).expect("cache dir");
        touch_backup(&cache_dir, "export", "pdb", "2020-01-01_00-00-00", b"older");

        let result = service
            .list_usb_backups(ListUsbBackupsRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
            })
            .expect("list backups");
        assert_eq!(result.items.len(), 2);
        assert!(
            result
                .items
                .iter()
                .any(|i| i.location == "usb" && i.filename.contains("2020-01-02"))
        );
        assert!(
            result
                .items
                .iter()
                .any(|i| i.location == "cache" && i.filename.contains("2020-01-01"))
        );
    }

    #[test]
    fn restore_usb_backup_overwrites_live_file_and_backs_up_current_state_first() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let live_pdb = vendor_pdb_path(usb_root);
        std::fs::create_dir_all(live_pdb.parent().unwrap()).unwrap();
        std::fs::write(&live_pdb, b"live-v2").unwrap();

        let backups_dir = usb_backups_dir(usb_root);
        let snapshot = touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"live-v1");
        let filename = snapshot.file_name().unwrap().to_string_lossy().to_string();

        let result = service
            .restore_usb_backup(RestoreUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                stem: "export".to_string(),
                filename: filename.clone(),
            })
            .expect("restore");
        assert!(result.restored);
        assert_eq!(std::fs::read(&live_pdb).unwrap(), b"live-v1");

        // The pre-restore live state ("live-v2") must itself have been backed up.
        let remaining = list_stem_files(&backups_dir, "export", "pdb");
        assert!(
            remaining
                .iter()
                .any(|p| std::fs::read(p).map(|b| b == b"live-v2").unwrap_or(false)),
            "restore must back up the current live state before overwriting it"
        );
    }

    #[test]
    fn restore_usb_backup_errors_when_snapshot_missing() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let err = service
            .restore_usb_backup(RestoreUsbBackupRequest {
                usb_root: usb.path().to_string_lossy().to_string(),
                stem: "export".to_string(),
                filename: "export_1999-01-01_00-00-00.pdb".to_string(),
            })
            .expect_err("missing snapshot must error");
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn delete_usb_backup_removes_the_file() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let backups_dir = usb_backups_dir(usb_root);
        let snapshot = touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"snap");
        let filename = snapshot.file_name().unwrap().to_string_lossy().to_string();

        let result = service
            .delete_usb_backup(DeleteUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                stem: "export".to_string(),
                filename: filename.clone(),
            })
            .expect("delete");
        assert!(result.deleted);
        assert!(!snapshot.is_file());

        let err = service
            .delete_usb_backup(DeleteUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                stem: "export".to_string(),
                filename,
            })
            .expect_err("deleting again must error");
        assert!(matches!(err, BackendError::NotFound(_)));
    }
}
