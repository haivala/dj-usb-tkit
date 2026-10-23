# Connecting a USB

Everything the app does with a USB starts from the one you've selected. This page covers how to
select one, what happens when you do, and what changes in the app while one is selected.

The app doesn't notice a stick being plugged in or pulled out. A USB counts as connected once
you've selected it, and stays connected until you select another one or quit the app.

## Selecting a USB

Open **USB Browser** in the sidebar and click **Select USB Folder**. Pick the top folder of the
stick: the one that holds (or will hold) the `PIONEER` folder. Any folder works, so a folder on
your hard drive can stand in for a stick, which is handy for testing.

Every USB you've selected before is listed under **Recent USBs**. Click one to select it again.
The list closes once a USB is selected, and clicking its heading opens it again. The **×** next to
an entry removes it from the list, which also frees its name for another drive.

The app remembers the last USB between sessions, but doesn't connect to it by itself on start:
select it again, from **Recent USBs**, once it's plugged in.

## What happens when you select one

1. **The folder is checked.** A folder without the player databases offers **Initialize USB
   Structure**, which creates an empty library the players can read. A stick that is mounted
   read-only can be browsed but not exported to.
2. **The drive's name is read.** The first time a drive is selected, the app asks for a name,
   pre-filled with the stick's volume label when it has one. Give every drive its own name, even
   sticks that share a volume label: the name is how the app tells drives apart, and it keeps
   each drive's backups and local working copy separate. **Not now** leaves the drive unnamed.
3. **Copies are recognised.** A stick or folder copied from a named drive carries that drive's
   name. Once the copy is a different size from the original, or the original has had an export
   the copy doesn't have, the app asks for a name of its own. Until it gets one, it's treated as
   unnamed, so nothing is shared with the original.
4. **The databases are staged.** The stick's player databases are copied to your computer, or
   the copy from last time is reused if the stick hasn't changed since. See
   [Staging](#staging-the-local-working-copy) below.
5. **Diagnostics run.** The result shows as the dot next to the drive's name in the header:
   good, warnings, or issues found (hover it to see which). **Health & Diagnostics** in the USB
   Browser has the details ([DIAGNOSTICS_REPAIRS.md](DIAGNOSTICS_REPAIRS.md)).

## Staging: the local working copy

The app never works on the stick's player databases directly. Selecting a USB copies them to your
computer, and everything the app reads from or changes in those databases uses the copy:
browsing playlists and history, diagnostics, the export estimate, exports, repairs and edits.
Reading a stick is slow, so this keeps the app quick: the databases are read from the stick once
per connect. Music, analysis and artwork files are still read and written on the stick itself.

- **Changes are written back in one go.** An export, a repair or an edit changes the copy first.
  At the end, the copy is written back to the stick, and only if something actually changed. The
  stick's file is replaced in one step, so pulling the stick or a crash in the middle never leaves
  a half-written database on it. Still wait for the footer to say the export has finished before
  pulling the stick: the music files are copied straight to the stick.
- **Changes made elsewhere need a reconnect.** While a USB is selected, the app doesn't look at
  the stick's databases again, and it doesn't notice the stick being pulled out. So when you take
  the stick to a player (which records history on it), or change it with rekordbox or the app on
  another computer, select it again in **Recent USBs** once it's back. If you don't, the app's
  next write is refused instead of overwriting those changes, with an error asking you to re-open
  the USB and try again.
- **Each drive has its own copy**, kept under its name, so it's reused whichever port or computer
  path the stick gets. A drive that isn't named yet, or a copy of a named drive that hasn't got
  its own name, gets one of its own too. Renaming a drive moves its copy along with it.
- **Where it is:** the `usb_cache` folder in the app's data folder (on Linux,
  `~/.local/share/art.chiph.djusbtkit/usb_cache`). The Event Log shows the exact folder the first
  time it's created for a drive. A named drive's older backups are moved there too: only the
  newest one stays on the stick ([USB_EXPORT.md](USB_EXPORT.md), "Backup").

The technical details are under "Local HDD staging" in [USB_EXPORT.md](USB_EXPORT.md).

## While a USB is connected

- **Header:** shows the drive's name ("Unnamed" for a drive without one). Click the name to
  rename the drive, or to name it. Its backups and local working copy move with it.
- **USB views:** **Playlists**, **History** and **Player Menu** appear under **USB Browser** in
  the sidebar, and **Import Playlists**, **Import History** and **Backups** work
  ([USB_IMPORT.md](USB_IMPORT.md)).
- **Sidebar playlists:** the checkmark next to a playlist means "on this USB": the USB has a
  playlist with that name.
- **Playlist page:**
  - The line under the title starts with "On this USB." or "Not on this USB.", followed by when
    the playlist was last exported and to which drive.
  - The title row shows how many tracks and how much data an export would still copy, and how
    much space is free on the USB. It says "Doesn't fit" when the export won't fit. Tracks
    already on the USB aren't counted, and working this out doesn't read the USB.
  - The export button names the drive: **Export to USB: \<name\>**. In additive sync mode, when
    the USB already has a playlist with the same name, it reads **Append to (\<playlist\>) on USB**
    instead, and the playlist's tracks can't be reordered by dragging, since an append keeps the
    order already on the USB ([PLAYLISTS_PLAYBACK.md](PLAYLISTS_PLAYBACK.md)).
- **Track editing:** cues, beat grids and tags can be edited straight on the USB's tracks
  ([CUE_EDITOR.md](CUE_EDITOR.md)).

## While the app is working on the USB

During an export, a playlist or history import, diagnostics or repairs, or a save to the USB's
tracks, the selected USB can't change:

- **Select USB Folder** and the **Recent USBs** entries are disabled, and the drive can't be
  renamed.
- **Export is disabled from the moment you click it until the export has finished**, so a second
  click never starts a second export. Another export, or one started while a save to the USB's
  tracks is still running, is refused.
- Saving edits to the USB's tracks waits for the running job to finish instead of being dropped.

The footer shows the progress. At the end of an export it says when it's waiting for the stick
to finish writing the copied files, which can take a while on slow sticks.

## Without a USB

- The sidebar checkmark means the playlist has been exported at some point.
- The line under a playlist's title says when it was last exported and to which drive, named as
  the drive was the last time it was connected.
- The export button reads **Select USB first** and takes you to the USB Browser.
- The USB views under **USB Browser** are hidden.
