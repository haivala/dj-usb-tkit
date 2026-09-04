# USB Import

## How it works

USB import reads USB database data from the selected USB root and builds one in-app view of playlists, histories, and tracks. When possible, the import path tolerates incomplete or corrupted USB metadata and returns warnings instead of failing the entire operation.

For large USB libraries, import is metadata-first. Playlist and history lists can load without reading full waveform/artwork payload bytes for every track. Rich track previews are hydrated on demand when the UI requests them.

## Deep technical details

USB import is implemented as a merge pipeline across multiple database representations, not as a single-file parser. The backend reads from PDB and eDB, and can use the master DB as an additional source when available. The merge stage resolves conflicts and missing fields into one API payload per playlist/history/track set.

For field-level structure details, see `docs/PDB.md` and `docs/eDB.md`.

Known DB data surfaces used by import:

- From eDB:
  - playlist container rows (`playlist`)
  - ordered playlist membership rows (`playlist_content`)
  - track metadata rows (`content`)
  - artwork path linkage (`image` via `content.image_id`)
  - dictionary metadata such as artist/album/key IDs and labels when present
  - history rows (`history`, `history_content`) when available
- From PDB:
  - track rows (title/artist/album/key linkage, tempo, duration, media path, analysis path)
  - playlist tree and playlist-entry tables (playlist structure and order)
  - artwork and dictionary ID references used by track rows
  - history-family tables (`t11/t12`, `t17/t18`, and `t19`)

Track metadata resolution for imported playlist entries is multi-source. The importer attempts to resolve each referenced track ID through PDB row data, then eDB content data, then optional master-DB fallback, and skips unresolvable orphan entries instead of failing the whole playlist import.

### Cue points and beat grid on import

When `materialize_usb_track_row` creates or matches a local `tracks` row for a
USB track, `import_anlz_cues_for_track` reads cue points from the on-USB
`.EXT` (preferred, for colour + comment) or `.DAT` (`read_cues_from_anlz`) and
seeds `track_cues`, and seeds `tracks.first_beat_ms` from the `PQTZ`/`PQT2`
anchor. The app writes each cue as a memory + hot pair on export, so entries are
**deduped by position** back into one `track_cues` row (cap 8). This only
happens when the local track has **no cues yet** and (for the first beat) **no
value yet** — local edits always win over what is on the stick, so re-importing
a stick you exported never clobbers newer local work.

`detect_external_master_db` locates the local rekordbox `master.db` by checking
a fixed candidate list in order and returning the first path that exists:

1. `$DJUSBTKIT_MASTER_DB_PATH` env override, when set (used for tests/debugging)
2. macOS: `~/Library/Application Support/Pioneer DJ/rekordbox/master.db`
3. macOS: `~/Library/Application Support/Pioneer/rekordbox/master.db` (older installs)
4. macOS: `~/Library/Pioneer/rekordbox/master.db` (current rekordbox installs, which
   store the db directly under `Library` rather than `Application Support`)
5. Windows: `%APPDATA%/Pioneer/rekordbox/master.db`
6. Windows: `%USERPROFILE%/AppData/Roaming/Pioneer/rekordbox/master.db`

There is no Linux candidate: rekordbox has no official Linux install, so no
real install path exists to check there.

See `external_master_db_candidates` in `backend/src/service/usb_utils.rs`.

The import service deliberately favors responsiveness over eager payload loading. `fetch_usb_playlists` and `fetch_usb_histories` resolve the playlist/history **list** and its counts, not per-track payloads. Expensive per-track hydration (waveform preview bytes, artwork data URLs) is deferred and paid one page at a time.

Hydration is split into two surfaces, both server-side:

- `fetch_usb_playlists` / `fetch_usb_histories` return the list of playlists/sessions with
  metadata + counts.
- `fetch_usb_playlist_tracks` / `fetch_usb_history_tracks` return one
  paginated/searched/sorted page of a selected list's tracks, with waveform/artwork **hydrated
  for that page only** (see the "Paginated track lists" envelope in `docs/COMMANDS.md`). The
  frontend renders the page directly.

This keeps table scrolling predictable because only the visible page pays the hydration I/O,
and filtering/sorting run over the whole list server-side rather than only the loaded rows.

`inspect_usb_track` (singular) survives for the resolve-one-row-before-playback path.
`inspect_usb_tracks` (batch) is retained for compatibility but the paginated fetch commands
have replaced its use as the list-selection hydration path.

Parser hardening includes compatibility behavior observed in real exports:

- `nrs` row-count wrapping recovery for pages that exceed 255 rows
- `num_rl=8191` sentinel handling where page row count tracking uses fallback rules

These rules are important for robust import on mixed-vendor or older USB content where strict naive parsing would drop rows.

History source selection is explicit. Import keeps the two history-table
families separate and selects one family for a read: `t11/t12` is used
whenever either table is populated, otherwise import uses `t17/t18`. This
avoids mixing incompatible history-table families and prevents seeded
`t17/t18` template rows from surfacing as real user history on initialized or
freshly exported drives.

An imported history session can be exported as a plain-text tracklist
("Export Tracklist" on the USB History panel), with a choice of which track
in the session the list starts from and an option to include estimated
cumulative per-track times, placed before or after each `Artist - Title`
line. These times are always an estimate — summed from track `durationMs`
in the session's play order starting from the chosen track — because CDJs
never record a per-track playback timestamp in the USB at all: neither PDB
history entry rows (`t11/t12` or `t17/t18`) nor eDB
(`history`/`history_content`) carry any
per-track timestamp field. Even the session-level date shown elsewhere in
the History panel isn't guaranteed to be a real hardware-recorded value —
PDB `t19` rows can carry a date per history session when the CDJ wrote one,
but when that's absent the app estimates it from the session's own tracks'
`date_added` metadata instead (`apply_history_dates_from_track_date_created`
in `backend/src/service/usb.rs`). This export is pure frontend formatting
over already-imported `UsbHistory` data
(`vanilla-ui/track_utils.mjs::buildTracklistText`); it does not read PDB/eDB
again and is unrelated to the USB export/write pipeline.

Corruption tolerance is intentional: unreadable optional analysis artifacts should produce warnings rather than aborting whole playlist/history imports. The command returns partial-but-usable data whenever safe to do so.

Implementation anchors:

- playlist import: `backend/src/service/usb.rs`
- history import: `backend/src/service/usb.rs`
- per-track/batched hydration: `backend/src/service/usb.rs`
- parser compatibility logic: `backend/src/pdb_reader.rs`

## Verification links

- USB import workflows: `backend/tests/export_functional.rs`, `backend/tests/user_flow_functional.rs`
- Frontend USB flows: `vanilla-ui/tests/usb_workflows.test.mjs`, `vanilla-ui/tests/e2e/usb_flows.spec.mjs`
- Parser compatibility coverage: `backend/src/pdb_reader.rs` tests (wrapping and sentinel handling)
