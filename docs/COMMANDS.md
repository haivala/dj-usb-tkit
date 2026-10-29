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

### Paginated track lists

`browse_source_files`, `get_playlist_tracks` and the USB per-list track
commands (`fetch_usb_playlist_tracks`, `fetch_usb_history_tracks`) share a
pagination shape: the request carries `query`, `sortBy` (`title` | `artist` |
`album` | `format` | `bpm` | `durationMs` | `key`; absent ⇒ natural order),
`sortDir` (`asc` | `desc`), `offset` or `cursor`, and `limit` (`0` ⇒
unpaginated). The response carries `items`, `total` (filtered count),
`nextCursor`, `hasMore`, plus any whole-set aggregates (`totalDurationMs`,
`sourceRootAnalysis`). Filtering and sorting are server-side so they span the
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

USB-scoped jobs lock the selected USB root in the frontend until the matching
`job.completed` or `job.failed` event arrives. Locking job types are
`usb_read`, `usb_write`, `diagnostics`, and `export`; this covers playlist and
history reads, player-menu reads/writes, initialization, export, parity,
diagnostics, and repair. The lock prevents a late response from an earlier
drive selection from being rendered against a newer selected drive.

## Command groups

### Library

- `scan_library`
- `scan_rekordbox_db`
- `scan_mixxx_db`
- `browse_source_files`
- `check_source_roots`
- `resolve_track_identity`
- `remove_tracks_by_source_roots`
- `relocate_source_root`
- `get_tracks_by_ids_with_previews`
- `get_track_detail` — returns `{ track, firstBeatMs, cues, detailWaveform,
  keyOptions, genre, rating, releaseDate, genreOptions }` for the cue editor
  (see `docs/CUE_EDITOR.md`). `rating` is 0–5 stars; `releaseDate` is the date
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
  memory; outside the 8).
- `get_usb_track_detail` — `{ usbRoot, usbAnalysisPathRaw, usbMediaPathRaw? }` →
  reads `{ firstBeatMs, cues, detailWaveform, keyOptions }` straight off an
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
  rather than a silent partial write. Returns `{ firstBeatMs, cues, bpm,
  bpmAnalyzer, key, keySource, anlzUpdated, edbUpdated, localUpdated,
  localTrackId }` (`localUpdated: false` / no `localTrackId` only when no local
  track matched; `localTrackId` is the library track the backend resolved, which
  the UI refreshes). Runs as a background `usb_write` job
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
  year, releaseDate }] }`, `releaseDate` being the date export writes (as
  `get_track_detail`'s). The cue editor sends it with one id instead of
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
  anything is written. Leaves the ANLZ bundle, BPM, key and cues alone.
  Returns `{ items: [{ usbMediaPathRaw, localTrackId?, year?, releaseDate? }]
  }` with the PDB's year and date after the save. A background `usb_write`
  job, serialized by `usb_write_lock` like `save_usb_track_analysis_edits`.
- `get_tracks_metadata_summary` — `{ trackIds }` /
  `get_usb_tracks_metadata_summary` — `{ usbRoot, usbMediaPathsRaw }` → what
  the tracks share, for the selection bar's Edit: `{ count, genre, rating,
  releaseDate, genreOptions }`, each field `{ value?, mixed }` (`mixed` when the
  tracks differ). The USB form reads the stick's PDB rows; `genreOptions`
  adds its genres to the library's.
- `get_selection_duration` — `{ view, trackIds, … }`, the list named by
  `view`: `library` with its filter (`sourceRoots`, `includeRekordboxDb`,
  `includeMixxxDb`), `playlist` with `id`, `usbPlaylist` / `usbHistory` with
  `usbRoot` and `id` → `{ totalDurationMs, durationUnknownCount, trackCount }` for
  the selection bar's time: the ticked tracks summed by the same rule as that
  list's Total time, over the whole list, not the searched one.

### Settings

- `get_frontend_settings`
- `set_frontend_setting`

### Playlists

- `create_playlist`
- `rename_playlist`
- `reorder_playlists` — `{ orderedPlaylistIds }`, every local playlist once, top first; saves the
  sidebar order (local only, it never changes anything on the USB). `list_playlists` returns that
  order, with playlists never reordered newest first
- `delete_playlist`
- `list_playlists`
- `get_playlist_tracks`
- `get_playlist_export_space` — `{ playlistId, usbRoot }` → `exportSpace`: how many tracks and
  bytes an export of the playlist would still copy to the USB, its free space, and whether it
  fits. Worked out from recorded data only (audio sizes from the app DB, what is already on the
  USB from the staged PDB), so no file is read. `null` when it can't be made. The frontend asks
  for it after the track list is shown, so it never holds the list up
- `add_library_selection_to_playlist`
- `add_track_candidates_to_playlist`
- `add_library_selection_to_playlists` / `add_track_candidates_to_playlists` — the same
  selection as the two above, resolved once and added to every playlist in `playlistIds`;
  returns what each playlist took (`items`) and the totals
- `get_playlist_membership` — `{ trackIds }`, one entry per selected track (its local id, or
  `null` for a track with no local row yet); every local playlist in sidebar order with how
  many of them it holds (`contained`) and `state`: `none`, `some` or `all`
- `remove_tracks_from_playlist`
- `reorder_playlist_tracks`
- `refresh_playlist_export_status` — recomputes every playlist's
  `PlaylistUsbExportStatus` (`sameNameExistsOnUsb` + `locksReorder`) against the
  connected USB and the current export sync-mode setting, reading only the
  staged PDB/eDB (no USB access). Called when the sync mode changes.

### USB import/export

- `validate_usb_root`
- `list_usb_devices`
- `prune_usb_device`
- `get_usb_device_name`
- `set_usb_device_name`
- `list_usb_backups`
- `restore_usb_backup`
- `delete_usb_backup`
- `fetch_usb_playlists`
- `fetch_usb_histories`
- `fetch_usb_playlist_tracks`
- `fetch_usb_history_tracks`
- `get_usb_player_menu_config`
- `update_usb_player_menu_config`
- `sync_usb_player_menu_edb_to_pdb`
- `remove_usb_playlist`
- `reorder_usb_playlists`
- `inspect_usb_track`
- `initialize_usb`
- `export_to_usb`
- `detect_external_rekordbox_db`

### External libraries

See `docs/EXTERNAL_LIBRARIES.md` for behavior and the fields read.

- `detect_external_rekordbox_db` / `detect_external_mixxx_db` — `{ found, path }`
- `scan_rekordbox_db` / `scan_mixxx_db` — `{ path? }`; imports every track of the
  library, returns the `scan_library` result shape (`indexed`, `updated`,
  `removed`, `notFound`, `warnings`)
- `list_rekordbox_playlists` / `list_mixxx_playlists` — `{ path? }`; returns
  `items: [{ id, name, kind, trackCount, existingPlaylist? }]` where `id` is
  the source library's own id as a string, `kind` is `playlist` | `crate` |
  `history` (only lists with tracks), and `existingPlaylist` (`{ id, name }`)
  is the local playlist an earlier import of that list made
- `import_rekordbox_playlist` / `import_mixxx_playlist` — `{ path?, kind, id,
  force? }`; imports the list's tracks into the playlist an earlier import of
  it made (`updatedExisting: true`) or a new one, returns `{ playlistId, name,
  updatedExisting, added, indexed, notFound, warnings }`. `force` makes the
  tracks take BPM, key and cues from the library over the app's own values.
  Fails with a validation error, changing nothing, when none of the list's
  tracks can be imported

`export_to_usb` options:

- `pruneStale = true` -> mirror mode (target playlist membership rewritten from current manifest)
- `pruneStale = false` -> additive mode (new members added, existing members preserved)
- `backupBeforeExport = true` (default) -> copies PDB and eDB to a backups folder next to them with a timestamp before each export; no-op if the files do not yet exist
- `backupBeforeExport = false` -> skips backup step

Every command that changes the USB's rekordbox data (`export_to_usb`, `reorder_usb_playlists`,
`remove_usb_playlist`, `save_usb_track_analysis_edits`, `save_usb_track_metadata_edits`,
`restore_usb_backup`, `repair_usb_diagnostics` with `apply`, and the player-menu updates) then
rebuilds the Engine DJ
library (`Engine Library/Database2`), keeping the player's play history. The
`ui_export_engine_library_v1` setting (`"1"` = on; off by default) controls it. A failure only
adds a warning (see `docs/ENGINE_DJ.md`). `repair_usb_diagnostics` returns the setting as
`engineLibraryEnabled`, since its `keep_engine_library_up_to_date` fix turns it on.

`get_usb_device_name`/`set_usb_device_name` read/write the user-assigned drive identity (see
`docs/USB_EXPORT.md`'s "USB drive naming" section); `get_usb_device_name` also returns a
best-effort `suggestedName` drawn from the OS filesystem label when the drive is unnamed.

`fetch_usb_playlist_tracks` / `fetch_usb_history_tracks` return one
paginated/searched/sorted page of a selected USB playlist or history session
(see the "Paginated track lists" envelope above), with the waveform-preview
bytes and artwork data URLs already hydrated server-side **for that page only** —
the frontend renders the page directly, no follow-up hydration round-trip.
`fetch_usb_playlists` / `fetch_usb_histories` still resolve the playlist/session
list and its counts.

`list_usb_backups`/`restore_usb_backup`/`delete_usb_backup` back the Backups panel described in
`docs/USB_EXPORT.md`'s "Backup" section — listing, restoring, and deleting the timestamped
PDB+eDB snapshot pairs taken before every USB-mutating operation.

### Diagnostics and repairs

- `run_usb_diagnostics`
- `run_usb_parity_report`
- `repair_usb_diagnostics`

`add_track_candidates_to_playlist` accepts frontend row candidates, resolves/materializes safe
local source rows server-side, preserves the USB-origin no-fuzzy-match rule, and then delegates
to playlist membership insertion.

`reorder_playlist_tracks` persists a new order to the `playlist_tracks.position` column for a
local (app-owned) playlist, in one of three request shapes — `orderedTrackIds` (the full order),
`sortBy`/`sortDir` (server sorts the whole playlist by that column and persists it — commits a
client column sort), or `moveTrackId` + optional `beforeTrackId` (single-move for a paginated
drag-reorder). `get_playlist_tracks` also returns `unanalyzedCount` over the whole filtered
playlist, since a paginated client can't count them from one page. See `docs/PLAYLISTS_PLAYBACK.md`
for when the UI disables reordering. `fetch_usb_playlists` and `run_usb_diagnostics` both return a
`playlistUsbExportStatus` array (one entry per local playlist) so the frontend doesn't have to
re-derive that lock condition from raw USB-scan/export-setting state.

`analyze_new_tracks` returns hydrated changed track rows in `items`, so the UI can patch rows from
the analysis response without a separate `get_tracks_by_ids_with_previews` call. It takes the raw
`bpmRange` string (e.g. `"70-180"`) from the settings dropdown and parses/validates it server-side
(`bpmMin`/`bpmMax` are the older pre-parsed form, still accepted as a fallback).

When `repair_usb_diagnostics` is called with `apply: true`, the response may include a fresh
`diagnostics` report for the post-repair state.

### Analysis

- `analyze_new_tracks`
- `set_analysis_paused`
- `cancel_analysis`
- `download_essentia`
- `cancel_essentia_download`
- `remove_essentia`

### Playback

- `resolve_playback_source`
- `play_resolved_track`
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
  show the in-app banner. Never fails: a network/parse error logs a warning
  and returns a "no update" verdict.

## Host utility commands

These are desktop host commands, not backend API envelope commands:

- `clear_frontend_log`
- `append_frontend_log`
- `get_backend_log_buffer`
- `pick_source_folders`
- `pick_usb_folder`
- `save_text_file`
- `allow_asset_paths`
- `show_window`
- `set_theme_background`
