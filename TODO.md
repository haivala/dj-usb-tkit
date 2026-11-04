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

## Hardware: does the CDJ-3000X use eDB play history?

The eDB has its own play history (`history`, `history_content`), but no player
tested so far has been seen using it. Test on a CDJ-3000X: play a few tracks from an exported
USB, then check whether `history`/`history_content` in `exportLibrary.db`
gained rows.

If it does, the mirror export's stale-file prune must protect tracks in eDB
history too. Today `filter_prunable_stale_paths_for_playlist`
(`backend/src/service/export_helpers/export_paths.rs`) keeps the files of
tracks in PDB history but reads only `playlist_content` from the eDB, so a
track only in eDB history and in no playlist could lose its audio, analysis
and artwork files. "Remove Missing Audio References" already counts both
histories.

## Possibility: copy cues added on a CDJ back to the library

Cues added on a CDJ don't reach a library track that already has cues:
`import_anlz_cues_for_track` (`backend/src/service/cues.rs`) skips those. The
next export then replaces the stick's cues with the library's
(`ensure_analysis_bundle_ppth` in
`backend/src/service/export_helpers/export_paths.rs`), so the CDJ's cues are
lost. Found by reading the code, not yet seen on a CDJ.

Idea: when a track's cues on the stick have changed since the last export,
replace the library track's cues with the stick's, all of them, with no
cue-by-cue merge. CDJ removals then carry over too.

- Export saves what it wrote for each track on that stick. The library has no
  `cueUpdateCount` of its own, and the app's count and rekordbox's can differ,
  so the stick is compared with the saved value.
- Ask when the track was also edited in the library since the last export: a
  rewrite would lose those edits, and "local edits win" is the rule today.
- The import's time shift for transcoded files still applies.

First test on a CDJ: export a track, add a hot cue and a memory cue on the
player, and compare `exportLibrary.db`, `export.pdb` and the track's `.DAT` and
`.EXT` before and after. If `cueUpdateCount` changes, it can trigger the rewrite.
If it doesn't, or only on some models, compare the stick's ANLZ cue list with
the list the last export wrote instead.

## Check: app timing of MP3s with a LAME header frame

The app's analysis (waveform, beat grid, cues) decodes an MP3 without the
Info/Xing header frame that most MP3 encoders write first. rekordbox played
the header frame of the app's earlier MP3s (tagged `LAME4.0`) as one more
frame of audio, but skipped the one ffmpeg writes (tagged `Lavc`); what it
goes by is unknown, and no MP3 from the LAME encoder itself (`LAME3.100`), the
usual kind in a library, has been checked. If rekordbox plays those header
frames too, the app's grids and cues on such MP3s sit 1152 samples (about
26 ms) early on players.

To settle it without a new rekordbox export: on a stick rekordbox has
analyzed, compare the app's waveform of its LAME MP3s with rekordbox's `PWV3`
(cross-correlation; beat grids are too noisy to tell). Transcodes the app
makes are not affected: they are written without a header frame, and their
timing was checked against rekordbox.
