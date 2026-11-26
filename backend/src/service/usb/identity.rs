//! User-assigned identity for a USB root: a name, stored in a marker file on
//! the root itself (so it survives being plugged into a different computer)
//! and mirrored into the drive's `usb_devices` row (its name shown while it
//! isn't plugged in, and the uniqueness index). A named drive's row is found
//! by its name, not its mount path: drives with the same volume label mount
//! at the same path, and on Windows every stick can get the same letter.
//!
//! The app has no real USB device enumeration -- `resolve_usb_root` accepts
//! any directory, not just a real removable device -- so a hardware
//! serial/UUID wouldn't be meaningfully more "real" than a name the user
//! assigns to whatever root they point the app at. The marker file lives
//! next to `PIONEER/`, not inside it, so it's untouched by rekordbox itself
//! and by this app's own validation/repair scans (neither enumerates the
//! root's directory listing, only known paths).
//!
//! The same file carries the stick's USB audio formats ([`UsbAudioFormats`]): which
//! audio formats export puts on it, so they travel with the stick too.

use super::root::resolve_usb_root;
use crate::audio_format::UsbAudioFormat;
use crate::error::{BackendError, BackendResult};
use crate::i18n::{Text, tr};
use crate::service::{BackendService, now};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MARKER_DIR: &str = ".dj-usb-tkit";
const MARKER_FILE: &str = "drive.json";
const MAX_NAME_LEN: usize = 80;

/// `drive.json`. Every write reads the file first and changes only its own
/// field, so a rename keeps the formats and the other way round. Fields this
/// version doesn't know (written by a newer one) are kept as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DriveMarker {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    /// USB audio formats, in the user's order of preference; see [`UsbAudioFormats`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    formats: Vec<MarkerFormat>,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarkerFormat {
    format: String,
    allowed: bool,
}

/// The formats export puts on a stick (USB audio formats in the USB panel), as
/// the full ordered list: an unticked format keeps its place.
/// The order is the user's preference among the ticked ones. At least one is
/// always ticked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsbAudioFormats(Vec<(UsbAudioFormat, bool)>);

impl Default for UsbAudioFormats {
    /// All ticked, lossless first: an unconfigured stick warns about nothing
    /// new.
    fn default() -> Self {
        Self(
            UsbAudioFormat::DEFAULT_ORDER
                .into_iter()
                .map(|format| (format, true))
                .collect(),
        )
    }
}

impl UsbAudioFormats {
    pub(crate) fn entries(&self) -> &[(UsbAudioFormat, bool)] {
        &self.0
    }

    pub(crate) fn allows(&self, format: UsbAudioFormat) -> bool {
        self.0
            .iter()
            .any(|&(entry, allowed)| entry == format && allowed)
    }

    /// A list from `drive.json`, which may be hand-edited or from another
    /// version: unknown ids and repeats are dropped, missing formats are
    /// appended ticked in default order, and a list with nothing ticked reads
    /// as the default.
    fn from_marker(raw: &[MarkerFormat]) -> Self {
        let mut entries: Vec<(UsbAudioFormat, bool)> = Vec::new();
        for item in raw {
            if let Some(format) = UsbAudioFormat::parse(&item.format)
                && !entries.iter().any(|&(entry, _)| entry == format)
            {
                entries.push((format, item.allowed));
            }
        }
        for format in UsbAudioFormat::DEFAULT_ORDER {
            if !entries.iter().any(|&(entry, _)| entry == format) {
                entries.push((format, true));
            }
        }
        if entries.iter().any(|&(_, allowed)| allowed) {
            Self(entries)
        } else {
            Self::default()
        }
    }

    /// A list the user sent: every format exactly once, at least one ticked.
    /// Only "none ticked" is the user's; the others mean the UI sent a broken list.
    pub(crate) fn from_request(items: &[(String, bool)]) -> BackendResult<Self> {
        let mut entries: Vec<(UsbAudioFormat, bool)> = Vec::new();
        for (id, allowed) in items {
            let format = UsbAudioFormat::parse(id)
                .ok_or_else(|| BackendError::Internal(format!("Unknown audio format: {id}")))?;
            if entries.iter().any(|&(entry, _)| entry == format) {
                return Err(BackendError::Internal(format!(
                    "{} is listed twice",
                    format.label()
                )));
            }
            entries.push((format, *allowed));
        }
        if let Some(missing) = UsbAudioFormat::DEFAULT_ORDER
            .into_iter()
            .find(|format| !entries.iter().any(|&(entry, _)| entry == *format))
        {
            return Err(BackendError::Internal(format!(
                "{} is missing from the list",
                missing.label()
            )));
        }
        if !entries.iter().any(|&(_, allowed)| allowed) {
            return Err(BackendError::Validation(tr::usb_audio_formats_last_ticked()));
        }
        Ok(Self(entries))
    }

    fn to_marker(&self) -> Vec<MarkerFormat> {
        self.0
            .iter()
            .map(|&(format, allowed)| MarkerFormat {
                format: format.id().to_string(),
                allowed,
            })
            .collect()
    }
}

/// The app's own `.dj-usb-tkit/` directory at the root of a USB drive --
/// untouched by rekordbox and by this app's own validation/repair scans,
/// same reasoning as the drive-name marker file below. Shared with other
/// modules that want to store their own app-owned metadata on the drive
/// (e.g. `export_log`).
pub(crate) fn app_marker_dir(root: &Path) -> PathBuf {
    root.join(MARKER_DIR)
}

fn marker_path(root: &Path) -> PathBuf {
    app_marker_dir(root).join(MARKER_FILE)
}

/// Mount paths (normalized, as `root_path_key`) whose marker names a
/// different drive -- a copy of a named drive, or a drive that got another
/// one's name (see `BackendService::check_drive_identity`). Re-decided on
/// every connect, so it only lives for this process.
static NAME_CONFLICTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn root_key(root: &Path) -> String {
    crate::service::normalize_source_root_for_matching(&root.to_string_lossy())
}

pub(crate) fn set_name_conflict(root: &Path, conflicted: bool) {
    let key = root_key(root);
    let mut conflicts = NAME_CONFLICTS.lock().unwrap();
    conflicts.retain(|k| *k != key);
    if conflicted {
        conflicts.push(key);
    }
}

pub(crate) fn has_name_conflict(root: &Path) -> bool {
    let key = root_key(root);
    NAME_CONFLICTS.lock().unwrap().contains(&key)
}

/// The drive's name as the rest of the app should use it (cache and backup
/// keys, labels): the marker's name, unless that name belongs to another
/// drive, in which case the drive counts as unnamed until it gets its own.
pub(crate) fn drive_name(root: &Path) -> Option<String> {
    if has_name_conflict(root) {
        return None;
    }
    read_drive_name(root)
}

/// Read the user-assigned name from the marker file at `root`, if any.
/// Absence/corruption is treated as "unnamed", never an error.
pub(crate) fn read_drive_name(root: &Path) -> Option<String> {
    let marker = read_marker(root)?;
    let trimmed = marker.name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Validate and normalize a user-supplied drive name (trim, non-empty, length cap).
pub(crate) fn validate_name(name: &str) -> Result<String, Text> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(tr::usb_error_name_empty());
    }
    if trimmed.chars().count() > MAX_NAME_LEN {
        return Err(tr::usb_error_name_too_long(MAX_NAME_LEN));
    }
    Ok(trimmed.to_string())
}

/// Write `name` to the marker file at `root`, creating the marker directory
/// if needed. Caller is responsible for uniqueness checks against other
/// known drives (see `find_registered_drive`).
pub(crate) fn write_drive_name(root: &Path, name: &str) -> std::io::Result<()> {
    let validated =
        validate_name(name).map_err(|text| std::io::Error::other(text.render_english()))?;
    let mut marker = read_marker(root).unwrap_or_default();
    marker.name = validated;
    write_marker(root, &marker)
}

/// The stick's USB audio formats. A missing or unreadable `drive.json` reads as
/// the default (everything ticked), never an error.
pub(crate) fn read_usb_audio_formats(root: &Path) -> UsbAudioFormats {
    read_marker(root)
        .map(|marker| UsbAudioFormats::from_marker(&marker.formats))
        .unwrap_or_default()
}

/// Write the stick's USB audio formats, keeping its name.
pub(crate) fn write_usb_audio_formats(
    root: &Path,
    formats: &UsbAudioFormats,
) -> std::io::Result<()> {
    let mut marker = read_marker(root).unwrap_or_default();
    marker.formats = formats.to_marker();
    write_marker(root, &marker)
}

/// `None` for a missing or corrupt file.
fn read_marker(root: &Path) -> Option<DriveMarker> {
    let bytes = std::fs::read(marker_path(root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_marker(root: &Path, marker: &DriveMarker) -> std::io::Result<()> {
    std::fs::create_dir_all(app_marker_dir(root))?;
    let bytes = serde_json::to_vec_pretty(marker)?;
    std::fs::write(marker_path(root), bytes)
}

/// Filesystem/cache-key-safe slug: lowercase ascii alphanumerics with runs of
/// anything else collapsed to a single hyphen, trimmed of trailing hyphens.
/// Empty/all-non-ascii input falls back to `"drive"` so callers always get a
/// non-empty key.
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_sep = false;
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_was_sep = false;
        } else if !last_was_sep && !out.is_empty() {
            out.push('-');
            last_was_sep = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "drive".to_string()
    } else {
        out
    }
}

/// Best-effort guess at a name for `usb_root`, drawn from the OS's own
/// filesystem label -- only offered when the root is actually on a
/// removable USB device (never suggested for an arbitrary local directory).
/// The user still confirms/edits it; a failed guess just means an empty
/// field, never an error.
#[cfg(target_os = "linux")]
pub(crate) fn suggest_drive_name(usb_root: &Path) -> Option<String> {
    let canon = std::fs::canonicalize(usb_root).ok()?;
    let device = linux_mount_source_for_path(&canon)?;
    if !linux_device_is_usb(&device) {
        return None;
    }
    linux_fs_label_for_device(&device)
}

/// Windows/macOS: delegate to `sysinfo`'s disk enumeration (backed by
/// `GetVolumeInformationW`/`GetDriveTypeW` on Windows, DiskArbitration on
/// macOS) rather than hand-rolling the equivalent FFI ourselves -- unlike
/// the Linux path above (plain files under `/proc` and `/dev`, easy to get
/// right and to unit test), the Windows/macOS APIs are binary FFI that
/// can't be exercised or verified from this Linux-only dev environment, so
/// reusing an established, widely-used crate is the safer bet.
#[cfg(not(target_os = "linux"))]
pub(crate) fn suggest_drive_name(usb_root: &Path) -> Option<String> {
    let canon = std::fs::canonicalize(usb_root).ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list();

    // Same "longest matching mount point wins" rule as the Linux path.
    let disk = disks
        .list()
        .iter()
        .fold(None, |best: Option<&sysinfo::Disk>, disk| {
            let mount = disk.mount_point();
            if !canon.starts_with(mount) {
                return best;
            }
            match best {
                Some(b) if b.mount_point().as_os_str().len() >= mount.as_os_str().len() => best,
                _ => Some(disk),
            }
        })?;

    if !disk.is_removable() {
        return None;
    }
    let name = disk.name().to_string_lossy().trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

#[cfg(target_os = "linux")]
fn linux_unescape_mount_field(raw: &str) -> String {
    // /proc/mounts octal-escapes space/tab/newline/backslash in its fields
    // (see proc(5)) -- undo that so paths compare correctly.
    raw.replace("\\134", "\\")
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
}

/// A `/proc/mounts` entry: what is mounted where, as which filesystem.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinuxMount {
    /// The mount source, e.g. `/dev/sdb1`.
    pub(crate) source: PathBuf,
    pub(crate) mount_point: PathBuf,
    /// The filesystem type as the kernel names it (`vfat`, `exfat`, `fuseblk`, ...).
    pub(crate) fs_type: String,
}

/// The `/proc/mounts` entry covering `path`: the one whose mount point is the
/// longest matching prefix of `path`, the same "most specific mount wins" rule
/// `df`/`lsblk` use.
#[cfg(target_os = "linux")]
pub(crate) fn linux_mount_for_path(path: &Path) -> Option<LinuxMount> {
    let content = std::fs::read_to_string("/proc/mounts").ok()?;
    linux_mount_from_mounts_content(&content, path)
}

#[cfg(target_os = "linux")]
fn linux_mount_source_for_path(path: &Path) -> Option<PathBuf> {
    linux_mount_for_path(path).map(|mount| mount.source)
}

#[cfg(target_os = "linux")]
fn linux_mount_from_mounts_content(content: &str, path: &Path) -> Option<LinuxMount> {
    let mut best: Option<LinuxMount> = None;
    for line in content.lines() {
        let mut fields = line.split_whitespace();
        let (Some(fs_spec), Some(fs_file), Some(fs_vfstype)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let mount_point = PathBuf::from(linux_unescape_mount_field(fs_file));
        if !path.starts_with(&mount_point) {
            continue;
        }
        let is_longer = best.as_ref().is_none_or(|current| {
            mount_point.as_os_str().len() > current.mount_point.as_os_str().len()
        });
        if is_longer {
            best = Some(LinuxMount {
                source: PathBuf::from(linux_unescape_mount_field(fs_spec)),
                mount_point,
                fs_type: fs_vfstype.to_string(),
            });
        }
    }
    best
}

/// Whether `device` is backed by USB (has a `usb-*` entry under
/// `/dev/disk/by-id/` resolving to it) -- the same technique the `sysinfo`
/// crate uses for its own `Disk::is_removable()`, reimplemented here
/// directly since enabling sysinfo's `disk` feature would pull in
/// Windows/macOS-only dependencies this codebase has no other use for.
#[cfg(target_os = "linux")]
pub(crate) fn linux_device_is_usb(device: &Path) -> bool {
    let Ok(device_canon) = device.canonicalize() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir("/dev/disk/by-id/") else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("usb-"))
            && entry.path().canonicalize().is_ok_and(|p| p == device_canon)
    })
}

/// Reads the filesystem label for `device` via `/dev/disk/by-label/`
/// (populated by udev from the partition's own volume label).
#[cfg(target_os = "linux")]
fn linux_fs_label_for_device(device: &Path) -> Option<String> {
    let device_canon = device.canonicalize().ok()?;
    let entries = std::fs::read_dir("/dev/disk/by-label/").ok()?;
    for entry in entries.flatten() {
        let Ok(target) = entry.path().canonicalize() else {
            continue;
        };
        if target != device_canon {
            continue;
        }
        let raw = entry.file_name().to_string_lossy().to_string();
        let label = linux_unescape_udev_label(&raw);
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// udev encodes non-alphanumeric bytes in `by-label` symlink names as
/// `\xHH` hex escapes; undo that to recover the real label text.
#[cfg(target_os = "linux")]
fn linux_unescape_udev_label(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && bytes.get(i + 1) == Some(&b'x')
            && i + 3 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 2..i + 4])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `usb_devices` id of the drive at `usb_root`, if this computer knows
/// it: a named drive's row by its name, wherever it was seen; an unnamed
/// drive's by its mount path (see `db::USB_DEVICES_COLUMNS`).
pub(crate) fn usb_device_id_for_root(
    conn: &rusqlite::Connection,
    usb_root: &Path,
) -> BackendResult<Option<String>> {
    Ok(match drive_name(usb_root) {
        Some(name) => conn
            .query_row(
                "SELECT id FROM usb_devices WHERE name_slug = ?1",
                params![slug(&name)],
                |row| row.get(0),
            )
            .optional()?,
        None => unnamed_device_id(conn, &root_key(usb_root))?,
    })
}

/// The unnamed row for a mount path, a forgotten one included.
fn unnamed_device_id(conn: &rusqlite::Connection, root_key: &str) -> BackendResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM usb_devices WHERE root_path_key = ?1 AND name_slug IS NULL",
            params![root_key],
            |row| row.get(0),
        )
        .optional()?)
}

impl BackendService {
    /// The recent list, each drive with whether this computer keeps a cache
    /// of it (`missing` when it keeps none). Without a cache root (only tests
    /// run without one) nothing is missing.
    pub fn list_usb_devices(&self) -> BackendResult<crate::models::ListUsbDevicesData> {
        let conn = self.db.connect()?;
        let cached: Option<std::collections::HashSet<String>> =
            super::staging::cache_dirs().map(|dirs| dirs.into_iter().map(|(key, _)| key).collect());
        let mut stmt = conn.prepare(
            "SELECT id, root_path, label, mounted, first_seen_at, last_seen_at, name_slug
             FROM usb_devices
             WHERE deleted_at IS NULL
             ORDER BY last_seen_at DESC",
        )?;
        let items = stmt
            .query_map([], |row| {
                let root_path: String = row.get(1)?;
                let label: Option<String> = row.get(2)?;
                let name_slug: Option<String> = row.get(6)?;
                let root = Path::new(&root_path);
                // Its name's cache, or the one from before it was named.
                let missing = cached.as_ref().is_some_and(|cached| {
                    let name = name_slug.and(label.clone());
                    !cached.contains(&super::staging::cache_key(name.as_deref(), root))
                        && !cached.contains(&super::staging::cache_key(None, root))
                });
                Ok(crate::models::UsbDeviceSummary {
                    id: row.get(0)?,
                    tooltip_text: if root_path.is_empty() {
                        tr::usb_recent_no_folder_tooltip().render()
                    } else if missing {
                        tr::usb_recent_no_cache_tooltip(&root_path).render()
                    } else {
                        root_path.clone()
                    },
                    root_path,
                    label,
                    mounted: row.get::<_, i64>(3)? != 0,
                    first_seen_at: row.get(4)?,
                    last_seen_at: row.get(5)?,
                    missing,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(crate::models::ListUsbDevicesData { items })
    }

    /// What removing the list entry `id` would delete, for the dialog before
    /// `prune_usb_device`: forgets the drive in a transaction it rolls back,
    /// to count the tracks that would go.
    pub fn preview_prune_usb_device(
        &self,
        req: crate::models::PruneUsbDeviceRequest,
    ) -> BackendResult<crate::models::PreviewPruneUsbDeviceData> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let drive = forget_usb_device(&tx, &req.id, &now())?
            .ok_or_else(|| BackendError::NotFound(tr::usb_error_device_not_listed(&req.id)))?;
        tx.rollback()?;
        Ok(forget_confirm_texts(&drive))
    }

    /// `sync_recent_usbs` at app start (`new()`). A failure is logged; the
    /// list then shows what it had, and the next start syncs again.
    pub(crate) fn sync_recent_usbs_at_start(&self) {
        let synced = self
            .db
            .connect()
            .and_then(|conn| sync_recent_usbs(&conn, &now()));
        match synced {
            Ok(0) => {}
            Ok(n) => crate::logging::debug(
                "usb-devices",
                format!("{n} USB cache folders came back into the recent list"),
            ),
            Err(err) => crate::logging::debug(
                "usb-devices",
                format!("syncing the recent USB list with the cache failed: {err:?}"),
            ),
        }
    }

    /// Forgets a drive removed from the list (`forget_usb_device`), and its
    /// local cache: staged databases and the backups archived from it.
    pub fn prune_usb_device(
        &self,
        req: crate::models::PruneUsbDeviceRequest,
    ) -> BackendResult<crate::models::PruneUsbDeviceData> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let forgotten = forget_usb_device(&tx, &req.id, &now())?;
        tx.commit()?;
        drop(conn);
        if let Some(drive) = &forgotten {
            for key in &drive.cache_keys {
                crate::service::usb::staging::remove_cache_dir(key).map_err(|e| {
                    BackendError::Internal(format!("failed to remove the drive's cache: {e}"))
                })?;
            }
            self.invalidate_usb_parse_cache();
        }
        Ok(crate::models::PruneUsbDeviceData {
            pruned: forgotten.is_some(),
            devices: self.list_usb_devices()?.items,
        })
    }

    /// Reads the user-assigned name for `req.usb_root` from the on-drive
    /// marker (survives a different computer). A drive without one is
    /// unnamed, even when `usb_devices` has a name cached for its mount path:
    /// drives with the same volume label mount at the same path, so that
    /// name may belong to a different drive. A drive whose marker names
    /// another drive is unnamed too (`check_drive_identity`). See
    /// `usb::identity` for why a name, not a hardware serial, is this app's
    /// notion of drive identity.
    pub fn get_usb_device_name(
        &self,
        req: crate::models::GetUsbDeviceNameRequest,
    ) -> BackendResult<crate::models::GetUsbDeviceNameData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let conn = self.db.connect()?;
        let name_taken_by = self.check_drive_identity(&conn, &usb_root)?;
        if name_taken_by.is_none()
            && let Some(name) = crate::service::usb::identity::read_drive_name(&usb_root)
        {
            return Ok(crate::models::GetUsbDeviceNameData {
                name: Some(name),
                suggested_name: None,
                name_taken_by: None,
            });
        }

        // The OS label is often the other drive's name: suggesting it would
        // have the two share it again.
        let suggested_name = crate::service::usb::identity::suggest_drive_name(&usb_root)
            .map(|suggested| suggested.trim().to_string())
            .filter(|suggested| {
                name_taken_by.as_deref().is_none_or(|taken| {
                    crate::service::usb::identity::slug(taken)
                        != crate::service::usb::identity::slug(suggested)
                })
            });
        Ok(crate::models::GetUsbDeviceNameData {
            name: None,
            suggested_name,
            name_taken_by,
        })
    }

    /// Decides whether the drive at `usb_root` is the drive its name marker
    /// names, or a different one carrying that name: a copy of the drive
    /// (stick or folder), or a drive that took its name through an old bug.
    /// It's a different drive when the drive this computer knows by that
    /// name had a different size, or got an export (logged on it, see
    /// `export_log`) that this drive's export log doesn't have. A different
    /// drive counts as unnamed (`usb::identity::drive_name`) so it gets its
    /// own cache and backups; returns the name it carries.
    ///
    /// Otherwise registers the name, so drives named on another computer
    /// count as taken here too, and labels the mount path with it.
    pub(crate) fn check_drive_identity(
        &self,
        conn: &rusqlite::Connection,
        usb_root: &Path,
    ) -> BackendResult<Option<String>> {
        let marker = crate::service::usb::identity::read_drive_name(usb_root);
        let registered = match &marker {
            Some(name) => find_registered_drive(conn, name)?,
            None => None,
        };
        let total_bytes = fs4::total_space(usb_root).ok();
        let conflict = registered.as_ref().is_some_and(|drive| {
            let size_differs =
                matches!((drive.total_bytes, total_bytes), (Some(known), Some(now)) if known != now);
            // An unreadable log proves nothing either way.
            let export_missing = drive.last_export_at.as_deref().is_some_and(|exported_at| {
                crate::service::export_log::has_export_log_record(usb_root, exported_at) == Some(false)
            });
            size_differs || export_missing
        });

        if crate::service::usb::identity::has_name_conflict(usb_root) != conflict {
            crate::service::usb::identity::set_name_conflict(usb_root, conflict);
            crate::service::usb::staging::forget_cache_key_for_root(usb_root);
            self.invalidate_usb_parse_cache();
        }
        if conflict {
            return Ok(registered.map(|drive| drive.name));
        }
        if let Some(name) = &marker {
            register_drive(conn, usb_root, name, &now(), false)?;
            if let Some(total_bytes) = total_bytes {
                fill_registered_drive_size(conn, name, total_bytes)?;
            }
        }
        Ok(None)
    }

    /// Assigns `req.name` to `req.usb_root`, rejecting names already taken by
    /// a different drive (names double as cache/backup-archive keys). Also
    /// renames: the drive's cache dir moves to the new name and its old name
    /// is freed. A drive carrying another drive's name may take that name
    /// back (it is that drive after all), which makes it the drive the name
    /// is checked against. Writes the on-drive marker (source of truth) and the
    /// drive's `usb_devices` row.
    pub fn set_usb_device_name(
        &self,
        req: crate::models::SetUsbDeviceNameRequest,
    ) -> BackendResult<crate::models::SetUsbDeviceNameData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let name = crate::service::usb::identity::validate_name(&req.name)
            .map_err(BackendError::Validation)?;
        let same_name = |other: &str| {
            crate::service::usb::identity::slug(other) == crate::service::usb::identity::slug(&name)
        };
        let marker = crate::service::usb::identity::read_drive_name(&usb_root);
        let conflicted = crate::service::usb::identity::has_name_conflict(&usb_root);
        // A carried name isn't this drive's: renaming away from it must not
        // take the other drive's cache or free its name.
        let current = marker.as_deref().filter(|_| !conflicted);
        let updated = current != Some(name.as_str());
        let renaming_away = current.filter(|current| !same_name(current));
        let claims_name = conflicted && marker.as_deref().is_some_and(same_name);

        let conn = self.db.connect()?;
        // Keeping its own name (or changing only its case) is not a collision.
        let keeps_own_name = current.is_some() && renaming_away.is_none();
        if !keeps_own_name
            && !claims_name
            && let Some(taken) = find_registered_drive(&conn, &name)?
        {
            return Err(BackendError::Validation(tr::usb_error_name_taken(
                &name,
                &taken.name,
            )));
        }

        if let Some(old_name) = renaming_away {
            crate::service::usb::staging::rename_cache_dir(old_name, &name).map_err(|e| {
                BackendError::Internal(format!("failed to move the drive's cache: {e}"))
            })?;
        }
        if let Err(e) = crate::service::usb::identity::write_drive_name(&usb_root, &name) {
            if let Some(old_name) = renaming_away {
                let _ = crate::service::usb::staging::rename_cache_dir(&name, old_name);
            }
            return Err(BackendError::Internal(format!(
                "failed to write drive name marker: {e}"
            )));
        }
        crate::service::usb::identity::set_name_conflict(&usb_root, false);
        crate::service::usb::staging::forget_cache_key_for_root(&usb_root);
        let now_ts = now();
        if let Some(old_name) = renaming_away {
            rename_drive(&conn, old_name, &name, &now_ts)?;
            // Earlier exports to this drive name it as it is now called. By
            // its old name, not its mount path: another stick may have used
            // the same path, and names are unique to one drive.
            conn.execute(
                "UPDATE playlists SET last_exported_usb_name = ?1 WHERE last_exported_usb_name = ?2",
                params![name, old_name],
            )?;
        }
        register_drive(&conn, &usb_root, &name, &now_ts, true)?;
        if !keeps_own_name {
            // This drive is now the one the name is checked against.
            set_registered_drive(
                &conn,
                &name,
                crate::service::export_log::latest_export_log_record_at(&usb_root).as_deref(),
                fs4::total_space(&usb_root).ok(),
            )?;
        }
        self.invalidate_usb_parse_cache();

        Ok(crate::models::SetUsbDeviceNameData {
            status_text: if updated {
                tr::usb_drive_renamed(&name)
            } else {
                tr::usb_drive_name_unchanged()
            }
            .render(),
            name,
            updated,
        })
    }
}

/// The `usb_devices` row of the drive at `root`, made if this computer
/// doesn't know the drive yet: a named drive's (`register_drive`), else the
/// mount path's unnamed row. Any touch brings a forgotten path's row back.
/// `mark_mounted=true` additionally makes this the ONLY mounted device --
/// every other row is unmounted in the same call (`validate_usb_root`'s
/// success path is the sole caller that passes `true`). `mark_mounted=false`
/// only bumps `last_seen_at`, leaving mounted state untouched everywhere
/// else (the USB browse/materialize and export paths must not imply "just
/// mounted" or silently unmount whatever the user actually has selected).
pub(crate) fn upsert_usb_device(
    conn: &rusqlite::Connection,
    root: &Path,
    mark_mounted: bool,
    now_ts: &str,
) -> BackendResult<String> {
    let id = match drive_name(root) {
        Some(name) => register_drive(conn, root, &name, now_ts, false)?,
        None => upsert_unnamed_device(conn, root, now_ts)?,
    };

    if mark_mounted {
        conn.execute(
            "UPDATE usb_devices SET mounted = 0, updated_at = ?1 WHERE mounted != 0 AND id != ?2",
            rusqlite::params![now_ts, id],
        )?;
        conn.execute(
            "UPDATE usb_devices SET mounted = 1, updated_at = ?1 WHERE id = ?2",
            rusqlite::params![now_ts, id],
        )?;
    }

    Ok(id)
}

fn upsert_unnamed_device(
    conn: &rusqlite::Connection,
    root: &Path,
    now_ts: &str,
) -> BackendResult<String> {
    let root_path = root.to_string_lossy().to_string();
    let key = root_key(root);
    if let Some(id) = unnamed_device_id(conn, &key)? {
        conn.execute(
            "UPDATE usb_devices SET root_path = ?1, last_seen_at = ?2, updated_at = ?2, deleted_at = NULL WHERE id = ?3",
            rusqlite::params![root_path, now_ts, id],
        )?;
        return Ok(id);
    }
    insert_device(conn, &root_path, &key, None, now_ts)
}

fn insert_device(
    conn: &rusqlite::Connection,
    root_path: &str,
    root_key: &str,
    name: Option<&str>,
    now_ts: &str,
) -> BackendResult<String> {
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO usb_devices (id, root_path, root_path_key, label, name_slug, mounted,
                                  first_seen_at, last_seen_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6, ?6, ?6)",
        rusqlite::params![id, root_path, root_key, name, name.map(slug), now_ts],
    )?;
    Ok(id)
}

/// The row of the drive named `name`, now seen at `root`; made when no
/// drive holds the name. Registering the name makes it taken here too, also
/// for a drive named on another computer. `adopt_path_row`: the drive at hand
/// is being named, and the USB links read from it so far are its mount
/// path's unnamed row's, so that row becomes the drive's.
pub(crate) fn register_drive(
    conn: &rusqlite::Connection,
    root: &Path,
    name: &str,
    now_ts: &str,
    adopt_path_row: bool,
) -> BackendResult<String> {
    let root_path = root.to_string_lossy().to_string();
    let key = root_key(root);
    let mut id: Option<String> = conn
        .query_row(
            "SELECT id FROM usb_devices WHERE name_slug = ?1",
            rusqlite::params![slug(name)],
            |row| row.get(0),
        )
        .optional()?;
    if id.is_none() && adopt_path_row {
        id = conn
            .query_row(
                "SELECT id FROM usb_devices
                 WHERE root_path_key = ?1 AND name_slug IS NULL AND deleted_at IS NULL",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .optional()?;
    }
    let Some(id) = id else {
        return insert_device(conn, &root_path, &key, Some(name), now_ts);
    };
    conn.execute(
        "UPDATE usb_devices
         SET name_slug = ?1, label = ?2, root_path = ?3, root_path_key = ?4,
             last_seen_at = ?5, updated_at = ?5, deleted_at = NULL
         WHERE id = ?6",
        rusqlite::params![slug(name), name, root_path, key, now_ts, id],
    )?;
    Ok(id)
}

/// A named drive: its name, plus what tells it apart from a copy (see
/// `BackendService::check_drive_identity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredDrive {
    pub name: String,
    /// `exportedAt` of the last export this computer logged on the drive.
    pub last_export_at: Option<String>,
    /// The drive's (or, for a folder, its disk's) size.
    pub total_bytes: Option<u64>,
}

/// The drive whose name shares `name`'s slug, if any. Names double as
/// cache/backup-archive keys (`usb::identity::slug`), so "My USB" and
/// "my-usb" are the same name here.
pub(crate) fn find_registered_drive(
    conn: &rusqlite::Connection,
    name: &str,
) -> BackendResult<Option<RegisteredDrive>> {
    let row = conn
        .query_row(
            "SELECT label, last_export_at, total_bytes FROM usb_devices WHERE name_slug = ?1",
            rusqlite::params![slug(name)],
            |row| {
                Ok(RegisteredDrive {
                    name: row.get(0)?,
                    last_export_at: row.get(1)?,
                    total_bytes: row.get::<_, Option<i64>>(2)?.map(|n| n.max(0) as u64),
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Make the drive at hand the one holding `name`: from now on it's the
/// drive whose last export and size the name is checked against.
pub(crate) fn set_registered_drive(
    conn: &rusqlite::Connection,
    name: &str,
    last_export_at: Option<&str>,
    total_bytes: Option<u64>,
) -> BackendResult<()> {
    conn.execute(
        "UPDATE usb_devices SET last_export_at = ?1, total_bytes = ?2 WHERE name_slug = ?3",
        rusqlite::params![last_export_at, total_bytes.map(|n| n as i64), slug(name)],
    )?;
    Ok(())
}

/// Remember the size of the drive holding `name`, if it isn't known yet.
pub(crate) fn fill_registered_drive_size(
    conn: &rusqlite::Connection,
    name: &str,
    total_bytes: u64,
) -> BackendResult<()> {
    conn.execute(
        "UPDATE usb_devices SET total_bytes = ?1 WHERE name_slug = ?2 AND total_bytes IS NULL",
        rusqlite::params![total_bytes as i64, slug(name)],
    )?;
    Ok(())
}

/// Remember the export just logged on the drive holding `name`.
pub(crate) fn record_drive_export(
    conn: &rusqlite::Connection,
    name: &str,
    exported_at: &str,
) -> BackendResult<()> {
    conn.execute(
        "UPDATE usb_devices SET last_export_at = ?1 WHERE name_slug = ?2",
        rusqlite::params![exported_at, slug(name)],
    )?;
    Ok(())
}

/// The drive named `old_name` takes `new_name`, which no other drive holds
/// (the caller checked); `old_name` is free again.
pub(crate) fn rename_drive(
    conn: &rusqlite::Connection,
    old_name: &str,
    new_name: &str,
    now_ts: &str,
) -> BackendResult<()> {
    conn.execute(
        "UPDATE usb_devices SET name_slug = ?1, label = ?2, updated_at = ?3 WHERE name_slug = ?4",
        rusqlite::params![slug(new_name), new_name, now_ts, slug(old_name)],
    )?;
    Ok(())
}

/// What `forget_usb_device` forgot, and what it leaves to clear off this
/// computer's disk: the drive's cache, keyed by its name, or by the path for
/// an unnamed drive (`staging::cache_key`). A named drive also has its path's
/// cache from before it was named, unless a listed drive mounts there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForgottenDrive {
    /// Its name, or its folder when it has none.
    pub shown_name: String,
    pub cache_keys: Vec<String>,
    /// The track rows that went with its links.
    pub tracks: usize,
}

/// Forgets the drive of row `id`, one the user removed from the list (usually
/// someone else's stick, read to fix it): its USB links and export records go,
/// then the tracks only those links kept (`delete_unreferenced_tracks`), and
/// its name is free for another drive. The row goes too, unless it's the only
/// one of its mount path: then it stays as that path's forgotten row (see
/// `all_usb_device_root_paths`). `None` when no listed drive has the id.
pub(crate) fn forget_usb_device(
    conn: &rusqlite::Connection,
    id: &str,
    now_ts: &str,
) -> BackendResult<Option<ForgottenDrive>> {
    let row: Option<(String, String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT root_path, root_path_key, name_slug, label FROM usb_devices
             WHERE id = ?1 AND deleted_at IS NULL",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((root_path, key, name_slug, label)) = row else {
        return Ok(None);
    };
    let linked: Vec<String> = conn
        .prepare("SELECT DISTINCT track_id FROM track_usb_links WHERE usb_device_id = ?1")?
        .query_map(rusqlite::params![id], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    conn.execute(
        "DELETE FROM track_usb_links WHERE usb_device_id = ?1",
        rusqlite::params![id],
    )?;
    conn.execute(
        "DELETE FROM usb_device_exports WHERE usb_device_id = ?1",
        rusqlite::params![id],
    )?;
    let path_has_other_rows: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM usb_devices WHERE root_path_key = ?1 AND id != ?2)",
        rusqlite::params![key, id],
        |row| row.get(0),
    )?;
    if path_has_other_rows {
        conn.execute(
            "DELETE FROM usb_devices WHERE id = ?1",
            rusqlite::params![id],
        )?;
    } else {
        conn.execute(
            "UPDATE usb_devices
             SET name_slug = NULL, label = NULL, last_export_at = NULL, total_bytes = NULL,
                 mounted = 0, deleted_at = ?1, updated_at = ?1
             WHERE id = ?2",
            rusqlite::params![now_ts, id],
        )?;
    }
    let tracks = crate::service::delete_unreferenced_tracks(conn, &linked)?;
    let name = name_slug.and(label);
    let root = Path::new(&root_path);
    let mut cache_keys = vec![super::staging::cache_key(name.as_deref(), root)];
    let path_listed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM usb_devices WHERE root_path_key = ?1 AND deleted_at IS NULL)",
        rusqlite::params![key],
        |row| row.get(0),
    )?;
    let path_key = super::staging::cache_key(None, root);
    if !root_path.is_empty() && !path_listed && !cache_keys.contains(&path_key) {
        cache_keys.push(path_key);
    }
    Ok(Some(ForgottenDrive {
        shown_name: name.unwrap_or(root_path),
        cache_keys,
        tracks,
    }))
}

/// Syncs the recent list with this computer's USB cache, on every app
/// start: a cache dir no listed drive has (staged databases and archived
/// backups whose drive was removed while removing kept them, or whose delete
/// failed) puts its drive back in the list, to see and forget like any
/// other. A forgotten row the dir is for comes back; otherwise the dir comes
/// back as a drive named by its key, with no known folder. Answers how many
/// came back.
pub(crate) fn sync_recent_usbs(conn: &rusqlite::Connection, now_ts: &str) -> BackendResult<usize> {
    let dirs = super::staging::cache_dirs().unwrap_or_default();
    if dirs.is_empty() {
        return Ok(0);
    }
    let rows: Vec<(String, String, Option<String>, Option<String>, bool)> = conn
        .prepare("SELECT id, root_path, name_slug, label, deleted_at IS NOT NULL FROM usb_devices")?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut listed = std::collections::HashSet::new();
    let mut forgotten = std::collections::HashMap::new();
    for (id, root_path, name_slug, label, deleted) in rows {
        let root = Path::new(&root_path);
        // A forgotten row has no name: its cache is its path's.
        let path_key = super::staging::cache_key(None, root);
        if deleted {
            forgotten.entry(path_key).or_insert(id);
        } else {
            // A drive read before it was named staged under its path's key.
            listed.insert(super::staging::cache_key(
                name_slug.and(label).as_deref(),
                root,
            ));
            listed.insert(path_key);
        }
    }
    let mut synced = 0;
    // A dir whose name no drive's key can be isn't the app's.
    let unlisted = dirs
        .into_iter()
        .filter(|(key, _)| !listed.contains(key) && slug(key) == *key);
    for (key, _) in unlisted {
        if let Some(id) = forgotten.get(&key) {
            conn.execute(
                "UPDATE usb_devices SET deleted_at = NULL, mounted = 0, updated_at = ?1
                 WHERE id = ?2",
                params![now_ts, id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO usb_devices (id, root_path, root_path_key, label, name_slug,
                                          mounted, first_seen_at, last_seen_at, created_at,
                                          updated_at)
                 VALUES (?1, '', '', ?2, ?3, 0, ?4, ?4, ?4, ?4)",
                params![uuid::Uuid::now_v7().to_string(), key, key, now_ts],
            )?;
        }
        synced += 1;
    }
    Ok(synced)
}

/// "Delete …?" before `prune_usb_device`: what deleting the entry `id` takes
/// off this computer.
fn forget_confirm_texts(drive: &ForgottenDrive) -> crate::models::PreviewPruneUsbDeviceData {
    let mut lines = Vec::new();
    if drive.tracks > 0 {
        lines.push(tr::usb_forget_tracks(drive.tracks).render());
    }
    let usages: Vec<_> = drive
        .cache_keys
        .iter()
        .filter_map(|key| super::staging::cache_dir_usage(key))
        .collect();
    if !usages.is_empty() {
        let bytes = usages.iter().map(|usage| usage.bytes).sum::<u64>();
        let backups = usages
            .iter()
            .map(|usage| super::backups::snapshot_count(&usage.backups_dir))
            .sum::<usize>();
        lines.push(tr::usb_forget_cache(crate::utils::byte_size_precise(bytes), backups).render());
    }
    let mut message = if lines.is_empty() {
        tr::usb_forget_nothing_kept()
    } else {
        tr::usb_forget_intro()
    }
    .render();
    for line in lines {
        message.push_str("\n• ");
        message.push_str(&line);
    }
    message.push('\n');
    message.push_str(&tr::usb_forget_usb_backup_stays().render());
    crate::models::PreviewPruneUsbDeviceData {
        title_text: tr::usb_forget_title(&drive.shown_name).render(),
        message_text: message,
    }
}

/// All known device root paths (display form), ANY mount state, ANY age,
/// INCLUDING soft-deleted devices. Used only for the "is this path
/// untrustworthy as durable local storage" check (materialize matching,
/// `resolve_playback_source` exclusion) -- a path that was ever on removable
/// media stays untrustworthy even after the user prunes it from their UI
/// list, since pruning doesn't change the underlying filesystem fact.
///
/// Note the asymmetry with `list_usb_devices` (UI-facing, filters
/// `WHERE deleted_at IS NULL`) on purpose -- those two read paths have
/// different correctness requirements and must not share a single query.
pub(crate) fn all_usb_device_root_paths(conn: &rusqlite::Connection) -> BackendResult<Vec<String>> {
    // A drive back from the cache with no known folder has none to count.
    let mut stmt = conn.prepare("SELECT root_path FROM usb_devices WHERE root_path != ''")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for row in rows {
        paths.push(row?);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_drive_name_is_none_when_marker_absent() {
        let dir = tempdir().unwrap();
        assert_eq!(read_drive_name(dir.path()), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempdir().unwrap();
        write_drive_name(dir.path(), "  My Stick  ").unwrap();
        assert_eq!(read_drive_name(dir.path()), Some("My Stick".to_string()));
    }

    #[test]
    fn write_drive_name_rejects_empty() {
        let dir = tempdir().unwrap();
        let err = write_drive_name(dir.path(), "   ").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }

    #[test]
    fn read_drive_name_tolerates_corrupt_marker() {
        let dir = tempdir().unwrap();
        let marker_dir = dir.path().join(MARKER_DIR);
        std::fs::create_dir_all(&marker_dir).unwrap();
        std::fs::write(marker_dir.join(MARKER_FILE), b"not json").unwrap();
        assert_eq!(read_drive_name(dir.path()), None);
    }

    fn write_raw_marker(root: &Path, json: &str) {
        let marker_dir = root.join(MARKER_DIR);
        std::fs::create_dir_all(&marker_dir).unwrap();
        std::fs::write(marker_dir.join(MARKER_FILE), json).unwrap();
    }

    fn ids(formats: &UsbAudioFormats) -> Vec<(&'static str, bool)> {
        formats
            .entries()
            .iter()
            .map(|&(format, allowed)| (format.id(), allowed))
            .collect()
    }

    #[test]
    fn an_old_name_only_marker_reads_as_default_formats() {
        let dir = tempdir().unwrap();
        write_raw_marker(dir.path(), r#"{"name":"Old Stick"}"#);
        assert_eq!(
            read_usb_audio_formats(dir.path()),
            UsbAudioFormats::default()
        );
        assert_eq!(read_drive_name(dir.path()), Some("Old Stick".to_string()));
    }

    #[test]
    fn rename_keeps_formats_and_formats_write_keeps_name() {
        let dir = tempdir().unwrap();
        write_drive_name(dir.path(), "Club Stick").unwrap();
        let formats = UsbAudioFormats::from_request(&[
            ("mp3".to_string(), true),
            ("aiff".to_string(), true),
            ("wav".to_string(), false),
            ("flac".to_string(), false),
            ("alac".to_string(), false),
            ("aac".to_string(), true),
        ])
        .unwrap();
        write_usb_audio_formats(dir.path(), &formats).unwrap();
        assert_eq!(read_drive_name(dir.path()), Some("Club Stick".to_string()));
        write_drive_name(dir.path(), "Booth Stick").unwrap();
        assert_eq!(read_usb_audio_formats(dir.path()), formats);
        assert_eq!(read_drive_name(dir.path()), Some("Booth Stick".to_string()));
    }

    #[test]
    fn writes_keep_fields_this_version_does_not_know() {
        let dir = tempdir().unwrap();
        write_raw_marker(dir.path(), r#"{"name":"Stick","future":{"a":1}}"#);
        write_usb_audio_formats(dir.path(), &UsbAudioFormats::default()).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(marker_path(dir.path())).unwrap()).unwrap();
        assert_eq!(raw["future"]["a"], 1);
        assert_eq!(raw["name"], "Stick");
    }

    #[test]
    fn hand_edited_formats_are_normalized() {
        let dir = tempdir().unwrap();
        write_raw_marker(
            dir.path(),
            r#"{"formats":[{"format":"mp3","allowed":true},{"format":"ogg","allowed":true},
                {"format":"MP3","allowed":false},{"format":"flac","allowed":false}]}"#,
        );
        assert_eq!(
            ids(&read_usb_audio_formats(dir.path())),
            vec![
                ("mp3", true),
                ("flac", false),
                ("alac", true),
                ("aiff", true),
                ("wav", true),
                ("aac", true),
            ]
        );
        assert_eq!(read_drive_name(dir.path()), None);

        write_raw_marker(
            dir.path(),
            r#"{"formats":[{"format":"flac","allowed":false},{"format":"alac","allowed":false},
                {"format":"aiff","allowed":false},{"format":"wav","allowed":false},
                {"format":"mp3","allowed":false},{"format":"aac","allowed":false}]}"#,
        );
        assert_eq!(
            read_usb_audio_formats(dir.path()),
            UsbAudioFormats::default()
        );

        write_raw_marker(dir.path(), "not json");
        assert_eq!(
            read_usb_audio_formats(dir.path()),
            UsbAudioFormats::default()
        );
    }

    #[test]
    fn requested_formats_must_list_each_once_with_one_ticked() {
        let all = |allowed: bool| -> Vec<(String, bool)> {
            UsbAudioFormat::DEFAULT_ORDER
                .into_iter()
                .map(|format| (format.id().to_string(), allowed))
                .collect()
        };
        assert!(UsbAudioFormats::from_request(&all(true)).is_ok());
        assert_eq!(
            UsbAudioFormats::from_request(&all(false))
                .unwrap_err()
                .english(),
            "validation error: At least one format stays ticked"
        );
        let mut twice = all(true);
        twice[1].0 = "flac".to_string();
        assert!(UsbAudioFormats::from_request(&twice).is_err());
        let mut unknown = all(true);
        unknown.push(("ogg".to_string(), true));
        assert!(UsbAudioFormats::from_request(&unknown).is_err());
        assert!(UsbAudioFormats::from_request(&all(true)[1..]).is_err());
    }

    #[test]
    fn slug_sanitizes_and_collapses() {
        assert_eq!(slug("My USB Stick #1"), "my-usb-stick-1");
        assert_eq!(slug("  leading/trailing  "), "leading-trailing");
        assert_eq!(slug("已经"), "drive");
        assert_eq!(slug(""), "drive");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mount_from_mounts_content_picks_longest_matching_mount_point() {
        let mounts = "/dev/sda1 / ext4 rw 0 0\n\
             /dev/sdb1 /media/user/CLUBSTICK vfat rw 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n";
        let path = std::path::Path::new("/media/user/CLUBSTICK/PIONEER/rekordbox");
        assert_eq!(
            linux_mount_from_mounts_content(mounts, path),
            Some(LinuxMount {
                source: PathBuf::from("/dev/sdb1"),
                mount_point: PathBuf::from("/media/user/CLUBSTICK"),
                fs_type: "vfat".to_string(),
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mount_from_mounts_content_unescapes_octal_sequences() {
        let mounts = "/dev/sdb1 /media/user/My\\040Stick vfat rw 0 0\n";
        let path = std::path::Path::new("/media/user/My Stick/PIONEER");
        assert_eq!(
            linux_mount_from_mounts_content(mounts, path).map(|m| m.source),
            Some(PathBuf::from("/dev/sdb1"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mount_from_mounts_content_returns_none_when_no_mount_covers_path() {
        let mounts = "/dev/sda1 /home ext4 rw 0 0\n";
        let path = std::path::Path::new("/media/user/CLUBSTICK");
        assert_eq!(linux_mount_from_mounts_content(mounts, path), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_unescape_udev_label_decodes_hex_escapes() {
        assert_eq!(linux_unescape_udev_label("CLUB\\x20STICK"), "CLUB STICK");
        assert_eq!(linux_unescape_udev_label("PLAIN"), "PLAIN");
        assert_eq!(linux_unescape_udev_label("trailing\\x2"), "trailing\\x2");
    }

    fn test_db() -> (tempfile::TempDir, crate::db::Db) {
        let dir = tempdir().expect("tempdir");
        let db = crate::db::Db::new(dir.path()).expect("db init");
        (dir, db)
    }

    #[test]
    fn upsert_usb_device_inserts_then_updates_last_seen_without_duplicating() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        let id1 = upsert_usb_device(&conn, Path::new("/mnt/usb1"), false, "2026-01-01T00:00:00Z")
            .expect("first upsert");
        let id2 = upsert_usb_device(&conn, Path::new("/mnt/usb1"), false, "2026-01-02T00:00:00Z")
            .expect("second upsert");
        assert_eq!(id1, id2, "same root should resolve to the same device row");

        let count: i64 = conn
            .query_row("SELECT COUNT(1) FROM usb_devices", [], |row| row.get(0))
            .expect("count devices");
        assert_eq!(count, 1);

        let last_seen: String = conn
            .query_row(
                "SELECT last_seen_at FROM usb_devices WHERE id = ?1",
                params![id1],
                |row| row.get(0),
            )
            .expect("last_seen_at");
        assert_eq!(last_seen, "2026-01-02T00:00:00Z");
    }

    #[test]
    fn upsert_usb_device_mark_mounted_unmounts_every_other_device() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        let id_a = upsert_usb_device(&conn, Path::new("/mnt/usbA"), true, "2026-01-01T00:00:00Z")
            .expect("mount A");
        let mounted_a: i64 = conn
            .query_row(
                "SELECT mounted FROM usb_devices WHERE id = ?1",
                params![id_a],
                |row| row.get(0),
            )
            .expect("mounted A");
        assert_eq!(mounted_a, 1);

        let id_b = upsert_usb_device(&conn, Path::new("/mnt/usbB"), true, "2026-01-01T00:01:00Z")
            .expect("mount B");
        let mounted_a_after: i64 = conn
            .query_row(
                "SELECT mounted FROM usb_devices WHERE id = ?1",
                params![id_a],
                |row| row.get(0),
            )
            .expect("mounted A after");
        let mounted_b: i64 = conn
            .query_row(
                "SELECT mounted FROM usb_devices WHERE id = ?1",
                params![id_b],
                |row| row.get(0),
            )
            .expect("mounted B");
        assert_eq!(
            mounted_a_after, 0,
            "A should be unmounted once B is mounted"
        );
        assert_eq!(mounted_b, 1);
    }

    #[test]
    fn upsert_usb_device_without_mark_mounted_never_changes_mount_state() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        let id_a = upsert_usb_device(&conn, Path::new("/mnt/usbA"), true, "2026-01-01T00:00:00Z")
            .expect("mount A");
        upsert_usb_device(&conn, Path::new("/mnt/usbB"), false, "2026-01-01T00:01:00Z")
            .expect("touch B without mounting");
        let mounted_a: i64 = conn
            .query_row(
                "SELECT mounted FROM usb_devices WHERE id = ?1",
                params![id_a],
                |row| row.get(0),
            )
            .expect("mounted A");
        assert_eq!(
            mounted_a, 1,
            "browsing another root must not unmount the currently mounted device"
        );
    }

    #[test]
    fn upsert_usb_device_clears_deleted_at_on_any_touch() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        let id = upsert_usb_device(&conn, Path::new("/mnt/usb1"), false, "2026-01-01T00:00:00Z")
            .expect("first upsert");
        conn.execute(
            "UPDATE usb_devices SET deleted_at = ?1 WHERE id = ?2",
            params!["2026-01-02T00:00:00Z", id],
        )
        .expect("soft delete");
        upsert_usb_device(&conn, Path::new("/mnt/usb1"), false, "2026-01-03T00:00:00Z")
            .expect("re-touch");
        let deleted_at: Option<String> = conn
            .query_row(
                "SELECT deleted_at FROM usb_devices WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("deleted_at");
        assert_eq!(
            deleted_at, None,
            "re-picking the same root should un-prune it"
        );
    }

    #[test]
    fn all_usb_device_root_paths_includes_soft_deleted_devices() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        let id = upsert_usb_device(&conn, Path::new("/mnt/usb1"), false, "2026-01-01T00:00:00Z")
            .expect("upsert");
        conn.execute(
            "UPDATE usb_devices SET deleted_at = ?1 WHERE id = ?2",
            params!["2026-01-02T00:00:00Z", id],
        )
        .expect("soft delete");
        let paths = all_usb_device_root_paths(&conn).expect("root paths");
        assert!(paths.iter().any(|p| p == "/mnt/usb1"));
    }
}
