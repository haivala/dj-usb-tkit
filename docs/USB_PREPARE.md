# Preparing a USB Drive

How to partition and format a USB drive so DJ players can read it. The app does not format
drives itself; these are the manual steps for each OS.

Formatting erases everything on the drive, including the PDB, the eDB, the analysis files, the
backups in `PIONEER/rekordbox/backups/` and the app's drive name marker.

## Target layout

| Setting | Value | Why |
|---|---|---|
| Partition table | MBR (also called "msdos" or "Master Boot Record") | Players read MBR reliably; GPT is not supported everywhere |
| Partitions | One primary partition spanning the whole drive | Players only look at the first partition |
| Filesystem | FAT32 | Every Pioneer/AlphaTheta and Denon/Engine DJ player reads it |
| Volume label | Up to 11 characters, uppercase letters, digits, `_` | FAT label limits; the app suggests the label as the drive name |

exFAT is supported only by newer players. Check the player's manual before choosing it over
FAT32. FAT32's 4 GB per-file limit does not matter for audio files.

## Before you start

1. Back up the drive if anything on it matters. Copy the whole drive, not just `Contents/`, since
   the PDB, eDB and `PIONEER/` folder hold cues, grids and playlists.
2. Close the drive in the app, and in rekordbox or Engine DJ if they're open.
3. Unplug every other USB storage device. With only one stick attached, it's much harder to pick
   the wrong disk.
4. Identify the drive by **size and model**, not by its position in a list. Disk numbers and
   device names can change between plugs.
5. Always select the **whole disk** (`/dev/sdb`, `disk4`, `Disk 2`), not a partition or volume on
   it. The steps below replace the partition table.

## Linux

Needs root. Replace `sdX` with the stick's device name. Check it twice.

```sh
# 1. Find the stick: look for TRAN=usb, RM=1 and the expected size.
lsblk -o NAME,SIZE,TYPE,TRAN,RM,MODEL,FSTYPE,LABEL,MOUNTPOINTS

# 2. Unmount every partition on it (repeat for each mounted partition).
sudo umount /dev/sdX1

# 3. Remove old filesystem and partition-table signatures.
sudo wipefs -a /dev/sdX

# 4. New MBR table with one FAT32 (LBA) partition over the whole disk, 1 MiB aligned.
sudo parted -s /dev/sdX mklabel msdos mkpart primary fat32 1MiB 100% set 1 lba on

# 5. Create the filesystem.
sudo mkfs.fat -F 32 -n DJUSB /dev/sdX1

# 6. Verify: one partition, vfat, the expected label.
lsblk -o NAME,SIZE,FSTYPE,LABEL /dev/sdX
sudo sync
```

GNOME Disks or KDE Partition Manager do the same thing. Choose "MBR / DOS" as the partitioning
type and "FAT" as the filesystem.

## macOS

```sh
# 1. Find the stick: check the size and that it's listed as "external, physical".
diskutil list external physical

# 2. Erase the whole disk: MBR table, one FAT32 partition, label DJUSB.
#    Replace diskN with the stick's identifier (the disk, not diskNs1).
diskutil eraseDisk FAT32 DJUSB MBRFormat /dev/diskN

# 3. Verify.
diskutil list /dev/diskN
```

`diskutil` requires an uppercase label for FAT32.

In Disk Utility, choose **View → Show All Devices** first, then select the top-level device
(not the volume under it). Choose **Erase** with Format **MS-DOS (FAT)** and Scheme
**Master Boot Record**. Without "Show All Devices", Disk Utility erases only the volume and keeps
the existing partition table.

## Windows

Windows' built-in formatter offers FAT32 only up to 32 GB on many versions. Use `diskpart` for
small drives and a third-party formatter for larger ones.

### Drives up to 32 GB: diskpart

Run from an administrator Command Prompt or PowerShell:

```
diskpart
list disk
select disk N          <- the stick, matched by size; check twice
detail disk            <- confirm model and that it's USB
clean                  <- wipes the partition table
convert mbr
create partition primary
format fs=fat32 quick label=DJUSB
assign
exit
```

`clean` acts on the selected disk at once, without asking. Run `detail disk` and read its
output before `clean`.

### Drives over 32 GB: Rufus

1. Download Rufus from its official site, rufus.ie.
2. **Device**: the stick. Check the size.
3. **Boot selection**: Non bootable.
4. **Partition scheme**: MBR.
5. **File system**: FAT32 (Large FAT32 is used automatically for big drives).
6. **Volume label**: e.g. `DJUSB`. Then **Start**.

## After formatting

- Eject the drive safely and plug it back in to confirm it mounts.
- In the app, select the drive's root folder. The name marker was erased, so the app asks for a
  drive name again and suggests the volume label.
- The first export creates the PDB, eDB and `PIONEER/` structure. See `docs/USB_EXPORT.md`.
