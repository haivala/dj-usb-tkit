# External Libraries (rekordbox + Mixxx)

## How it works

Besides music folders, the library can take tracks from two DJ applications'
own libraries on the same computer:

- **rekordbox**: the desktop library `master.db`
- **Mixxx**: the library `mixxxdb.sqlite`

Both are found automatically at startup. When one is found, the Media Library
gets a source chip for it (**rekordbox** / **Mixxx**) and an import button
(**Import RB** / **Import Mixxx**). If no music folder has been added yet, the
empty library screen offers the same imports.

Both databases are only ever read. Nothing is written back to rekordbox or
Mixxx, and either application can stay open while importing.

### Importing a library

**Import RB** / **Import Mixxx** brings every track of that library into the
app's library and turns its source chip on:

| | rekordbox | Mixxx |
| --- | --- | --- |
| Title, artist, album | yes | yes |
| BPM, key, length | yes | yes (plus sample rate) |
| Cover image | yes (rekordbox's artwork file) | only a cover image *file* next to the track; embedded covers come from the app's own analysis |
| Waveform | rekordbox's own analysis files are used in place | no; the app's analysis makes one |
| Hot cues | yes: hot cues (and hot loops) on pads A–H, with colour and name; a memory cue before the first hot cue becomes the playback-start cue | yes: hot cues and saved loops on pads 1–8, with colour and label; the main cue becomes the playback-start cue |

Mixxx stores its waveforms and beat grids in its own formats, so Mixxx tracks
need the app's analysis before export, like any folder track. rekordbox tracks
point at rekordbox's analysis files instead.

Running an import again updates the tracks already imported. Your own changes
win: BPM, key and length are only filled in when the track has none yet, and
cues are only added to a track that has no cues in the app.

The app's cue list is up to 8 cue points plus one playback-start cue, and
cue points take pads A–H in position order. So an imported hot cue can land on
a different pad letter than in rekordbox or Mixxx if the pads weren't in
position order there. rekordbox memory cues other than the one before the
first hot cue aren't imported; the Event Log counts them.

Some tracks are skipped, with the reason in the Event Log:

- the file no longer exists (a track imported earlier is removed from the library)
- the file isn't one the library takes, such as a video or a tracker module
  (`.it`, `.xm`, …), the same rule the folder scan uses
- the track was deleted in rekordbox / Mixxx

### Source chips

A library's chip only filters the library view: it shows or hides that
library's tracks and never re-imports. The two chips are independent. A track
that is in both libraries shows when either chip is on.

### Importing a playlist

The **Import** button under **New** in the sidebar's Playlists section appears
when either library is found. It opens a picker listing, per library:

- **rekordbox playlists**, in rekordbox's order. A playlist inside a folder
  shows as `Folder / Name`.
- **rekordbox history**, newest first.
- **Mixxx playlists**, including Auto DJ when it has tracks.
- **Mixxx crates**.
- **Mixxx history** (set logs), newest first.

Only lists with tracks are shown. rekordbox smart playlists aren't listed,
because rekordbox doesn't store their tracks.

The chosen list becomes a new playlist with the same name and track order
(Mixxx crates have no order, so their tracks are sorted by artist and title).
Its tracks are imported into the library the same way as with Import RB /
Import Mixxx, the library's source chip is turned on, and the new playlist
opens. A track listed twice is added once.

If none of the list's tracks can be imported, for example a rekordbox library
from another computer whose file paths don't exist here, no playlist is
created and the status line says why. Importing the same list twice creates
two playlists.

### When both libraries have the same file

Tracks are matched by file path, so a file in both libraries is one track in
the app:

- it carries both source flags and shows when either chip is on
- title, artist and album: whichever import ran last
- BPM, key, length: the first value found is kept
- waveform: rekordbox's analysis file
- cover image: whichever import copied one last
- hot cues: from whichever import finds the track without cues first

## Deep technical details

### Where the databases are looked for

| Library | Environment override | Locations tried, in order |
| --- | --- | --- |
| rekordbox | `DJUSBTKIT_MASTER_DB_PATH` | macOS `~/Library/Application Support/Pioneer DJ/rekordbox/master.db`, then `~/Library/Application Support/Pioneer/rekordbox/master.db`, then `~/Library/Pioneer/rekordbox/master.db`; Windows `%APPDATA%\Pioneer\rekordbox\master.db` (or the same under `%USERPROFILE%\AppData\Roaming`) |
| Mixxx | `DJUSBTKIT_MIXXX_DB_PATH` | Linux `~/.mixxx/mixxxdb.sqlite`; macOS `~/Library/Containers/org.mixxx.mixxx/Data/Library/Application Support/Mixxx/mixxxdb.sqlite`, then `~/Library/Application Support/Mixxx/mixxxdb.sqlite`; Windows `%LOCALAPPDATA%\Mixxx\mixxxdb.sqlite` |

`master.db` is SQLCipher-encrypted with rekordbox's fixed desktop key.
`mixxxdb.sqlite` is plain SQLite. Both are opened with
`SQLITE_OPEN_READ_ONLY`.

### Code layout

- `backend/src/service/rekordbox_import.rs`: `scan_master_db`,
  `list_rekordbox_playlists`, `import_rekordbox_playlist`
- `backend/src/service/mixxx_import.rs`: `detect_external_mixxx_db`,
  `scan_mixxx_db`, `list_mixxx_playlists`, `import_mixxx_playlist`

Each module has one per-track importer (`MasterDbTrackImporter`,
`MixxxTrackImporter`) that the whole-library import and the playlist import
share, so a track is treated the same whichever way it arrives. The playlist
import runs in one transaction; when it finds no importable track it returns
an error before creating the playlist, which also rolls back the track
upserts.

`tracks.master_db_source` and `tracks.mixxx_db_source` record where a track
came from. The library filter requests (`browse_source_files`,
`list_matching_track_ids`, `add_library_selection_to_playlist`,
`analyze_new_tracks`) carry `includeMasterDb` / `includeMixxxDb`; a track is
included when `(includeMasterDb && masterDbSource) || (includeMixxxDb &&
mixxxDbSource)`.

### rekordbox `master.db` fields read

- Tracks: `djmdContent` (text `ID`; `FolderPath` is the full file path;
  `BPM` is centi-BPM; `Length` in seconds), joined to `djmdArtist`,
  `djmdAlbum` and `djmdKey.ScaleName`. `AnalysisDataPath` / `ImagePath` are
  `/PIONEER/...` paths under rekordbox's `share` folder.
- Playlists: `djmdPlaylist` (`Attribute` 0 = playlist, 1 = folder,
  4 = smart playlist; top level has `ParentID = 'root'`; ordered by `Seq`)
  and `djmdSongPlaylist` (`PlaylistID`, `ContentID`, ordered by `TrackNo`).
- Cues: `djmdCue` (`ContentID`, `InMsec` in milliseconds, `Comment`,
  `ColorTableIndex`). `Kind` 0 is a memory cue and hot-cue pads A–H are
  `Kind` 1, 2, 3, 5, 6, 7, 8, 9 (rekordbox skips 4). Hot cues become cue
  points; the earliest memory cue becomes the playback-start cue when it lies
  before the first hot cue. `ColorTableIndex` is read with the same codes the
  app writes to the USB eDB's `cue.colorTableIndex` (its palette ids 1–8);
  unset or other values get the default colour. Colours set in rekordbox are
  not yet validated against real data.
- History: `djmdHistory` (sessions are `Attribute` 0 under year / month
  folders; newest first by `DateCreated`) and `djmdSongHistory`
  (`HistoryID`, `ContentID`, `TrackNo`).
- Rows with `rb_local_deleted = 1` are ignored everywhere.

### Mixxx `mixxxdb.sqlite` fields read

- Tracks: `library` joined to `track_locations` on `library.location`;
  rows with `library.mixxx_deleted` or `track_locations.fs_deleted` set are
  ignored. `duration` is in seconds, `bpm` a float.
- Key: `library.key_id` is Mixxx's ChromaticKey (1–12 = C..B major, 13–24 =
  Cm..Bm minor), mapped to the app's classic key names. When it's unset, the
  `key` text is used if it's a key the app recognises; Mixxx writes that text
  in the user's chosen notation (`Am`, `8A`, `1m`, …).
- Cover: `coverart_type = 2` (file) with `coverart_location` relative to the
  track's folder (or absolute). Embedded covers (`coverart_type = 1`) are
  left to analysis.
- Cues: `cues.type` 1 (hot cue) and 4 (saved loop) with `hotcue >= 0`
  become cue points, ordered by position, deduplicated by position and
  capped at 8. Type 2 (main cue) becomes the playback-start cue when it lies
  before the first hot cue. Intro, outro and other markers are ignored.
  `position` is in interleaved stereo samples:
  `ms = position / (2 × samplerate) × 1000`. `color` (`0xRRGGBB`) maps to the
  nearest colour of the app's cue palette.
- Playlists: `Playlists.hidden` 0 = playlist, 1 = Auto DJ, 2 = history set
  log (others, such as the history placeholder, are skipped), with
  `PlaylistTracks` ordered by `position`. Crates: `crates` +
  `crate_tracks`.
- Columns that older Mixxx versions lack (`key_id`, `coverart_*`,
  `mixxx_deleted`, `fs_deleted`) are read as `NULL`.

Validated against Mixxx 2.5.6 and a rekordbox 6 `master.db`.
