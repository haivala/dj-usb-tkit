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
- `list_playlists` reads container metadata; `get_playlist_tracks` reads ordered
  membership, paginated/searched/sorted server-side (see the "Paginated track
  lists" envelope in `docs/COMMANDS.md`; `limit: 0` returns the whole playlist).
  The whole-playlist `totalDurationMs` is computed over the filtered set, not the
  page.
- `add_track_candidates_to_playlist` resolves frontend row candidates and delegates to `add_tracks_to_playlist`
- `add_tracks_to_playlist`, `remove_tracks_from_playlist` mutate membership rows
- `reorder_playlist_tracks` persists a new track order to `playlist_tracks.position`,
  in one of three modes: `orderedTrackIds` (the complete order, must match the
  current set), `sortBy`/`sortDir` (server sorts the whole playlist by that
  column — the same comparator `get_playlist_tracks` uses — and persists it;
  this is how a client column sort is committed on navigate-away / export), or
  `moveTrackId` + optional `beforeTrackId` (reposition one track relative to a
  neighbour, so a paginated client can drag-reorder without holding every row).

Playlist export is blocked when the selected playlist contains local tracks
under a known missing source root. The user must relocate the source folder or
explicitly remove that source before export proceeds. This prevents a moved or
unmounted music folder from producing an empty or partially empty USB playlist
without an explicit user decision.

Track rows within an app playlist can be reordered by dragging. A column sort
is a reversible view op while browsing (it re-queries the page sorted, like
every other list) and only becomes the playlist's persisted order when
committed — on navigate-away or export — via `reorder_playlist_tracks`'s
`sortBy` mode. Dragging while sorted is allowed; the drop clears the sort and
sends a single-move. Dragging is disabled while a search filter is active
(the rendered rows are a filtered subset). It is also locked when the current
export sync mode is additive
(`pruneStale = false`, see `docs/USB_EXPORT.md`) and a same-named playlist
already exists on the connected USB, since an additive export never rewrites
the order of entries already on the device — a reorder there wouldn't be
reflected on next export. That lock condition is computed once, server-side,
as `locks_reorder` on `PlaylistUsbExportStatus`
(`backend/src/service/export.rs`, `compute_playlist_usb_export_status`) and
returned alongside `fetch_usb_playlists`/`run_usb_diagnostics` rather than
re-derived per UI spot from raw state; the same status also backs the Export
button's "Append to..." label.

Playback architecture is backend-owned so transport behavior stays consistent across views and source types. The frontend requests playback actions, but audio lifecycle state is emitted by backend events. The UI subscribes to those events and updates controls/playhead state from the push stream.

Playback resolution is source-aware:

1. The app starts playback with `play_resolved_track`, passing the originating `trackId` whenever possible.
2. The backend uses `resolve_playback_source`; if that id belongs to a non-USB-rooted local track, it resolves directly to that row.
3. If the id is missing or points at a stale USB placeholder, the backend falls through to fingerprint/title matching.
4. USB-rooted candidate rows are excluded from local substitution, so placeholder rows do not masquerade as real local media.
5. If no verified local candidate exists, the backend can fall back to USB path playback when the path is under the selected USB root.

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
