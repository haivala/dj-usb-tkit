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
