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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};

use crate::error::{BackendError, BackendResult};
use crate::models::{
    DeleteUsbBackupData, DeleteUsbBackupRequest, ListUsbBackupsData, ListUsbBackupsRequest,
    RestoreUsbBackupData, RestoreUsbBackupRequest, UsbBackupEntry, UsbBackupFile,
};

use super::usb_utils::resolve_usb_root;
use super::usb_vendor_compat::{
    BackupReason, backup_reason_path, vendor_db_dir, vendor_edb_path, vendor_pdb_path,
};
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

/// Timestamps with a `{timestamp}.reason.json` sidecar present in `dir`.
fn list_reason_timestamps(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".reason.json").map(|s| s.to_string())
        })
        .collect()
}

/// Whether any PDB/eDB snapshot for `timestamp` still exists in `dir`.
fn any_stem_file_for_timestamp(dir: &Path, timestamp: &str) -> bool {
    STEMS
        .iter()
        .any(|(stem, ext)| dir.join(format!("{stem}_{timestamp}.{ext}")).is_file())
}

/// Reads the reason recorded for `timestamp`, checking the USB dir first,
/// then the cache dir. Missing or unparsable sidecars (backups made before
/// this feature existed, or on a best-effort write failure) yield `None`
/// rather than an error.
fn read_backup_reason(dirs: &[(PathBuf, &str)], timestamp: &str) -> Option<String> {
    dirs.iter().find_map(|(dir, _)| {
        let bytes = std::fs::read(backup_reason_path(dir, timestamp)).ok()?;
        serde_json::from_slice::<BackupReason>(&bytes)
            .ok()
            .map(|marker| marker.reason)
    })
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
fn reconcile_backups(usb_root: &Path, retention_count: u32, protect_timestamp: Option<&str>) -> Vec<String> {
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
            // Never prune the snapshot a restore is currently reading from --
            // if it's among the oldest, skip it and prune the next-oldest
            // instead, even if that leaves the drive one entry over
            // `retention_count` until the next reconcile.
            let prunable = combined.into_iter().filter(|path| {
                protect_timestamp
                    .map(|ts| !path.file_name().is_some_and(|n| n.to_string_lossy().contains(ts)))
                    .unwrap_or(true)
            });
            let mut pruned = 0usize;
            for path in prunable.take(overflow) {
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

    // Reason sidecars are timestamp-keyed (covering the whole PDB+eDB pair),
    // not stem-keyed, so they're reconciled in their own pass once both
    // stems above have settled into their final locations: a sidecar
    // follows its pair to the cache dir once neither stem file is left on
    // the USB, and is deleted once no stem file for its timestamp remains
    // anywhere at all.
    if let Some(cache_dir) = &cache_dir {
        for timestamp in list_reason_timestamps(&usb_dir) {
            if !any_stem_file_for_timestamp(&usb_dir, &timestamp) {
                let src = backup_reason_path(&usb_dir, &timestamp);
                let dest = backup_reason_path(cache_dir, &timestamp);
                let _ = move_file(&src, &dest);
            }
        }
        for timestamp in list_reason_timestamps(cache_dir) {
            if !any_stem_file_for_timestamp(&usb_dir, &timestamp)
                && !any_stem_file_for_timestamp(cache_dir, &timestamp)
            {
                let _ = std::fs::remove_file(backup_reason_path(cache_dir, &timestamp));
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

/// PDB and eDB are always backed up together under one shared timestamp (see
/// `usb_vendor_compat::backup_usb_databases`), so entries are grouped by
/// timestamp into a single bundle covering both files rather than one row
/// per stem. `reconcile_backups` prunes/archives each stem independently,
/// but since both stems are created (and thus retained/archived) on the same
/// timestamp schedule, a pair's files normally share a location; a bundle is
/// labeled "usb" if either half is still USB-side, "cache" otherwise.
fn entries_by_timestamp(dirs: &[(PathBuf, &str)]) -> Vec<UsbBackupEntry> {
    let mut by_timestamp: BTreeMap<String, Vec<(UsbBackupFile, &str)>> = BTreeMap::new();
    for (dir, location) in dirs {
        for (stem, ext) in STEMS {
            for path in list_stem_files(dir, stem, ext) {
                let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let timestamp = filename
                    .strip_prefix(&format!("{stem}_"))
                    .and_then(|s| s.strip_suffix(&format!(".{ext}")))
                    .unwrap_or(&filename)
                    .to_string();
                let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                by_timestamp.entry(timestamp).or_default().push((
                    UsbBackupFile { stem: stem.to_string(), filename, size_bytes },
                    location,
                ));
            }
        }
    }

    let mut items: Vec<UsbBackupEntry> = by_timestamp
        .into_iter()
        .map(|(timestamp, entries)| {
            let size_bytes = entries.iter().map(|(f, _)| f.size_bytes).sum();
            let location = if entries.iter().any(|(_, loc)| *loc == "usb") { "usb" } else { "cache" };
            let reason = read_backup_reason(dirs, &timestamp);
            let files = entries.into_iter().map(|(f, _)| f).collect();
            UsbBackupEntry { timestamp, location: location.to_string(), size_bytes, files, reason }
        })
        .collect();
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    items
}

/// Finds every backup file (PDB and/or eDB) sharing `timestamp`, checking
/// the USB backups dir first, then the HDD cache backups dir, since a pair's
/// two files aren't guaranteed to share a location after retention pruning.
fn files_for_timestamp(usb_root: &Path, timestamp: &str) -> Vec<(String, PathBuf)> {
    let mut dirs = vec![usb_backups_dir(usb_root)];
    dirs.extend(cache_backups_dir(usb_root));

    let mut out = Vec::new();
    for dir in dirs {
        for (stem, ext) in STEMS {
            let candidate = dir.join(format!("{stem}_{timestamp}.{ext}"));
            if candidate.is_file() {
                out.push((stem.to_string(), candidate));
            }
        }
    }
    out
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
    pub(crate) fn backup_usb_databases_with_retention(&self, usb_root: &Path, reason: &str) -> Vec<String> {
        self.backup_usb_databases_with_retention_protecting(usb_root, reason, None)
    }

    /// Same as `backup_usb_databases_with_retention`, but `protect_timestamp`
    /// (a backup's snapshot timestamp) is exempted from this reconcile's
    /// pruning pass. Used by `restore_usb_backup` so the very snapshot being
    /// restored from can't be deleted out from under it by the pre-restore
    /// backup of the current live state.
    fn backup_usb_databases_with_retention_protecting(
        &self,
        usb_root: &Path,
        reason: &str,
        protect_timestamp: Option<&str>,
    ) -> Vec<String> {
        let mut notes = super::usb_vendor_compat::backup_usb_databases(usb_root, reason);
        let retention_count = self
            .db
            .connect()
            .ok()
            .map(|conn| read_backup_retention_count(&conn))
            .unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT);
        notes.extend(reconcile_backups(usb_root, retention_count, protect_timestamp));
        notes
    }

    pub fn list_usb_backups(&self, req: ListUsbBackupsRequest) -> BackendResult<ListUsbBackupsData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let mut dirs = vec![(usb_backups_dir(&usb_root), "usb")];
        if let Some(cache_dir) = cache_backups_dir(&usb_root) {
            dirs.push((cache_dir, "cache"));
        }
        let items = entries_by_timestamp(&dirs);
        Ok(ListUsbBackupsData { items })
    }

    pub fn restore_usb_backup(
        &self,
        req: RestoreUsbBackupRequest,
    ) -> BackendResult<RestoreUsbBackupData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let files = files_for_timestamp(&usb_root, &req.timestamp);
        if files.is_empty() {
            return Err(BackendError::NotFound(format!(
                "backup not found: {}",
                req.timestamp
            )));
        }

        // Preserve the current live state as its own backup before
        // overwriting it, same as every other mutating USB command. The
        // snapshot we're restoring from is protected from this reconcile's
        // pruning pass so it can't be deleted before the copy below reads it.
        self.backup_usb_databases_with_retention_protecting(
            &usb_root,
            "Before restore",
            Some(&req.timestamp),
        );

        for (stem, snapshot) in &files {
            if let Some(live_path) = live_path_for_stem(&usb_root, stem) {
                std::fs::copy(snapshot, &live_path)?;
            }
        }
        Ok(RestoreUsbBackupData { restored: true })
    }

    pub fn delete_usb_backup(&self, req: DeleteUsbBackupRequest) -> BackendResult<DeleteUsbBackupData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let files = files_for_timestamp(&usb_root, &req.timestamp);
        if files.is_empty() {
            return Err(BackendError::NotFound(format!(
                "backup not found: {}",
                req.timestamp
            )));
        }
        for (_, snapshot) in &files {
            std::fs::remove_file(snapshot)?;
        }
        // Best-effort: the reason sidecar may live in either dir (or not
        // exist at all for backups predating this feature).
        let _ = std::fs::remove_file(backup_reason_path(&usb_backups_dir(&usb_root), &req.timestamp));
        if let Some(cache_dir) = cache_backups_dir(&usb_root) {
            let _ = std::fs::remove_file(backup_reason_path(&cache_dir, &req.timestamp));
        }
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

    fn touch_reason(dir: &Path, ts: &str, reason: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = backup_reason_path(dir, ts);
        let encoded = serde_json::to_string(&BackupReason { reason: reason.to_string() }).unwrap();
        std::fs::write(&path, encoded).unwrap();
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

        reconcile_backups(usb_root, 10, None);

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

        reconcile_backups(usb_root, 10, None);

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

        reconcile_backups(usb_root, 2, None);

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
                .any(|i| i.location == "usb"
                    && i.files.iter().any(|f| f.filename.contains("2020-01-02")))
        );
        assert!(
            result
                .items
                .iter()
                .any(|i| i.location == "cache"
                    && i.files.iter().any(|f| f.filename.contains("2020-01-01")))
        );
    }

    #[test]
    fn list_usb_backups_bundles_pdb_and_edb_sharing_a_timestamp() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"pdb");
        touch_backup(
            &backups_dir,
            "exportLibrary",
            "db",
            "2020-01-01_00-00-00",
            b"edb",
        );

        let result = service
            .list_usb_backups(ListUsbBackupsRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
            })
            .expect("list backups");

        assert_eq!(result.items.len(), 1, "PDB+eDB from one event must bundle into a single entry");
        let entry = &result.items[0];
        assert_eq!(entry.timestamp, "2020-01-01_00-00-00");
        assert_eq!(entry.size_bytes, 6);
        assert_eq!(entry.files.len(), 2);
        assert!(entry.files.iter().any(|f| f.stem == "export"));
        assert!(entry.files.iter().any(|f| f.stem == "exportLibrary"));
        assert_eq!(entry.reason, None, "no sidecar written -- legacy backup has no recorded reason");
    }

    #[test]
    fn list_usb_backups_surfaces_reason_from_sidecar() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"pdb");
        touch_backup(&backups_dir, "exportLibrary", "db", "2020-01-01_00-00-00", b"edb");
        touch_reason(&backups_dir, "2020-01-01_00-00-00", "Before export");

        let result = service
            .list_usb_backups(ListUsbBackupsRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
            })
            .expect("list backups");

        assert_eq!(result.items[0].reason.as_deref(), Some("Before export"));
    }

    #[test]
    fn restore_usb_backup_overwrites_both_live_files_and_backs_up_current_state_first() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let live_pdb = vendor_pdb_path(usb_root);
        let live_edb = vendor_edb_path(usb_root);
        std::fs::create_dir_all(live_pdb.parent().unwrap()).unwrap();
        std::fs::write(&live_pdb, b"pdb-live-v2").unwrap();
        std::fs::write(&live_edb, b"edb-live-v2").unwrap();

        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"pdb-live-v1");
        touch_backup(
            &backups_dir,
            "exportLibrary",
            "db",
            "2020-01-01_00-00-00",
            b"edb-live-v1",
        );

        let result = service
            .restore_usb_backup(RestoreUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                timestamp: "2020-01-01_00-00-00".to_string(),
            })
            .expect("restore");
        assert!(result.restored);
        assert_eq!(std::fs::read(&live_pdb).unwrap(), b"pdb-live-v1");
        assert_eq!(std::fs::read(&live_edb).unwrap(), b"edb-live-v1");

        // The pre-restore live state must itself have been backed up, for both files.
        let remaining_pdb = list_stem_files(&backups_dir, "export", "pdb");
        assert!(
            remaining_pdb
                .iter()
                .any(|p| std::fs::read(p).map(|b| b == b"pdb-live-v2").unwrap_or(false)),
            "restore must back up the current live PDB before overwriting it"
        );
        let remaining_edb = list_stem_files(&backups_dir, "exportLibrary", "db");
        assert!(
            remaining_edb
                .iter()
                .any(|p| std::fs::read(p).map(|b| b == b"edb-live-v2").unwrap_or(false)),
            "restore must back up the current live eDB before overwriting it"
        );
    }

    /// Regression test: at retention_count=1, the pre-restore backup of the
    /// current live state used to be able to bump the restore target's own
    /// snapshot out of the retention window and prune it, right before the
    /// copy step tried to read it. The target must survive its own restore.
    #[test]
    fn restore_usb_backup_survives_pruning_its_own_snapshot_at_low_retention() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        name_drive(usb_root, "Club Stick");
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        service
            .set_frontend_setting(crate::models::SetFrontendSettingRequest {
                key: SETTING_UI_BACKUP_RETENTION_COUNT.to_string(),
                value: Some("1".to_string()),
            })
            .expect("set retention count");

        let live_pdb = vendor_pdb_path(usb_root);
        let live_edb = vendor_edb_path(usb_root);
        std::fs::create_dir_all(live_pdb.parent().unwrap()).unwrap();
        std::fs::write(&live_pdb, b"pdb-live").unwrap();
        std::fs::write(&live_edb, b"edb-live").unwrap();

        // The restore target already sits alone in the cache (e.g. archived
        // by an earlier reconcile), so it's the oldest -- and only -- entry
        // pruning would consider once the fresh pre-restore backup displaces
        // it as "newest".
        let cache_dir = cache_backups_dir(usb_root).expect("cache dir for named drive");
        touch_backup(&cache_dir, "export", "pdb", "2020-01-01_00-00-00", b"pdb-target");
        touch_backup(&cache_dir, "exportLibrary", "db", "2020-01-01_00-00-00", b"edb-target");

        let result = service
            .restore_usb_backup(RestoreUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                timestamp: "2020-01-01_00-00-00".to_string(),
            })
            .expect("restore must not fail even though it would otherwise prune its own source");
        assert!(result.restored);
        assert_eq!(std::fs::read(&live_pdb).unwrap(), b"pdb-target");
        assert_eq!(std::fs::read(&live_edb).unwrap(), b"edb-target");
    }

    #[test]
    fn restore_usb_backup_errors_when_snapshot_missing() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let err = service
            .restore_usb_backup(RestoreUsbBackupRequest {
                usb_root: usb.path().to_string_lossy().to_string(),
                timestamp: "1999-01-01_00-00-00".to_string(),
            })
            .expect_err("missing snapshot must error");
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn delete_usb_backup_removes_both_files() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let backups_dir = usb_backups_dir(usb_root);
        let pdb_snapshot =
            touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"pdb");
        let edb_snapshot = touch_backup(
            &backups_dir,
            "exportLibrary",
            "db",
            "2020-01-01_00-00-00",
            b"edb",
        );

        let result = service
            .delete_usb_backup(DeleteUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                timestamp: "2020-01-01_00-00-00".to_string(),
            })
            .expect("delete");
        assert!(result.deleted);
        assert!(!pdb_snapshot.is_file());
        assert!(!edb_snapshot.is_file());

        let err = service
            .delete_usb_backup(DeleteUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                timestamp: "2020-01-01_00-00-00".to_string(),
            })
            .expect_err("deleting again must error");
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn delete_usb_backup_removes_reason_sidecar() {
        let (_dir, service) = test_service();
        let usb = tempdir().unwrap();
        let usb_root = usb.path();
        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"pdb");
        let sidecar = touch_reason(&backups_dir, "2020-01-01_00-00-00", "Before export");

        service
            .delete_usb_backup(DeleteUsbBackupRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                timestamp: "2020-01-01_00-00-00".to_string(),
            })
            .expect("delete");

        assert!(!sidecar.is_file());
    }

    #[test]
    fn reconcile_moves_reason_sidecar_alongside_archived_pair() {
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        name_drive(usb_root, "Club Stick");
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        let backups_dir = usb_backups_dir(usb_root);
        touch_backup(&backups_dir, "export", "pdb", "2020-01-01_00-00-00", b"old");
        touch_reason(&backups_dir, "2020-01-01_00-00-00", "Before export");
        touch_backup(&backups_dir, "export", "pdb", "2020-01-02_00-00-00", b"newest");
        touch_reason(&backups_dir, "2020-01-02_00-00-00", "Before playlist reorder");

        reconcile_backups(usb_root, 10, None);

        let cache_dir = cache_backups_dir(usb_root).expect("cache dir for named drive");
        assert!(
            backup_reason_path(&cache_dir, "2020-01-01_00-00-00").is_file(),
            "sidecar for the archived (older) pair must follow it to the cache dir"
        );
        assert!(
            !backup_reason_path(&backups_dir, "2020-01-01_00-00-00").is_file(),
            "sidecar must not be left behind on the USB once its pair has fully moved"
        );
        assert!(
            backup_reason_path(&backups_dir, "2020-01-02_00-00-00").is_file(),
            "sidecar for the pair kept on the USB (newest) must stay put"
        );
    }

    #[test]
    fn reconcile_prunes_reason_sidecar_when_pair_fully_pruned() {
        let usb = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let usb_root = usb.path();
        name_drive(usb_root, "Club Stick");
        let _guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));

        let backups_dir = usb_backups_dir(usb_root);
        for i in 1..=3 {
            let ts = format!("2020-01-0{i}_00-00-00");
            touch_backup(&backups_dir, "export", "pdb", &ts, format!("v{i}").as_bytes());
            touch_reason(&backups_dir, &ts, "Before export");
        }

        reconcile_backups(usb_root, 1, None);

        let cache_dir = cache_backups_dir(usb_root).expect("cache dir for named drive");
        assert!(
            !backup_reason_path(&cache_dir, "2020-01-01_00-00-00").is_file(),
            "sidecar for a fully-pruned pair must be deleted, not orphaned in the cache dir"
        );
        assert!(
            backup_reason_path(&backups_dir, "2020-01-03_00-00-00").is_file(),
            "sidecar for the surviving newest pair must remain"
        );
    }
}
