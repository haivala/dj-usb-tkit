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

`browse_source_files` and `get_playlist_tracks` (and, when they land, the USB
per-list track commands) share a keyset-pagination shape: the request carries
`query`, `sortBy` (`title` | `artist` | `album` | `format` | `bpm` |
`durationMs` | `key`; absent ⇒ natural order), `sortDir` (`asc` | `desc`),
`cursor`, and `limit` (`0` ⇒ unpaginated). The response carries `items`,
`total` (filtered count), `nextCursor`, `hasMore`, plus any whole-set
aggregates (`totalDurationMs`, `sourceRootAnalysis`). Filtering and sorting are
server-side so they span the entire list, not just the loaded page. A `cursor`
is bound by signature to its exact `query`/`sortBy`/`sortDir`/scope and is
rejected with a validation error if any of those changed.

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
- `search_tracks`
- `list_tracks`
- `browse_source_files`
- `check_source_roots`
- `materialize_source_track`
- `resolve_track_identity`
- `remove_tracks_by_source_roots`
- `relocate_source_root`
- `get_tracks_by_ids_with_previews`
- `get_track_detail` — returns `{ track, firstBeatMs, cues, detailWaveform }` for
  the cue editor (see `docs/CUE_EDITOR.md`). `detailWaveform` is
  the **base64** of the raw PWV5 colour-detail waveform payload from the `.EXT`
  (2 bytes/entry, BE u16; absent when the track has no analysis). Cues are
  ordered by `sortOrder`.
- `save_track_analysis_edits` — `{ trackId, firstBeatMs?, cues?, bpm?, key? }` →
  atomically replaces the track's cue list (`cues: Some([...])`, max 8 hot cues)
  and/or updates the beat-grid anchor (`firstBeatMs: Some(ms)`), BPM and key,
  rewrites the cached local ANLZ bundle, and clears `last_exported_*` on every
  playlist containing the track. A `null` field is left unchanged. `cues[]`
  entries are `{ positionMs, colorId?, name?, playbackStart? }`; each hot cue
  becomes a memory point + a hot-cue pad on export. At most one entry may have
  `playbackStart: true`: the memory-only playback-start cue (no pad, colour or
  name, never after the first hot cue, dropped without hot cues; outside the 8).
- `get_usb_track_detail` — `{ usbRoot, usbAnalysisPathRaw }` → reads
  `{ firstBeatMs, cues, detailWaveform }` straight off an **on-USB** ANLZ bundle
  for the cue editor opened from a USB playlist / history row (no local `tracks`
  row need exist).
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
  per save.

### Settings

- `get_frontend_settings`
- `set_frontend_setting`

### Playlists

- `create_playlist`
- `rename_playlist`
- `delete_playlist`
- `list_playlists`
- `get_playlist_tracks`
- `add_tracks_to_playlist`
- `add_track_candidates_to_playlist`
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
- `merge_orphaned_usb_placeholder_tracks`
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
- `inspect_usb_tracks`
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

`get_usb_device_name`/`set_usb_device_name` read/write the user-assigned drive identity (see
`docs/USB_EXPORT.md`'s "USB drive naming" section); `get_usb_device_name` also returns a
best-effort `suggestedName` drawn from the OS filesystem label when the drive is unnamed.

`inspect_usb_tracks` is the batched form of `inspect_usb_track`: it hydrates waveform/BPM/
key/artwork metadata for a chunk of USB tracks in one call (parsing the PDB and opening the eDB
once per chunk) instead of one backend call per track. It is now used only for the
resolve-one-row-before-playback path.

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
- `play_track_native`
- `stop_playback_native`
- `pause_playback_native` / `resume_playback_native` — hold the loaded track at
  its position / continue it; `paused` on the returned status and events.
- `get_playback_status_native`
- `playback_preflight_native`
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
