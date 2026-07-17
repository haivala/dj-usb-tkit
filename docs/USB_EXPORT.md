# USB Export

## How it works

USB export writes selected local playlist content into DJ-player-compatible USB structures. The export path includes media copies, playlist membership, and analysis/artwork references required for player compatibility.

Export supports two playlist sync modes:

- `mirror`: exported playlist membership on USB is replaced with the current local playlist membership.
- `additive`: exported tracks are added to existing USB playlist membership without removing existing members.

In practical terms:

- use `mirror` when you want the USB playlist to exactly match the current local playlist
- use `additive` when you want to keep existing USB playlist members and only append missing local members
- both modes affect only the exported playlist target; unrelated playlists are left as-is

Export is intentionally tied to a quick prep loop:

- users can prepare playlists first
- analyze only missing tracks for that playlist
- then export once required analysis fields are present

Export is designed for deterministic re-runs. Re-exporting the same playlist should produce predictable results, and optional cleanup can prune stale export-owned files when enabled.

## Backup

Before each export, the app copies PDB and eDB to a backups folder next to them with a timestamp. Backups land in `PIONEER/rekordbox/backups/` on the USB drive with filenames like `export_2025-04-23_14-32-01.pdb` and `exportLibrary_2025-04-23_14-32-01.db`. Files are only copied if they already exist — a first export with no prior databases skips silently.

This behavior is on by default and can be disabled in Settings → Export Settings.

Strict parity validation is handled as a separate diagnostics surface so users can distinguish between operationally usable media and strict database parity.

## Deep technical details

Export is a coordinated write pipeline, not a single file copy. For each export operation, the backend prepares playlist membership, track metadata, and file-path mappings, then writes both database surfaces (PDB and eDB) plus filesystem assets under the expected USB tree.

For full field inventories and mapping notes, see `docs/PDB.md` and `docs/eDB.md`.

Known DB data written by export:

- eDB is the richer relational surface and is used for:
  - playlist rows (`playlist`)
  - ordered membership rows (`playlist_content`)
  - track metadata rows (`content`)
  - artwork lookup rows (`image`) and linked references
  - menu/category/sort/property baseline tables during USB initialization
  - history tables when present and synchronized by explicit flows
- PDB is the binary player-oriented surface and is used for:
  - track rows including core playback metadata and file/analysis paths
  - playlist tree and playlist-entry tables for hardware-visible playlist order
  - artwork/dictionary references used by track rows
  - history/runtime tables when already present or when first-export runtime
    rows must be synthesized in place

Known PDB track-row data currently mapped in export includes:

- core numeric fields: track ID, artist/album/key IDs, artwork ID, track number, tempo (`tempo_x100`), duration, file size, sample rate, bitrate, and app-owned identity fields (`master_db_id`, `master_content_id`, `content_link`)
- string-slot data: title, media path, analysis path range, export date (`date_added`), release date policy value, and other structural slots preserved for compatibility

Known eDB track-row data currently mapped in export includes:

- `content.length` from local `duration_ms` (seconds)
- `content.dateAdded` from export run day
- `content.dateCreated` fallback chain (`recorded_date` -> `release_date` -> `file_modified_at`)
- `content.releaseDate` from `release_date` or `file_modified_at`
- `content.analysisDataFilePath`, media path, artwork linkage, and identity fields

Before writing exporter-owned metadata text, the exporter strips embedded NUL
bytes, caps any grapheme cluster (a base character plus its combining marks)
to 8 codepoints, caps the number of distinct Unicode scripts a single string
may mix to 3, and truncates to 255 Unicode characters. This applies to
playlist names, track title/subtitle/search/comment/KUVO-comment text, and
artist/album/genre metadata. It does not apply this metadata sanitizer to
key/tonality strings, media paths, or analysis paths.

The grapheme-cluster cap exists because "zalgo" text — dozens to hundreds of
Unicode combining marks stacked on one base character — has been observed to
hang CDJ hardware when it tries to render the track title, even though the
total character count is well under the 255-character limit. The
script-diversity cap exists because a separate, shallower pathological case
was also observed to hang CDJ hardware: a name mixing many unrelated scripts
(e.g. Braille, Yi, Georgian, Tibetan, Tamil, Bengali, Arabic all in ~25
characters) hung both the CDJ's Artist browse menu and its track-load screen,
even though no individual grapheme cluster was deep enough to trip the
combining-mark cap. Rather than curating a list of which scripts are "safe"
CDJ hardware can render, the cap is purely self-referential: once a string
has touched `MAX_DISTINCT_SCRIPTS` distinct scripts, characters belonging to
any further script are dropped — the allowed set is just whichever scripts
the string happens to touch first. Script-neutral characters (punctuation,
digits, combining marks that inherit their base's script) never count against
the budget.

Both caps are applied to the on-disk `/Contents/...` file and folder names the
exporter generates (see `sanitize_contents_component` in `export_paths.rs`),
since that text is also embedded verbatim in the ANLZ `PPTH` chunk written
into the waveform/beatgrid bundle (see `docs/WAVEFORMS.md`). Neither cap is
applied to the literal media path used to read the audio file itself — that
must always match the real file on disk.

The main technical stages are:

1. Resolve export target and selected playlist content.
2. Ensure required local analysis/artwork/media references exist or report missing prerequisites.
3. Copy/update media and related assets into expected USB paths.
4. Write playlist/track metadata into PDB and eDB.
5. Emit progress and warnings through the job event model.

If playlist tracks are missing required analysis, export is blocked and UI instructs the user to run "Analyze Missing Tracks" for that playlist before retrying export.

Analysis bundle handling is copy-only during export. Export requires the track to already have a `DAT/EXT/2EX` bundle and copies/reuses it even if the bundle is older or low-detail. Export does not decode source audio or regenerate ANLZ files; missing bundle files block export before media copy starts. See `docs/WAVEFORMS.md`.

WAV media copy has one exception to plain byte-for-byte copying: if a source WAV's `fmt ` chunk uses `WAVE_FORMAT_EXTENSIBLE` and wraps plain PCM or IEEE-float data (flagged during scan — see `docs/LIBRARY_ANALYSIS.md`), export rewrites the header to a standard 16-byte PCM/float `fmt ` chunk before writing it to the USB drive, since some Pioneer CDJs reject the extensible form outright. This is a lossless, header-only rewrite: only the `fmt ` chunk bytes and the overall RIFF size are changed, sample data is streamed through unmodified. If the extensible header wraps some other subformat, there's nothing safe to rewrite and the file is copied unchanged (`copy_if_different`), keeping the hard warning from scan surfaced in the UI. See `backend/src/wav_format.rs` (`rewrite_extensible_to_pcm`) and `copy_wav_normalized_if_needed` in `export_paths.rs`.

Current media-copy behavior is intentionally conservative: file copy/write steps
are sequential per export operation. This keeps ordering deterministic and
failure handling straightforward across USB devices.

### Mirror vs additive behavior

Both modes use the same manifest build path, but they differ in how existing playlist membership is handled.

- Mirror mode (`pruneStale = true`):
  - eDB path deletes existing `playlist_content` rows for the target playlist ID before relinking manifest members.
  - PDB path removes target playlist-entry rows in place, preserving table chains and inactive/tombstone state.
  - stale export-owned files from previous runs can be pruned when no longer part of the current manifest.
- Additive mode (`pruneStale = false`):
  - existing playlist membership rows are preserved.
  - new manifest members are linked only when not already present.
  - entry sequencing continues from existing max order instead of resetting.

Scope is playlist-local in both modes: unrelated playlists are not implicitly rewritten to mirror the exported playlist state.

Existing PDBs are topology-locked. Normal export does not fall back to a full
PDB rebuild, does not call `apply_growth_shape`, and does not rewrite menu
tables. If an export cannot be represented as an in-place additive/mirror patch,
the PDB write fails before commit rather than producing a player-corrupt shape.
For PDB `t07` playlist_tree, new playlist rows are added into existing tail
capacity first, including tombstone-preserving tail pages. When existing
capacity is exhausted, a pure tail append (new pages chained after the
existing last page, with every pre-existing page remaining at its same
position in the chain) is allowed — this is how reference exporters
themselves grow playlist_tree, and multi-page chains are known to work on
hardware. Only a `last_page` change that is not a pure append — page
relocation, reordering, or a broken chain — is rejected.
The exported playlist is placed first in the playlist menu by patching only the
fixed-width `t07.sort_order` fields of active sibling playlist-tree rows. This
matches the intended playlist sibling order while preserving row lengths,
inactive rows, and table-chain topology. eDB `playlist.sequenceNo` is updated to
the same parent-scoped order. Reference exporters follow the same field semantics,
but their ordering comes from the app model and their PDB writers repack from a
static layout rather than preserving arbitrary existing USB topology.
For all reused PDB pages, normal export preserves existing row footer state:
existing `rowpf` and `tranrf` groups are not normalized, and only appended rows
OR in their own row-presence and transaction bits. This is required for
older PDB-first hardware compatibility even when the table topology is unchanged.
Strict-repair metadata fixes use the same path: t00 track rows may receive
direct scalar patches and same-length string-slot patches. When a mapped string
slot grows or shrinks, the old active t00 row is marked inactive and a
replacement row with the same track id is appended to the t00 chain. Unrelated
PDB shape, inactive rows, and menu tables are not normalized.

Deterministic rerun behavior is handled at this pipeline level. Re-exporting the same playlist uses stable identity/linkage rules so metadata and membership can be updated predictably without duplicating entries by default.

Identity-linkage policy is app-owned and persistent:

- one stable `masterDbId` per app data directory
- one stable `masterContentId` per local track
- one stable `contentLink` scope per app data directory

This is intentional: app-exported tracks must keep app-owned identity values so USB-side ANLZ data is resolved from the exported stick for those tracks instead of collapsing resolution to unrelated local cache state.

### Player menu categories

Player browse-category metadata is stored in two related places:

- PDB `columns` table (t16) stores the legacy/player browse-category catalog.
- eDB `menuItem` stores the newer-player category catalog.
- eDB `category` stores the active/visible category subset and order through
  `isVisible` and `sequenceNo`.

App-initialized USBs seed the current 27-row PDB catalog, but existing USBs can
carry smaller or source-specific PDB menu profiles. Normal playlist export
preserves PDB t16/t17/t18 and eDB `menuItem`/`category`/`sort` state exactly.
Menu changes are explicit menu operations, not export side effects.

The Menu Editor's Save action:

1. Resolves the requested active kind list.
2. Rewrites eDB `category` so those kinds are visible in the requested order.
3. Leaves PDB t16 unchanged.
4. If the eDB write fails, eDB is restored from its snapshot.

The PDB restore action is separate: `sync_usb_cdj_menu_edb_to_pdb` rebuilds
PDB t16 from the full eDB `menuItem` catalog when older app code or manual
editing has removed catalog rows from PDB. See `docs/PDB.md` for the t16 row
layout and observed kind codes.

Strict parity is intentionally separated from basic export success. A USB may be playable while still failing strict parity checks. The parity report path validates deeper consistency such as:

- playlist membership equivalence across DB surfaces
- duplicate playlist-entry detection
- required metadata presence in PDB-linked rows
- media/analysis path consistency
- dictionary-id resolution integrity (artist/album/key/artwork references)

Strict parity also recognizes a small allowed-difference set where values are app-owned or analysis-path-specific. Current allowed-difference fields are `content.analysisDataFilePath`, `content.contentLink`, `content.masterContentId`, `content.masterDbId`, and `key.name`. Other compared fields are treated as blocking mismatches.

When strict mismatches are found, correction is handled through explicit repair actions rather than silent mutation inside report commands.

Implementation anchors:

- export command façade: `backend/src/commands.rs:215`
- Tauri export handler: `backend/src/tauri_commands.rs:1070`
- desktop invoke wiring: `desktop/src-tauri/src/main.rs:622`

## Verification links

- Export behavior: `backend/tests/export_functional.rs`, `backend/tests/export_shape_parity_functional.rs`, `backend/tests/pdb_writer_functional.rs`
- Frontend export flows: `vanilla-ui/tests/usb_export_label.test.mjs`, `vanilla-ui/tests/usb_workflows.test.mjs`
