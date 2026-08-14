# App Data Model

This document describes the core entities exchanged between backend commands and UI flows.

For USB database field-level inventories, see `docs/PDB.md` and `docs/eDB.md`.

## Core entities

### Track

Represents a local library track. Core fields include identity, display metadata (title/artist/album), timing metadata (duration, BPM), and analysis/artwork paths.

`isUsbPath` is derived, not stored: every command that returns `Track` rows to the frontend (`list_tracks`, `search_tracks`, `get_tracks_by_ids_with_previews`, `get_playlist_tracks`, `browse_source_files`) computes it fresh via `apply_is_usb_path`, matching `file_path` against every known USB device root in the `usb_devices` registry (including pruned ones — same `untainted_usb_root_paths`/`browse_path_matches_root` logic `resolve_playback_source` already uses for playback safety). This replaced a frontend heuristic that only checked the currently-selected USB root.

### Playlist and PlaylistTrack

`Playlist` is the user-managed container. `PlaylistTrack` stores ordered membership and position within the playlist.

### UsbDevice, UsbPlaylist, UsbTrack, UsbHistory

These entities represent USB-side discovered state:

- connected USB root and identity
- imported playlist and history metadata
- USB track metadata and optional preview payload fields

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
