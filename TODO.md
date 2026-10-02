# TODO

## Slow USB playlist loading on Windows (Defender)

On Windows, opening a USB playlist that hasn't been opened before can take
minutes. Resource Monitor shows `MsMpEng.exe` (Windows Defender real-time
protection) as the main reader of the stick: Defender scans every file the
app opens. Each page load opens, per track, the `.EXT` analysis file
(~130 KB) for the row waveform and the artwork file, so a 28-track playlist
means ~56 scans. Playlists already opened load instantly (files already
scanned). Linux is unaffected.

Fixes, in order:

1. **Local cache of what is read from the stick.** Save each track's waveform
   preview and artwork data locally (app data dir, per device), keyed by the
   file's path, size and mtime (stat doesn't trigger a scan, opening does).
   Browsing a stick then opens each file at most once, even across restarts.
   Hook: `paginate_and_hydrate_usb_tracks` / `hydrate_usb_track_in_place` in
   `backend/src/service/usb.rs`.
2. **Use the library's own data for tracks you already have.** Page loads
   already find the matching local track (`resolve_usb_track_page_local_ids`).
   When there is one, take waveform and artwork from the app's own files
   instead of the stick. Covers most tracks on sticks exported by this app,
   even on the first visit. Resolve local ids before hydrating.
3. **Only if 1 and 2 aren't enough:** read the preview from `.DAT` (~8 KB)
   instead of `.EXT`. Its `PWAV` preview has 400 points vs the 2400 used now,
   so row waveforms would look coarser.

The per-page timing and slow-file lines in the Event Log
(`fetch_usb_playlist_tracks: ...`) show the effect on Windows.
