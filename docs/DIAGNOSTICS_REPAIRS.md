# Diagnostics and Repairs

This document describes USB diagnostics, strict parity reports, and repair
actions implemented by this repository.

## How Diagnostics and Repairs Work

Diagnostics read a USB export and report whether the databases, media paths,
analysis references, playlists, and player menu state look usable. Diagnostics
do not write to the USB.

Repairs are separate explicit actions. A repair request can run in preview mode
or apply mode. Preview mode reports proposed fixes, unsupported issues,
estimated writes, and estimated deletes. Apply mode writes only selected fixes,
or the default supported set when no `selectedFixIds` are supplied.

There are two report types:

- `run_usb_diagnostics`: operational USB health and import/export readiness.
- `run_usb_parity_report`: strict PDB/eDB comparison for reproducible player
  compatibility.

Operational diagnostics and strict parity are not the same thing. A USB can be
usable on hardware while strict parity still reports differences between PDB
and eDB.

## Diagnostic Scope

`run_usb_diagnostics` checks these areas:

| Area | What it checks |
| --- | --- |
| PDB integrity | `export.pdb` exists, parses, has expected header/page state, and has parseable playlists/history |
| eDB access | encrypted DB can be opened, required tables can be read, and history counts can be inspected |
| Contents integrity | PDB and eDB indexed media-path sets agree at DB level |
| Analysis integrity | PDB/eDB analysis-path references exist in database rows |
| Playlist resolution | playlist rows resolve to tracks across PDB and eDB |
| Player menu divergence | eDB visible menu categories are compared with PDB `t16` kinds |

Operational diagnostics are deliberately DB-focused. They do not walk every
file in `PIONEER/USBANLZ` or validate every analysis file. Repair preview may
run an explicit ANLZ scan when looking for empty or malformed analysis bundles.

`run_usb_parity_report` does deeper comparison:

| Area | What it checks |
| --- | --- |
| Playlist identity | matching playlist ids across PDB and eDB |
| Playlist membership | tracks only in PDB, tracks only in eDB, and duplicate PDB entries |
| Playlist order | common entries appear in the same order |
| Track metadata | title, artist, album, key, track number, BPM, duration |
| Paths | media path and analysis path parity |
| Artwork | artwork presence on both sides |
| PDB dictionaries | artist, album, key, and artwork ids resolve when linked metadata exists |
| Raw audio coverage | indexed files under `Contents/` exist and extra audio files are reported |
| Reference-only eDB fields | populated documented eDB fields that are reported for reference but outside strict PDB/parity scope |

Strict parity can match tracks by normalized media path, analysis path, metadata
fallback, or id fallback depending on which data is available.

## Repair Flow

`repair_usb_diagnostics` runs operational diagnostics first, then strict parity
preview, then builds a repair catalog from the current findings.

When `apply=false`, no files are changed.

When `apply=true`:

- database backups are created before repair writes;
- selected fixes are applied if `selectedFixIds` is non-empty;
- if `selectedFixIds` is empty, all supported non-optional fixes are selected;
- `sync_edb_history_from_pdb` is optional and is not selected by default;
- strict parity upgrade runs before structural PDB page repairs;
- report commands still remain read-only.

Repair results are returned as applied fixes, skipped fixes, failed fixes,
warnings, estimated writes, and estimated deletes.

## Current Repair Catalog

The current code can propose these repair IDs:

| Repair ID | Applies to | What it does |
| --- | --- | --- |
| `upgrade_export_data_to_strict_parity` | PDB and eDB playlist parity failures | Merges playlists from both databases, preserves membership from both sides, rewrites PDB and eDB through the export writers |
| `fix_empty_analysis_files` | empty USB analysis files with resolvable source audio | Regenerates `DAT/EXT/2EX` bundles for the affected analysis directory |
| `repair_pdb_header_compatibility_field` | PDB header bytes `0x10..0x14` | Writes only that 4-byte field, using the newest compatible local backup value when available or fallback value `5` |
| `repair_pdb_sentinel_u5_on_data_pages` | data pages whose `u5` is sentinel `0x1FFF` | Rewrites `u5` and, only when needed, `num_rl` to the per-table data-page convention |
| `repair_pdb_wrong_page_flags` | data pages with invalid `page_flags` | Patches byte `0x1b` to the accepted value for that table family |
| `repair_pdb_zero_tranrf_on_track_pages` | row-footer groups with active rows and zero `tranrf` | Patches only zero `tranrf` groups; it does not normalize non-zero transaction masks |
| `repair_pdb_wrong_track_u5_num_rl` | invalid active `t00` track-page footer shape | Patches the affected track page footer fields |
| `repair_pdb_wrong_history_page_shape` | `t16`, `t17`, or `t18` pages with `(1, nrs-1)` shape | Changes those pages to `(nrs, 0)` |
| `repair_pdb_stale_sentinel_btree` | sentinel pages with stale B-tree entries | Resets the sentinel B-tree index area to the empty state |
| `repair_pdb_wrong_playlist_tree_shape` | `t07` playlist-tree pages with wrong footer shape | Sets `u5=nrs` and `num_rl=0` |
| `repair_pdb_tombstoned_playlist_tree_ids` | tombstoned `t00` or `t07` slots duplicating active ids | Zeros only the id field in affected tombstoned slots |
| `repair_pdb_t00_multipage_active_pages` | predecessor `t00` pages marked active in a multi-page chain | Sets those pages to sealed flag `0x24` and `(1, nrs-1)` |
| `repair_pdb_ec_data_page_conflict` | table `empty_candidate` pointer aliasing another table's data page | Assigns each conflicting table a new empty candidate beyond the current file tail and updates `next_unused_page` |
| `manual_reimport_unindexed_audio` | audio files under `Contents/` not indexed by PDB/eDB | Guidance-only proposal; no automatic deletion |
| `remove_missing_audio_references` | DB references to audio files missing from USB | Removes eDB content/playlist links and PDB playlist entries only when no unindexed audio drift is present |
| `sync_edb_history_from_pdb` | eDB history counts differ from PDB-derived history payload | Replaces eDB `history` and `history_content` rows from current PDB history data |

## Unsupported and Manual Cases

Some findings intentionally do not have automatic repairs:

| Finding | Behavior |
| --- | --- |
| malformed entries under `PIONEER/USBANLZ` | reported as unsupported; inspect event-log warnings and re-export affected tracks |
| unindexed canonical audio files under `Contents/` | proposed as `manual_reimport_unindexed_audio`, which is guidance-only |
| missing audio references while unindexed audio is also present | `remove_missing_audio_references` becomes preview-only because automatic deletion could remove the wrong DB side first |
| parity preview unavailable | repair preview continues, but strict parity upgrade is not proposed |

## Player Menu Behavior

Diagnostics can report `usb.diagnostics.cdj-menu-divergence` when eDB
`category` has visible menu kinds missing from PDB `t16`.

Normal playlist export preserves PDB `t16`, `t17`, and `t18` and does not
rewrite eDB `menuItem`, `category`, or `sort` menu state.

Menu commands are separate from diagnostics repair:

| Command | Behavior |
| --- | --- |
| `update_usb_cdj_menu_config` | treats eDB `category` as the Active Category source, keeps required player menu kinds visible, writes eDB category rows, and updates the PDB `t17` category snapshot when the visible set changes |
| `sync_usb_cdj_menu_edb_to_pdb` | restores PDB `t16` from the full eDB `menuItem` catalog and updates the PDB `t17` category snapshot |

## Strict Parity Upgrade

`upgrade_export_data_to_strict_parity` uses a collect-merge-write flow:

- parse PDB once;
- read eDB playlists with metadata;
- merge playlist names from both sources;
- preserve existing PDB playlist ids and sort order for matched playlists;
- keep PDB-only playlist members instead of pruning them;
- prefer eDB track metadata when present, falling back to PDB fields;
- write PDB through the current writer;
- re-read PDB playlist identities;
- write eDB using the final playlist identity.

The strict repair path is intended to converge on rerun. It does not compact
PDB tables, rebuild the entire PDB from scratch, or silently remove playlist
membership that exists on only one side.

## Implementation Anchors

| Area | File |
| --- | --- |
| operational diagnostics | `backend/src/service/diagnostics.rs` |
| strict parity report | `backend/src/service/diagnostics.rs` |
| repair catalog and apply flow | `backend/src/service/repair.rs` |
| PDB/eDB field context | `docs/PDB.md`, `docs/eDB.md` |

## Verification

Relevant test areas:

- `backend/tests/diagnostics_functional.rs`
- `backend/tests/export_shape_parity_functional.rs`
- `backend/src/service/diagnostics.rs` unit tests
- `backend/src/service/repair.rs` unit tests
- `vanilla-ui/tests/diagnostics_ui_behavior.test.mjs`
- `vanilla-ui/tests/usb_parity_detail.test.mjs`
