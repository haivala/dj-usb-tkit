//! The selected USB's partition layout and filesystem, compared with what
//! `docs/USB_PREPARE.md` sets up: one MBR partition spanning the drive, FAT32.
//!
//! A stick that differs can work with little data on it and then make a player
//! ask to format it, which looks like the export's fault. Diagnostics names the
//! difference instead. There is no repair: the app doesn't format drives.
//!
//! Only a folder that is exactly the mount point of a USB/removable device is
//! looked at. A folder used as a USB, or a folder on a stick, gets no check.
//! On Linux everything comes from files any user can read (`/proc/mounts`,
//! udev's database, sysfs); elsewhere only the filesystem type is known.

use std::path::Path;

use crate::i18n::{Text, tr};

/// Filesystem size below this share of its partition counts as not filling it.
/// A FAT filesystem as made by `mkfs.fat` or `diskutil` fills its partition to
/// within a cluster or so.
const FS_FILL_MIN_RATIO: f64 = 0.9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FsKind {
    Fat32,
    /// FAT16 or FAT12, by its name.
    OlderFat(String),
    ExFat,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PartitionTable {
    Mbr,
    Gpt,
    /// The filesystem starts at the drive's first sector ("superfloppy").
    WholeDisk,
    Other(String),
}

/// What could be learned about the drive behind a USB root. `None` fields are
/// unknown on this system and never warned about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriveLayout {
    pub(crate) fs: FsKind,
    pub(crate) table: Option<PartitionTable>,
    /// 1-based, as the OS numbers partitions.
    pub(crate) partition_number: Option<u32>,
    pub(crate) partitions_on_disk: Option<usize>,
    pub(crate) fs_size: Option<u64>,
    pub(crate) partition_size: Option<u64>,
}

/// Each way `layout` differs from the prepared layout, as a sentence for the
/// report. Empty when it matches.
pub(crate) fn layout_problems(layout: &DriveLayout) -> Vec<Text> {
    let mut problems = Vec::new();
    match &layout.fs {
        FsKind::Fat32 => {}
        FsKind::OlderFat(version) => problems.push(tr::diagnostics_drive_older_fat(version)),
        FsKind::ExFat => problems.push(tr::diagnostics_drive_exfat()),
        FsKind::Other(name) => problems.push(tr::diagnostics_drive_other_fs(name)),
    }
    match &layout.table {
        None | Some(PartitionTable::Mbr) => {}
        Some(PartitionTable::Gpt) => problems.push(tr::diagnostics_drive_gpt()),
        Some(PartitionTable::WholeDisk) => problems.push(tr::diagnostics_drive_no_table()),
        Some(PartitionTable::Other(name)) => problems.push(tr::diagnostics_drive_other_table(name)),
    }
    match (layout.partition_number, layout.partitions_on_disk) {
        (Some(number), _) if number != 1 => problems.push(tr::diagnostics_drive_not_first(number)),
        (_, Some(count)) if count > 1 => {
            problems.push(tr::diagnostics_drive_more_partitions(count));
        }
        _ => {}
    }
    if let (Some(fs_size), Some(partition_size)) = (layout.fs_size, layout.partition_size)
        && (fs_size as f64) < partition_size as f64 * FS_FILL_MIN_RATIO
    {
        problems.push(tr::diagnostics_drive_fs_smaller(
            crate::utils::byte_size(fs_size),
            crate::utils::byte_size(partition_size),
        ));
    }
    problems
}

/// Names a filesystem as the OS reports it: `fs_type` is the mount's type
/// (`vfat`, `exfat`, `ntfs3`, Windows' `FAT32`, macOS' `msdos`), `fat_version`
/// a FAT filesystem's own version (`FAT32`, `FAT16`) when known.
pub(crate) fn classify_fs(fs_type: &str, fat_version: Option<&str>) -> FsKind {
    let fs = fs_type.trim().to_ascii_lowercase();
    match fs.as_str() {
        // macOS can't tell FAT16 from FAT32 by name; assume FAT32 rather than warn.
        "vfat" | "msdos" | "fat32" => match fat_version.map(|v| v.trim().to_ascii_uppercase()) {
            Some(v) if !v.is_empty() && v != "FAT32" => FsKind::OlderFat(v),
            _ => FsKind::Fat32,
        },
        // Windows names FAT16 and FAT12 just "FAT".
        "fat" => FsKind::OlderFat("FAT16".into()),
        "fat16" | "fat12" => FsKind::OlderFat(fs.to_ascii_uppercase()),
        "exfat" => FsKind::ExFat,
        _ => FsKind::Other(fs_type.trim().to_string()),
    }
}

/// The layout of the drive mounted exactly at `usb_root`, when that is a USB
/// or removable device. `None` for anything else.
pub(crate) fn read_drive_layout(usb_root: &Path) -> Option<DriveLayout> {
    platform::read_drive_layout(usb_root)
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::HashMap;
    use std::path::Path;

    use super::{DriveLayout, PartitionTable, classify_fs};
    use crate::service::usb::identity::{linux_device_is_usb, linux_mount_for_path};

    /// sysfs counts block device sizes in 512-byte sectors, whatever the
    /// device's own sector size.
    const SYSFS_SECTOR: u64 = 512;

    pub(super) fn read_drive_layout(usb_root: &Path) -> Option<DriveLayout> {
        let root = std::fs::canonicalize(usb_root).ok()?;
        let mount = linux_mount_for_path(&root)?;
        if mount.mount_point != root || !linux_device_is_usb(&mount.source) {
            return None;
        }
        let device = std::fs::canonicalize(&mount.source).ok()?;
        let name = device.file_name()?.to_str()?;
        let sysfs = Path::new("/sys/class/block").join(name);
        let dev_number = std::fs::read_to_string(sysfs.join("dev")).ok()?;
        let udev = std::fs::read_to_string(format!("/run/udev/data/b{}", dev_number.trim()))
            .map(|content| udev_properties(&content))
            .unwrap_or_default();
        let read_number = |file: &str| -> Option<u64> {
            std::fs::read_to_string(sysfs.join(file))
                .ok()?
                .trim()
                .parse()
                .ok()
        };

        // ntfs-3g and exfat-fuse mount as `fuseblk`; udev saw what's inside.
        let fs_type = if mount.fs_type == "fuseblk" {
            udev.get("ID_FS_TYPE").map_or("fuseblk", String::as_str)
        } else {
            mount.fs_type.as_str()
        };
        let fs = classify_fs(fs_type, udev.get("ID_FS_VERSION").map(String::as_str));

        let partition_number = read_number("partition").and_then(|n| u32::try_from(n).ok());
        let (table, partitions_on_disk) = if partition_number.is_some() {
            let scheme = udev
                .get("ID_PART_ENTRY_SCHEME")
                .or_else(|| udev.get("ID_PART_TABLE_TYPE"));
            let disk = std::fs::canonicalize(&sysfs)
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf));
            (
                scheme.map(|s| partition_table(s)),
                disk.and_then(|d| count_partitions(&d)),
            )
        } else {
            (Some(PartitionTable::WholeDisk), None)
        };

        let partition_size = read_number("size").map(|sectors| sectors * SYSFS_SECTOR);
        let fs_size = udev
            .get("ID_FS_SIZE")
            .and_then(|s| s.parse().ok())
            .or_else(|| statvfs_total(&root));

        Some(DriveLayout {
            fs,
            table,
            partition_number,
            partitions_on_disk,
            fs_size,
            partition_size,
        })
    }

    /// The `E:KEY=value` lines of a udev database entry.
    pub(super) fn udev_properties(content: &str) -> HashMap<String, String> {
        content
            .lines()
            .filter_map(|line| line.strip_prefix("E:")?.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    pub(super) fn partition_table(scheme: &str) -> PartitionTable {
        match scheme {
            "dos" => PartitionTable::Mbr,
            "gpt" => PartitionTable::Gpt,
            other => PartitionTable::Other(other.to_string()),
        }
    }

    /// Partitions of the disk at sysfs path `disk`: its child block devices,
    /// each with a `partition` file.
    fn count_partitions(disk: &Path) -> Option<usize> {
        let entries = std::fs::read_dir(disk).ok()?;
        Some(
            entries
                .flatten()
                .filter(|entry| entry.path().join("partition").is_file())
                .count(),
        )
    }

    /// The filesystem's size as `statvfs` reports it, for a udev without
    /// `ID_FS_SIZE`. It leaves out the FAT's own tables, a small share.
    #[allow(clippy::unnecessary_cast)]
    fn statvfs_total(path: &Path) -> Option<u64> {
        let stats = crate::service::usb::utils::statvfs(path)?;
        Some(stats.f_blocks as u64 * stats.f_frsize as u64)
    }
}

/// Windows/macOS: `sysinfo`'s disk list tells the filesystem type of a
/// removable disk, not its partition table or partitions.
#[cfg(not(target_os = "linux"))]
mod platform {
    use std::path::Path;

    use super::{DriveLayout, classify_fs};

    pub(super) fn read_drive_layout(usb_root: &Path) -> Option<DriveLayout> {
        let canon = std::fs::canonicalize(usb_root).ok();
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let disk = disks.list().iter().find(|disk| {
            let mount = disk.mount_point();
            mount == usb_root || canon.as_deref().is_some_and(|c| c == mount)
        })?;
        if !disk.is_removable() {
            return None;
        }
        Some(DriveLayout {
            fs: classify_fs(&disk.file_system().to_string_lossy(), None),
            table: None,
            partition_number: None,
            partitions_on_disk: None,
            fs_size: None,
            partition_size: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn prepared() -> DriveLayout {
        DriveLayout {
            fs: FsKind::Fat32,
            table: Some(PartitionTable::Mbr),
            partition_number: Some(1),
            partitions_on_disk: Some(1),
            fs_size: Some(32 * GIB - 1024 * 1024),
            partition_size: Some(32 * GIB),
        }
    }

    fn keys(layout: &DriveLayout) -> Vec<&'static str> {
        layout_problems(layout).iter().map(Text::key).collect()
    }

    #[test]
    fn the_prepared_layout_has_no_problems() {
        assert!(keys(&prepared()).is_empty());
    }

    #[test]
    fn unknown_facts_are_not_warned_about() {
        let layout = DriveLayout {
            table: None,
            partition_number: None,
            partitions_on_disk: None,
            fs_size: None,
            partition_size: None,
            ..prepared()
        };
        assert!(keys(&layout).is_empty());
    }

    #[test]
    fn each_difference_is_named() {
        let layout = DriveLayout {
            fs: FsKind::ExFat,
            table: Some(PartitionTable::Gpt),
            partition_number: Some(2),
            partitions_on_disk: Some(2),
            fs_size: Some(8 * GIB),
            partition_size: Some(32 * GIB),
        };
        assert_eq!(
            keys(&layout),
            [
                tr::diagnostics_drive_exfat().key(),
                tr::diagnostics_drive_gpt().key(),
                tr::diagnostics_drive_not_first(2).key(),
                tr::diagnostics_drive_fs_smaller("", "").key(),
            ]
        );
    }

    #[test]
    fn extra_partitions_after_the_first_are_named() {
        let layout = DriveLayout {
            partitions_on_disk: Some(3),
            ..prepared()
        };
        assert_eq!(
            keys(&layout),
            [tr::diagnostics_drive_more_partitions(3).key()]
        );
    }

    #[test]
    fn a_filesystem_on_the_whole_disk_is_named() {
        let layout = DriveLayout {
            table: Some(PartitionTable::WholeDisk),
            partition_number: None,
            partitions_on_disk: None,
            ..prepared()
        };
        assert_eq!(keys(&layout), [tr::diagnostics_drive_no_table().key()]);
    }

    #[test]
    fn filesystems_are_classified_as_each_os_names_them() {
        assert_eq!(classify_fs("vfat", Some("FAT32")), FsKind::Fat32);
        assert_eq!(
            classify_fs("vfat", Some("FAT16")),
            FsKind::OlderFat("FAT16".into())
        );
        // A udev without ID_FS_VERSION: assume FAT32 rather than warn.
        assert_eq!(classify_fs("vfat", None), FsKind::Fat32);
        assert_eq!(classify_fs("FAT32", None), FsKind::Fat32);
        assert_eq!(classify_fs("msdos", None), FsKind::Fat32);
        assert_eq!(classify_fs("FAT", None), FsKind::OlderFat("FAT16".into()));
        assert_eq!(classify_fs("exfat", None), FsKind::ExFat);
        assert_eq!(classify_fs("exFAT", None), FsKind::ExFat);
        assert_eq!(classify_fs("ntfs3", None), FsKind::Other("ntfs3".into()));
        assert_eq!(classify_fs("NTFS", None), FsKind::Other("NTFS".into()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn udev_properties_and_partition_tables_are_read() {
        let content = "S:disk/by-id/usb-Stick-0:0-part1\n\
                       E:ID_FS_VERSION=FAT32\n\
                       E:ID_FS_SIZE=31914983424\n\
                       E:ID_PART_ENTRY_SCHEME=dos\n\
                       G:systemd\n";
        let props = platform::udev_properties(content);
        assert_eq!(
            props.get("ID_FS_VERSION").map(String::as_str),
            Some("FAT32")
        );
        assert_eq!(
            props.get("ID_FS_SIZE").map(String::as_str),
            Some("31914983424")
        );
        assert_eq!(props.len(), 3);
        assert_eq!(platform::partition_table("dos"), PartitionTable::Mbr);
        assert_eq!(platform::partition_table("gpt"), PartitionTable::Gpt);
        assert_eq!(
            platform::partition_table("atari"),
            PartitionTable::Other("atari".into())
        );
    }

    #[test]
    fn a_folder_that_is_not_a_mount_point_is_not_checked() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_drive_layout(dir.path()), None);
    }
}
