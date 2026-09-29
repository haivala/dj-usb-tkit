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
- `api_client.mjs`: Tauri command wrapper
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
- `add_track_candidates_to_playlist`
- `remove_tracks_from_playlist`
- `reorder_playlist_tracks`
- `resolve_track_identity`
- `resolve_playback_source`
- `play_resolved_track`
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

The frontend runs against the Tauri backend. When the Tauri runtime is unavailable it
falls back to `window.__TAURI__.core.invoke` if a host injects one (the Playwright e2e
suite does this per spec); otherwise every command returns an `INTERNAL_ERROR` envelope.

Key behavior:

- playlists open as tabs with their own track views;
- Library, USB, and History views can add tracks into the current playlist;
- add-to-playlist sends row candidates to the backend instead of resolving local IDs in JS;
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

## Docs media

The GIFs and screenshots in `docs/assets/` (used by `docs/CUE_EDITOR.md` and
the top-level README) are recorded from the real frontend build: the three
`cue-editor-*.gif`, `DJ-USB-Tkit.png` and `cue-editor.png`. Re-record them
after a UI change:

```bash
npm run docs:media --prefix vanilla-ui
```

It needs `cargo` and `ffmpeg`. The script synthesises a made-up library (10
tracks on 2 albums in two source folders), scans and analyzes it through the
real backend (`dump_doc_gif_fixture` bin), and drives the app in headless
Chromium with Tauri's invoke answered from that data, plus made-up playlists
and a connected USB named "Chiphead". To record only some, name them:
`npm run docs:media --prefix vanilla-ui -- cue-editor cue-editor-drag-cues`.
See `scripts/doc-media/record.mjs`.
