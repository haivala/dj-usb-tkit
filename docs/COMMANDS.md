# Commands

This document defines the backend command surface used by the desktop app.

For database field-level details used by USB import/export commands, see `docs/PDB.md` and `docs/eDB.md`.

## Request/response envelope

Successful responses follow:

```json
{
  "ok": true,
  "data": {}
}
```

Failure responses follow:

```json
{
  "ok": false,
  "error": {
    "code": "ERROR_CODE",
    "message": "Human-readable summary",
    "details": {}
  }
}
```

Text a response carries (`statusText`, `message`, labels and tooltips) is in the
app's current language, from the text catalog (`backend/locales/<language>.ftl`);
the log file gets the same lines in English. `code`, `validationType` and other
ids never change with the language.

### Paginated track lists

`browse_source_files`, `get_playlist_tracks` and the USB per-list track
commands (`fetch_usb_playlist_tracks`, `fetch_usb_history_tracks`) share a
pagination shape: the request carries `query`, `sortBy` (`title` | `artist` |
`album` | `format` | `bpm` | `durationMs` | `key`; absent ⇒ natural order),
`sortDir` (`asc` | `desc`), `offset` or `cursor`, and `limit` (`0` ⇒
unpaginated). The response carries `items`, `total` (filtered count),
`nextCursor`, `hasMore`, plus any whole-set aggregates (`totalDurationMs`,
`durationUnknownCount`, and `durationText`, the footer's "Total time: 1:02:03
(2 without length)"). Filtering and sorting are server-side so they span the
entire list, not just the loaded page.

`offset` is a position in the filtered, sorted list and takes precedence over
`cursor`; the track tables use it to load whichever part of a list is on
screen. A `cursor` is bound by signature to its exact
`query`/`sortBy`/`sortDir`/scope and is rejected with a validation error if any
of those changed.

## Event model

Long-running commands emit job lifecycle events:

- `job.started`
- `job.progress`
- `job.completed`
- `job.failed`

The desktop host also emits a unified `job:event` channel carrying the same payload shape.

Analysis (`analyze_new_tracks`) reports each track on `job.progress`: a
`trackStarted` event when a worker picks it up, then pieces in the row's
column order (artwork, waveform, length, BPM / key) as each is found, then a
`trackReady` event. The pieces carry only their own fields; the `trackReady`
event carries `track`, the track's whole library row as saved (or as it was,
when the analysis failed), which the frontend shows in place of its copy, and
`libraryDurationText`, the library footer's Total time with that track's length. A
failed track's event has `failed: true` and `failedText`, its Event Log entry
("Track analysis failed: Title (id) - why").

### Whole track rows

A command that changes tracks answers with their whole rows, in the shape the
track lists show them: `Track` for library tracks (as `browse_source_files`,
with the waveform preview) and `UsbTrack` for a USB file (as
`fetch_usb_playlist_tracks`, read back from the stick). The frontend's
`showTrackRow` puts each one in place of every loaded copy of that track (a
USB file by its `usbMediaPath`, in every USB playlist and the history) and
redraws those rows; it merges nothing from the old copy.

Rows carry their display text ready to show: `formatCompat.badge` and
`formatCompat.tooltip` for the FMT column (the file's details, then the
compatibility warning), `bpmSourceText` for the BPM pill's tooltip (also on the
analysis' BPM piece), and "Unknown Title" / "Unknown Artist" for a file with
none.

A listed file not yet in the library has its path as its row id. When playing
or adding it gives it a library row, `resolve_track_identity` and each add
resolution carry that row as `track` (only then), and `showTrackRow` shows it
in place of the row whose id was the path.

USB-scoped jobs lock the selected USB root in the frontend until the matching
`job.completed` or `job.failed` event arrives. Locking job types are
`usb_read`, `usb_write`, `diagnostics`, and `export`; this covers playlist and
history reads, player-menu reads/writes, initialization, export, parity,
diagnostics, and repair. The lock prevents a late response from an earlier
drive selection from being rendered against a newer selected drive.

## Command groups

### Library

- `scan_library` — `{ sourceRoots?, incremental }` → `{ indexed, updated,
  removed, warnings, statusText }`; `statusText` is the status line
  after adding folders (the folder count and the tracks indexed).
- `scan_rekordbox_db`
- `scan_mixxx_db`
- `browse_source_files` — lists the library as its sources are set (below);
  the request carries no folder or library filter. The page's rows carry
  their waveforms (`waveformPreview`, or `waveformColorData` for rekordbox
  tracks), read for that page only, as a USB page's are.
- `get_library_sources` → `{ folders, rekordboxDbEnabled, mixxxDbEnabled,
  everConfigured, filterStatusText, filtering }`, each folder `{ path, enabled,
  missing, fullyAnalyzed }`. `filterStatusText` is the status line after a switch
  ("Source filters: 2/3 enabled"); `filtering` is on while some source's tracks
  are hidden (a folder off or missing, an imported library off), which the
  library's filter mark shows.
  The backend stores the source folders and their switches; every library
  command (`browse_source_files`, `list_matching_track_ids`,
  `get_selection_duration`, `add_library_selection_to_playlist(s)`,
  `analyze_new_tracks`, and `scan_library` without `sourceRoots`) uses the
  switched-on folders that are present plus the switched-on imported
  libraries. A library whose database is no longer found is switched off.
- `add_source_folders` — `{ paths }` → `{ sources, added, statusText }`;
  `added` are the paths not already configured (switched on), indexed in the
  same command (indexing only, no analysis; reported as a `scan` job).
  `statusText` is that scan's line, or the folder count when nothing was
  added. If the indexing fails the folders stay added and `statusText` says
  so; the next scan indexes them.
- `remove_source_folder` — `{ path }` → `{ sources, removed, statusText }`;
  removes the folder and its tracks, keeping tracks inside another source
  folder.
- `set_source_folder_enabled` — `{ path, enabled }` → the sources.
- `set_external_library_enabled` — `{ library: "rekordbox" | "mixxx",
  enabled }` → the sources.
- `resolve_track_identity` — `{ trackId, resolvedBy, materialized, track? }`;
  `track` is the new library row when a listed file was materialized.
- `relocate_source_root` — `{ oldRoot, newRoot }` → the rewrite counts
  (`unresolved` = `missingAtNewRoot` + `conflicts`) plus `sources` and
  `statusText`: once every track moved, the new folder replaces the old one;
  otherwise both stay. Then it indexes the new folder (indexing only, reported
  as a `scan` job), and `statusText` adds the files that indexed, or that the
  indexing failed.
- `get_track_detail` — returns `{ track, firstBeatMs, cues, detailWaveform,
  keyOptions, maxHotCues, genre, rating, releaseDate, genreOptions }` for the cue
  editor (see `docs/CUE_EDITOR.md`). `maxHotCues` is the editor's hot-cue slots
  (8, the save's limit); `rating` is 0–5 stars; `releaseDate` is the date
  export writes to the PDB and the eDB (the track's date, completed from its
  file timestamp as export does); `genreOptions` is every library genre,
  sorted ignoring case. `detailWaveform` is
  the **base64** of the raw PWV5 colour-detail waveform payload from the `.EXT`
  (2 bytes/entry, BE u16; absent when the track has no analysis). Cues are
  ordered by `sortOrder`.
- `save_track_analysis_edits` — `{ trackId, firstBeatMs?, cues?, bpm?, key? }` →
  atomically replaces the track's cue list (`cues: Some([...])`, max 8 hot cues)
  and/or updates the beat-grid anchor (`firstBeatMs: Some(ms)`), BPM and key,
  rewrites the cached local ANLZ bundle, and clears `last_exported_*` on every
  playlist containing the track. A `null` field is left unchanged. `cues[]`
  entries are `{ positionMs, colorId?, name?, playbackStart?, memory? }`; each
  hot cue becomes a memory point + a hot-cue pad on export. `memory: true`
  marks a memory cue: a memory point only, with an optional name and colour
  (no default colour), any number of them, outside the 8; one at a hot cue's
  or the start cue's position is dropped. At most one entry may have
  `playbackStart: true`: the memory-only playback-start cue (no pad, colour or
  name, never after the first hot cue, dropped without another cue, hot or
  memory; outside the 8). Returns the saved values, `track`, the track's
  whole row, and `statusText` ("Saved 3 cues", noting when the analysis cache
  wasn't rewritten).
- `get_usb_track_detail` — `{ usbRoot, usbAnalysisPathRaw, usbMediaPathRaw? }` →
  reads `{ firstBeatMs, cues, detailWaveform, keyOptions, maxHotCues }` straight off an
  **on-USB** ANLZ bundle for the cue editor opened from a USB playlist / history
  row (no local `tracks` row need exist), plus `{ genre, rating, releaseDate }`
  from the track's PDB row at `usbMediaPathRaw` (none when it's missing; the
  date is the row's release date string, else its year) and `genreOptions`
  (the library's genres and the PDB's).
- `save_usb_track_analysis_edits` — `{ usbRoot, usbAnalysisPathRaw,
  usbMediaPathRaw, bpm?, key?, durationMs?, firstBeatMs?, cues?, localTrackId?,
  title?, artist?, album? }` → writes the on-device ANLZ, eDB and `export.pdb`
  (t00 tempo and key; a new key gets a t05 row) **in place**, and also writes
  the resolved local master (so they never diverge), clearing
  `last_exported_*` on every playlist containing the track. The USB must be
  connected: a not-connected root, a missing bundle, or a track absent from the
  USB's eDB or PDB blocks the save with an error before anything is written,
  rather than a silent partial write. For a track absent from one database the
  error points to Preview Fixes, whose strict parity repair brings the two
  databases back in line (`track_not_in_usb_database` in `service/cues.rs`). Returns `{ firstBeatMs, cues, bpm,
  bpmAnalyzer, key, keySource, anlzUpdated, edbUpdated, localUpdated,
  localTrackId }` (`localUpdated: false` / no `localTrackId` only when no local
  track matched; `localTrackId` is the library track the backend resolved, which
  the UI refreshes), plus the whole rows: `track` (the file's stick row, absent
  when no playlist or history lists it) and `localTrack` (`localTrackId`'s
  library row), and `statusText` ("Saved 3 cues to USB"). Runs as a background `usb_write` job
  (`run_usb_job_with_progress`), so the window never freezes: `job:event`
  progress reports each write (checking, ANLZ, `export.pdb`, library,
  `exportLibrary.db`) to the footer bar. Saves are serialized by
  `BackendService::usb_write_lock` (each one read-modify-writes the staged
  `export.pdb`), and the UI waits for any running USB job (export,
  diagnostics, another save) to finish before starting one. No backup is taken
  per save. Without `cues` (null), the on-device cue chunks (`PCOB`/`PCO2`)
  and eDB `cue` rows are left exactly as they are; the editor sends `cues` only
  when they were edited, so a BPM, key or first-beat edit never rewrites cues
  (such as rekordbox memory cues) the app's cue model can't represent.
- `save_track_metadata_edits` — `{ trackIds, genre?, rating?, releaseDate? }`
  → sets every listed library track's genre, star rating (0–5) and/or release
  date to the same values and nothing else, clearing `last_exported_*` on
  every playlist containing them. `releaseDate` is `YYYY`, `YYYY-MM` or
  `YYYY-MM-DD` (month and day may have one digit), a real date in a year from
  1000 to next year; it is stored zero-padded as entered, with its year as
  the release year, and export completes a year or month as it does a tag's.
  A `null` field is left unchanged; an empty `genre` or `releaseDate` clears
  it (the date clears the year too). The genre gets `genre_source = 'user'`,
  so rescans and rekordbox / Mixxx imports keep it over the file's tag; the
  date gets `release_date_source = 'user'`, so rescans keep it and its year.
  An unknown id blocks the save. Returns `{ items: [{ trackId, genre, rating,
  year, releaseDate }], tracks, statusText }`, `releaseDate` being the date export writes
  (as `get_track_detail`'s), `tracks` the saved tracks' whole rows and `statusText`
  "Saved track details for 2 tracks". The cue editor sends it with one id instead of
  `save_track_analysis_edits` when only the genre / rating / release date
  changed; the selection bar's Edit sends the selection.
- `save_usb_track_metadata_edits` — `{ usbRoot, tracks: [{ usbMediaPathRaw,
  localTrackId?, title?, artist?, album? }], genre?, rating?, releaseDate? }`
  → the same edit on USB tracks: each `export.pdb` track row (t00 genre,
  rating, release year and date; a new genre gets one t01 row for all of
  them) and eDB `content` row **in place**, plus each resolved library track.
  The stick holds only full dates, so a `YYYY` or `YYYY-MM` keeps the date
  already there when it falls inside it, else becomes the first day of that
  year or month. A track missing from either database blocks the save before
  anything is written, with the same error as `save_usb_track_analysis_edits`. Leaves the ANLZ bundle, BPM, key and cues alone.
  Returns `{ items: [{ usbMediaPathRaw, localTrackId?, year?, releaseDate? }],
  tracks, localTracks, statusText }` with the PDB's year and date after the save, the
  edited files' whole stick rows, the library tracks' whole rows and "Saved track details
  for 2 tracks to USB". A background `usb_write`
  job, serialized by `usb_write_lock` like `save_usb_track_analysis_edits`.
- `get_tracks_metadata_summary` — `{ trackIds }` /
  `get_usb_tracks_metadata_summary` — `{ usbRoot, usbMediaPathsRaw }` → what
  the tracks share, for the selection bar's Edit: `{ count, genre, rating,
  releaseDate, genreOptions }`, each field `{ value?, mixed }` (`mixed` when the
  tracks differ). The USB form reads the stick's PDB rows; `genreOptions`
  adds its genres to the library's.
- `get_selection_duration` — `{ view, trackIds, … }`, the list named by
  `view`: `library` (as its sources are set), `playlist` with `id`, `usbPlaylist` / `usbHistory` with
  `usbRoot` and `id` → `{ totalDurationMs, durationUnknownCount, trackCount,
  durationText, durationUnknownText }` (the time, "· 3:45" or "· 3:45+", and its
  tooltip, empty when every length is known) for
  the selection bar's time: the ticked tracks summed by the same rule as that
  list's Total time, over the whole list, not the searched one.

### Settings

- `get_frontend_settings`
- `set_frontend_setting` — the UI's own settings (theme, sidebar, export options, ...).
- `set_export_sync_mode` — `{ pruneStale, playlistId?, sort? }` → `{ pruneStale, locked,
  statusText }` plus the playlists state. Saves the export sync mode
  (`ui_export_prune_stale_v1`, which `set_frontend_setting` refuses). When the new mode locks
  the open playlist's order (additive, same name on the selected USB), its column `sort`
  (`{ sortBy, sortDir }`) is saved as its order first; `locked` says the order is locked now,
  and `statusText` says whether the sort couldn't be saved.
  Not the language: `ui_language_v1` is refused here.
- `set_language` — `{ language }`, a language code the app has a catalog for
  (`i18n::languages()`): saves it and switches the backend's language straight away.
  An unknown code is a validation error, and so is a change while any job runs (USB
  jobs, library scan, imports, analysis, Convert, update install): the UI reloads on
  a change, which would lose the job's progress.

### UI texts

- `get_ui_texts` → `{ language, texts, languages }`: every `ui-` key of the text catalog
  (`backend/locales/<language>.ftl`) in the current language, English where it
  has none. Called once at startup, before anything renders (`ui_texts.mjs`). A
  text is a string with `{name}` placeholders for `t`, or, for a text with a
  count, its forms by plural category (`{ "one": …, "other": … }`) for
  `tCount`, which picks one with `Intl.PluralRules` for `language`. `languages`
  is every language the app has, `[{ code, name }]`, English first, each named
  in itself, for Settings' language list.

### Playlists

Every command that changes playlists (`create_playlist`, `rename_playlist`, `delete_playlist`,
`remove_tracks_from_playlist`, `reorder_playlist_tracks`, the `add_*_to_playlist(s)` commands,
`import_external_playlists`, `export_to_usb`, `export_to_usb_with_progress`,
`set_usb_device_name`, `restore_usb_backup`, and `select_usb`, `set_export_sync_mode`,
`fetch_usb_playlists`, `run_usb_diagnostics` and `repair_usb_diagnostics`, which change or re-read
the export status) also answers with the whole playlist state, the same fields
`list_playlists` returns, next to its own: `playlists` and `playlistUsbExportStatus`. The UI
applies them as they come and never asks for the list again after a change.

- `create_playlist` → `{ playlistId, name, statusText }`
- `rename_playlist`
- `reorder_playlists` — `{ orderedPlaylistIds }`, every local playlist once, top first; saves the
  sidebar order (local only, it never changes anything on the USB). `list_playlists` returns that
  order, with playlists never reordered newest first
- `delete_playlist` → `{ playlistId, deleted, statusText }`
- `list_playlists` → `{ playlists, playlistUsbExportStatus }`. `playlistUsbExportStatus` is each
  playlist's `PlaylistUsbExportStatus` (`sameNameExistsOnUsb`, `locksReorder` with its
  `reorderLockTooltip`, the sidebar's `markLabel`, and `exportStatusText`, the export line with
  "On this USB." in front when it is) against the selected USB and the current export sync-mode
  setting, reading only the staged PDB/eDB (no USB access). The UI calls it at startup and when
  the sync mode changes. Each playlist also carries `deleteConfirmText`, what the dialog asks before
  deleting it, and `exportStatusText` / `exportStatusTooltip`, the playlist page's export
  line ("Last exported 2026-04-07 13:00 to MYUSB (12 tracks).")
- `get_playlist_tracks` — with `usbRoot` (the selected USB), each track also carries
  `exportFile` when export puts something other than its own file on that USB:
  `{ kind: "transcode" | "convert" | "refused", badge, tooltip }`, the FMT badge's text and
  tooltip (naming the track's own file as the source); the transcode that goes has `exported`
  set. See `docs/TRANSCODES.md`. Each track also carries `fileSource`:
  `{ kind: "usb" | "local", missing, tooltip }`, the row's source icon, read for the selected USB
  (see `docs/PLAYLISTS_PLAYBACK.md`)
- `get_playlist_export_space` — `{ playlistId, usbRoot }` → `exportSpace`: how many tracks and
  bytes an export of the playlist would still copy to the USB, its free space, and whether it
  fits. Worked out from recorded data only (audio sizes from the app DB, what is already on the
  USB from the staged PDB and eDB, see "Tracks already on the USB" in `docs/USB_EXPORT.md`), so
  no audio file is read. Sizes are those of the files export would copy: a transcode's own, or
  an estimate for one export still has to make. Also `formatNotice`, the title row's line about
  the tracks export converts first or refuses (lossy, in a format the USB audio formats
  don't allow; see `docs/TRANSCODES.md`), or `null`, and `formatNoticeBlocks` (a track is
  refused, so the export is blocked). `null` when it can't be made. The frontend asks
  for it after the track list is shown, so it never holds the list up
- `add_library_selection_to_playlist` / `add_track_candidates_to_playlist` — both answer with
  `statusText` ("Added 2 tracks (skipped 0) to Warmup"), which the backend also logs as the
  Event Log entry (`playlist_add.result`, its counts in the log file); the candidates' is
  empty when none resolved
- `add_library_selection_to_playlists` / `add_track_candidates_to_playlists` — the same
  selection as the two above, resolved once and added to every playlist in `playlistIds`;
  returns what each playlist took (`items`), the totals and `statusText` ("Added 3 tracks
  (skipped 1) to Warmup, Peak", empty when none resolved), also logged as the Event Log entry
- `get_playlist_membership` — `{ trackIds }`, one entry per selected track (its local id, or
  `null` for a track with no local row yet); every local playlist in sidebar order with how
  many of them it holds (`contained`), `state`: `none`, `some` or `all`, and `noteText`
  ("Already in", "1 of 2 in", or empty)
- `remove_tracks_from_playlist` → `{ playlistId, removed, statusText }`
- `reorder_playlist_tracks`

### USB import/export

- `select_usb` — `{ path }`: selects the USB, everything the USB page needs in one answer. It
  checks the folder (`valid`, `normalizedRoot`, `statusText`, `initHintText`, `canInitialize`);
  a valid root becomes the backend's selected USB, the one playlist export status is worked out
  against, and an invalid one or an empty `path` leaves none selected. It saves the folder as
  the one the app shows at startup (`ui_usb_root_v1`, which `set_frontend_setting` refuses). For a
  valid USB it also answers `audioFormats` (below) and `deviceName` (`{ name, suggestedName,
  nameTakenBy }`), each `null` when it can't be read (an Event Log warning says why); `devices`
  is the known drives, as `list_usb_devices` lists them. Plus the playlist state (see Playlists)
- `list_usb_devices`
- `prune_usb_device` — `{ id }` → `{ pruned, devices }`: `devices` is the list
  `list_usb_devices` would answer after the prune.
- `set_usb_device_name`
- `audioFormats` (`select_usb`) →
  `{ formats: [{ format, label, allowed, locked, tooltip }], writable, hint }`: the USB's Player
  formats (`flac`, `alac`, `aiff`, `wav`, `mp3`, `aac`) in its order of preference, from the
  stick's `.dj-usb-tkit/drive.json`. A stick without the setting gets all six ticked. `locked`:
  the box can't change (the last ticked one, with `tooltip` saying so, or a read-only stick);
  `hint` is the row's line.
- `set_usb_audio_formats` — `{ usbRoot, formats: [{ format, allowed }] }` → the same shape. The list
  must hold every format once with at least one allowed; a read-only stick is refused.
- `list_usb_backups`
- `restore_usb_backup`
- `delete_usb_backup`
- `fetch_usb_playlists`
- `fetch_usb_histories`
- `fetch_usb_playlist_tracks`
- `fetch_usb_history_tracks`
- `get_usb_history_tracklist_options` — `{ usbRoot, id }`, answers `{ startTracks }`, the Export
  Tracklist dialog's labels. Reads the cached history, not the stick.
- `build_usb_history_tracklist` — `{ usbRoot, id, startIndex, timeMode }` (`off`, `before`, `after`),
  answers `{ suggestedFileName, contents, exportedText, cancelledText }`. Reads the cached history,
  not the stick; the desktop layer's `save_text_file` writes the file.
- `get_usb_player_menu_config`
- `update_usb_player_menu_config` — `{ usbRoot, action, kind }`, one editor edit
  (`add`, `remove`, `moveUp`, `moveDown`) applied to the stick's current menu;
  an edit that changes nothing writes nothing.
- `get_usb_background_color` — `{ pdbColor, edbColor, writable, reason, updated, warnings }`:
  the player background colour each database holds (0 = default, 1–8 = pink,
  red, orange, yellow, green, aqua, blue, purple; `null` where a database has
  no place for it), and whether one can be set.
- `set_usb_background_color` — `{ usbRoot, color }`, sets `color` in both
  databases and saves both to the USB or neither; answers like
  `get_usb_background_color`. Setting the colour both already hold writes nothing.
- `remove_usb_playlist`
- `begin_usb_order_edit` — `{ usbRoot }`, starts Edit order on the USB Playlists page: a
  pending order of the USB's playlists and of the tracks in them, held by the backend and written
  only by `commit_usb_order_edit`. Refused (`usb_read_only`) when the USB can't be written, and
  while another USB write runs. `fetch_usb_playlists` and `fetch_usb_playlist_tracks` show the
  pending order while the edit is on; `fetch_usb_playlists` answers with `orderEdit` (`active`,
  `changed`, `editedPlaylistIds`, `editingText`), each playlist's `orderEdited`, and
  `orderEditBlockedText` (why Edit order can't start, e.g. a read-only USB). Selecting another
  USB ends the edit.
- `move_usb_playlist` — `{ usbRoot, playlistId, beforePlaylistId }`, moves a playlist to just
  before another one (`null`: to the end) in the pending order.
- `move_usb_playlist_track` — `{ usbRoot, playlistId, moveTrackId, beforeTrackId }`, the same
  for one track of a playlist. A history list (`usb-h-…`) is refused.
- `sort_usb_playlist_tracks` — `{ usbRoot, playlistId, sortBy, sortDir }`, makes a column sort
  the playlist's pending order.
- `cancel_usb_order_edit` — `{ usbRoot }`, drops the pending order; works when the USB is gone.
- `commit_usb_order_edit` — `{ usbRoot }`, a USB write job: one backup ("Before order edit",
  replacing the previous one of the same reason), the new playlist order and track order written
  to the PDB (t07 sort orders, t08 `entry_index`) and to the eDB (`playlist.sequenceNo`,
  `playlist_content.sequenceNo`), each saved to the USB once, then one Engine DJ rebuild. Before
  saving, it checks with the parity report's comparison that the two databases hold the same order
  for what the edit changed (an edited playlist's shared tracks, a moved playlist's place); if
  not, nothing is saved and the error points at the strict parity repair. A
  failure keeps the edit on (Save again retries on fresh USB data); a failure after the PDB was
  saved but before the eDB was says so (`partial`). The move, sort, cancel and begin commands
  answer `{ orderEdit, statusText, warnings, warningText }`, as this one does.
- `inspect_usb_track` — `{ source, track, warnings }`; `track` is the whole
  row as a page shows it (library id resolved), with `needsHydration` false.
- `initialize_usb`
- `export_to_usb` — refused before anything is written, with `validationType`
  `usb_audio_format_refused` and the `tracks`, when a track can't go on the USB in any format its
  USB audio formats tick (see `docs/TRANSCODES.md`), or `usb_read_only` when the USB can't be
  written. Transcodes it makes first report as "USB: Converting track n/m".
- `detect_external_rekordbox_db`

### External libraries

See `docs/EXTERNAL_LIBRARIES.md` for behavior and the fields read.

- `detect_external_rekordbox_db` / `detect_external_mixxx_db` — `{ found, path }`
- `scan_rekordbox_db` / `scan_mixxx_db` — `{ path? }`; imports every track of the
  library, returns the `scan_library` result shape (`indexed`, `updated`,
  `removed`, `warnings`, `statusText`: what the import brought in). Each track
  whose file is missing is skipped and logged as a warning
  (`scan.rekordbox-db.file-not-found` / `scan.mixxx-db.file-not-found`)
- `list_rekordbox_playlists` / `list_mixxx_playlists` — `{ path? }`; returns
  `items: [{ id, name, kind, trackCount, existingPlaylist?, noteText }]` where `id` is
  the source library's own id as a string, `kind` is `playlist` | `crate` |
  `history` (only lists with tracks), and `existingPlaylist` (`{ id, name }`)
  is the local playlist an earlier import of that list made; `noteText` is the
  dialog's note on the list ("3 tracks · updates "My Set"")
- `import_external_playlists` — `{ items: [{ library, name, path?, kind, id, force? }] }`
  (`library`: `rekordbox` | `mixxx`); imports the lists one after another, a
  failed one not stopping the rest. Each imports its tracks into the playlist
  an earlier import of that list made (`updatedExisting: true`) or a new one.
  Returns `{ results, imported, failed, added, skipped, statusText }`
  (`name` only words the status line's failed lists): `results` follows
  `items`, each `{ playlist }` (`{ playlistId, name, updatedExisting, added,
  skipped, indexed, warnings }`) or `{ error }`, and the counts total
  them. `skipped` counts list entries not placed (missing or unsupported files,
  tracks gone from the library, repeats). `force` makes the tracks take BPM,
  key and cues from the library over the app's own values. A list none of
  whose tracks can be imported fails, changing nothing. Each failed list is
  logged in the Event Log (`playlist_import.failed`) with its reason; when none
  imported, `statusText` is the first failure's reason

`export_to_usb` takes `sort` (`{ sortBy, sortDir }`, or `null`), the open playlist's column
sort: it is saved as the playlist's track order before the export, as `reorder_playlist_tracks`'s
`sortBy` mode saves it, unless the order is locked on the USB (see below).

`export_to_usb` options:

- `pruneStale = true` -> mirror mode (target playlist membership rewritten from current manifest)
- `pruneStale = false` -> additive mode (new members added, existing members preserved)
- `backupBeforeExport = true` (default) -> copies PDB and eDB to a backups folder next to them with a timestamp before each export; no-op if the files do not yet exist
- `backupBeforeExport = false` -> skips backup step

Every command that changes the USB's rekordbox data (`export_to_usb`, `commit_usb_order_edit`,
`remove_usb_playlist`, `save_usb_track_analysis_edits`, `save_usb_track_metadata_edits`,
`restore_usb_backup`, `repair_usb_diagnostics` with `apply`, and the player-menu updates) then
rebuilds the Engine DJ
library (`Engine Library/Database2`), keeping the player's play history. The
`ui_export_engine_library_v1` setting (`"1"` = on; off by default) controls it. A failure only
adds a warning (see `docs/ENGINE_DJ.md`). `repair_usb_diagnostics` returns the setting as
`engineLibraryEnabled`, since its `keep_engine_library_up_to_date` fix turns it on.

`select_usb`'s `deviceName` and `set_usb_device_name` read/write the user-assigned drive identity
(see `docs/USB_EXPORT.md`'s "USB drive naming" section); `deviceName` also carries a best-effort
`suggestedName` drawn from the OS filesystem label when the drive is unnamed.

`fetch_usb_playlist_tracks` / `fetch_usb_history_tracks` return one
paginated/searched/sorted page of a selected USB playlist or history session
(see the "Paginated track lists" envelope above), with the waveform-preview
bytes and artwork data URLs already hydrated server-side **for that page only** —
the frontend renders the page directly, no follow-up hydration round-trip.
`fetch_usb_playlists` / `fetch_usb_histories` still resolve the playlist/session
list and its counts, newest session first, and don't send each item's tracks.
They also send the panel's text (`summaryText`, `statusText`), and each playlist its
`selectedText` and `removeConfirmText`; the track pages send `listSummaryText` for the heading, as
`browse_source_files` and `get_playlist_tracks` do.

The USB commands send the text the UI shows, ready to display:

- `select_usb`: `statusText`, `initHintText`, and `canInitialize` for the Initialize button.
  `initialize_usb`: `statusText`.
- `set_usb_device_name`: answers `{ name, updated, statusText }`. `name` is as saved, and `updated`
  is false when the drive already had exactly that name.
- `run_usb_diagnostics` and `run_usb_parity_report`: `statusText`, `durationText` and
  `statusLabel`, the badge's text. `overallStatus` stays a value, for the badge's style.
  `run_usb_diagnostics` also sends `healthText` ("USB health: good"), the health dots' tooltip.
- `repair_usb_diagnostics`: `statusText`, `nothingToFix`, and `rows`, the fix list ready to show,
  including unsupported items.
- `get_usb_player_menu_config` / `update_usb_player_menu_config`: `statusText`, and on each item
  `label`, `canMoveUp` and `canMoveDown`.
- `get_usb_background_color` / `set_usb_background_color`: `chips` (`color`, `label`, `selected`),
  `noteText` (empty when there's nothing to say) and `statusText`.
- `export_to_usb`: a refusal's `details` carry `statusText` with `validationType`. Refusals without
  details have only the error's `message`.

`list_usb_backups`/`restore_usb_backup`/`delete_usb_backup` back the Backups panel described in
`docs/USB_BACKUPS.md` — listing, restoring, and deleting the timestamped
PDB+eDB snapshot pairs taken before every USB-mutating operation. The list sends its summary, and each
backup its row and dialog text. A restore and a delete answer with the list as it is afterwards
(`backups`); a restore also with the playlists state (`WithPlaylists`), since the restored
databases change the playlists' USB marks.

### Diagnostics and repairs

- `run_usb_diagnostics`
- `run_usb_parity_report`
- `repair_usb_diagnostics`

`add_track_candidates_to_playlist` accepts frontend row candidates, resolves/materializes safe
local source rows server-side, preserves the USB-origin no-fuzzy-match rule, and then delegates
to playlist membership insertion. Each resolution names the candidate's `previousId` and resolved
`trackId`, plus the new library row (`track`) when a listed file was materialized.

`reorder_playlist_tracks` persists a new order to the `playlist_tracks.position` column for a
local (app-owned) playlist, in one of three request shapes — `orderedTrackIds` (the full order),
`sortBy`/`sortDir` (server sorts the whole playlist by that column and persists it — commits a
client column sort; left unsaved, `reordered` 0, while the playlist's order is locked on the
selected USB), or `moveTrackId` + optional `beforeTrackId` (single-move for a paginated
drag-reorder). `get_playlist_tracks` also returns `unanalyzedCount` over the whole filtered
playlist, since a paginated client can't count them from one page, with the Analyze Missing
button's `analyzeMissingLabel` and `analyzeMissingTooltip`. See `docs/PLAYLISTS_PLAYBACK.md`
for when the UI disables reordering. `fetch_usb_playlists`, `run_usb_diagnostics` and
`repair_usb_diagnostics` answer with the playlist state (`WithPlaylists`), so the frontend doesn't
have to re-derive that lock condition from raw USB-scan/export-setting state.

`analyze_new_tracks` adds to the analysis job and returns at once: `jobId`, `queued`, `skipped` and
the status line's `message`. The job's events carry the rest, including each track's saved row and,
on `job.completed`, the final `message` and, when the job logged warnings, `warningText`. Rows waiting
their turn come on `job.tracks_waiting` (`waitingTrackIds`, `waitingTooltip`) and leave on the
track's first progress event, or on `job.tracks_not_waiting` after a cancel.

`fetch_usb_playlists`, `fetch_usb_histories`, `commit_usb_order_edit`,
`remove_usb_playlist` and `export_to_usb` return `warningText` beside `statusText`: the
status line's link to the Event Log ("(2 warnings)", counting the warnings and errors among
`warnings`), empty when there are none. The UI shows it after the status line; nothing in
`statusText` is parsed.

`analyze_new_tracks` takes the raw
`bpmRange` string (e.g. `"70-180"`) from the settings dropdown and parses/validates it server-side
(`bpmMin`/`bpmMax` are the older pre-parsed form, still accepted as a fallback).

When `repair_usb_diagnostics` is called with `apply: true`, the response may include a fresh
`diagnostics` report for the post-repair state.

### Analysis

- `analyze_new_tracks`
- `set_analysis_paused`
- `cancel_analysis`
- `transcode_tracks` — `{ trackIds, format }` (`"aiff"`, `"wav"` or `"mp3"`) →
  `{ transcoded, skipped, failed, cancelled, messages, tracks, summary }` (`summary`: the
  status bar's line). A job
  (`jobType: "transcode"`, one `job.progress` per finished track). The backend
  picks which tracks need a transcode; `messages` has one line per skipped or
  failed track, `tracks` the selected tracks' rows. See `docs/TRANSCODES.md`.
- `cancel_transcode` — stops the running transcode job; finished files stay.
- `remove_track_transcodes` — `{ trackIds }` → `{ removed, tracks, summary }`.
- `get_transcode_formats` — `{ formats: [{ format, label, isDefault }] }`: the Convert…
  dialog's choices in its order (`"mp3"` / "MP3 320 kbps" first and picked, then AIFF and WAV),
  every format `transcode_tracks` takes.
- `get_transcode_folder` / `set_transcode_folder` — `{ path? }` (none goes back
  to the default) → `{ path, isDefault }`. A folder inside a source folder is
  refused.
- `download_essentia`
- `cancel_essentia_download`
- `remove_essentia`

### Playback

- `resolve_playback_source`
- `play_resolved_track` — answers with where it plays from (`source`) and
  `playingText`, the status line ("Playing from Library: Artist - Title")
- `stop_playback_native`
- `pause_playback_native` / `resume_playback_native` — hold the loaded track at
  its position / continue it; `paused` on the returned status and events.
- `get_playback_status_native`
- `set_playback_metronome` — `{ enabled, firstBeatMs?, bpm? }` → `{ enabled }`.
  The engine mixes a click on every grid beat (accented on bar starts) into
  whatever is playing, live, until disabled. Stays off (`enabled: false`)
  without a usable BPM. Used by the cue editor; see `docs/CUE_EDITOR.md`.

### App

- `check_for_update` — fetches GitHub Releases, compares against the running
  version, and returns `{ updateAvailable, severity, currentVersion,
  latestVersion, releaseUrl }`. `severity` is `"none"`, `"normal"`,
  `"feature"` (a newer release's notes carry `**Severity:** feature`) or
  `"critical"` (`**Severity:** critical`; wins over `"feature"`); the last two
  show the in-app banner. Also `statusText`, the settings update line ("Version 1.2.3 is
  available.", "Couldn't check for updates." or "Up to date."), and `bannerText` (empty
  unless the banner shows). Never fails: a network/parse error logs a warning
  and returns a "no update" verdict with `checkFailed`.

## Host utility commands

These are desktop host commands, not backend API envelope commands:

- `clear_frontend_log`
- `append_frontend_log`
- `get_backend_log_buffer`
- `pick_source_folders`
- `pick_usb_folder`
- `pick_transcode_folder`
- `save_text_file`
- `show_window`
- `set_theme_background`

No command lets the window widen the asset protocol's scope (the files it may load, such
as covers). Beyond what `tauri.conf.json` allows, the backend lets in the library's source
folders (at startup, and after `add_source_folders` / `relocate_source_root`) and a USB
root `select_usb` found valid.
