# App Data Model

This document describes the core entities exchanged between backend commands and UI flows.

For USB database field-level inventories, see `docs/PDB.md` and `docs/eDB.md`.

## Core entities

### Track

Represents a local library track. Core fields include identity, display metadata (title/artist/album), timing metadata (duration, BPM), and analysis/artwork paths.

`isUsbPath` is derived, not stored: every command that returns `Track` rows to the frontend (`list_tracks`, `search_tracks`, `get_tracks_by_ids_with_previews`, `get_playlist_tracks`, `browse_source_files`) computes it fresh via `apply_is_usb_path`, matching `file_path` against every known USB device root in the `usb_devices` registry (including pruned ones — same `untainted_usb_root_paths`/`browse_path_matches_root` logic `resolve_playback_source` already uses for playback safety). This replaced a frontend heuristic that only checked the currently-selected USB root.

`formatExt` is always populated on the wire, never inferred by the frontend. The scanner sets it from the file extension at index time; `row_to_track` falls back to `utils::format_ext_from_path(file_path)` for rows whose column is `NULL` (legacy rows, master.db import, USB placeholder merge). `UsbTrack` carries the same field, derived from the PDB track path. This replaced a frontend `describeTrackFormat` path-inference branch.

### Playlist and PlaylistTrack

`Playlist` is the user-managed container. `PlaylistTrack` stores ordered membership and position within the playlist.

### TrackCue and the beat-grid first beat

The `track_cues` table holds user-editable cue points, one row per cue,
`ON DELETE CASCADE` from `tracks`. This app targets CDJ playback directly, so a
cue is just a position + colour + name — there is no user-facing
memory-vs-hot distinction. **At most 8 cues** per track; on export each cue is
written as *both* a memory point and a hot-cue pad (A–H by position order).

| column | notes |
| --- | --- |
| `position_ms` | cue position from track start |
| `color_id` | palette index (`service::cues::HOTCUE_PALETTE`, 1–8); defaults to green |
| `name` | optional comment/label |
| `sort_order` | insert order |

The beat-grid anchor is `tracks.first_beat_ms` (already present). A companion
`tracks.first_beat_ms_source` column (`'estimated'` default, `'user'` once
edited) tells re-analysis to keep a user-set value instead of overwriting it
with `estimate_first_beat_ms`.

Cues and the first beat are read via `get_track_detail` (returns `TrackDetail`
= `Track` + `firstBeatMs` + ordered `cues` + `detailWaveform`, the raw PWV5
colour-detail waveform for the modal) and replaced atomically via
`save_track_analysis_edits` (`cues: [{ positionMs, colorId, name }]`).
`TrackCue` is **not** on the grid `Track` model — only the per-track modal
fetches it. On save the local ANLZ cache (`.DAT`/`.EXT` at `waveform_peaks_path`)
is rewritten in place (`PQTZ`/`PQT2` + `PCOB`/`PCPT` + `PCO2`/`PCP2`, each cue
doubled to a memory + hot entry) and `last_exported_*` is cleared on every
playlist containing the track. See `docs/USB_EXPORT.md` / `docs/USB_IMPORT.md`
for the USB round-trip.

### UsbDevice, UsbPlaylist, UsbTrack, UsbHistory

These entities represent USB-side discovered state:

- connected USB root and identity
- imported playlist and history metadata
- USB track metadata (including `formatExt`, derived from the PDB path) and optional preview payload fields

`UsbDevice` state is backed by the local `usb_devices` table, keyed by a
normalized root path and carrying mount/first-seen/last-seen state. It
replaces the old frontend-only recent-roots list for device bookkeeping.
`track_usb_links` records which local track row corresponds to a given
USB-device media path, and `usb_device_exports` records per-device playlist
export history even when that drive is not currently mounted.

A `UsbDevice`'s `label` column mirrors the user-assigned drive name, if any
(see `docs/USB_EXPORT.md`'s "USB drive naming" section) — the name of record
is a marker file on the drive itself; `label` is a fast local cache/uniqueness
index over it, kept in sync whenever the drive is (re)named.

### WarningEntry

Typed non-fatal warning/error payload used in diagnostics, import, export, and repair responses.

## Response payload families

Common command payload groups include:

- USB fetch payloads (`FetchUsbPlaylistsData`, `FetchUsbHistoriesData`)
- diagnostics and parity payloads (`RunUsbDiagnosticsData`, `RunUsbParityReportData`)
- repair payloads (`RepairUsbDiagnosticsData`)
- export payloads (`ExportToUsbData`)

## Identity and export linkage

Export maintains app-owned identity values for stable metadata linking:

- app-level `masterDbId`
- app-level `contentLink`
- per-track `masterContentId`

These are intentionally app-owned and persisted locally rather than copied from arbitrary external USB references.

## Minimum relationship expectations

- `PlaylistTrack.playlistId -> Playlist.id`
- `PlaylistTrack.trackId -> Track.id`
- `UsbPlaylist.usbDeviceId -> UsbDevice.id`
- `UsbTrack.usbDeviceId -> UsbDevice.id`
- `UsbHistory.usbDeviceId -> UsbDevice.id`
- `TrackUsbLink.trackId -> Track.id`
- `TrackUsbLink.usbDeviceId -> UsbDevice.id`
- `UsbDeviceExport.usbDeviceId -> UsbDevice.id`

## Settings model

Settings are stored as key/value records in backend storage, with frontend persistence mirroring selected keys for UX continuity.
