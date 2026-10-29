# TODO

## Monitor: track rows outside the media library

The library database can collect track rows that no longer belong to the
media library. Every library listing reads past them, so they slow down each
page, search and select-all as they pile up. Known sources:

- tracks under a source folder that was removed before removing a folder also
  removed its tracks;
- USB placeholder rows (`materialize_usb_track_row`) for USB tracks that
  matched no library file, including some stored with a path relative to the
  USB root instead of the mount point;
- tracks outside every source folder whose files still exist.

Watch whether rows outside every source folder keep growing with normal use
(USB imports, history, folder changes). If they do, the app should prune them
itself: a row outside every source folder, not from the Mixxx or rekordbox
library, and in no playlist or history goes. Tracks whose files are in the
media library stay, also when they are on a USB too.

## Hardware check: t19 activation on older players

The t19 activation (export, full rewrite and `repair_pdb_t19_template_state`)
is verified in rekordbox desktop but not yet on an older player, which is the
player that showed 0 tracks. Test a repaired stick and a fresh
app-initialized export: the stick mounts without a mount/eject loop, the track
count and playlists show, tracks load.

If tracks still don't show, the remaining differences from rekordbox's output
are on the t19 sentinel (page 39): its `u5`/`num_rl` and the B-tree entry's low
bits (`0x140` vs rekordbox's `0x143`). Some rekordbox exports match ours there.

## t19 row counter (bytes 4..8)

Each t19 row has a counter at bytes `4..8`. On rekordbox exports it follows the
library's track count (one row per step, and the 96-row page wraps around like
a ring). The app's activation always writes 1. If an older player shows 1 song
instead of the real count, write the track count here, the way rekordbox does.

## Drive name in the PDB and on rename

The drive name now goes into the eDB (`property.deviceName`) on every eDB
write. Still open:

- rekordbox also appends t19 rows carrying the name (the short string at row
  offset `0x1e`, empty `0x03` otherwise). The app doesn't write them.
- Renaming the drive in the app updates the eDB only on the next export.

## Startup: parallel commands, maybe a boot payload

`initApp` (`vanilla-ui/startup_bootstrap.mjs`) awaits its startup commands one
after another, though they don't depend on each other: the frontend settings,
the USB devices, the library sources, and the rekordbox and Mixxx detection.
If startup feels slow, start the independent ones together (`Promise.all`)
and draw once they're all in. The window stays hidden until `show_window`, so
only the total wait matters.

Further option: a Tauri init script (`WebviewWindowBuilder::initialization_script`
in `desktop/src-tauri/src/main.rs`, which then builds the window instead of
`tauri.conf.json`) carrying the startup state (settings, theme, sources) so the
first render needs no command. Costs: the payload is a snapshot taken when
the window is built (stale after a webview reload, so the commands stay as the
fallback), its DB reads and folder checks delay creating the window, and e2e
stubs would set the boot global as well as answer commands. Worth it at most
for the theme (no wrong-colour first paint); app data stays behind commands.
The Sources bar starting folded and opening once its chips are drawn doesn't
depend on either.
