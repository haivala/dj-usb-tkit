# Tauri Integration

This document describes how to wire the `backend` crate into a Tauri host.

Behavior and payload contracts are documented in `docs/COMMANDS.md`.

## 1) Enable backend tauri feature

In your Tauri host `Cargo.toml`:

```toml
backend = { path = "../backend", features = ["tauri"] }
```

## 2) Register required managed state

`backend::tauri_commands` expects these states to be managed:

- `BackendCommands`
- `backend::tauri_commands::EssentiaDownloadCancel`

Recommended startup pattern:

```rust
use std::sync::{Arc, atomic::AtomicBool};

use backend::commands::BackendCommands;

fn main() {
    let data_dir = std::path::PathBuf::from("./app-data");
    let commands = BackendCommands::new(&data_dir).expect("backend init");

    tauri::Builder::default()
        .setup(move |app| {
            backend::tauri_commands::start_playback_event_pump(app.handle().clone(), commands.clone());
            app.manage(commands);
            app.manage(backend::tauri_commands::EssentiaDownloadCancel(
                Arc::new(AtomicBool::new(false)),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Register backend::tauri_commands::* you need here.
        ])
        .run(tauri::generate_context!())
        .expect("tauri run failed");
}
```

## Backend Tauri Commands

Commands currently exposed in `backend/src/tauri_commands.rs`:

- Library:
  - `scan_library`
  - `search_tracks`
  - `list_tracks`
  - `browse_source_files`
  - `materialize_source_track`
  - `remove_tracks_by_source_roots`
  - `get_system_parallelism`
  - `get_tracks_by_ids_with_previews`
- Playlists:
  - `create_playlist`
  - `rename_playlist`
  - `delete_playlist`
  - `list_playlists`
  - `get_playlist_tracks`
  - `add_tracks_to_playlist`
  - `remove_tracks_from_playlist`
- Frontend settings:
  - `get_frontend_settings`
  - `set_frontend_setting`
- USB/export:
  - `set_usb_edb_key`
  - `validate_usb_root`
  - `fetch_usb_playlists`
  - `fetch_usb_histories`
  - `get_usb_cdj_menu_config`
  - `update_usb_cdj_menu_config`
  - `remove_usb_playlist`
  - `inspect_usb_track`
  - `run_usb_diagnostics`
  - `run_usb_parity_report`
  - `repair_usb_diagnostics`
  - `detect_external_master_db`
  - `initialize_usb`
  - `export_to_usb`
- Analysis:
  - `analyze_new_tracks`
  - `analyze_track_piece`
  - `download_essentia`
  - `cancel_essentia_download`
  - `remove_essentia`
- Playback:
  - `resolve_playback_source`
  - `play_track_native`
  - `stop_playback_native`
  - `get_playback_status_native`
  - `playback_preflight_native`

## Event Channels

### Job events

- Direct events: `job:started`, `job:progress`, `job:completed`, `job:failed`
- Aggregated channel: `job:event`

`job:event` payload fields include:

- `event`, `jobId`, `jobType`, `stage`, `current`, `total`, `percent`, `message`, `timestamp`

### Playback events

- Channel: `playback:event`
- Event values include: `playback.started`, `playback.seeked`, `playback.stopped`, `playback.error`

## Scope note

This file is for backend-to-Tauri integration wiring.

For the full command contract (payloads, responses, and semantics), use `docs/COMMANDS.md`.
