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
  - history-family tables (`t17/t18/t19` primary, `t11/t12` fallback)

Track metadata resolution for imported playlist entries is multi-source. The importer attempts to resolve each referenced track ID through PDB row data, then eDB content data, then optional master-DB fallback, and skips unresolvable orphan entries instead of failing the whole playlist import.

The import service deliberately favors responsiveness over eager payload loading. `fetch_usb_playlists` and `fetch_usb_histories` focus on metadata and membership, while expensive payload hydration (waveform preview bytes, artwork data URLs) is deferred. This keeps initial USB imports fast on large drives.

Hydration is explicitly split into two surfaces:

- list commands return track rows with path metadata and optional preview fields often unset
- `inspect_usb_track` performs per-track hydration when UI needs detailed preview data

This split also keeps table scrolling and batch rendering predictable because the UI can hydrate only visible rows rather than all imported rows.

Parser hardening includes compatibility behavior observed in real exports:

- `nrs` row-count wrapping recovery for pages that exceed 255 rows
- `num_rl=8191` sentinel handling where page row count tracking uses fallback rules

These rules are important for robust import on mixed-vendor or older USB content where strict naive parsing would drop rows.

History source selection is explicit. Import prefers PDB `t17/t18` history tables and only falls back to `t11/t12` when `t17/t18` are empty. This avoids mixing incompatible history-table families and makes history provenance predictable.

Corruption tolerance is intentional: unreadable optional analysis artifacts should produce warnings rather than aborting whole playlist/history imports. The command returns partial-but-usable data whenever safe to do so.

Implementation anchors:

- playlist import: `backend/src/service/usb.rs:368`
- history import: `backend/src/service/usb.rs:667`
- per-track hydration: `backend/src/service/usb.rs:1257`
- parser compatibility logic: `backend/src/pdb_reader.rs`

## Verification links

- USB import workflows: `backend/tests/export_functional.rs`, `backend/tests/user_flow_functional.rs`
- Frontend USB flows: `vanilla-ui/tests/usb_workflows.test.mjs`, `vanilla-ui/tests/e2e/usb_flows.spec.mjs`
- Parser compatibility coverage: `backend/src/pdb_reader.rs` tests (wrapping and sentinel handling)
