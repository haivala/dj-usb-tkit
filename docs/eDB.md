# eDB

This document describes the eDB database used on USB exports. It
records facts implemented by this repository and facts required for export,
import, diagnostics, and repair behavior.

## How eDB Works

`exportLibrary.db` is the encrypted SQLite database stored at
`PIONEER/rekordbox/exportLibrary.db` on a USB export. In this repository, eDB
means encrypted DB.

eDB sits beside `export.pdb`, the legacy binary PDB database:

- CDJ-3000 reads eDB as the primary database.
- CDJ-2000NXS2 and older players read PDB as the primary database.
- Desktop DJ software validates the USB as a combined database set.

eDB contains the richer relational model for tracks, playlists, dictionaries,
history, navigation menus, user tags, and recommendation metadata. PDB contains
the legacy player-oriented binary surface. A compatible export has matching
playlist and track meaning across both databases, but the two files are not
interchangeable mirrors.

Normal export writes eDB playlist and track data first, then writes PDB so the
PDB playlist ids and ordering match the eDB result. `exportExt.pdb` can carry
newer extension tag data on some vendor USBs, but this app does not write that
file and keeps any existing `exportExt.pdb` byte-stable.

## Compatibility Rules

Known eDB rules:

- eDB is a SQLCipher-encrypted SQLite database.
- The baseline schema listed here is what this repository initializes for fresh
  USB roots.
- Existing USBs can have schema variants. The implementation probes table and
  column existence before writing optional fields.
- eDB `playlist_content` is authoritative for eDB-side playlist membership and
  order.
- eDB `category` is the Active Category state; hidden categories remain present
  with `isVisible = 0` and `sequenceNo = 0`.
- eDB `menuItem` remains the full category catalog; normal playlist export does
  not trim it.
- Strict parity compares eDB against PDB, but parity reports do not mutate
  either database.

## Schema Layout

The baseline eDB schema contains these table families:

| Table family | Tables | Current use |
| --- | --- | --- |
| content and metadata dictionaries | `content`, `artist`, `album`, `key`, `genre`, `label`, `color`, `image` | track metadata and id-to-name/path resolution |
| playlist and membership | `playlist`, `playlist_content` | playlist tree rows and ordered membership |
| history and membership | `history`, `history_content` | imported and repaired history sessions |
| cue and hot-cue banks | `cue`, `hotCueBankList`, `hotCueBankList_cue` | baseline schema and cue/hot-cue references |
| navigation/config | `property`, `menuItem`, `category`, `sort` | device/db metadata, browse catalog, visible category state, sort state |
| user tags and recommendations | `myTag`, `myTag_content`, `recommendedLike` | user/My Tag tree, tag-to-track links, recommendation rows |

Baseline indexes currently initialized:

| Index | Table/column |
| --- | --- |
| `index_hotCueBankList_cue_hotCueBankList_id` | `hotCueBankList_cue.hotCueBankList_id` |
| `index_myTag_content_content_id` | `myTag_content.content_id` |
| `index_myTag_content_myTag_id` | `myTag_content.myTag_id` |
| `index_playlist_content_playlist_id` | `playlist_content.playlist_id` |

## Content Rows

`content` is the central track metadata table used by import, export, parity,
and repair.

Known `content` columns:

| Column | Usage in app |
| --- | --- |
| `content_id` | primary track identity key; playlist/history membership linkage |
| `title` | core display/parity metadata |
| `titleForSearch` | search-oriented normalized title metadata |
| `subtitle` | optional subtitle/mix-name family |
| `bpmx100` | tempo parity and DJ metadata |
| `length` | duration parity and API duration mapping |
| `trackNo` | track number parity |
| `discNo` | optional disc metadata |
| `artist_id_artist` | primary artist dictionary linkage |
| `artist_id_remixer` | remixer linkage |
| `artist_id_originalArtist` | original-artist linkage |
| `artist_id_composer` | composer linkage |
| `artist_id_lyricist` | lyricist linkage |
| `album_id` | album dictionary linkage |
| `genre_id` | genre dictionary linkage |
| `label_id` | label dictionary linkage |
| `key_id` | key dictionary linkage |
| `color_id` | color dictionary linkage |
| `image_id` | artwork linkage into `image` table |
| `djComment` | comment metadata used in parity/roundtrip |
| `rating` | rating metadata used in diagnostics/parity |
| `releaseYear` | release-year metadata |
| `releaseDate` | export release-date policy target |
| `dateCreated` | origin-date fallback chain target |
| `dateAdded` | export-run date target |
| `path` | core media path used for identity and parity |
| `fileName` | optional file-name metadata |
| `fileSize` | metadata parity and diagnostics |
| `fileType` | metadata parity |
| `bitrate` | metadata parity |
| `bitDepth` | metadata parity |
| `samplingRate` | metadata parity |
| `isrc` | identifier metadata |
| `djPlayCount` | play-count metadata |
| `isHotCueAutoLoadOn` | hot-cue auto-load export flag |
| `isKuvoDeliverStatusOn` | publish/export flag |
| `kuvoDeliveryComment` | optional delivery-comment metadata |
| `masterDbId` | app-owned export identity scope |
| `masterContentId` | app-owned per-track export identity |
| `analysisDataFilePath` | analysis bundle path used by import/export/parity |
| `analysedBits` | analysis/status bitfield used in export policy |
| `contentLink` | app-owned identity/linkage value |
| `hasModified` | change state tracking |
| `cueUpdateCount` | cue-update counter metadata |
| `analysisDataUpdateCount` | analysis-update counter metadata |
| `informationUpdateCount` | information-update counter metadata |

Export updates `content` rows for exported tracks from the app's canonical
playlist/track model.

## Playlist and History Rows

Playlist rows live in `playlist`:

| Column | Usage |
| --- | --- |
| `playlist_id` | playlist identity |
| `sequenceNo` | sibling/order field |
| `name` | UI-visible playlist or folder name |
| `image_id` | optional playlist artwork link |
| `attribute` | folder/list semantics and filtering |
| `playlist_id_parent` | hierarchy support |

Playlist membership rows live in `playlist_content`:

| Column | Usage |
| --- | --- |
| `playlist_id` | playlist id |
| `content_id` | linked track id |
| `sequenceNo` | order inside the playlist |

History rows live in `history` and `history_content`:

| Table | Usage |
| --- | --- |
| `history` | history playlist identity, order, name, attribute, and parent id |
| `history_content` | ordered track membership for history sessions |

Import and repair can synchronize history from PDB-derived history payloads.

## Dictionary and Artwork Rows

Dictionary and artwork tables resolve ids used by `content` rows:

| Table | Usage |
| --- | --- |
| `artist` | artist id to name/search name |
| `album` | album id to name, artist id, optional image id, search name |
| `key` | key id to key name |
| `genre` | genre id to name |
| `label` | label id to name |
| `color` | color id to name |
| `image` | artwork image id to path |

Diagnostics and strict parity use these tables to detect dictionary-id
collapses, for example a non-resolving artist, album, key, or artwork id where
linked metadata should exist.

## Navigation and Menu Tables

Navigation/config rows live in `property`, `menuItem`, `category`, and `sort`.

| Table | Usage |
| --- | --- |
| `property` | device/db metadata, content count, background color type, My Tag master DB id |
| `menuItem` | full browse-category catalog: kind and display name |
| `category` | Active Category state: linked `menuItem`, visible order, hidden state |
| `sort` | visible/selected sort settings per menu item |

Initialization seeds these tables with known-good defaults so fresh USB roots
start with complete menu and sort metadata.

### Active Category Behavior

`menuItem` is the category catalog. `category` is the Active Category state: it
links selected `menuItem` rows and stores their visible order.

When a category is removed from Active Category, vendor software keeps the
catalog row and hides the `category` row instead of deleting the corresponding
PDB `t16` catalog row. For KEY (`kind = 139`), the resulting state is:

- `menuItem`: KEY remains present.
- `category`: KEY remains present but hidden (`isVisible = 0`, `sequenceNo = 0`).
- PDB `t16`: KEY remains present in the full 27-row catalog.

The app follows this behavior for menu saves. PDB `t16` repair is a separate
operation used only when the PDB catalog itself has been truncated.

## User Tag Rows

User tag rows live in `myTag` and `myTag_content`:

| Table | Usage |
| --- | --- |
| `myTag` | user/My Tag tree: top-level tag groups and child tags |
| `myTag_content` | tag-to-track links through `myTag_id` and `content_id` |

Fresh initialization seeds the user tag tree with the baseline groups and child
tags used by the current schema. `exportExt.pdb` can also carry newer extension
tag data on vendor USBs, but this app does not currently write it.

## Export Writer Behavior

Export writes eDB from one canonical playlist/track model, including:

- playlist rows and ordered playlist membership;
- core `content` metadata for exported tracks;
- media path, analysis path, identity fields, and dictionary/artwork links;
- date fields with explicit policy: `dateAdded`, `dateCreated`, and
  `releaseDate`;
- eDB `playlist.sequenceNo` matching the same parent-scoped order used for PDB
  `t07.sort_order`.

Export writes eDB before PDB. The PDB writer then uses the eDB playlist id and
sort order so both database surfaces describe the same exported playlist.

## Repair and Parity Behavior

Repair flows can:

- remove stale `content` and `playlist_content` references;
- synchronize eDB history tables from parsed PDB-derived history payloads;
- apply strict parity upgrade behavior while preserving the report-vs-repair
  boundary;
- mirror eDB `category` state from PDB menu kinds for explicit menu repair;
- rebuild PDB `t16` and PDB `t17` menu/category surfaces from eDB menu data
  when the user explicitly requests the PDB menu restore action.

Strict parity compares concrete PDB/eDB relationships:

- playlist existence on both sides;
- playlist membership counts and ordering;
- per-track core metadata presence and value alignment;
- media and analysis path consistency;
- dictionary-id resolution for artist, album, key, and artwork references.

Most shared fields are `must_match`. Current allowed-difference fields are
`analysisDataFilePath`, `contentLink`, `masterContentId`, `masterDbId`, and
`key.name`.

## Known Unknowns

These eDB areas are intentionally handled conservatively:

- optional/variant schema columns on USBs not initialized by this app;
- optional columns such as `imageFilePath_id` that may appear on some USBs;
- tag data carried only through `exportExt.pdb`, which this app keeps
  byte-stable rather than rewriting;
- optional cue, hot-cue-bank, recommendation, and metadata families not actively
  mutated by normal export.

The current policy is to probe schema shape before writing optional fields and
to keep report commands read-only.
