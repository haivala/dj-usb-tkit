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
| `t11` | history_playlists_alt | not named in the Deep Symmetry table list; left empty by the current writer; reader fallback when `t17` is empty |
| `t12` | history_entries_alt | not named in the Deep Symmetry table list; left empty by the current writer; reader fallback when `t18` is empty |
| `t13` | artwork | artwork path dictionary rows |
| `t14` | unlisted/other | not named in the Deep Symmetry table list; left empty by the current writer |
| `t15` | unlisted/other | not named in the Deep Symmetry table list; left empty by the current writer |
| `t16` | columns | player browse-category catalog; Deep Symmetry marks details as unconfirmed |
| `t17` | history_playlists | Deep Symmetry: History menu playlist list; normal export preserves it; menu repair may rewrite it as an eDB category snapshot |
| `t18` | history_entries | Deep Symmetry: links tracks to history playlist entries; normal export preserves it |
| `t19` | history | Deep Symmetry: history synchronization data; runtime rows synthesized for initialized templates |

Deep Symmetry documents table type numbers in hexadecimal. In this document the
`tNN` labels are decimal table numbers, so Deep Symmetry `0x11`, `0x12`, and
`0x13` correspond to `t17`, `t18`, and `t19` here. Deep Symmetry does not name
decimal `t09`, `t10`, `t11`, `t12`, `t14`, or `t15` in its `export.pdb` table
list.

The current writer leaves `t09`-`t12` and `t14`-`t15` empty during normal
export. The reader can parse `t11` and `t12` as legacy history-family fallback
tables when `t17` and `t18` are empty.

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
- `tt=19` writes a runtime profile where `rowpf` marks the last row bit and
  `tranrf` carries the last row bit plus the previous row bit when present.

For reused pages, the additive writer preserves existing footer groups and ORs
in only the bits for appended rows.

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

The parser handles two compatibility cases:

- `nrs` can wrap on pages with more than 255 rows.
- `num_rl=8191` means the page does not track `num_rl`; the parser uses `nrs`
  for row traversal.

The writer cannot rely on the parser's tolerance. It must emit page footer
values accepted by players.

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
| `0x10` | file size |
| `0x14` | master content id |
| `0x18` | master DB id |
| `0x1c` | artwork id |
| `0x20` | key id |
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
| `7` | autoload hot cues flag (`ON` when enabled) |
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

- short ASCII strings use a one-byte length marker;
- long ASCII strings use marker `0x40` plus length;
- non-ASCII strings use marker `0x90` plus UTF-16LE payload;
- path strings (slot 20) do **not** include a trailing NUL — `total_len` encodes exact char count.

Before metadata strings are encoded, export strips embedded NUL bytes and
truncates to 255 Unicode characters. This covers PDB playlist/dictionary names
and track slots `16`, `17`, and `19`. The metadata sanitizer is not applied to
track media paths, analysis paths, or key/tonality strings.

### Slot 20 must start at a 4-byte aligned row offset

Tested legacy hardware uses MIPS processors. MIPS requires natural alignment for
memory loads — a 4-byte read must be at a 4-byte aligned address. The DeviceSQL
UTF-16 string header (`0x90 [total_len: u16 LE] 0x00`) is 4 bytes wide. If slot
20's row-relative offset is not divisible by 4, the MIPS CPU raises an Address
Error exception and the hardware player can freeze when loading a track.

**The writer must pad slot 19 (filename) with zero bytes to ensure slot 20 starts
at a 4-byte aligned row-relative offset.** Confirmed against reference exports:
all observed reference-exported slot 20 offsets are divisible by 4.

The same alignment rule applies to any other row type where a UTF-16 string
header is read by the hardware decoder with a 4-byte load:
- `tt=2` artist row near-variant (0x0060): name at offset 12 (% 4 = 0 ✓).
  Offset 10 (% 4 = 2) caused a hardware freeze on UTF-16 artist names.
- `tt=3` album row near-variant (0x0080): reference exports place the name at
  offset 22 (% 4 = 2). The writer keeps this shape for parity and still uses
  the normal DeviceSQL string encoder, including UTF-16LE for non-ASCII album
  names.
- `tt=0` slot 17 (title): starts at offset 232 (% 4 = 0 ✓) when all tracks have
  the same structure for slots 0–16.

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
  and append a replacement row with the same track id;
- patch `t07.sort_order` in place;
- synthesize `t19` runtime rows for initialized templates.

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
