# PDB

This document describes the PDB file used by the USB export format. It records
facts implemented by this repository and facts required for hardware
compatibility.

## How PDB Works

`export.pdb` is the legacy binary database stored at
`PIONEER/rekordbox/export.pdb` on a USB export. It sits beside
`exportLibrary.db`, the encrypted SQLite eDB database.

The two files are not interchangeable mirrors:

- Older PDB-first hardware players read PDB as the primary database.
- Newer eDB-first hardware players read eDB as the primary database and use PDB
  as a secondary compatibility surface.
- Desktop library software validates PDB structure when loading USB exports.

PDB contains the player-facing track list, playlist tree, playlist membership,
dictionary rows, artwork references, history/runtime rows, and player browse
catalog rows. eDB contains the richer relational model. A compatible export has
matching playlist and track meaning across both databases, but each file has
its own format and validation rules.

The app treats existing PDB files as topology-locked. Normal export patches the
existing file in place instead of rebuilding it from scratch. This matters
because older players validate page layout, row footer history, table chains,
and transaction markers. A PDB can have correct playlist data and still be
rejected if those structural details are rewritten in a shape the player does
not accept.

`exportExt.pdb` is an optional extension database seen on some compatible USB
exports. It is used for newer extension data, including user/My Tag
categorization and tag-to-track associations. This app does not require it for
hardware-compatible playlist export, does not write it, and keeps any existing
`exportExt.pdb` byte-stable during normal export.

Thanks to the Deep Symmetry documentation for the published `export.pdb` and
`exportExt.pdb` table-type research used to cross-check these names. Deep
Symmetry documents two `exportExt.pdb` table types: `0x03` (`tags`) and `0x04`
(`tag_tracks`).

User tags are represented on the eDB side by `myTag` and `myTag_content`.
`myTag` stores the tag tree and `myTag_content` links tags to tracks.

## Compatibility Rules

Full compatibility means the export is accepted by all relevant validators:
desktop library software, older PDB-first hardware players, and newer eDB-first
hardware players.

Known PDB rules:

- Existing populated PDBs must not be rebuilt or compacted during normal export.
- Existing table `first_page` pointers must not move during topology-locked
  export.
- Normal export preserves PDB menu tables `t16`, `t17`, and `t18`.
- PDB `t07` playlist-tree `last_page` is guarded during normal export.
- Existing inactive/tombstoned rows are preserved unless a targeted safe repair
  changes only the affected fixed-width field.
- Existing `rowpf` and `tranrf` footer groups on reused pages are transaction
  history. Normal export preserves them and ORs in only newly appended row bits.
- Do not normalize existing `tranrf` to equal `rowpf`; that shape is unsafe on
  older PDB-first hardware players.
- `tt=8` playlist-entry pages are strictly validated as
  `u5=1, num_rl=trc-1`.
- `tt=8` playlist-entry rows are identified by `(playlist_id, track_id)`.
  `entry_index` is a mutable position, never part of identity: a track whose
  order within an already-exported playlist changes is patched in place
  (`patch_t08_entry_indices_in_place`), not removed and re-added. See "t08
  duplicate playlist-entry accumulation" below.
- PDB/eDB strict parity failures are repaired through explicit repair actions,
  not by report commands.

Normal export uses the topology-locked additive writer for every existing PDB,
including the initialized zero-track template. `EXPORTER_PDB_WRITE_MODE=fresh`
is rejected when a PDB already exists. If the additive writer cannot represent
the change safely, export fails before commit instead of falling back to a fresh
PDB rebuild.

## File Layout

PDB pages are 4096 bytes. Page 0 is the file header. The remaining pages are
sentinel/index pages and data pages.

Known file header fields:

| Offset | Field | Meaning |
| --- | --- | --- |
| `0x04` | `len_page` | page size in bytes (always 4096) |
| `0x08` | `num_tables` | number of table pointer slots (always 20) |
| `0x0c` | `next_unused_page` | next page index to allocate; must be `> max(empty_candidate)` |
| `0x10` | `compat_field` | unknown semantics; see note below |
| `0x14` | `seqdb` | global transaction counter; must be `> max(seqpage)` across all pages |
| `0x1c` | table pointers | 20 × 16-byte table pointer slots |

**File header `0x10` compatibility field**: our writer emits `5` here.
Version-6 reference exports also use `5`; version-7 fresh reference exports
commonly use `1`. Reference testing confirmed this field is not a
desktop-library rejection cause when set to `5`. A version-7 fresh reference
export patched from `1` to `5` was accepted by hardware players. Value `5` is
accepted by tested desktop and hardware validators.

There are 20 table pointer slots in the file header. Each table pointer stores:

| Field | Meaning |
| --- | --- |
| `table_type` | numeric table id |
| `empty_candidate` | expected next empty page for the table |
| `first_page` | first page in the table chain, normally the sentinel |
| `last_page` | last page in the table chain |

For accepted exports, `empty_candidate` is kept consistent with the last page's
`next_page` value.

**ec conflict hazard**: When a fresh write allocates overflow pages for `tt=0`,
the overflow page index may coincide with `tt=19`'s `last_page + 1`. The writer
must detect this case and assign `tt=19` a virtual page index beyond the
physical file instead of the conflicting value. Desktop library software rejects
USBs where any table's `empty_candidate` points into a data page owned by
another table.

**Torn additive-growth write (interrupted export, e.g. USB disconnected
mid-write)**: An additive-growth write first extends the file and/or
earmarks existing `empty_candidate` pages as the next slot for one or more
tables, then writes the new page contents. If the drive is disconnected
between those two steps, the signature observed (verified against a
real-world interrupted export) is:

- one or more table `empty_candidate` pages, though still within the header's
  own allocated range (`< next_unused_page`), contain non-zero garbage
  instead of the all-zero blank page the additive writer requires before
  reusing them (`append_rows_to_chain_in_place` in
  `backend/src/pdb_writer/additive.rs` only reuses an existing
  `empty_candidate` when every byte on that page is zero);
- the physical file can also extend past the header's `next_unused_page`
  with non-zero content — capacity that had already been grown into the
  file but never populated. Per the header's own bookkeeping this region is
  unallocated and unreferenced by any table's chain or `empty_candidate`.
- every page reachable through a table's `first_page`/`next_page` chain
  remains untouched and valid; only unreferenced pages carry garbage.

This is not evidence of PDB rebuild/relocation damage — no chain moved. It
is safely repaired by zeroing the garbage `empty_candidate` page(s) (restoring
exactly the pre-interruption "not yet used" state) and truncating any
never-populated tail beyond `next_unused_page`, then recomputing `seqdb` so
it stays greater than every remaining `seqpage`. See
`repair_pdb_torn_growth_pages` in `docs/DIAGNOSTICS_REPAIRS.md`.

**Truncated table chain (a more severe interrupted-export variant): declared
`last_page` beyond the physical file.** Unlike `empty_candidate` — which
healthy exports routinely set beyond the file's current physical length as
reserved headroom for future growth (confirmed on both `USB_REKORDBOX6` and
`USB_LOTS_OF_TRACKS_RB`, where several tables' `empty_candidate`/
`next_unused_page` exceed `total_pages` in an otherwise clean, accepted
export) — a table's `last_page` must always reference a page that physically
exists in the file. It is not a future candidate; it is the table's real,
already-written tail.

One corrupted reference export (`USB_ANOTHER_FIX`, tracks table) had:

- physical file: 510 pages (valid indices `0..509`);
- `t00` table pointer: `empty_candidate=512`, `last_page=511` — both beyond
  the physical file;
- the real last physical `t00` page (509) already had `next_page=510`,
  correctly anticipating the growth that never landed, confirming
  `empty_candidate == last_page.next_page` still held for the *true* last
  page — only the header's `last_page`/`empty_candidate` fields had been
  advanced past what was actually flushed to disk.

The repair only ever needs to correct these two header fields — no page
content changes, since the true last page's own `next_page` already
anticipated the corrected `empty_candidate`. See
`repair_pdb_truncated_table_chain` in `docs/DIAGNOSTICS_REPAIRS.md`. This
repair must run *before* the strict-parity upgrade fix, not after like the
other structural repairs: the additive track-append path
(`mutate_tracks_in_place` in `backend/src/pdb_writer/additive.rs`) hard-fails
immediately when a table's chain is unreachable, which otherwise blocks (or makes
order-dependent) every strict-parity playlist mutation that needs a new or
changed track row.

**t08 duplicate playlist-entry accumulation.** Found on a real-world export
that failed to load on hardware at all (CDJ menu never appeared): a playlist
re-exported many times over its history had one track
duplicated across 7 separate `tt=8` rows (another, 4 times; four more, once
each) — 87 rows for a playlist eDB said should have 74. The playlist that had
been exported in a single session, on the same drive, was clean; only the
one repeatedly re-exported was affected.

Root cause: the additive writer used to identify a `tt=8` row by
`(playlist_id, track_id, entry_index)` — all three fields — instead of
`(playlist_id, track_id)` alone. Every time a track's position within an
already-exported playlist changed, that was computed as "old
`(playlist,track,old_index)` removed, new `(playlist,track,new_index)`
added" instead of "this entry's position changed." In a single clean write
that pair completes correctly and leaves no trace; the accumulation only
shows up after enough repeated position changes across a playlist's export
history that the removal half of some pair doesn't keep up with the
addition half. Fixed by giving repositioning its own path
(`patch_t08_entry_indices_in_place`): a track that's still in the playlist
but at a new `entry_index` gets that field overwritten in place at its
existing row location, so there is no remove+add pair for a reposition to
begin with. `try_write_pdb_additive_in_place` also now runs
`remove_duplicate_playlist_entries_inplace` unconditionally on every
additive write (previously only from one specific repair path) as a
self-healing sweep for USBs that already accumulated duplicates before this
fix — its survivor tie-break keeps the *highest* `entry_index` per
`(playlist_id, track_id)` (the most recently written, i.e. correct, position
for this failure mode; do not revert to "keep lowest" without re-reading
`remove_duplicate_playlist_entries_inplace`'s doc comment, which also
protects an unrelated stale-page-relocation scenario).

Known table families used by this repository:

| Table | Name | Current use |
| --- | --- | --- |
| `t00` | tracks | track metadata, media path, analysis path, dictionary links |
| `t01` | genres | dictionary rows |
| `t02` | artists | dictionary rows |
| `t03` | albums | dictionary rows with artist linkage |
| `t04` | labels | dictionary rows |
| `t05` | keys | musical-key dictionary rows |
| `t06` | colors | standard player color rows |
| `t07` | playlist_tree | folders/playlists and sibling order |
| `t08` | playlist_entries | playlist membership and order |
| `t09` | unlisted/other | not named in the Deep Symmetry table list; left empty by the current writer |
| `t10` | unlisted/other | not named in the Deep Symmetry table list; left empty by the current writer |
| `t11` | history_playlists_alt | not named in the Deep Symmetry table list; left empty by the current writer; import uses this history family when populated |
| `t12` | history_entries_alt | not named in the Deep Symmetry table list; left empty by the current writer; import uses this history family when populated |
| `t13` | artwork | artwork path dictionary rows |
| `t14` | unlisted/other | not named in the Deep Symmetry table list; left empty by the current writer |
| `t15` | unlisted/other | not named in the Deep Symmetry table list; left empty by the current writer |
| `t16` | columns | player browse-category catalog; Deep Symmetry marks details as unconfirmed |
| `t17` | history_playlists | Deep Symmetry: History menu playlist list; normal export preserves it; menu repair may rewrite it as an eDB category snapshot |
| `t18` | history_entries | Deep Symmetry: links tracks to history playlist entries; normal export preserves it |
| `t19` | history | Deep Symmetry: history synchronization data; a template-state page is activated on export |

Deep Symmetry documents table type numbers in hexadecimal. In this document the
`tNN` labels are decimal table numbers, so Deep Symmetry `0x11`, `0x12`, and
`0x13` correspond to `t17`, `t18`, and `t19` here. Deep Symmetry does not name
decimal `t09`, `t10`, `t11`, `t12`, `t14`, or `t15` in its `export.pdb` table
list.

The current writer leaves `t09`-`t12` and `t14`-`t15` empty during normal
export. The reader can parse `t11` and `t12` as an alternate runtime
history-family surface. Runtime history import keeps `t11/t12` separate from
`t17/t18` and uses `t11/t12` whenever either table is populated; otherwise it
uses `t17/t18`.

### t17/t18 blank template rows on fresh/never-played exports

Verified byte-level against several unrelated rekordbox exports, none ever
played on hardware, including a rekordbox-initialized drive with zero tracks
or playlists. All of them ship `t17`/`t18` pre-populated with a fixed,
byte-identical block of blank template/seed rows (same ids, same shapes,
regardless of the export's actual library or track count):

- `t17` (`history_playlists`) rows have `name` empty (or a single stray
  control byte). Real rekordbox-recorded sessions are always named
  `"HISTORY NNN"`.
- `t18` (`history_entries`) rows are physically 8 bytes — no `track_id`
  field at all (`parse_history_entry_row`'s short-row branch always yields
  `track_id: None` for these).

These are not corruption and not something this app writes; they ship as
part of rekordbox's own export template and are present even when `t11`/`t12`
(the hardware-only legacy fallback tables) are completely empty, i.e. on the
most common case of a freshly exported or never-played drive. Any reader that
surfaces history rows as user-facing history (import, diagnostics, or repair
payload derivation) must filter on the `"HISTORY "` name prefix first. This
is the same rule applied by `is_named_history_playlist` (`usb_helpers.rs`)
and `derive_history_sync_payload` (`repair/history.rs`).

## Page Header

Every page begins with a 40-byte header. Data rows grow forward from `0x28`.
The row footer grows backward from the end of the page.

Known page-header fields:

| Offset | Field | Meaning |
| --- | --- | --- |
| `0x04` | `page_index` | page's own index |
| `0x08` | `table_type` | table id |
| `0x0c` | `next_page` | next page in the table chain |
| `0x10` | `seq` | page transaction sequence |
| `0x18..0x1a` | row counts | packed row-slot and live-row counts |
| `0x1b` | `page_flags` | page class |
| `0x1c` | `free_size` | free bytes between heap and footer |
| `0x1e` | `used_size` | row heap bytes in use |
| `0x20` | `u5` | transaction row count/convention field |
| `0x22` | `num_rl` | transaction row index/convention field |
| `0x24` | `u6` | zero in current writer output |
| `0x26` | `u7` | sentinel B-tree write pointer/high-water field |

Page flags used by the writer:

| Flag | Meaning |
| --- | --- |
| `0x64` | sentinel/index page |
| `0x24` | normal/sealed data page |
| `0x34` | active transaction-class data page |

Current writer behavior:

- `tt=0` and `tt=19` use `0x34` for active baseline data pages.
- Other fresh data pages use `0x24`.
- True overflow pages appended beyond the file tail are sealed with `0x24`.
- When a reused baseline page later receives overflow pages, the baseline
  `tt=0` page is sealed to `0x24`.

Existing PDB pages can carry older transaction state. Normal export does not
normalize those pages just because their flags or footer state differ from
freshly written pages.

Observed `tt=0` track chains from reference exports include more than one valid
multi-page profile:

- all populated data pages sealed with `flags=0x24` and no `tt=0` sentinel
  B-tree entry (`ne=0`);
- a transaction/tombstone page with `flags=0x34` inside a multi-page chain while
  a later terminal page remains sealed. One multi-page reference export had this
  shape: page 55 is `flags=0x34` with one inactive row and six active rows,
  page 57 is the terminal sealed page, and the `tt=0` sentinel B-tree points at
  page 55.

Do not enforce a blanket rule that every multi-page `tt=0` page must be sealed,
or that only the terminal `tt=0` page may carry `flags=0x34`. Treat these flags
as transaction history unless a more specific cross-validator rejection case is
confirmed.

### Fresh write vs additive patch: what must match, what may differ

A populated PDB is only ever produced by the in-place additive writer (the
from-scratch `write_pdb` runs solely for the `initialize_usb` template). For the
tables whose footer the writer owns (`t00`–`t05`, `t07`, `t08`, `t13`, `t19`), an
additively-patched page's **footer convention must match what the from-scratch
writer would emit for the same row set**: `u5` (`0x20`), `num_rl` (`0x22`), and the
per-16-row-group `rowpf`/`tranrf` bitmasks. `apply_page_footer_convention` in
`pdb_writer/fresh.rs` is the single writer-side source of truth for this — both the
from-scratch writer (`build_data_page`) and every in-place page rewrite that
changes a page's row count call it, so the shape can't drift between the two paths.

These differences are expected and legitimate (they are *not* shape violations):

| Aspect | Why it differs |
| --- | --- |
| page indices, chain order, blank gap pages | additive appends onto the template; `first_page` is immovable |
| `seq` (`0x10`), `seqdb` (`0x14`), sentinel `seq` | additive continues the existing file's transaction counter |
| `page_flags` transaction bits (`0x24`/`0x34`) | additive never re-seals or normalises pre-existing pages |
| accumulated `tranrf` bits on **reused** `u5=1` heap pages | additive preserves existing footer groups and ORs in only the bits for appended rows (see "Row Footer" below) |
| `empty_candidate` under torn growth | can legitimately point past the physical file end |
| header `0x10` compatibility byte | additive copies the pre-export value verbatim |
| `t16`–`t18` menu pages | preserved verbatim from the template |
| seeded `t19` history page | a template-state or app-written page is rewritten for the track count; any other is preserved (see "`tt=19` activation" below) |

## Row Footer

Each data page has a footer at the end of the page. The footer is organized in
groups of up to 16 rows:

```text
[row offsets for the group] [rowpf] [tranrf]
```

`rowpf` marks active/live row slots. `tranrf` records transaction-row bits.
Tombstoned rows keep their row bytes but have their active bit cleared.

For freshly written pages:

- tables using `u5=1` write `tranrf` with only the bit for `num_rl` set;
- tables using `u5=trc` write `tranrf=rowpf`;
- `tt=19` writes a runtime profile where `rowpf` marks the newest row (the
  last row until the ring wraps) and `tranrf` carries that bit plus the
  previous row's bit when present.
  This holds across every footer group once a `tt=19` page needs more than
  one (more than 16 rows): only the group holding the true last row gets
  the active bit(s); every earlier group is fully zeroed. Confirmed against
  real rekordbox's own multi-group `tt=19` output (a reference export of 88
  history rows across 6 groups): `rowpf` has exactly one bit set globally,
  `tranrf` exactly two, both in the last group. A prior implementation only
  handled the single-group case (`n<=16`) and left every row in every later
  group falsely marked active once history accumulated past one group —
  this produced a PDB that crashed rekordbox desktop on load and caused a
  CDJ-2000NXS2 mount/eject loop; see `write_t19_runtime_footer_groups` in
  `pdb_writer/fresh.rs`.

For reused pages, the additive writer preserves existing footer groups and ORs
in only the bits for appended rows — **except `tt=19`**, whose additive and
in-place activation paths rewrite every footer group from scratch to preserve the
single-active-row invariant above; the generic OR-in-bits append path declines to
handle `tt=19` at all (`append_rows_to_existing_page_preserving_footer_state`
asserts against it). `apply_page_footer_convention` in `pdb_writer/fresh.rs` is the
single writer-side source of truth for `u5`/`num_rl` and the per-group `tranrf`
shape; every in-place page rewrite that changes a page's row count funnels
through it.

### `tt=19` activation

An empty device's `tt=19` page (rekordbox's own template and the one
`initialize_usb` writes) holds **one** sealed row: `nrs=1`, flags `0x24`,
`u5=1`, `rowpf=tranrf=0x0001`. Older players count **0 songs** while the page has this
shape, however many tracks `t00` holds.
rekordbox-initialized sticks never stay in this state, since rekordbox's own
export activates the page, so it only occurs on app-initialized USBs.

On its first export rekordbox keeps that template row, appends one more, and
turns the page active. A 1-track rekordbox reference export:

| field | template | after rekordbox's first export |
| --- | --- | --- |
| `nrs` / `0x19` | `1` / `0x20` | `2` / `0x20` |
| flags (`0x1b`) | `0x24` | `0x34` |
| `u5` / `num_rl` | `1` / `0` | `2` / `0` |
| `rowpf` / `tranrf` | `0x0001` / `0x0001` | `0x0002` / `0x0003` |

```text
row 0: 80 02 00 00 00 00 00 00 00 00 00 00 17 "2026-03-13" 19 1e 0b "1000" 03 00…
row 1: 80 02 20 00 01 00 00 00 00 00 00 00 17 "2026-03-13" 19 1e 0b "1000" 03 00…
```

Row 1 is a copy of row 0 with bytes `0..4` = `0x0280 + slot * 0x0020_0000` and
bytes `4..8` = its id; every other byte (date string, `"1000"`, trailing `03`)
is copied from row 0. rekordbox also points the `tt=19` sentinel's B-tree at
the now-active page (`ne=1`, `u7=1`).

**The page is a ring whose newest id is the song count.** Older players read
the newest row id as the USB's song count, not `t00`: rekordbox's 1-track
export ends at id 1, its 2-track export at id 2 (`nrs=3`, `num_rl=1`,
`rowpf=0x0004`, `tranrf=0x0006`). Ids fill slots in order until the page
holds 96 rows (96 × 40 bytes plus 6 footer groups fill the page exactly,
`free_size=0`); after that each new id overwrites slot `id % 2`, so slots
`2..95` keep ids `2..95`. Wrapped rekordbox rings seen on real USBs:

| rows | slots 0 / 1 / 2.. | `rowpf` / `tranrf` | `num_rl` |
| --- | --- | --- | --- |
| 96 × 40 bytes | ids 228 / 229 / 2.. | `0x0002` / `0x0003` | 0 |
| 80 × 48 bytes (rows carry a string) | ids 0 / 332 / 331, then 1.. | `0x0002` / `0x0006` | 1 |

**The rows follow the eDB's `property` row for the name and color.** Each
row holds, after the slot/id words: a flags word whose byte 9 is
`backGroundColorType`, then PDB strings for a date, two bytes holding the row
offsets of the next two strings (`19 1e`), the version `"1000"` and
`deviceName`, then at least 4
zero bytes up to a 4-byte boundary, and at least 40 bytes in all. A
non-ASCII name is a UTF-16 long string; rekordbox moves it from offset 30 to
32 with two zero bytes so its header is 4-byte aligned (`19 20`), as for other
long string headers (see the alignment section). Names up to 80 characters
(the app's limit) keep this row shape; rekordbox uses another one (`0x0284`,
u16 offsets) only for far longer names. The newest id is the PDB's track count. Seen on rekordbox's own USBs:

| USB | eDB `property` | t19 rows |
| --- | --- | --- |
| 2 tracks, unnamed | count 2, `2026-09-11`, color 0 | newest id 2, `2026-09-11`, byte 9 = 0, empty name |
| named `"APECAR_red"` | color 1, `2026-08-20` | `"APECAR_red"`, byte 9 = 1, `2026-08-20` |

Slot 0 (the template row) carries no name and color 0. Unnamed rows are 40
bytes; rekordbox's `"APECAR_red"` rows are 48, so its ring holds 80 slots,
and once wrapped its ids alternate in slots 1/2 (slot 0 keeps the template
row). A CDJ writes the same shape into rows it adds (`"Chiphead 256GB"`, 52
bytes). Players compare t19 with the eDB and give communication errors when
they disagree on the name or song count. Byte 9 is the player's background
(menu highlight) colour, which rekordbox keeps apart from the eDB's
`backGroundColorType` and lets differ (see "`tt=19` background colour" below).

The writer builds the rows from the eDB's `property` row (`T19Property`,
read by `edb_t19_property` after the eDB export has updated it): the name from
the eDB, the version always the app's `"1000"` (`T19_DB_VERSION`), and the
colour from the eDB only on a template-state page; a ring keeps its own colour.
The version is not copied from the eDB's `dbVersion`: libraries rekordbox
upgraded say `10000` there while their t19 rows keep `"1000"`, and `"1000"`
is what players were validated against. The date is the eDB's `createdDate`
only when the page is built from template state; a ring keeps its own date,
as rekordbox leaves the date in existing rows (older libraries keep a
2020/2021 date beside a newer `createdDate`). Without a readable eDB, a page
keeps the property its rows carry. `initialize_usb` stamps the template row
and the eDB's `createdDate` with the same date, the day it initializes the
USB, as rekordbox's own init does.

#### `tt=19` background colour

Byte 9 of a `t19` row is the player background colour, in rekordbox's order:
0 = default, 1 pink, 2 red, 3 orange, 4 yellow, 5 green, 6 aqua, 7 blue,
8 purple (rekordbox desktop showed 1 and 5 as pink and green; a CDJ-2000NXS
and CDJ-2000NXS2 showed them the same with every row of the live page set to
the colour). rekordbox keeps it apart
from the eDB's `backGroundColorType`: on a reference USB, a colour changed for
the PDB alone left the eDB at 0, and one changed for the eDB alone left the PDB
as it was. `DEVSETTING.DAT` does not change.

rekordbox writes a new colour into one more row: the next ring slot gets the
newest id and the colour, and `rowpf` marks only that row (a 154-track
reference USB: slot 1 = id 154, colour 6, `rowpf 0x0002`; slot 0 keeps id 154,
colour 0). After a rename on a full page the colour goes into the new page's rows.
The page is no longer a ring after such a change, so export leaves it alone.

The app reads the colour from the newest row on the live page (the last page
of the chain): the highest id among the rows whose `rowpf` bit is set, the last
such slot on a tie (`t19_background_color`). A CDJ-2000NXS and CDJ-2000NXS2
read the same row: with slot 0 and slot 1 holding different colours, they
showed the colour of the row `rowpf` marked, whichever slot that was, and
ignored the eDB's colour. `set_t19_background_color` sets every row anyway, so
the players show the colour whichever row they read:

- A ring (`t19_ring_state`) is rewritten as the ring for its own newest id with
  the new colour (`sync_t19_ring_page`), so it stays a ring that export keeps at
  the track count. Slot 0 stays the colourless template row. A CDJ-2000NXS
  and CDJ-2000NXS2 showed the colour and browsed the USB without a
  communication error, so they don't read the colour from slot 0 alone.
- Any other page (rekordbox's after a colour change or rename, the init
  template, rows a player added) gets byte 9 of every row on the live page set
  and nothing else changed: no footer, seq, row layout or chain.

Export and `repair_pdb_t19_template_state` keep a ring's colour (`t19_sync_target`).

In every one, `rowpf` marks the newest id's slot, `tranrf` that slot plus the
slot of the id before it, and `num_rl` is the lower of those two slots. The
writer derives the footer from the row ids the same way
(`write_t19_runtime_footer_groups` in `pdb_writer/fresh.rs`).

The writer builds this ring (`t19_ring_rows`) with the newest id at the
track count. `sync_t19_ring_page` in `pdb_writer/additive.rs` rewrites the
page on any export with tracks when `t19_sync_target` asks for it: the page
is still in template state (`t19_is_template_state`), or it is a ring
(`t19_ring_state`: exactly the rows `t19_ring_rows` builds for the property
they carry) whose newest id is not the track count or whose name, color or
version differs. This includes an export onto a PDB that already has tracks, so a
USB an older build left at 0 songs, or at the fixed 2-row page (players then
show 1 track and can hang on a communication error), recovers on its next
export. A page a player has added rows to is not a ring (a CDJ copies the
newest id into its rows) and is left untouched: the writer never rewrites or
drops a player's rows. Nor is a `tt=19` with more than one data page: only
rekordbox adds pages (a rename whose longer row doesn't fit retires the full
page and goes on in a new one), and the ring code reads and writes only a
single data page. `repair_pdb_t19_template_state` applies the same
change to an existing USB (`docs/DIAGNOSTICS_REPAIRS.md`).

The fresh writer (`write_pdb`) builds the same page: for a library with tracks
it turns a single `tt=19` row (or none, using the `initialize_usb` template row)
into the same ring, and indexes the active page in the sentinel B-tree.
More than one row is real history and passes through. All three paths share
`t19_ring_rows`, so a full rewrite, an additive export and the repair leave
the same `tt=19` page.

## Page Footer Conventions

`trc` means the total row-slot count on the page.

The current writer uses these `(u5, num_rl)` values for freshly written active
data pages:

| Table | Convention |
| --- | --- |
| `t00`, `t01`, `t02`, `t03`, `t04`, `t05`, `t08`, `t13` | `(1, trc - 1)` |
| `t06`, `t07`, `t16`, `t17`, `t18` | `(trc, 0)` |
| `t19` | `(2, max(trc - 2, 0))` |

Sentinel pages usually override these conventions and use `u5=8191` and
`num_rl=8191`. Observed `tt=0` sentinels can carry other transaction state,
including `u5=1, num_rl=0`.

Sealed overflow pages use `(1, trc - 1)`.

The row-slot count used for row traversal is read from the packed 3-byte
field at `0x18..0x1b` (`packed_page_row_slot_count` in `utils.rs`, the
canonical decode every module below imports rather than reimplementing), not
from `nrs` (the single byte at `0x18`) combined with `num_rl` (`0x22`).
Reading `nrs` alone wraps past 255 rows — a real risk for `t08`
playlist_entries specifically, since its 12-byte rows mean byte capacity
alone lets a single 4096-byte page hold well over 255 of them (see the
`MAX_ROWS_PER_PAGE` paragraph below). Earlier parser versions worked around
the wrap with a heuristic (`num_rl==8191` meant "page does not track
`num_rl`, trust `nrs`"; otherwise `max(nrs, num_rl)`), which could still
misread by up to a few rows depending on table convention. Decoding the
packed field directly is correct up to 8191 rows/page and needs no such
fallback. Call sites that switched from the old heuristic to this decode:
`pdb_writer`'s `page_has_transaction_tombstones`, `read_page_footer_state`,
`parse_t08_entries_from_page`, and `parse_page_row_slots`; `pdb_reader.rs`'s
`parse_page_rows` (the row parser behind the main public `parse_pdb_bytes`
entry point); and `repair/pdb_fixes.rs`'s `detect_pdb_zero_tranrf_pages_for_tables`,
where the old heuristic could silently skip inspecting a large playlist
page's true last footer group (257 rows is the smallest case that both wraps
and crosses a 16-row group boundary), letting real transaction-footer
corruption there go undetected by "repair USB." The `dump_*` developer
diagnostic binaries in `src/bin/` carry the same fix for consistency, though
they aren't part of the shipped app.

Two of the fixed sites keep a defensive Phase 2 scan that extends past the
header-reported count when the page's footer index space can hold more rows
than the header claims, in case the header count is ever wrong for a reason
other than the (now-fixed) byte wrap: `pdb_writer`'s
`parse_t08_entries_from_page` and `pdb_reader.rs`'s `parse_page_rows`, both
through `pdb_page::scan_footer_tolerant`.

The writer cannot rely on the parser's tolerance. It must emit page footer
values accepted by players.

Because `nrs` (offset `0x18`) is a single byte, no page may hold more than 255
rows. The writer enforces this as a hard cap (`MAX_ROWS_PER_PAGE` in
`pdb_writer/mod.rs`), independent of remaining byte capacity, in both the from-scratch
writer and in-place chain appends. The cap is reached first by tables with small
fixed-size rows: `t08` playlist_entries rows are 12 bytes, so byte capacity alone
would allow well over 255 on a single 4096-byte page. A page at 255 rows spills to
the next page in the chain, the same as a byte-full page.

## Sentinel B-Tree

Sentinel pages anchor table chains. The current `rebuild_sentinel_btrees_inplace`
path updates sentinel B-tree entries for `tt=0` and `tt=19`.

Known sentinel facts:

- sentinel pages use `page_flags=0x64`;
- most sentinel pages use `u5=8191` and `num_rl=8191`; observed `tt=0`
  sentinels can instead carry transaction state such as `u5=1, num_rl=0` while
  still using `page_flags=0x64`;
- B-tree entries store `page_index * 8`;
- entries index data pages with `page_flags=0x34`;
- stale or missing sentinel entries can make desktop library software reject the
  database.

After rebuilding sentinel entries, the writer bumps `seqdb` so it is greater
than the maximum page sequence number, with a minimum fresh-template floor of
`34`.

## Track Rows

Track rows live in `t00`. Known fixed fields written by the current encoder:

| Offset | Field |
| --- | --- |
| `0x00` | row subtype/header flags (`0x24` for normal track rows) |
| `0x02` | page-local `index_shift` |
| `0x04` | `content_link` |
| `0x08` | sample rate |
| `0x0c` | composer id (`t02` artist) |
| `0x10` | file size |
| `0x14` | master content id |
| `0x18` | master DB id |
| `0x1c` | artwork id |
| `0x20` | key id |
| `0x28` | label id (`t04` label) |
| `0x2c` | remixer id (`t02` artist) |
| `0x30` | bitrate |
| `0x34` | track number |
| `0x38` | tempo x100 |
| `0x3c` | genre id |
| `0x40` | album id |
| `0x44` | artist id |
| `0x48` | track id |
| `0x50` | release year |
| `0x52` | bit depth |
| `0x54` | duration seconds |
| `0x56` | constant `0x29` (u16) |
| `0x58` | colour: 0 (none) or a `t06` colour id 1–8, the eDB's `content.color_id` |
| `0x59` | star rating 0–5, the eDB's `content.rating` |
| `0x5a` | file type |
| `0x5e..` | 21 string offsets |

The writer assigns `index_shift = row_slot * 32` for `t00`, `t02`, and `t03`
rows that carry the field.

Track row length is not necessarily the end of the final visible string. Recent
reference exports include zero padding after string slot 20 (`track_file_path`)
that is part of the row payload and affects page capacity. In one multi-page
reference export, active `t00` rows average about 55 zero bytes after the encoded
media path; the app export of the same playlist currently writes only 0-4 bytes
of alignment tail. Readers must use the row-offset footer to find row
boundaries, not the final string length. Whether reference-style extra zero tail
matters to older hardware-player acceptance is unconfirmed.

Known file type codes used by export:

| Format | Code |
| --- | --- |
| MP3 | `1` |
| MP4 | `3` |
| M4A | `4` |
| FLAC | `5` |
| ALAC | `6` |
| WAV | `11` |
| AIFF | `12` |

## Track String Slots

Track rows contain 21 string slots. The current encoder writes these mapped
slots:

| Slot | Current meaning |
| --- | --- |
| `0` | ISRC |
| `2` | structural default |
| `3` | structural default |
| `6` | publish track info flag (`ON` when enabled) |
| `7` | autoload hot cues flag — written `ON` for every exported track, so hot cues written to the ANLZ/eDB auto-load on the player. Cue *data* itself lives in the ANLZ bundle + eDB `cue` table, not the PDB. |
| `10` | date added |
| `11` | release date |
| `14` | analysis path |
| `15` | date-added value used by current layout |
| `16` | DJ comment |
| `17` | title |
| `19` | file name |
| `20` | media path |

Unmapped string slots are kept structurally present. They are not removed or
collapsed.

String encoding used by the writer:

- short ASCII strings (up to 126 bytes) use a one-byte length marker;
- long ASCII strings use marker `0x40` plus length (Deep Symmetry has seen
  rekordbox write this form);
- non-ASCII strings use marker `0x90` plus UTF-16LE payload;
- path strings (slot 20) do **not** include a trailing NUL — `total_len` encodes exact char count.

Before metadata strings are encoded, export strips embedded NUL bytes, caps
any grapheme cluster (base character plus combining marks) to 8 codepoints,
caps the number of distinct Unicode scripts a single string may mix to 3, and
truncates to 255 Unicode characters. Long combining-mark stacks ("zalgo" text)
and names mixing many unrelated scripts (e.g. Braille, Yi, Georgian, Tibetan,
Tamil, Bengali, Arabic in one short artist name) have both independently been
observed to hang CDJ hardware — the first in text rendering, the second in
the Artist browse menu and track-load screen — even well under the
character-count limit and without any single grapheme cluster being deep.
See `docs/USB_EXPORT.md` for the full rationale. This covers PDB
playlist/dictionary names and track slots `16`, `17`, and `19`. The metadata
sanitizer is not applied to track media paths, analysis paths, or
key/tonality strings. Slot 20 (media path) is also exempt from this
sanitizer, but the path it holds is already built from sanitized components.

### Every long-form string slot (UTF-16 or long ASCII) must start at a 4-byte aligned row offset

Tested legacy hardware uses MIPS processors. MIPS requires natural alignment for
memory loads — a 4-byte read must be at a 4-byte aligned address. The DeviceSQL
UTF-16 string header (`0x90 [total_len: u16 LE] 0x00`) is 4 bytes wide. If a
UTF-16-encoded slot's row-relative offset is not divisible by 4, the MIPS CPU
raises an Address Error exception and the hardware player can freeze — while
loading a track (slot 20), while listing/browsing tracks (slot 17, title), or
elsewhere, depending on which slot the player happens to read at that moment.

This is not unique to slot 20 (media path). Any of the 21 track string slots
goes UTF-16 whenever its content is non-ASCII (title, filename, comment, etc.),
and each one needs the same alignment guarantee independently — a slot's
row-relative offset depends on the cumulative byte length of every slot before
it, so which offsets end up misaligned varies per track. Confirmed on real
hardware: a single track with a non-ASCII title (one curly apostrophe was
enough) produced a title slot at a misaligned offset and a COMM ERROR when the
CDJ tried to list the containing playlist, even though slot 20 for that same
row was correctly aligned.

Long ASCII strings (`0x40 [total_len: u16 LE] 0x00`, ASCII over 126 bytes)
have the same 4-byte header and need the same alignment. Confirmed on a
CDJ-2000nexus and a CDJ-2000NXS2: an app USB whose long ASCII comments (up to
255 characters) and one long title sat at row offset % 4 == 3 gave a
communication error as soon as the track list opened. Blanking those strings
made the list open; its long ASCII media paths (slot 20, always aligned) were
fine.

Rows an older build wrote misaligned stay as they are until the string repairs
are applied; diagnostics reports them as a FAIL ("Track/album string
alignment"), so the user sees they need Preview Fixes.

**The writer pads before any slot whose encoded bytes start with a long-form
marker (`0x90` UTF-16 or `0x40` long ASCII)**, uniformly across all 21 slots, plus slot 20 unconditionally
(kept unconditional for parity with reference exports, which always keep slot
20 aligned regardless of whether its content is ASCII or UTF-16). ASCII/empty
slots are untouched — their headers are a single byte, not read as an atomic
4-byte word, so alignment doesn't matter for them.

The same alignment rule applies to any other row type where a long-form string
header is read by the hardware decoder with a 4-byte load (artist and album
names over 126 ASCII bytes are padded like UTF-16 ones):
- `tt=2` artist row near-variant (0x0060): name at offset 12 (% 4 = 0 ✓).
  Offset 10 (% 4 = 2) caused a hardware freeze on UTF-16 artist names.
- `tt=3` album row near-variant (0x0080): reference exports place the name at
  offset 22 (% 4 = 2) for ASCII album names, which the writer keeps for
  parity — fine for the 1-byte ASCII string header. For non-ASCII (UTF-16)
  album names the writer instead pads to offset 24 (% 4 = 0 ✓); offset 22
  confirmed a hardware freeze in the CDJ's Album browse menu with a non-ASCII
  album name, the same failure mode as the artist row above. `ofs_name_near`
  (row byte 21) is self-describing — the reader must read it rather than
  assume a fixed offset, since it now varies by content.

## Playlist Rows

Playlist-tree rows live in `t07`:

| Offset | Field |
| --- | --- |
| `0x00` | parent playlist/folder id |
| `0x08` | sibling sort order |
| `0x0c` | playlist/folder id |
| `0x10` | folder flag |
| `0x14` | encoded name |

Playlist-entry rows live in `t08`:

| Offset | Field |
| --- | --- |
| `0x00` | entry order/index |
| `0x04` | track id |
| `0x08` | playlist id |

Mirror export removes target playlist-entry rows in place and preserves table
chains and tombstone state. Additive export preserves existing membership and
adds missing entries only.

The exported playlist is placed first among its siblings by fixed-width
`t07.sort_order` patches. eDB `playlist.sequenceNo` is updated to the same
parent-scoped order.

## Playlist Reordering

Outside of export, the `reorder_usb_playlists` command lets the user
drag-and-drop reorder the USB playlist list in the UI. Unlike the
export-time placement above, this reassigns `t07.sort_order` as one
monotonically increasing sequence across every PDB-backed leaf playlist
in the requested order — it does not scope by parent/folder, matching how
`fetch_usb_playlists` already sorts and displays the flat playlist list.
eDB-only playlists (no PDB row) are left untouched since they have no
`t07` row to patch. The command reuses the same in-place `t07.sort_order`
byte patcher used by strict-repair order restoration
(`patch_playlist_tree_sort_orders_in_place`), then re-syncs eDB
`playlist.sequenceNo` from the freshly patched PDB so both surfaces agree
and strict-parity ordering checks stay green.

## Menu Catalog

PDB `t16` stores the hardware browse-category catalog. Normal playlist export
does not rewrite `t16`, `t17`, or `t18`.

Fresh initialized USBs seed 27 browse categories:

| Kind | Label |
| --- | --- |
| `128` | GENRE |
| `129` | ARTIST |
| `130` | ALBUM |
| `131` | TRACK |
| `132` | PLAYLIST |
| `133` | BPM |
| `134` | RATING |
| `135` | YEAR |
| `136` | REMIXER |
| `137` | LABEL |
| `138` | ORIGINAL ARTIST |
| `139` | KEY |
| `140` | DATE ADDED |
| `141` | CUE |
| `142` | COLOR |
| `144` | FOLDER |
| `145` | SEARCH |
| `146` | TIME |
| `147` | BITRATE |
| `148` | FILE NAME |
| `149` | HISTORY |
| `150` | COMMENTS |
| `151` | DJ PLAY COUNT |
| `152` | HOT CUE BANK |
| `161` | DEFAULT |
| `162` | ALPHABET |
| `170` | MATCHING |

Each `t16` row contains:

| Offset | Field |
| --- | --- |
| `0x00` | id |
| `0x02` | kind code |
| `0x04` | UTF-16LE long-string marker `0x90` |
| `0x05` | string length |
| `0x07` | pad byte |
| `0x08` | display label wrapped in `U+FFFA` and `U+FFFB` |

eDB `menuItem` stores the catalog for the eDB side. eDB `category` stores the
active/visible subset and order. Removing a category from Active Category hides
it in eDB `category`; normal export does not delete the PDB `t16` catalog row.

## Export Writer Behavior

The additive writer classifies the desired next PDB against the existing PDB.
It accepts changes that can be represented without unsafe topology changes:

- append new dictionary rows, artwork rows, track rows, playlist-tree rows, and
  playlist-entry rows;
- patch known same-size `t00` scalar fields and same-length string slots;
- when a mapped `t00` string changes length, mark the old active row inactive
  and append a replacement row with the same track id. The replacement keeps
  the old row's fixed bytes and every string slot the change doesn't touch
  (including slots the writer doesn't model); only changed mapped slots are
  re-encoded, and the slots are laid out again with 4-byte alignment;
- patch `t07.sort_order` in place;
- rewrite a template-state or app-written `t19` page for the track count (see
  "`tt=19` activation").

It rejects changes that require unsafe rewrite behavior, including:

- moving critical table `first_page` pointers;
- moving `t07` `last_page` during normal export;
- rewriting menu tables during normal export;
- removing existing tracks from the PDB manifest;
- changing existing dictionary or playlist-tree names in ways not supported by
  fixed-width patching;
- appending a row too large to fit on a single PDB page.

The per-table append order is:

1. playlist-tree sort-order patches
2. playlist-tree rows
3. genres
4. labels
5. keys
6. artists
7. albums
8. artwork
9. new tracks
10. track metadata mutations
11. playlist-entry removals for mirror mode
12. playlist-entry additions
13. runtime history rows when needed

## Strict Parity

See `docs/DIAGNOSTICS_REPAIRS.md` "What Strict Parity Means" for why this
check exists (PDB and eDB are two independently-read databases that must
agree, not just each be individually valid) and how it differs from
operational diagnostics.

PDB participates in strict parity validation with eDB. Core checks include:

- matching playlist rows on both sides;
- matching playlist membership counts and order;
- no duplicate PDB playlist entries for the same playlist, track, and order;
- required metadata presence in PDB-linked track rows;
- media path and analysis path consistency;
- artist, album, key, and artwork dictionary id resolution.

A USB can be playable while still failing strict parity. Parity reports do not
mutate databases. Repairs are explicit actions.

## Known Unknowns

These PDB areas are still not fully decoded:

- semantics of several track string slots that the writer keeps structurally
  present but does not map to app data;
- some fixed-byte ranges in track rows outside pinned fields;
- exact meanings of all transaction-history states found on existing export
  pages;
- all optional metadata families beyond the fields currently written and
  validated by export/parity.

The current policy is to preserve unknown structural bytes where possible and
to reject unsafe rewrites instead of inventing unsupported field meanings.
