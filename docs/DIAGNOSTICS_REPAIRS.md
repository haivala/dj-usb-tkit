# Diagnostics and Repairs

This document describes USB diagnostics, strict parity reports, and repair
actions implemented by this repository.

## How Diagnostics and Repairs Work

Diagnostics read a USB export and report whether the databases, media paths,
analysis references, playlists, and player menu state look usable. Diagnostics
do not write to the USB.

![Health & Diagnostics: the USB's databases checked area by area](assets/usb-diagnostics.png)

The report opens with one plain-language line per player family and area. The
individual checks sit under a collapsed **Technical details**, grouped into
Databases (PDB, eDB and Engine DJ side by side), Library and Low-level:

![Health & Diagnostics: a summary per player, and the checks under Technical details](assets/usb-diagnostics-details.png)

Repairs are separate explicit actions. A repair request can run in preview mode
or apply mode. Preview mode reports proposed fixes, unsupported issues,
estimated writes, and estimated deletes. Apply mode writes only selected fixes,
or the default supported set when no `selectedFixIds` are supplied.

There are two report types:

- `run_usb_diagnostics`: operational USB health and import/export readiness.
- `run_usb_parity_report`: strict PDB/eDB comparison for reproducible player
  compatibility.

In the app, both live on the USB Browser page: once a USB is selected,
**Rediagnose USB**, **Parity Report** and **Preview Fixes** sit next to the
page title, and the report shows under Health & Diagnostics (it scrolls; the
title row stays). The Parity Report opens with a short explanation of what it
compares (the legacy PDB and the new eDB) and what a FAIL means. Preview Fixes
replaces the report with the proposed fixes, each with its own checkbox, and
**Apply Fixes** runs the checked ones.

Operational diagnostics and strict parity are not the same thing. A USB can be
usable on hardware while strict parity still reports differences between PDB
and eDB.

## What Strict Parity Means

A USB export is really two separate databases that both have to be trusted:
`export.pdb` (legacy binary, read as the primary database by older CDJ
hardware) and `exportLibrary.db`/eDB (encrypted SQLite, read as the primary
database by the CDJ-3000; desktop DJ software validates both). See
`docs/PDB.md` "How PDB Works" and `docs/eDB.md` "How eDB Works" for why the
app keeps both instead of one.

Operational diagnostics (`run_usb_diagnostics`) only asks whether *each*
database, taken on its own, looks structurally sound and playable. Strict
parity (`run_usb_parity_report`) asks a stricter, narrower question: does
eDB's view of the library — playlists, membership, order, per-track
metadata, media/analysis paths, dictionary-id resolution — match PDB's view,
field for field. See `## Diagnostic Scope` below for exactly what each report
checks.

These can disagree. A USB can play back correctly on the exact hardware you
tested (passing operational diagnostics) while still failing strict parity —
for example, eDB has a playlist entry PDB doesn't, or a track's BPM differs
between the two files. That gap might not visibly break playback on the one
player you tried, but it is a real divergence: since different players treat
different files as authoritative, the same USB can behave differently
depending on which one it lands on. Strict parity exists to catch that class
of problem before it surfaces as a hardware-specific bug report.

Strict parity is read-only as a report. `upgrade_export_data_to_strict_parity`
is the separate, explicit repair action that reconciles differences by
merging and rewriting both databases to match — see "Repair Flow" below.

## Diagnostic Scope

`run_usb_diagnostics` checks these areas:

| Area | What it checks |
| --- | --- |
| PDB integrity | `export.pdb` exists, parses, has expected header/page state, and has parseable playlists/history |
| eDB access | encrypted DB can be opened, required tables can be read, and history counts can be inspected |
| Contents integrity | PDB and eDB indexed media-path sets agree at DB level, matched by the same path key as the parity report's raw audio coverage |
| Analysis integrity | PDB/eDB analysis-path references exist in database rows |
| Shared analysis files (under Analysis integrity) | Database-only: warns when several PDB track rows point at the same analysis bundle, so players can show one track's waveform and beat grid for another. The Event Log lists each bundle and its tracks. Points to `separate_shared_analysis`; the repair preview also reads each bundle's `PPTH` (see "Analysis file ownership") |
| Beat grid format (under Analysis integrity) | Always shown. Warns about tracks whose latest export on the stick was written by an app version before 0.3.7 (an export-log record without `appVersion`) and whose `.DAT` still has the old `PQTZ` header. Only those tracks' `.DAT` files are read, and only their first 4 KB. Passes with "no tracks on this USB were exported by this app" when the stick has no export log. When the log can't be read, every track's `.DAT` is checked (first 4 KB each) instead. Points to `fix_beat_grid_header` or a re-export |
| Playlist resolution | playlist rows resolve to tracks across PDB and eDB |
| Player menu divergence | eDB visible menu categories are compared with PDB `t16` kinds |
| Engine DJ library | Without `Engine Library/Database2/m.db`: a passing "Not on this USB" note while "Write Engine DJ library" is off, saying to turn it on before using the USB in an Engine player; nothing while it's on. With one: warns when "Write Engine DJ library" is off, and when `m.db`'s `lastRekordBoxLibraryImportReadCounter` differs from the `export.pdb` header sequence (e.g. after an export from rekordbox itself). Points to `keep_engine_library_up_to_date` |

Operational diagnostics are deliberately DB-focused. They do not walk every
file in `PIONEER/USBANLZ` or validate every analysis file. Repair preview may
run an explicit ANLZ scan when looking for empty or malformed analysis bundles.

`run_usb_parity_report` does deeper comparison:

| Area | What it checks |
| --- | --- |
| Playlist identity | matching playlist ids across PDB and eDB |
| Playlist membership | tracks only in PDB, tracks only in eDB, and duplicate PDB entries |
| Playlist order | common entries appear in the same order within a playlist, and playlist sibling order (PDB `t07.sort_order`) matches eDB `playlist.sequenceNo`; both dimensions are reported together as "Playlist ordering parity" |
| Track metadata | title, artist, album, key, track number, BPM, duration |
| Paths | media path and analysis path parity |
| Artwork | artwork presence on both sides |
| PDB dictionaries | artist, album, key, and artwork ids resolve when linked metadata exists |
| Raw audio coverage | "Indexed audio file presence": every path a PDB track row or eDB `content` row holds exists under `Contents/`, and every audio file there is held by one of them. Paths match exactly except for a trailing space before the extension and letter case (FAT32/exFAT); a `-1`/`-2` suffix makes a different file, since rekordbox can write a track twice as `Track.mp3` and `Track-1.mp3`. The repair's audio fixes use the same scan |
| Rating and colour | a playlist track's star rating and colour match between the eDB and its PDB row (bytes 89/88). A mismatch is a warning: the "Track details" row and the overall parity status show Warn, not Fail. The playlist is counted for "Upgrade Export Data To Strict Parity", which carries the eDB's values into the PDB |

Every finding of the parity report has a repair or an entry under
"Unsupported and Manual Cases", and one pass of the default fixes leaves the
report clean.

Strict parity can match tracks by normalized media path, analysis path, metadata
fallback, or id fallback depending on which data is available.

## Repair Flow

`repair_usb_diagnostics` runs operational diagnostics first, then strict parity
preview, then builds a repair catalog from the current findings.

When `apply=false`, no files are changed.

![Preview Fixes for a renamed audio file, an empty analysis file and a playlist the PDB and eDB disagree on](assets/usb-repair-preview.png)

When `apply=true`:

- database backups are created before repair writes;
- selected fixes are applied if `selectedFixIds` is non-empty;
- if `selectedFixIds` is empty, all supported non-optional fixes are selected;
- `sync_edb_history_from_pdb` and `keep_engine_library_up_to_date` are optional and are not selected by default;
- `repair_pdb_truncated_table_chain` and `repair_pdb_torn_growth_pages` both run
  *before* strict parity upgrade — `repair_pdb_truncated_table_chain` because
  additive track appends hard-fail while a table's chain is unreachable, and
  `repair_pdb_torn_growth_pages` because its dirty-tail truncation boundary is
  computed once, up front, and would otherwise go stale and cut off
  tracks/playlists that strict parity had just written;
- `repair_pdb_t19_template_state` runs right after those two and before strict
  parity upgrade, so strict parity's additive write finds `t19` already counting
  the tracks and leaves it alone; all other structural PDB page repairs run *after* strict
  parity upgrade;
- the desktop UI's repair preview locks these two fixes' checkboxes checked
  whenever they're proposed — they cannot be deselected while leaving strict
  parity selected, since there is no safe outcome from skipping either one;
- rekordbox bookkeeping fixes (see below) are added to every apply, whatever
  `selectedFixIds` holds;
- report commands still remain read-only.

Repair results are returned as applied fixes, skipped fixes, failed fixes,
warnings, estimated writes, and estimated deletes.

## Current Repair Catalog

The current code can propose these repair IDs:

| Repair ID | Applies to | What it does |
| --- | --- | --- |
| `upgrade_export_data_to_strict_parity` | PDB and eDB playlist parity failures, including playlists whose only difference is artwork presence or a track's rating/colour | Merges playlists from both databases, preserves membership from both sides, rewrites PDB and eDB through the export writers, removes stale duplicate PDB playlist-entry rows, and syncs eDB `sequenceNo` from PDB `t07.sort_order` |
| `fix_empty_analysis_files` | empty USB analysis files with resolvable source audio | Regenerates `DAT/EXT/2EX` bundles for the affected analysis directory, baking in the track's known PDB tempo/duration (the source file's own length when the PDB has none; a track with no known length is reported as failed). With no known tempo the bundle gets no beat grid |
| `separate_shared_analysis` | analysis bundles used by more than one track, or whose `PPTH` names another track on the stick (see "Analysis file ownership") | The owner keeps the bundle and gets its `PPTH`. Every other track gets the next free `ANLZnnnn` in its own hash folder, analysed from its audio with its PDB tempo and length (plus the MP3/FLAC seek index), and its PDB row and eDB `analysisDataFilePath` point there. A track whose audio can't be analysed fails the fix, after the others are done. Marked destructive: the cues in the shared bundle stay with the owner. Runs before `fix_bpm_key_mismatch` |
| `add_analysis_track_path` | bundles only one track uses whose `.DAT`, `.EXT` or `.2EX` has no `PPTH` naming that track (all bundles exported by app versions that didn't write one) | Writes the track path into each file's `PPTH`; nothing else changes |
| `fix_bpm_key_mismatch` | tracks whose eDB `content.bpmx100` or key differs from the PDB `tempo_x100` / key, or whose ANLZ beat-grid tempo (`.DAT`/`.EXT`) differs from the PDB tempo. The automatic diagnosis shows only the database part, as the "BPM/key consistency" check under Analysis Files; the ANLZ files are read only by the repair preview/apply | Rebuilds `PQTZ`/`PQT2` from the PDB tempo and length (the audio file's own length when the PDB has none; with neither, the grid is left alone and a warning names the track) with `apply_analysis_edits_to_anlz` (keeping the bundle's first beat, cues, waveforms and any rekordbox `PSSI`), and sets eDB `bpmx100` and `key_id` from the PDB (key names match sharp/flat-insensitively, as on export). Mismatches are re-detected at apply time, so one pass leaves nothing behind. A track using a bundle that isn't its own (shared, not the owner) is left to `separate_shared_analysis`: rebuilding the grid for it would break it for the owner, and the next scan would flag the owner |
| `fix_beat_grid_header` | Bundles under `USBANLZ` whose beat grid is in the pre-0.3.7 app format: a `.DAT` `PQTZ` header with `00000008 00000000` where rekordbox writes `00000000 00080000`, or an `.EXT` `PQT2` whose checksum doesn't match the `PQTZ` beats. Found on disk, not through the PDB; rekordbox bundles never match | Rebuilds `PQTZ` and `PQT2` in rekordbox's format from the grid the `.DAT` already holds (beat times stay within 1 ms), keeping every other chunk byte for byte (`docs/WAVEFORMS.md`, "Beat-grid layout"). A grid that isn't one constant tempo is skipped with a `usb.repair.beat-grid.skipped` warning. Re-detected at apply time |
| `add_missing_mp3_seek_data` | MP3 tracks whose `.DAT` `PVBR` has a zero sample total (app bundles from before 0.3.7; rekordbox's always have the total). Only the bundles are read for the preview | Parses each MP3 on the USB and fills `PVBR` in place, byte for byte as rekordbox writes it, keeping every other chunk (`docs/WAVEFORMS.md`, "Seek-index chunks"). A file whose index can't be reproduced exactly (VBRI, APE/Lyrics3 tags, MPEG-2, lost sync…) is skipped with a `usb.repair.seek-data.skipped` warning naming the file and reason and asking for a report. The preview builds each flagged file's index the same way and lists the files the fix would skip as unsupported instead of proposing them. Re-detected at apply time, so bundles regenerated by `fix_empty_analysis_files` in the same pass are covered |
| `add_missing_flac_seek_data` | FLAC tracks whose `.EXT` has no `PVB2`. Only the bundles are read for the preview | Parses each FLAC on the USB and appends `PVB2` to the `.EXT`. For long tracks about 98% of its entries match rekordbox's (the rest are one frame later), which is why it's a separate fix and not written at analysis. Files that don't parse cleanly are skipped and logged like the MP3 fix |
| `repair_pdb_track_string_alignment` / `repair_pdb_album_string_alignment` | `t00` track or `t03` album rows with a long-form string slot (UTF-16 `0x90` or long ASCII `0x40`) that isn't 4-byte aligned: MIPS CDJs freeze or give communication errors while browsing, or (misaligned long ASCII comments) as soon as the track list opens. Shown as the "Track/album string alignment" check under Contents integrity | Re-encodes each row with the same content through the current, aligned encoder and relocates it. One pass leaves nothing flagged. Misaligned artist names go through `repair_pdb_artist_name_layout`. Every PDB write-back does the same for all three tables (`docs/USB_EXPORT.md`, "Local HDD staging"), so a USB the app writes to in any way doesn't keep these rows |
| `repair_pdb_artist_name_layout` | `t02` artist rows whose name isn't where rekordbox puts it: a short ASCII name not at offset 10 (written by app versions before 0.4.1, which put every name at 12), or a UTF-16 name not 4-byte aligned. Shown as the "Artist name layout" check under Contents integrity. CDJs read the ASCII rows fine, but an Engine DJ player's own import of `export.pdb` shows those tracks without an artist | Re-encodes each row with the same id and name (ASCII at 10, UTF-16 at 12) and relocates it within `t02`. One pass leaves nothing flagged |
| `repair_pdb_header_compatibility_field` | PDB header bytes `0x10..0x14` | Writes only that 4-byte field to the built-in compatibility value `5` when the current value is unrecognized; known-compatible values are not repaired just because they differ from a local backup snapshot |
| `repair_pdb_sentinel_u5_on_data_pages` | data pages whose `u5` is sentinel `0x1FFF` | Rewrites `u5` and, only when needed, `num_rl` to the per-table data-page convention |
| `repair_pdb_wrong_page_flags` | data pages with invalid `page_flags` | Patches byte `0x1b` to the accepted value for that table family |
| `repair_pdb_zero_tranrf_on_track_pages` | row-footer groups with active rows and zero `tranrf` | Patches only zero `tranrf` groups; it does not normalize non-zero transaction masks |
| `repair_pdb_wrong_track_u5_num_rl` | invalid active `t00` track-page footer shape | Patches the affected track page footer fields |
| `repair_pdb_wrong_history_page_shape` | `t16`, `t17`, or `t18` pages with `(1, nrs-1)` shape and `nrs > 1` | Changes those pages to `(nrs, 0)`; single-entry pages (`nrs == 1`, where `(1, nrs-1)` and `(nrs, 0)` coincide) are already valid and are not flagged |
| `repair_pdb_stale_sentinel_btree` | sentinel pages with stale B-tree entries | Resets the sentinel B-tree index area to the empty state |
| `repair_pdb_wrong_playlist_tree_shape` | `t07` playlist-tree pages with wrong footer shape | Sets `u5=nrs` and `num_rl=0` |
| `repair_pdb_tombstoned_playlist_tree_ids` | tombstoned `t00` or `t07` slots duplicating active ids | Zeros only the id field in affected tombstoned slots |
| `repair_pdb_t00_multipage_active_pages` | predecessor `t00` pages marked active in a multi-page chain | Sets those pages to sealed flag `0x24` and `(1, nrs-1)` |
| `repair_pdb_ec_data_page_conflict` | table `empty_candidate` pointer aliasing another table's data page | Assigns each conflicting table a new empty candidate beyond the current file tail and updates `next_unused_page` |
| `repair_pdb_torn_growth_pages` | torn additive-growth write left by an interrupted export (e.g. USB disconnected mid-write) | Zeroes `empty_candidate` page(s) that hold garbage instead of a blank reusable page, truncates any never-populated file tail beyond `next_unused_page`, and recomputes `seqdb` |
| `repair_pdb_t19_template_state` | the PDB has tracks but its `t19` page does not match them and the eDB's `property` row: still the empty-device template (one sealed row, flags `0x24`, `u5=1`, `rowpf=0x0001`; older players show 0 tracks), or a ring whose newest row id is not the track count (an older build always wrote ids `0..1`; older players show 1 track and can hang on a communication error) or whose date, version, name or color is not the eDB's. Shown as a WARN "PDB history page" check in the PDB section, and in the overview as older players showing 0 or 1 tracks | Rewrites the page as the ring rekordbox keeps for the PDB's track count and the eDB's `property` row (`docs/PDB.md`, "`tt=19` activation") and marks it active (flags `0x34`, `u5=2`). Also points the `t19` sentinel's B-tree at that page, as an export does; touches no other page. A page a player has added rows to is left alone. Re-checked at apply time, so a second run is a no-op |
| `repair_pdb_truncated_table_chain` | a table's declared last page is beyond the physical end of the file (interrupted export left growth pointers ahead of the actual written data) | Points the table's `last`/`empty_candidate` fields back at the real last written page (found by walking the chain to the last page that still holds live rows — a trailing all-zero reserve page linked in by the interrupted write is skipped); does not touch page content. Applied before strict parity, since additive track appends hard-fail while the chain is unreachable |
| `relink_moved_audio` | a referenced file is missing while the same track sits unindexed under another path (renamed file, moved or re-cased folder) | Pairs missing references with unindexed files of the same audio format by content fingerprint (size + title + artist), else by file name; only one-to-one pairs. Rewrites the PDB track row's path/file name/size in place (same id), updates the eDB `content` row's path, and the bundle's `PPTH`. Applied before the two fixes below, which only see unmatched items |
| `add_unindexed_audio_playlist` | audio files under `Contents/` not indexed by PDB/eDB (e.g. databases restored from an older backup), matched by exact path like the parity report's indexed-file presence check | Additive export of a USB-only playlist `Unindexed`: tags read from each file, BPM/first beat/cues from the file's canonical ANLZ bundle (bundle reused, not rewritten). A file without a bundle is analysed and gets one, as with `fix_empty_analysis_files`; only a file that can't be analysed is added without. Nothing is copied or deleted |
| `remove_duplicate_audio_copies` | an unindexed audio file under `Contents/` that is a byte-for-byte copy of an indexed file in the same folder (e.g. `Track.mp3` next to the exported `Track-1.mp3`) | Deletes the copy after comparing it with the original again; the indexed track and its playlists, cues and analysis are untouched. These files are taken out of `add_unindexed_audio_playlist`, so the same track is not added twice. Marked destructive |
| `remove_missing_audio_references` | PDB track rows or eDB `content` rows whose audio file is missing from the USB, by the parity report's presence scan | Removes the eDB `content` rows and their `playlist_content` links, and the PDB playlist entries, track rows and now-unused artwork rows. Paired references are relinked instead; tracks in a history playlist are never removed (see below) |
| `sync_edb_history_from_pdb` | eDB history counts differ from PDB-derived history payload | Replaces eDB `history` and `history_content` rows from current PDB history data |
| `keep_engine_library_up_to_date` | the USB has an Engine DJ library and "Write Engine DJ library" is off, or the library is behind `export.pdb` | Turns the setting on and rebuilds `m.db` from the PDB, after every other fix and the write-back. Play history (`hm.db`) is kept; changes made on the player in `m.db` (cues, loops, playlists) are replaced. Marked destructive for that reason. Not in the default set when no `selectedFixIds` are given (see `docs/ENGINE_DJ.md`) |

### rekordbox bookkeeping

Real rekordbox exports carry transaction bookkeeping in the PDB: sentinel
`u5`, `tranrf=0` on a last-row group, non-default `page_flags`, `t00`/`t07`
footer shapes the writer doesn't use, `ACTV` track pages inside a multi-page
chain, stale sentinel B-trees, and tombstoned rows that keep the id of an
active row. They appear and disappear with each rekordbox export, players and
rekordbox read them fine, and this app's exporter works on top of them
unrepaired. These fixes carry `bookkeeping: true`:

- `repair_pdb_sentinel_u5_on_data_pages`
- `repair_pdb_wrong_page_flags`
- `repair_pdb_zero_tranrf_on_track_pages`
- `repair_pdb_wrong_track_u5_num_rl`
- `repair_pdb_t00_multipage_active_pages`
- `repair_pdb_tombstoned_playlist_tree_ids`
- `repair_pdb_wrong_playlist_tree_shape`
- `repair_pdb_stale_sentinel_btree`

Diagnostics report them as Pass ("PDB page headers", "PDB structural
integrity") with the counts in the detail, and they are not detected issues.
The repair still proposes them, so tests can assert that the writer never
produces these shapes, but the editor neither lists nor counts them, and every
apply runs them alongside whatever else is applied. Since they always follow
fixes that rewrite the PDB (strict parity appends rows and pages), the apply
detects them again on the PDB as those fixes left it, not from the preview;
the stale-B-tree check runs last, after the page fixes that can expose it.

## Unsupported and Manual Cases

Some findings intentionally do not have automatic repairs:

| Finding | Behavior |
| --- | --- |
| malformed entries under `PIONEER/USBANLZ` | reported as unsupported; inspect event-log warnings and re-export affected tracks |
| parity preview unavailable | repair preview continues, but strict parity upgrade is not proposed |
| an MP3/FLAC whose seek index can't be built (e.g. a large zero-filled region in the audio) | reported as unsupported with the reason in the Event Log; a damaged file needs an intact copy |
| a track in a history playlist (PDB history or eDB `history_content`) whose audio file is missing | reported as unsupported; the track is kept so the play history stays intact. Copy the file back (the Event Log lists the paths) |
| indexed audio missing while `Contents/` holds no audio at all | reported as unsupported: a databases-only copy, not tracks to remove. Copy the audio back |

## Analysis File Ownership

A track's analysis bundle lives in `PIONEER/USBANLZ/Pnnn/<hash>/`, where the
folder is a hash of the track's USB path (`canonical_analysis_bundle_paths`).
Players find a track's bundle from its path, not from the database. The hash
is reduced to about 200,000 folders, so in a large library two tracks can land
in the same one. rekordbox then writes `ANLZ0000` for one and `ANLZ0001` for
the other, each naming its track in the `PPTH` chunk, which is how the two are
told apart. rekordbox reads an app-exported stick laid out this way correctly
(checked 2026-10-14 with three tracks in one folder and two in another); a CDJ
check is still open.

- Export picks the first `ANLZnnnn` in the hash folder that is free or already
  names the track (`analysis_bundle_paths_for_track`), and never reuses a
  bundle whose `PPTH` names another track. A bundle without `PPTH` is never
  taken over for a new track.
- The repair decides whose a bundle is with `scan_analysis_ownership`. The
  owner of a shared bundle is the track its `PPTH` names; else the one
  sharing track this app exported (its export wrote the current contents);
  else none, and every sharing track gets its own.

## Player Menu Behavior

Diagnostics can report `usb.diagnostics.cdj-menu-divergence` when eDB
`category` has visible menu kinds missing from PDB `t16`.

Normal playlist export preserves PDB `t16`, `t17`, and `t18` and does not
rewrite eDB `menuItem`, `category`, or `sort` menu state.

Menu commands are separate from diagnostics repair:

| Command | Behavior |
| --- | --- |
| `update_usb_player_menu_config` | treats eDB `category` as the Active Category source, keeps required player menu kinds visible, writes eDB category rows, and updates the PDB `t17` category snapshot when the visible set changes |
| `sync_usb_player_menu_edb_to_pdb` | restores PDB `t16` from the full eDB `menuItem` catalog and updates the PDB `t17` category snapshot |

## Strict Parity Upgrade

`upgrade_export_data_to_strict_parity` uses a collect-merge-write flow:

- parse PDB once;
- read eDB playlists with metadata;
- merge playlist names from both sources;
- preserve existing PDB playlist ids and sort order for matched playlists;
- keep PDB-only playlist members instead of pruning them;
- prefer eDB track metadata when present, falling back to PDB fields;
- write PDB through the current writer;
- remove stale duplicate PDB playlist-entry rows left by prior repairs;
- re-read PDB playlist identities;
- write eDB using the final playlist identity, including syncing
  `sequenceNo` from the current PDB `t07.sort_order` for every playlist.

The strict repair path is intended to converge on rerun. It does not compact
PDB tables, rebuild the entire PDB from scratch, or silently remove playlist
membership that exists on only one side.

## Implementation Anchors

| Area | File |
| --- | --- |
| operational diagnostics | `backend/src/service/diagnostics/` (`run.rs` holds the entry points) |
| strict parity report | `backend/src/service/diagnostics/parity.rs` |
| repair catalog and apply flow | `backend/src/service/repair/` (`run.rs` drives one pass) |
| PDB/eDB field context | `docs/PDB.md`, `docs/eDB.md` |

The `diagnostics` module's PDB reads (integrity checks, parity report) go through the local HDD staging
layer described in `docs/USB_EXPORT.md`'s "Local HDD staging" section, same as export. eDB access
is always staged transparently regardless of caller (`edb::open_edb_from_usb_root`/`open_edb_rw`).

The `repair` module's PDB/eDB reads and writes go through the same staging layer: its diagnostic scanners
(`detect_pdb_*`) share one staged read per diagnostics pass, and every `apply_*`/`apply_fix_*`
writer selected in a `repair_usb_diagnostics` apply request writes to the local staged copy only —
`apply_selected_fixes` flushes both databases back to the real USB drive exactly
once, at its end, after every selected fix has run (not once per fix).
`update_usb_player_menu_config` and `sync_usb_player_menu_edb_to_pdb` are staged the same way but
are standalone operations outside the apply-batch flow, so each backs up and flushes on its own.

## Verification

Relevant test areas:

- `backend/tests/diagnostics_functional.rs`
- `backend/tests/export_shape_parity_functional.rs`
- `backend/src/service/diagnostics/tests.rs` unit tests
- `backend/src/service/repair/tests.rs` unit tests
- `backend/src/edb.rs` unit tests
- `vanilla-ui/tests/diagnostics_ui_behavior.test.mjs`
- `vanilla-ui/tests/usb_parity_detail.test.mjs`
