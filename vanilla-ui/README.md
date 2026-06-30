# Frontend

This directory contains the framework-free frontend for DJ USB Tkit. It is
plain HTML, CSS, and JavaScript, bundled with `esbuild` for the Tauri desktop
host.

The build output is written to `vanilla-ui/dist/`, which is loaded by
`desktop/src-tauri`.

## Main Files

- `index.html`: application shell, panels, dialogs, and static templates
- `styles.css`: frontend styling
- `main.js`: application bootstrap and cross-component orchestration
- `api_client.mjs`: Tauri command wrapper and browser mock command layer
- `app_state.mjs`: initial state and shared state constructors
- `message_bus.mjs`: centralized status, progress, and event-log message routing
- `event_log.mjs`: event-log normalization and coalescing store
- `job_manager.mjs`: backend job-event handling
- `track_table.mjs`: shared track table rendering
- `track_utils.mjs`: track formatting, filtering, and normalization helpers
- `ui_controller.mjs`: top-level view and shell state helpers
- `waveform.mjs`: waveform color derivation and canvas rendering
- `components/`: feature-specific action and event modules
- `scripts/build.mjs`: frontend bundle and static asset staging

## Backend Command Contract

The UI invokes backend commands through Tauri. Core command names include:

- `scan_library`
- `search_tracks`
- `list_tracks`
- `create_playlist`
- `rename_playlist`
- `delete_playlist`
- `list_playlists`
- `get_playlist_tracks`
- `add_tracks_to_playlist`
- `remove_tracks_from_playlist`
- `resolve_playback_source`
- `play_track_native`
- `stop_playback_native`
- `fetch_usb_playlists`
- `fetch_usb_histories`
- `validate_usb_root`
- `initialize_usb`
- `export_to_usb`
- `remove_usb_playlist`
- `inspect_usb_track`
- `run_usb_diagnostics`
- `run_usb_parity_report`
- `repair_usb_diagnostics`

The Tauri host also provides app-shell helpers:

- `pick_source_folders`
- `pick_usb_folder`
- `allow_asset_paths`
- `append_frontend_log`
- `clear_frontend_log`
- `get_backend_log_buffer`
- `show_window`
- `set_theme_background`

For the full backend command contract, see `docs/COMMANDS.md`.

## Behavior

If the Tauri runtime is unavailable, the frontend uses local mock data so the UI
can be exercised in a browser. USB export is intentionally excluded from browser
mocks and requires the Tauri backend runtime.

Key behavior:

- playlists open as tabs with their own track views;
- Library, USB, and History views can add tracks into the current playlist;
- status and event-log messages flow through `message_bus`;
- track views share a table layout with cover and waveform preview columns;
- USB/history tracks hydrate waveform, artwork, BPM, and key metadata lazily;
- source folders are persisted and can be enabled, disabled, removed, or cleared;
- playback UI updates are event-driven through Tauri `playback:event`;
- frontend Tauri integration uses `@tauri-apps/api/core` and
  `@tauri-apps/api/event`.

## Build

Build the frontend bundle before running the Tauri shell directly:

```bash
npm run build --prefix vanilla-ui
```

From inside this directory:

```bash
npm run build
```

## Tests

Run unit and behavior tests:

```bash
npm run test:unit --prefix vanilla-ui
```

Run Playwright end-to-end tests:

```bash
npm run test:e2e --prefix vanilla-ui
```

Run the full frontend suite:

```bash
npm test --prefix vanilla-ui
```

Playwright tests live under `vanilla-ui/tests/e2e/`. The test suite covers core
rendering, command wiring, playlist workflows, source-root filtering, USB flows,
diagnostics/repair UI behavior, playback state, event-log behavior, and
message-routing contracts.
