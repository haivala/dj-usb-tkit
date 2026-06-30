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

## Event model

Long-running commands emit job lifecycle events:

- `job.started`
- `job.progress`
- `job.completed`
- `job.failed`

The desktop host also emits a unified `job:event` channel carrying the same payload shape.

## Command groups

### Library

- `scan_library`
- `search_tracks`
- `list_tracks`
- `browse_source_files`
- `materialize_source_track`
- `remove_tracks_by_source_roots`
- `get_system_parallelism`
- `get_tracks_by_ids_with_previews`

### Playlists

- `create_playlist`
- `rename_playlist`
- `delete_playlist`
- `list_playlists`
- `get_playlist_tracks`
- `add_tracks_to_playlist`
- `remove_tracks_from_playlist`

### USB import/export

- `set_usb_edb_key`
- `validate_usb_root`
- `fetch_usb_playlists`
- `fetch_usb_histories`
- `get_usb_cdj_menu_config`
- `update_usb_cdj_menu_config`
- `remove_usb_playlist`
- `inspect_usb_track`
- `initialize_usb`
- `export_to_usb`
- `detect_external_master_db`

`export_to_usb` options:

- `pruneStale = true` -> mirror mode (target playlist membership rewritten from current manifest)
- `pruneStale = false` -> additive mode (new members added, existing members preserved)
- `backupBeforeExport = true` (default) -> copies PDB and eDB to backups folder next to them with a timestamp before each export; no-op if the files do not yet exist
- `backupBeforeExport = false` -> skips backup step

### Diagnostics and repairs

- `run_usb_diagnostics`
- `run_usb_parity_report`
- `repair_usb_diagnostics`

### Analysis

- `analyze_new_tracks`
- `analyze_track_piece`

### Playback

- `resolve_playback_source`
- `play_track_native`
- `stop_playback_native`
- `get_playback_status_native`
- `playback_preflight_native`

## Host utility commands

These are desktop host commands, not backend API envelope commands:

- `clear_frontend_log`
- `append_frontend_log`
- `get_backend_log_buffer`
- `pick_source_folders`
- `pick_usb_folder`
- `allow_asset_paths`
- `show_window`
- `set_theme_background`
