# App Data Model

This document describes the core entities exchanged between backend commands and UI flows.

For USB database field-level inventories, see `docs/PDB.md` and `docs/eDB.md`.

## Core entities

### Track

Represents a local library track. Core fields include identity, display metadata (title/artist/album), timing metadata (duration, BPM), and analysis/artwork paths.

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
