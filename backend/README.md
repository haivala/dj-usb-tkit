# Backend

Implemented command set:

- Library:
  - `scan_library`
  - `search_tracks`
  - `list_tracks`
  - `remove_tracks_by_source_roots`
  - `get_tracks_by_ids_with_previews`
- Playlists:
  - `create_playlist`
  - `rename_playlist`
  - `delete_playlist`
  - `list_playlists`
  - `get_playlist_tracks`
  - `add_tracks_to_playlist`
  - `add_track_candidates_to_playlist`
  - `remove_tracks_from_playlist`
  - `reorder_playlist_tracks`
- Frontend settings:
  - `get_frontend_settings`
  - `set_frontend_setting`
- USB / export:
  - `set_usb_db_key`
  - `validate_usb_root`
  - `initialize_usb`
  - `export_to_usb`
  - `fetch_usb_playlists`
  - `fetch_usb_histories`
  - `remove_usb_playlist`
  - `inspect_usb_track`
  - `run_usb_diagnostics`
  - `repair_usb_diagnostics`
  - `run_usb_parity_report`
  - `detect_external_master_db`
- Analysis:
  - `analyze_new_tracks`
  - `set_analysis_paused`
  - `cancel_analysis`
- Library identity:
  - `materialize_source_track`
  - `resolve_track_identity`
- Playback:
  - `resolve_playback_source`
  - `play_resolved_track`
  - `play_track_native`
  - `stop_playback_native`
  - `get_playback_status_native`
  - `playback_preflight_native`

## Notes
- Data is persisted in SQLite at `<data_dir>/backend.db`.
- Track scanning supports common audio extensions and infers `artist/title` from `Artist - Title.ext` naming.
- UI can pass source scan roots via `scan_library.sourceRoots`.
- PDB parser handles `nrs` u8 wrapping (pages with >255 rows) via two-phase index scan.
- Track metadata is resolved from PDB → eDB → master.db; orphaned playlist entries are skipped.
- Tauri command wrappers emit job lifecycle events:
  - `job.started`
  - `job.progress`
  - `job.completed`
  - `job.failed`
- A combined event stream is also emitted as `job:event` with payload fields:
  - `event`, `jobId`, `jobType`, `stage`, `current`, `total`, `percent`, `message`, `timestamp`
- Native playback is performed by backend Rust using `rodio` and local filesystem paths.
- Playlist add can accept frontend row candidates through `add_track_candidates_to_playlist`;
  the backend resolves safe local identities before mutating playlist membership.

## Test

```bash
cd backend
cargo test
```

Warning contract coverage:
- Backend tests validate the typed warning-contract shape (`level` / `code` / `message` / `source`) under `backend/src/lib.rs`.

## Optional Tauri Integration
`tauri_commands` module is gated behind the `tauri` feature.
See `TAURI_INTEGRATION.md`.
