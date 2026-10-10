# Engine DJ library

Engine OS players (Denon Prime, Numark Mixstream) read their own library at
`Engine Library/Database2/` on the USB. When it's missing, they import
`export.pdb` themselves, but only the metadata, playlists and hot cues. That
import has no beat grid or waveform, so the player analyzes every track again
when it loads it, using its own grid.

With Settings → Export → "Write Engine DJ library" on, the app rebuilds that
library whenever it changes the stick's rekordbox data: export, playlist
reorder or removal, cue edits, backup restore, repairs and player-menu changes.
The setting is off by default. Health & Diagnostics shows an "Engine DJ Library"
section. While the setting is off, a stick without a library gets a passing
note to turn it on before using the stick in an Engine player. A stick that
already has one gets a warning when the setting is off or the library is
behind `export.pdb`. The `keep_engine_library_up_to_date` fix
turns the setting on and rebuilds it. A stick's first library therefore comes
from the player's own (slow) import unless the setting was already on.
`write_engine_db` does the same rebuild from the command line. Both build it
from what is already on the stick:
tracks and playlists from `export.pdb`, and beat grid, cues and waveform from each
track's ANLZ bundle, plus artwork from `PIONEER/Artwork`. The player then
uses our grid and skips analysis.

```sh
cargo run -p backend --bin write_engine_db -- /run/media/$USER/USB
```

Code: `backend/src/engine_db.rs` (format) and
`backend/src/service/engine_export.rs` (reading the USB, `rebuild_engine_library`).

## Files

| File | Written | Notes |
|---|---|---|
| `m.db` | rebuilt every run | the library |
| `hm.db` | only when missing or not schema 3.0.2 | play history; kept otherwise |
| `sm.db`, `stm.db` | only when missing or not schema 3.0.2 | empty as the device leaves them |
| `OverviewData/<uuid>/` | created for m, hm and stm | empty, as on a device-written stick |

The schema is **3.0.2**: `PRAGMA user_version = 4194305` and page size 4096.
`backend/src/engine_schema/*.sql` is the `sqlite_master` DDL of a library a
Mixstream Pro wrote, verbatim. A fresh database built from it has a `.schema`
identical to the device's.

`m.db` is built as `m.db.tmp` and renamed into place. Any `m.db-journal` is
removed first, because SQLite would otherwise replay it onto the new file.

## Things the player checks

- **Re-import counter.** `Information.lastRekordBoxLibraryImportReadCounter`
  and `Track.pdbImportKey` hold the `export.pdb` header sequence (u32 LE at
  offset 20). The player imports the PDB again when the sequence on the stick
  differs from the counter. Run `write_engine_db` after every export so they match.
- **Track ids are never reused.** A trigger aborts inserting an id at or below
  `sqlite_sequence`. A rebuild keeps the previous `m.db`'s `Information.uuid`
  and each track's id (matched by path), and new tracks get ids above the old
  high-water mark. History in `hm.db`, which points at (uuid, track id), stays valid.
  Play state (`isPlayed`, `timeLastPlayed`, `playedIndicator`) is carried over.
- **Paths** are relative to `Engine Library/`: `../Contents/...`.
- **Playlists** are linked lists. `Playlist.nextListId` points at the next
  sibling (0 = last), and `PlaylistEntity.nextEntityId` points at the next
  track (0 = last; the first is the entity nobody points to). This matches the
  device's import of the same PDB exactly. Folders are playlists with children.
  Titles must be unique per parent, so duplicates get ` (2)`, ` (3)` and so on.
  The `PlaylistPath` view numbers siblings back to front; that is how the device's own
  data reads too.

## PerformanceData blobs

`qCompress` = u32 BE uncompressed length + zlib stream.

| Column | Encoding | Layout |
|---|---|---|
| `trackData` | qCompress | f64 BE sample rate, i64 BE sample count, f64 BE average loudness (0.5), i32 BE key |
| `beatData` | qCompress | f64 BE sample rate, f64 BE sample count, u8 1, then the default and adjusted grid (same), each i64 BE count + markers *(f64 LE sample offset, i64 LE beat number, i32 LE beats to next marker, i32 LE 0)* |
| `overviewWaveFormData` | qCompress | i64 BE 1024, f64 BE samples per entry, 1024 × (u8 low, mid, high), then the per-band maximum |
| `quickCues` | qCompress | i64 BE 8, 8 × (u8 label length, UTF-8 label, f64 BE sample offset or -1, ARGB), f64 BE adjusted main cue, u8 0, f64 BE default main cue |
| `loops` | raw, LE | i64 8, 8 × (u8 label length, label, f64 start, f64 end, u8 start set, u8 end set, ARGB) |

The empty `quickCues` and `loops` blobs, and the two-cue blob of a test track,
match the device's byte for byte (unit tests in `engine_db.rs`).

Mapping from the USB:

- **Beat grid:** from the `.DAT`'s `PQTZ`. There is one marker per tempo change. The first
  marker is moved back to at or before sample 0, and a closing marker is placed at or after the
  end of the track. Engine beat numbers start at 0 on a downbeat, so a first beat that is
  rekordbox bar position *n* gets beat number *n − 1*.
- **Overview:** from the `.EXT`'s `PWV4` (1200 entries; bytes 3/4/5 are
  low/mid/high, 0–127), as the per-bin maximum over 1024 bins, × 2.
- **Hot cues:** the first 8 hot cues in order onto pads 1–8. The label is
  the cue name, else `Cue N` as the device names them. Positions are
  `ms × rate / 1000`, unrounded, as the device converts them. The colour is the palette RGB
  with alpha `ff`. The device snaps red (`e12424` → `e12525`) and blue (`2a5bd8` →
  `2f5fd9`) to its own palette, and those use its values.
- **Main cue:** the playback-start cue, else the first memory cue, else the
  first beat. The device's own import leaves it unset.
- **Key:** `Track.key` 0..23 goes round the circle of fifths: C = 0, Am = 1, G = 2,
  … F = 22, Dm = 23. It is derived from the Camelot position (`(n + 4) mod 12 × 2`, + 1
  for minor) and matches the device's import on all 297 tracks of the test stick.
- **Rating:** stars × 20. **dateAdded:** the PDB date, UTC midnight, in Unix seconds.
- **Artwork:** the `_m` JPEG (else the small one) as stored, one `AlbumArt`
  row per PDB artwork. Row 1 is an empty placeholder for tracks without art.

A track with no ANLZ bundle, or with an empty `PQTZ`, gets `isAnalyzed = 0` and
no `trackData`/`beatData`/overview. The player analyzes it itself.

## Not written

Loops (the ANLZ we write has none), smart lists, the prepare list, and
`averageLoudness` (written as 0.5).

## Hardware

Works on a Numark Mixstream Pro (2026-10-09), with `isMetadataImported = 1`
and `pdbImportKey` = PDB sequence (the device's own import writes 0 and the
sequence).

Still to check: history in `hm.db` surviving a re-run after another export.
