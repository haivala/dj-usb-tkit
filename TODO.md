# TODO

## 1.0.0: remove repairs for damage only old app versions wrote

Keep these until 1.0.0, then remove them (detector, repair, catalog entry in
`docs/DIAGNOSTICS_REPAIRS.md`, and their tests). The damage they fix came from
this app's own earlier writers. Sticks from those versions keep it, because
the additive export appends and patches instead of rewriting.

- rekordbox bookkeeping fixes (`bookkeeping: true`): stale sentinel B-tree,
  header field values 1/4. Keep a test-only check that the writer never
  produces these shapes.
- `repair_pdb_ec_data_page_conflict`
- the tt=8 duplicate playlist-entry sweep (standalone repair and strict
  parity's duplicate-entry phase)
- `repair_pdb_artist_name_layout`, `repair_pdb_track_string_alignment`,
  `repair_pdb_album_string_alignment`
- `repair_pdb_t19_template_state` (export still rewrites an app-written `t19`)
- ANLZ: `update_app_analysis_files`, which replaced `fix_beat_grid_header`,
  `add_missing_mp3_seek_data`, `add_analysis_track_path` and
  `separate_shared_analysis` and also replaces the waveforms the repairs and
  analysis up to 0.6.6 drew coarse or stretched.
- Revise how stretched waveforms are tracked, and handle it properly. Today
  the stick's export log decides: a long track whose latest record is an
  export without `waveformsSynced` (0.6.6 and earlier) is suspect until an
  export syncs it or the repair appends a `waveform-redrawn` record listing
  it (`waveforms_maybe_stretched`). That record is a marker in
  a log made for exports: every reader of exports has to skip it, tracks are
  known only by their stick path, and the "Long-track waveforms" check and
  the repair depend on it. Before 1.0.0, replace it with a proper record of
  what each stick's analysis files are (who drew them, with which version),
  or drop the check and the repair along with the other old-version fixes.
- strict parity's flattened-copy removal (`flattened_edb_copies`): top-level
  eDB copies of playlists in folders, which the repair wrote before it kept
  folders

Keep beyond 1.0.0, since other writers or interrupted writes cause these too:
torn growth pages, truncated table chain, unknown header compatibility values,
empty duplicate playlists, strict parity, the audio fixes, history sync, the
Engine DJ library, page footer transaction (a crashed rekordbox write breaks it
too), damaged page headers, the damaged track row merge and rebuild, and the
damaged playlist repair.

## Hardware: does the CDJ-3000X use eDB play history?

The eDB has its own play history (`history`, `history_content`), but no player
tested so far has been seen using it. Test on a CDJ-3000X: play a few tracks from an exported
USB, then check whether `history`/`history_content` in `exportLibrary.db`
gained rows.

If it does, the mirror export's stale-file prune must protect tracks in eDB
history too. Today `filter_prunable_stale_paths_for_playlist`
(`backend/src/service/export_helpers/export_paths.rs`) keeps the files of
tracks in PDB history but reads only `playlist_content` from the eDB, so a
track only in eDB history and in no playlist could lose its audio, analysis
and artwork files. "Remove Missing Audio References" already counts both
histories.

## Possibility: history repair merges both databases' sessions into both

First ask users whether they want this. USB History already lists the sessions
of both databases, matched by `history_id` (`merge_history_sessions` in
`backend/src/service/usb.rs`). The optional `sync_edb_history_from_pdb` repair
(`backend/src/service/repair/history.rs`) still only overwrites the eDB's
history with the PDB's, dropping sessions only the eDB has. The idea: merge
both databases' sessions the same way USB History does and write the result
to both, still as an optional fix.

Open points:

- Track IDs differ between the two databases on rekordbox-made sticks, so a
  session copied to the other database must map its tracks by file path. The
  current sync writes PDB track IDs into `history_content.content_id`
  unmapped, which names missing or wrong tracks there.
- An entry whose track isn't in the other database at all: leave it out of
  the copy, skip the whole session, or add the track to that database?
- Writing sessions to the PDB means appending to `t11/t12`, which the app has
  never done. A CDJ-3000 appends one `t11` page (one 16-byte row: id + name)
  and one `t12` page (12-byte rows: track id, session id, entry index) per
  session, flags `0x24`, `u5=1`, `num_rl=nrs-1`. `append_rows_to_chain_in_place`
  fills free space on existing pages first, so it would need a new-page-only
  mode to match. Needs a hardware check (NXS2 + rekordbox) before release.
- How and when players write history to each database isn't known (see
  `docs/eDB.md`), which the item above would help settle.
- The PDB reader and writer still have separate history entry row types
  (`PdbHistoryEntryRow` in `backend/src/pdb_reader.rs` and in
  `backend/src/pdb_writer/model.rs`). The reader's also reads the 8-byte
  `t17/t18` seed rows, so its track id is optional and it keeps which table
  the row came from. The writer's describes only the 12-byte `t12` rows it
  writes, always with a track id. Merge them once the writer appends
  sessions, deciding what it writes for a row without a track id; the
  playlist tree and entry rows are already one type.

## Possibility: copy cues added on a CDJ back to the library

Cues added on a CDJ don't reach a library track that already has cues:
`import_anlz_cues_for_track` (`backend/src/service/cues.rs`) skips those. The
next export then replaces the stick's cues with the library's
(`ensure_analysis_bundle_ppth` in
`backend/src/service/export_helpers/export_paths.rs`), so the CDJ's cues are
lost. Found by reading the code; the loss itself hasn't been seen on a stick
yet, but the CDJ side is now known (below).

Idea: when a track's cues on the stick have changed since the last export,
replace the library track's cues with the stick's, all of them, with no
cue-by-cue merge. CDJ removals then carry over too.

- Export saves the ANLZ cue list it wrote for each track on that stick, and the
  stick's list is compared with it. `cueUpdateCount` can't be used: the CDJ
  doesn't touch it (below).
- "The `.EXT` changed" isn't a cue change either: the CDJ also rewrites the
  `.EXT` of tracks it only played, to add `PVB2`. Compare the cue lists.
- Ask when the track was also edited in the library since the last export: a
  rewrite would lose those edits, and "local edits win" is the rule today.
- The import's time shift for transcoded files still applies.

Checked on a CDJ-2000NXS2 (firmware 1.87), stick copy in
`USB_CUE_POINTS_FROM_CDJ`. The app exported at 15:58, and the CDJ's own writes
are the files dated 2024-01-01 02:00, since it has no clock:

- `export.pdb` and `exportLibrary.db` are untouched. The eDB still has no `cue`
  rows and a NULL `cueUpdateCount` for every track.
- "Horse On Wheels" (`P051/00018F1B`) got two hot cues, A at 23 ms and B at
  9620 ms. They were written to both the `.DAT` (`PCOB`) and the `.EXT` (`PCOB`
  plus `PCO2`, colour 0), and the `.EXT` also got a `PVB2` tag. The `.2EX` was
  left alone.
- "Dry Your Eyes" (`P04C/00014CD0`) only had its `.EXT` rewritten, to add
  `PVB2`. Its cue lists are still empty.

Still unchecked:

- Memory cues. None were found on the stick, so the player may write them
  differently.
- Loops, cue comments and colours from players that have them.
- Other models: a CDJ-3000 or a player that reads `exportLibrary.db` may update
  `cueUpdateCount` or the eDB `cue` table as well.
- Re-exporting the stick to see the CDJ's cues get replaced.

## Hardware glance: Engine library from a stick without a PDB

On a stick with only the eDB, the Engine DJ library is written from the eDB,
and its PDB import counter (`lastRekordBoxLibraryImportReadCounter` /
`pdbImportKey`) is NULL, since there is no PDB for the player to import. No
player has read such a library yet. Next time an eDB-only stick is near the
Mixstream Pro, check that it opens the library without importing anything
and shows the tracks, playlists and cues.
