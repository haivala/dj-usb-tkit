# Playlists + Playback

## How it works

Playlist management follows a local flow: create playlists, add tracks, keep order by stored playlist position, and remove tracks when needed. The same playlist context remains available while moving between library and USB sections.

Playback is backend-driven. Playback requests include the originating track id
when one is available, regardless of whether the user started playback from
the library, a local playlist, a USB playlist, or USB history. The backend
uses that id first when it points at a genuine local track; otherwise it falls
back to verified metadata matching and can still use the USB source path when
no safe local match exists.

Transport state is pushed through backend events, so the UI reflects start/progress/stop updates without tight polling loops.

## Deep technical details

Playlist state is modeled as local entities with ordered mapping rows. In practice, this means playlist metadata (`name`, identity, timestamps) is stored separately from ordered membership (`playlistId`, `trackId`, position). Track ordering is therefore explicit and stable, which avoids accidental reshuffling during add/remove operations. Adding or removing playlist tracks also clears the playlist's cached USB export status so the UI stops showing stale "exported" state immediately.

The playlist command layer is intentionally CRUD-oriented:

- `create_playlist`, `rename_playlist`, `delete_playlist` change container metadata
- `list_playlists`, `get_playlist_tracks` read container and ordered membership state
- `add_tracks_to_playlist`, `remove_tracks_from_playlist` mutate membership rows

Playlist export is blocked when the selected playlist contains local tracks
under a known missing source root. The user must relocate the source folder or
explicitly remove that source before export proceeds. This prevents a moved or
unmounted music folder from producing an empty or partially empty USB playlist
without an explicit user decision.

Playback architecture is backend-owned so transport behavior stays consistent across views and source types. The frontend requests playback actions, but audio lifecycle state is emitted by backend events. The UI subscribes to those events and updates controls/playhead state from the push stream.

Playback resolution is source-aware:

1. The app passes the originating `trackId` to backend `resolve_playback_source` whenever possible.
2. If that id belongs to a non-USB-rooted local track, the backend resolves directly to that row.
3. If the id is missing or points at a stale USB placeholder, the backend falls through to fingerprint/title matching.
4. USB-rooted candidate rows are excluded from local substitution, so placeholder rows do not masquerade as real local media.
5. If no verified local candidate exists, playback can fall back to USB path playback.

This prevents the common failure mode of matching the wrong local file by loose metadata while still preserving USB playback for unresolved tracks. It also lets older playlist entries that still reference stale USB placeholder rows self-heal on next playback when a genuine local match exists.

Preflight checks (`playback_preflight_native`) and status queries (`get_playback_status_native`) allow the UI to render actionable state before or during transport actions. Stop behavior is explicit via `stop_playback_native`, which normalizes cleanup in both backend and UI.

Implementation anchors:

- playlist command façade: `backend/src/commands.rs`
- playback Tauri handlers: `backend/src/tauri_commands.rs`
- playback event emission: `backend/src/tauri_commands.rs`
- invoke registration: `desktop/src-tauri/src/main.rs`

## Verification links

- Playlist behavior: `backend/tests/user_flow_functional.rs`, `backend/tests/playlist_dedupe_functional.rs`
- Playback behavior: `vanilla-ui/tests/playback_controller.test.mjs`, `vanilla-ui/tests/playback_ui.test.mjs`, `vanilla-ui/tests/playback_resolution_behavior.test.mjs`
