//! User-assigned identity for a USB root: a name, stored in a marker file on
//! the root itself (so it survives being plugged into a different computer)
//! and mirrored into the local `usb_devices.label` column (display cache for
//! a drive that isn't plugged in) and the `usb_drive_names` registry (the
//! uniqueness index -- not keyed by mount path, since drives with the same
//! volume label usually mount at the same path).
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

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::audio_format::UsbAudioFormat;

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
    pub(crate) fn from_request(items: &[(String, bool)]) -> Result<Self, String> {
        let mut entries: Vec<(UsbAudioFormat, bool)> = Vec::new();
        for (id, allowed) in items {
            let format =
                UsbAudioFormat::parse(id).ok_or_else(|| format!("Unknown audio format: {id}"))?;
            if entries.iter().any(|&(entry, _)| entry == format) {
                return Err(format!("{} is listed twice", format.label()));
            }
            entries.push((format, *allowed));
        }
        if let Some(missing) = UsbAudioFormat::DEFAULT_ORDER
            .into_iter()
            .find(|format| !entries.iter().any(|&(entry, _)| entry == *format))
        {
            return Err(format!("{} is missing from the list", missing.label()));
        }
        if !entries.iter().any(|&(_, allowed)| allowed) {
            return Err("At least one format stays ticked".to_string());
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
    super::normalize_source_root_for_matching(&root.to_string_lossy())
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
pub(crate) fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Enter a name for this drive.".to_string());
    }
    if trimmed.chars().count() > MAX_NAME_LEN {
        return Err(format!(
            "Drive name must be {MAX_NAME_LEN} characters or fewer"
        ));
    }
    Ok(trimmed.to_string())
}

/// Write `name` to the marker file at `root`, creating the marker directory
/// if needed. Caller is responsible for uniqueness checks against other
/// known drives (see `usb_utils::find_registered_drive`).
pub(crate) fn write_drive_name(root: &Path, name: &str) -> std::io::Result<()> {
    let validated = validate_name(name).map_err(std::io::Error::other)?;
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

/// Finds the mount source (e.g. `/dev/sdb1`) covering `path`, by picking the
/// `/proc/mounts` entry whose mount point is the longest matching prefix of
/// `path` -- the same "most specific mount wins" rule `df`/`lsblk` use.
#[cfg(target_os = "linux")]
fn linux_mount_source_for_path(path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string("/proc/mounts").ok()?;
    linux_mount_source_from_mounts_content(&content, path)
}

#[cfg(target_os = "linux")]
fn linux_mount_source_from_mounts_content(content: &str, path: &Path) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, PathBuf)> = None;
    for line in content.lines() {
        let mut fields = line.split_whitespace();
        let (Some(fs_spec), Some(fs_file)) = (fields.next(), fields.next()) else {
            continue;
        };
        let mount_point = PathBuf::from(linux_unescape_mount_field(fs_file));
        if !path.starts_with(&mount_point) {
            continue;
        }
        let is_longer = best
            .as_ref()
            .is_none_or(|(mp, _)| mount_point.as_os_str().len() > mp.as_os_str().len());
        if is_longer {
            best = Some((
                mount_point,
                PathBuf::from(linux_unescape_mount_field(fs_spec)),
            ));
        }
    }
    best.map(|(_, dev)| dev)
}

/// Whether `device` is backed by USB (has a `usb-*` entry under
/// `/dev/disk/by-id/` resolving to it) -- the same technique the `sysinfo`
/// crate uses for its own `Disk::is_removable()`, reimplemented here
/// directly since enabling sysinfo's `disk` feature would pull in
/// Windows/macOS-only dependencies this codebase has no other use for.
#[cfg(target_os = "linux")]
fn linux_device_is_usb(device: &Path) -> bool {
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
            UsbAudioFormats::from_request(&all(false)).unwrap_err(),
            "At least one format stays ticked"
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
    fn linux_mount_source_from_mounts_content_picks_longest_matching_mount_point() {
        let mounts = "/dev/sda1 / ext4 rw 0 0\n\
             /dev/sdb1 /media/user/CLUBSTICK vfat rw 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n";
        let path = std::path::Path::new("/media/user/CLUBSTICK/PIONEER/rekordbox");
        assert_eq!(
            linux_mount_source_from_mounts_content(mounts, path),
            Some(std::path::PathBuf::from("/dev/sdb1"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mount_source_from_mounts_content_unescapes_octal_sequences() {
        let mounts = "/dev/sdb1 /media/user/My\\040Stick vfat rw 0 0\n";
        let path = std::path::Path::new("/media/user/My Stick/PIONEER");
        assert_eq!(
            linux_mount_source_from_mounts_content(mounts, path),
            Some(std::path::PathBuf::from("/dev/sdb1"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mount_source_from_mounts_content_returns_none_when_no_mount_covers_path() {
        let mounts = "/dev/sda1 /home ext4 rw 0 0\n";
        let path = std::path::Path::new("/media/user/CLUBSTICK");
        assert_eq!(linux_mount_source_from_mounts_content(mounts, path), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_unescape_udev_label_decodes_hex_escapes() {
        assert_eq!(linux_unescape_udev_label("CLUB\\x20STICK"), "CLUB STICK");
        assert_eq!(linux_unescape_udev_label("PLAIN"), "PLAIN");
        assert_eq!(linux_unescape_udev_label("trailing\\x2"), "trailing\\x2");
    }
}
