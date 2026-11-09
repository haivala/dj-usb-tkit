# The text catalog: every user-facing text, in English. Other languages are
# locales/<language>.ftl with the same keys; a key missing there falls back
# to this file.
#
# Keys: [ui-]<area>-<thing>[-<variant>][-<attribute>], lowercase kebab-case.
# `ui-` marks a key JS or index.html reads. See "Keys and comments" in
# USB_TEXT_CATALOG_PLAN.md until docs/ has the translator guide.
#
# Areas:
#   pages:    library, usb, usb-playlists, usb-history, usb-player-menu, playlist,
#             event-log, backups, track-detail, settings, header, sidebar
#   dialogs:  add-to, convert, drive-name, group-edit, help, playlist-import,
#             tracklist-export
#   JS only:  job (also the progress bar), update, playback
#   Rust:     export, diagnostics, parity, repair, track-list, analysis, scan, job,
#             migration, transcode, error
#   shared:   common (generic dialog buttons and words only); markup repeated on
#             several pages: selection-bar, track-table, rating; and one area per
#             <template> used on more than one page, named from its id
#             (tplTrackRow is track-row): track-row, track-row-empty, cover-img
# A piece of work that needs a new area adds it here.

## usb

# The warning count on a USB status line, after its result
# ("Playlist order saved | (2 warnings)").
# $count: how many warnings the operation logged; never 0, which shows nothing.
usb-warning-count = { $count ->
    [one] { $count } warning
   *[other] { $count } warnings
}

# The status line after choosing a USB folder that's ready. $path: the folder.
usb-root-selected = USB root selected: { $path }. Running diagnostics...
# The same, with what the check noticed. $warnings: the warnings, joined with " | ".
usb-root-selected-with-warnings = USB root selected: { $path } ({ $warnings }). Running diagnostics...
# The status line for a writable folder without the rekordbox structure, and the hint beside Initialize.
usb-root-not-initialized = USB selected but not initialized. Click "Initialize USB Structure" to continue.
usb-root-init-hint-missing-structure = USB folder is writable but missing External library structure
# $warnings: what the check found, joined with " | ".
usb-root-init-hint-missing-structure-with-warnings = USB folder is writable but missing External library structure ({ $warnings })
# The status line for a folder that can't be used, and the hint beside Initialize.
# $reason: what the check found, or usb-root-invalid-unknown.
usb-root-invalid = USB root invalid: { $reason }
usb-root-invalid-unknown = invalid USB root
usb-root-init-hint-not-ready = USB folder is not ready for initialization
# $warnings: what the check found, joined with " | ".
usb-root-init-hint-not-ready-with-warnings = USB folder is not ready for initialization ({ $warnings })
# The status line after Initialize USB Structure.
usb-initialized = USB initialized
# The status lines after naming the drive. $name: the new name.
usb-drive-renamed = Drive renamed to { $name }.
usb-drive-name-unchanged = Drive name unchanged.


# The progress bar while the job runs.
usb-progress-resolving-root = USB: Resolving root
usb-progress-parsing-pdb = USB: Parsing PDB
usb-progress-reading-edb = USB: Reading eDB
usb-progress-resolving-edb-candidates = USB: Resolving eDB candidates
usb-progress-resolving-playlists = USB: Resolving playlists
usb-progress-finalizing-playlist-import = USB: Finalizing playlist import
usb-progress-resolving-usb-root = USB: Resolving USB root
usb-progress-backing-up-usb-databases = USB: Backing up USB databases
usb-progress-computing-new-playlist-order = USB: Computing new playlist order
usb-progress-writing-pdb-playlist-order = USB: Writing PDB playlist order
usb-progress-writing-edb-playlist-order = USB: Writing eDB playlist order
usb-progress-playlist-order-saved = USB: Playlist order saved
usb-progress-resolving-playlist-identifiers = USB: Resolving playlist identifiers
usb-progress-analyzing-shared-tracks = USB: Analyzing playlists and shared tracks
usb-progress-deleting-exclusive-track-files = USB: Deleting exclusive track files
usb-progress-cleaning-edb = USB: Cleaning eDB
usb-progress-cleaning-empty-directories = USB: Cleaning empty directories
usb-progress-finalizing = USB: Finalizing
usb-progress-playlist-removal-completed = USB: Playlist removal completed
usb-progress-reading-supplemental-databases = USB: Reading supplemental databases
usb-progress-building-track-index = USB: Building track index
usb-progress-resolving-history-entries = USB: Resolving history entries
usb-progress-finalizing-history-import = USB: Finalizing history import

# The rename-drive dialog. $max: the longest name allowed.
usb-error-name-empty = Enter a name for this drive.
usb-error-name-too-long = Drive name must be { $max } characters or fewer
# $name: the name typed. $other: the drive that has it.
usb-error-name-taken = Drive name '{ $name }' is already used by another drive ('{ $other }')
usb-error-read-only-init = The USB is read-only, so it can't be initialized.
# $id: the track's id in the USB's databases.
usb-error-track-not-found = track { $id } not found on the USB

# Event Log warnings while connecting a USB (choosing its folder).
usb-warning-path-empty = USB path is empty
# $path: the folder chosen.
usb-warning-path-not-found = Path does not exist: { $path }
usb-warning-missing-vendor-root = Missing vendor root directory
usb-warning-missing-contents = Missing contents directory
usb-warning-read-only = USB appears read-only: imports may work but export will fail
# Event Log warnings while reading the stick. $root: the USB's folder.
usb-warning-pdb-not-found-edb-only = PDB not found under { $root }; continuing with eDB-only mode
usb-warning-pdb-not-found-db-fallback = PDB not found under { $root }; using DB fallback only
# $attempts: how many keys were tried.
usb-warning-edb-unreadable = eDB unreadable after trying { $attempts ->
    [one] { $attempts } SQLCipher key
   *[other] { $attempts } SQLCipher keys
}
# Event Log errors after changing the stick, when the changed database couldn't be copied
# back to it. $error: the reason, from the OS.
usb-warning-pdb-write-back-failed = PDB write-back to USB failed: { $error }
usb-warning-edb-write-back-failed = eDB write-back to USB failed: { $error }

## usb-playlists

# The USB Playlists panel's totals. $playlists: the stick's playlists. $tracks: their tracks together.
usb-playlists-summary = { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
}
# The status line after reading the stick's playlists; a warning count may follow.
# $count: how many playlists were read.
usb-playlists-loaded = USB playlists loaded: { $count }
# The progress bar when the read finishes. $summary: usb-playlists-summary.
usb-playlists-done = Done — { $summary }
# The status line after reordering the stick's playlists; a warning count may follow.
usb-playlists-order-saved = Playlist order saved
# The status line after removing a playlist from the stick; a warning count may follow.
# $name: the playlist. $edb, $pdb: the rows removed from each database.
usb-playlists-removed = Removed USB playlist: { $name } [db { $edb }, pdb { $pdb }]

# The status line when a USB playlist is clicked.
# $name: the playlist's name. $count: how many tracks it has.
usb-playlists-selected = USB playlist selected: { $name } ({ $count ->
    [one] { $count } track
   *[other] { $count } tracks
})

# What the dialog asks before removing a playlist from the stick.
# $name: the playlist's name.
usb-playlists-remove-confirm = Remove USB playlist "{ $name }" from the stick?

# Event Log notes while reading or removing the stick's playlists.
# $count: how many playlists. $names: their names, comma-separated.
usb-playlists-warning-empty-source = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} with zero static track entries in export data: { $names }
# $count: tracks of the removed playlist kept because other playlists use them.
usb-playlists-warning-shared-tracks-kept = { $count ->
    [one] { $count } shared track preserved (used by other playlists)
   *[other] { $count } shared tracks preserved (used by other playlists)
}
# $count: files the removal would have deleted that were already gone.
usb-playlists-warning-files-missing = { $count ->
    [one] { $count } file already missing from USB
   *[other] { $count } files already missing from USB
}
# $path: a file deleted from the stick with the playlist.
usb-playlists-warning-file-deleted = deleted: { $path }
# Removing a playlist left a database unchanged. For the eDB, why it couldn't be opened is
# logged just before.
usb-playlists-warning-edb-removal-skipped = eDB playlist removal skipped: the eDB couldn't be opened
usb-playlists-warning-pdb-removal-skipped = PDB playlist removal skipped: PDB missing
# Reordering the stick's playlists couldn't open its eDB, so only the PDB has the new order.
usb-playlists-warning-edb-order-skipped = reorder: unable to open eDB for playlist order

## usb-history

# The USB History panel's totals.
# $sessions: how many history sessions the stick has. $tracks: how many tracks they hold together.
usb-history-summary = { $sessions ->
    [one] { $sessions } session
   *[other] { $sessions } sessions
}, { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
}

# The status line after reading the stick's history; a warning count may follow.
# $sessions: how many sessions were read.
usb-history-loaded = USB histories loaded: { $sessions }
# Event Log warning when the stick has no PDB. $root: the USB's folder.
usb-history-warning-pdb-not-found = PDB not found under { $root }; history import requires PDB

## usb-player-menu

# Status lines after reading or changing the player menu.
usb-player-menu-loaded = Player menu loaded
usb-player-menu-updated = Player menu updated
usb-player-menu-unchanged = Player menu unchanged
# A menu item the databases give no name. $kind: its kind number.
usb-player-menu-unnamed = Menu { $kind }

usb-player-menu-error-no-edb = unable to open the USB's eDB for the player menu
usb-player-menu-error-tables = eDB missing menuItem/category tables
usb-player-menu-error-no-menu = this USB has no player menu in its eDB or PDB
usb-player-menu-error-cant-change = this USB's player menu can't be changed: it has no eDB and no menu rows in its PDB
# $name: the menu item. $kind: its number in the databases.
usb-player-menu-error-cant-remove = Player menu item "{ $name }" (kind { $kind }) cannot be removed
# Event Log notes for the player menu.
usb-player-menu-warning-databases-differ = The PDB and eDB show different player menus. Showing the eDB's: the next menu change writes it to both.
# $error: the reason. The menu change is then refused.
usb-player-menu-warning-edb-write-failed = eDB write failed ({ $error }); restoring eDB from snapshot

## usb-background-color

# The background colours rekordbox offers, as the chips name them.
usb-background-color-default = Default
usb-background-color-pink = Pink
usb-background-color-red = Red
usb-background-color-orange = Orange
usb-background-color-yellow = Yellow
usb-background-color-green = Green
usb-background-color-aqua = Aqua
usb-background-color-blue = Blue
usb-background-color-purple = Purple
# A colour number with no name. $color: the number.
usb-background-color-numbered = colour { $color }
# The note when the two databases hold different colours. $pdb, $edb: the colours' names.
usb-background-color-two-colors = rekordbox set two colours: { $pdb } for older players (PDB), { $edb } for rekordbox and the CDJ-3000 (eDB). Choosing a colour sets both.
# The note when no colour can be set.
usb-background-color-no-place = This USB has no place for a background colour.
usb-background-color-read-only = This USB is read-only, so its background colour can't be changed.
# Status lines after choosing a colour. $color: its name.
usb-background-color-set = Background colour set to { $color }
usb-background-color-unchanged = Background colour unchanged

usb-background-color-error-no-edb = unable to open the USB's eDB

## usb-audio-formats

# The USB audio formats list's hint, and its hint when the stick can't be written.
usb-audio-formats-hint = The formats export puts on this USB · drag to set the order of preference
usb-audio-formats-read-only = USB is read-only
# The tooltip of the only ticked format, which can't be unticked.
usb-audio-formats-last-ticked = At least one format stays ticked

usb-audio-formats-error-read-only = The USB is read-only, so its USB audio formats can't be changed
usb-audio-formats-error-none-ticked = At least one format stays ticked

## backups

# The Backups list's count. $count: how many backups.
backups-summary = { $count ->
    [one] { $count } backup
   *[other] { $count } backups
}
backups-empty = No backups yet.
# What a backup holds, for the texts below.
backups-files-both = eDB and PDB
backups-files-edb = eDB
backups-files-pdb = PDB
backups-files-none = Backup
# A backup's details: where it is, and the playlists the stick had.
backups-on-usb = On USB
backups-on-computer = On this computer
# $count: the stick's playlists when the backup was taken.
backups-playlists = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
}
# The dialogs before restoring and deleting. $files: backups-files-*. $time: when it was taken.
backups-restore-confirm = Restore { $files } from the backup taken at { $time }? The current files will themselves be backed up first.
backups-delete-confirm = Delete this { $files } backup permanently?
# Status lines after restoring and deleting. $files: backups-files-*.
backups-restored = Restored { $files } from backup
backups-deleted = Deleted { $files } backup
# Why a backup was taken, as the list shows it.
backups-reason-export = Before export
backups-reason-export-mirror = Before export (mirror)
backups-reason-export-additive = Before export (additive)
backups-reason-playlist-reorder = Before playlist reorder
backups-reason-playlist-removal = Before playlist removal
backups-reason-menu-update = Before menu update
backups-reason-background-color = Before background color change
backups-reason-repair = Before repair
backups-reason-restore = Before restore

# $backup: the backup's timestamp.
backups-error-not-found = backup not found: { $backup }

# Event Log notes when a command backs up the USB's databases before changing them.
# $file: the backup file's name.
backups-note-written = Backup: { $file }
# $error: the reason, from the OS.
backups-warning-folder-failed = Backup skipped: could not create the backups folder: { $error }
# $file: the database (export.pdb or exportLibrary.db).
backups-warning-copy-failed = Backup failed for { $file }: { $error }

## diagnostics

# The badge on the USB diagnostics report, for its overall result.
diagnostics-status-pass = PASS
diagnostics-status-warn = WARN
diagnostics-status-fail = FAIL
# The USB health dots' tooltip after diagnostics ran, for the same results.
diagnostics-health-good = USB health: good
diagnostics-health-warnings = USB health: warnings
diagnostics-health-issues = USB health: issues found
# The status line after diagnostics, and the report's header. $ms: how long it took, in milliseconds.
diagnostics-complete = Diagnostics complete ({ $ms }ms)
diagnostics-duration = Completed in { $ms }ms
# The same for the parity report. $ms: how long it took, in milliseconds.
parity-complete = Parity report complete ({ $ms }ms)
# The parity report's badge. $status: diagnostics-status-pass, -warn or -fail.
parity-status-label = Parity { $status }

# The diagnostics report's boxes, and the columns in them.
diagnostics-group-databases = Databases
diagnostics-group-library = Library
diagnostics-group-low-level = Low-level
diagnostics-group-comparison = Comparison
diagnostics-section-pdb = PDB
diagnostics-section-edb = eDB
diagnostics-section-engine = Engine DJ
diagnostics-section-playlists = Playlists
diagnostics-section-analysis = Analysis
diagnostics-section-player-counters = Player counters
diagnostics-section-page-format = Page format
diagnostics-section-track-details = Track details
diagnostics-section-music-files = Music files

# The overview rows' titles, above the diagnostics, parity and repair reports: what a DJ cares about.
diagnostics-area-older-players = Older Pioneer players
diagnostics-area-newer-players = Newer Pioneer players
diagnostics-area-playlists = Playlists
diagnostics-area-track-details = Track details
diagnostics-area-music-files = Music files
diagnostics-area-analysis = Waveforms & beat grids
diagnostics-area-engine = Denon / Numark players
# After an overview row's problems, when a repair can fix them.
diagnostics-preview-fixes-hint = Preview Fixes shows what can be repaired.

# The diagnostics overview's "Older Pioneer players" row.
diagnostics-overview-pdb-missing = This USB has no player database, so these players won't show any music.
diagnostics-overview-pdb-unreadable = The player database is missing or can't be read, so these players won't show any music.
# $playlists, $tracks: what the player database holds.
diagnostics-overview-pdb-ready = Ready: { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
}.
diagnostics-overview-pdb-track-count = These players will show 0 or 1 tracks: part of the player database doesn't match the music on this USB.
diagnostics-overview-pdb-irregular = Readable, but parts of the player database are irregular and desktop DJ software may complain.
diagnostics-overview-pdb-errors = The player database has errors that can make these players freeze or show wrong data.
# The "Newer Pioneer players" row.
diagnostics-overview-edb-missing = This USB has no database for newer players, so they may not show this USB's music.
diagnostics-overview-edb-not-yet = This USB has no database for newer players yet.
diagnostics-overview-edb-unreadable = The database newer players read can't be opened, so they may not show this USB's music.
# $count: the tracks only the older players' database has.
diagnostics-overview-edb-pdb-only = { $count ->
    [one] { $count } track is missing from the database newer players read, so it may not show up there.
   *[other] { $count } tracks are missing from the database newer players read, so they may not show up there.
}
# $count: the tracks only the newer players' database has.
diagnostics-overview-edb-edb-only = { $count ->
    [one] { $count } track is only in the newer players' database, so older Pioneer players won't show it.
   *[other] { $count } tracks are only in the newer players' database, so older Pioneer players won't show them.
}
diagnostics-overview-edb-bpm-key = Some tracks may show a different BPM or key on newer players than on older ones.
diagnostics-overview-edb-analysis = Some tracks may have no waveform or beat grid on newer players.
diagnostics-overview-edb-irregular = Parts of the newer players' database are irregular.
# $count: the tracks the newer players' database holds.
diagnostics-overview-edb-ready = Ready: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}.
# The "Playlists" row. $count: the playlists.
diagnostics-overview-playlists-ready = Every track in your { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} is found.
# $count: the playlist tracks that can't be found.
diagnostics-overview-playlists-missing = { $count ->
    [one] { $count } track in your playlists can't be found.
   *[other] { $count } tracks in your playlists can't be found.
}
diagnostics-overview-playlists-mismatch = Some playlist data doesn't line up between the databases.
# The "Waveforms & beat grids" row.
diagnostics-overview-analysis-ready = Every track has waveform and beat grid data.
diagnostics-overview-analysis-problems = Some tracks' waveform or beat grid data is missing or outdated.
# The "Denon / Numark players" row.
diagnostics-overview-engine-next-change = Will be added the next time this app changes the USB.
diagnostics-overview-engine-not-set-up = Not set up. Only needed for Denon Prime / Numark Mixstream players: turn on Settings → Export → "Write Engine DJ library".
diagnostics-overview-engine-updates-off = Updates are turned off, so changes made in this app don't reach these players.
diagnostics-overview-engine-out-of-date = Out of date, so the player will analyze every track again.
diagnostics-overview-engine-ready = Ready.

# The parity report's overview rows: one sentence per problem kind.
# $count: the playlists whose ids differ.
parity-overview-playlist-ids = { $count ->
    [one] { $count } playlist doesn't line up between the two databases.
   *[other] { $count } playlists don't line up between the two databases.
}
# $count: the playlist tracks only the older players' database has.
parity-overview-only-in-pdb = { $count ->
    [one] { $count } playlist track is only in the database older Pioneer players read.
   *[other] { $count } playlist tracks are only in the database older Pioneer players read.
}
# $count: the playlist tracks only the newer players' database has.
parity-overview-only-in-edb = { $count ->
    [one] { $count } playlist track is only in the database newer Pioneer players read.
   *[other] { $count } playlist tracks are only in the database newer Pioneer players read.
}
# $count: the playlists in a different order.
parity-overview-order = { $count ->
    [one] { $count } playlist is in a different order depending on the player.
   *[other] { $count } playlists are in a different order depending on the player.
}
# $count: the tracks listed twice.
parity-overview-duplicates = { $count ->
    [one] { $count } track is listed twice in a playlist on older Pioneer players.
   *[other] { $count } tracks are listed twice in a playlist on older Pioneer players.
}
# $count: the tracks with missing details.
parity-overview-pdb-details = { $count ->
    [one] { $count } track is missing track details on older Pioneer players.
   *[other] { $count } tracks are missing track details on older Pioneer players.
}
# $count: the tracks with missing details.
parity-overview-edb-details = { $count ->
    [one] { $count } track is missing track details in the database newer Pioneer players read.
   *[other] { $count } tracks are missing track details in the database newer Pioneer players read.
}
# $count: the tracks with a missing artist, album or key.
parity-overview-dictionary = { $count ->
    [one] { $count } track refers to an artist, album or key that's missing on older Pioneer players.
   *[other] { $count } tracks refer to an artist, album or key that's missing on older Pioneer players.
}
# $count: the tracks whose artwork differs.
parity-overview-artwork = { $count ->
    [one] { $count } track has artwork on one kind of Pioneer player but not the other.
   *[other] { $count } tracks have artwork on one kind of Pioneer player but not the other.
}
# $count: the tracks whose rating or colour differs.
parity-overview-rating-colour = { $count ->
    [one] { $count } track has a different rating or colour on older Pioneer players.
   *[other] { $count } tracks have a different rating or colour on older Pioneer players.
}
# $count: the missing audio files.
parity-overview-files-missing = { $count ->
    [one] { $count } audio file is missing from the USB, so that track won't play.
   *[other] { $count } audio files are missing from the USB, so those tracks won't play.
}
# $count: the audio files in no playlist.
parity-overview-files-extra = { $count ->
    [one] { $count } audio file on the USB isn't in any playlist.
   *[other] { $count } audio files on the USB aren't in any playlist.
}
# $count: the tracks whose files differ.
parity-overview-paths = { $count ->
    [one] { $count } track points to a different audio or waveform file depending on the player.
   *[other] { $count } tracks point to a different audio or waveform file depending on the player.
}
# The rows' text when there's no problem. $count: the playlists.
parity-overview-playlists-ready = Both databases have the same { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
}, with the same tracks in the same order.
parity-overview-tracks-ready = Every playlist track has complete details and the same artwork in both databases.
parity-overview-files-ready = Every playlist track's audio file is on the USB, and both databases point to the same files.

# The parity report's playlist rows: short badges for each problem. $count: the tracks it affects.
parity-issue-only-in-pdb = +PDB { $count }
parity-issue-only-in-edb = +eDB { $count }
parity-issue-duplicates = dup PDB { $count }
parity-issue-pdb-gaps = PDB gaps { $count }
parity-issue-edb-gaps = eDB gaps { $count }
parity-issue-path-mismatch = path mismatch { $count }
parity-issue-dictionary = dict issues { $count }
parity-issue-artwork = art mismatch { $count }
parity-issue-rating-colour = rating/colour mismatch { $count }
parity-issue-field-notes = field notes { $count }
# The same, for a playlist-wide problem.
parity-issue-id-mismatch = id mismatch
parity-issue-sort-mismatch = sort mismatch
parity-issue-order-mismatch = order mismatch

# The diagnostics report's checks: each check's label, then what it found. The words in
# code style (num_rl, nrs, u5, page_flags, tranrf, empty_candidate, playlist_tree, ACTV,
# repair_usb_diagnostics) are the PDB format's own names: keep them as they are.
diagnostics-check-pdb-exists = PDB exists
# $path: the file the check looked for.
diagnostics-check-pdb-exists-missing = File not found: { $path }
diagnostics-check-pdb-exists-found = Found
diagnostics-check-pdb-parseable = PDB parseable
# $error: why reading failed, in English.
diagnostics-check-pdb-parseable-error = Parse error: { $error }
# What the PDB holds. $tracks, $artists, $albums, $genres, $labels, $keys, $artworks: its rows of each.
diagnostics-check-pdb-parseable-counts = { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
}, { $artists ->
    [one] { $artists } artist
   *[other] { $artists } artists
}, { $albums ->
    [one] { $albums } album
   *[other] { $albums } albums
}, { $genres ->
    [one] { $genres } genre
   *[other] { $genres } genres
}, { $labels ->
    [one] { $labels } label
   *[other] { $labels } labels
}, { $keys ->
    [one] { $keys } key
   *[other] { $keys } keys
}, { $artworks ->
    [one] { $artworks } artwork
   *[other] { $artworks } artworks
}
diagnostics-check-pdb-header = PDB header compatibility
# $value: the number in the file header.
diagnostics-check-pdb-header-value = value { $value } at bytes 0x10..0x14
diagnostics-check-pdb-header-unknown = value { $value } at bytes 0x10..0x14; expected a known-compatible value
diagnostics-check-pdb-page-headers = PDB page headers
diagnostics-check-pdb-page-headers-valid = u5, page_flags, and tranrf are valid on all data pages
# $parts: what rekordbox left, joined with "; ".
diagnostics-check-pdb-page-headers-bookkeeping = valid; rekordbox transaction bookkeeping: { $parts }
# The parts. $count: the pages.
diagnostics-check-pdb-page-headers-sentinel-u5 = { $count ->
    [one] { $count } page
   *[other] { $count } pages
} with sentinel u5=0x1FFF
diagnostics-check-pdb-page-headers-page-flags = { $count ->
    [one] { $count } page
   *[other] { $count } pages
} with non-default page_flags
diagnostics-check-pdb-page-headers-tranrf = { $count ->
    [one] { $count } data page
   *[other] { $count } data pages
} with tranrf=0 on last-row group
diagnostics-check-pdb-structure = PDB structural integrity
diagnostics-check-pdb-structure-ok = sentinel b-tree, page shapes, row IDs, and write pointers OK
# $parts: what rekordbox left, joined with "; ".
diagnostics-check-pdb-structure-ok-bookkeeping = sentinel b-tree, page shapes, row IDs, and write pointers OK; rekordbox transaction bookkeeping: { $parts }
# $parts: the problems, joined with "; ". The two lines are joined with " | ".
diagnostics-check-pdb-structure-fails = { $parts } — run repair_usb_diagnostics to fix
diagnostics-check-pdb-structure-warns = { $parts } — player-tolerant; repair available
# The parts. $count: what the part counts.
diagnostics-check-pdb-structure-stale-btree = { $count ->
    [one] { $count } sentinel
   *[other] { $count } sentinels
} stale b-tree
diagnostics-check-pdb-structure-playlist-tree-shape = { $count ->
    [one] { $count } playlist_tree page
   *[other] { $count } playlist_tree pages
} with non-default shape
diagnostics-check-pdb-structure-tombstoned = { $count ->
    [one] { $count } tombstoned row
   *[other] { $count } tombstoned rows
} non-zero id
diagnostics-check-pdb-structure-track-page-shape = { $count ->
    [one] { $count } track page
   *[other] { $count } track pages
} with non-default shape
diagnostics-check-pdb-structure-track-page-active = { $count ->
    [one] { $count } track page
   *[other] { $count } track pages
} ACTV in multi-page chain
diagnostics-check-pdb-structure-history-shape = { $count ->
    [one] { $count } history page
   *[other] { $count } history pages
} wrong shape
diagnostics-check-pdb-structure-ec-conflict = { $count ->
    [one] { $count } table
   *[other] { $count } tables
} write-pointer conflict
diagnostics-check-pdb-structure-truncated = { $count ->
    [one] { $count } table
   *[other] { $count } tables
} with declared last page beyond physical file end
diagnostics-check-pdb-structure-torn-pages = { $count ->
    [one] { $count } empty_candidate page
   *[other] { $count } empty_candidate pages
} torn from interrupted export
diagnostics-check-pdb-structure-torn-tail = PDB tail torn from interrupted export
diagnostics-check-pdb-playlists = PDB playlists
# $nodes: the playlist tree's rows. $entries: the playlists' tracks.
diagnostics-check-pdb-playlists-counts = { $nodes ->
    [one] { $nodes } tree node
   *[other] { $nodes } tree nodes
}, { $entries ->
    [one] { $entries } entry
   *[other] { $entries } entries
}
diagnostics-check-pdb-history = PDB history
# $playlists, $entries, $rows: the history tables' rows.
diagnostics-check-pdb-history-counts = { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $entries ->
    [one] { $entries } entry
   *[other] { $entries } entries
}, { $rows ->
    [one] { $rows } row
   *[other] { $rows } rows
}
diagnostics-check-pdb-history-empty = empty
diagnostics-check-num-rl-pages = num_rl=8191 pages
# $count: the pages with the value. $total: all pages.
diagnostics-check-num-rl-pages-detail = { $count } of { $total ->
    [one] { $total } page
   *[other] { $total } pages
} (using nrs fallback)
diagnostics-check-nrs-wrapping = nrs wrapping
# $count: the pages.
diagnostics-check-nrs-wrapping-detail = { $count ->
    [one] { $count } page
   *[other] { $count } pages
} with row count exceeding nrs header
diagnostics-check-orphaned-entries = Orphaned entries
# $entries: the playlist rows. $tracks: the missing track ids they point to.
diagnostics-check-orphaned-entries-detail = { $entries ->
    [one] { $entries } entry
   *[other] { $entries } entries
} referencing { $tracks ->
    [one] { $tracks } track ID
   *[other] { $tracks } track IDs
} not in PDB
diagnostics-check-parse-warnings = Parse warnings
# $count: the warnings.
diagnostics-check-parse-warnings-detail = { $count ->
    [one] { $count } warning
   *[other] { $count } warnings
} during parsing

diagnostics-check-edb = eDB
diagnostics-check-edb-missing = File not found
diagnostics-check-edb-opened = Opened successfully
diagnostics-check-edb-unable = Unable to open
diagnostics-check-pdb-paths = PDB indexed paths
diagnostics-check-edb-paths = eDB indexed paths
# $count: the track paths the database lists.
diagnostics-check-paths-count = { $count ->
    [one] { $count } canonical track path
   *[other] { $count } canonical track paths
}
diagnostics-check-path-agreement = PDB/eDB path agreement
diagnostics-check-path-agreement-agree = PDB/eDB canonical-path sets agree
# The parts of a disagreement, joined with "; ". $count: the paths or tracks.
diagnostics-check-path-agreement-pdb-only = { $count ->
    [one] { $count } PDB-only path
   *[other] { $count } PDB-only paths
}
diagnostics-check-path-agreement-edb-only = { $count ->
    [one] { $count } eDB-only path
   *[other] { $count } eDB-only paths
}
diagnostics-check-path-agreement-history-only = { $count ->
    [one] { $count } history-only track
   *[other] { $count } history-only tracks
}
diagnostics-check-beat-grid = Beat grid format
diagnostics-check-beat-grid-not-exported = no tracks on this USB were exported by this app
diagnostics-check-beat-grid-current = every beat grid this app wrote to this USB is in the current format
# $count: the tracks. $examples: up to three of their paths, joined with ", ".
diagnostics-check-beat-grid-outdated = Tracks with a beat grid in an outdated format that some players (e.g. the XDJ-AZ) reject, so Beat Sync is unavailable on them: { $count }; e.g. { $examples }; re-export or run the repair (fix_beat_grid_header) to fix
diagnostics-check-pdb-analysis = PDB analysis refs
# $with: the track rows with an analysis path. $total: all track rows. $refs: the different analysis files they point to.
diagnostics-check-pdb-analysis-detail = { $with }/{ $total } track rows with analysis paths; { $refs ->
    [one] { $refs } unique bundle reference
   *[other] { $refs } unique bundle references
}
diagnostics-check-no-pdb-data = No PDB data available
diagnostics-check-edb-analysis = eDB analysis refs
# $with: the content rows with an analysis path. $total: the playlists' content rows. $refs: the different analysis files they point to. $entries: the playlist rows.
diagnostics-check-edb-analysis-detail = { $with }/{ $total } playlist-linked content rows with analysis paths; { $refs ->
    [one] { $refs } unique bundle reference
   *[other] { $refs } unique bundle references
} from { $entries ->
    [one] { $entries } playlist entry
   *[other] { $entries } playlist entries
}
diagnostics-check-edb-analysis-no-tracks = No tracks in the eDB
diagnostics-check-edb-analysis-no-data = No eDB playlist data available
diagnostics-check-playlist-data = Playlist data
# A row named after a playlist only the eDB has. $count: its tracks.
diagnostics-check-playlist-edb-only = present in eDB only ({ $count ->
    [one] { $count } entry
   *[other] { $count } entries
}), missing from PDB
diagnostics-check-overall-resolution = Overall resolution
# $resolved: the playlist rows whose track exists. $total: all playlist rows. $percent: the share, such as 98.5. $playlists: the playlists.
diagnostics-check-overall-resolution-detail = { $resolved }/{ $total } entries resolve ({ $percent }%) across { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}
diagnostics-check-db-only-playlists = DB-only playlists
# $count: the playlists.
diagnostics-check-db-only-playlists-detail = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} in eDB but not in PDB
diagnostics-check-key-overlap = PDB vs eDB key overlap (informational)
# $matched: the tracks both databases list. $pdbshare, $edbshare: their share of each database's tracks, such as 98.5. $pdb, $edb: each database's tracks.
diagnostics-check-key-overlap-detail = matched { $matched } track keys; PDB { $pdbshare }% ({ $matched }/{ $pdb }), DB { $edbshare }% ({ $matched }/{ $edb })
diagnostics-check-operational = Operational interpretation
diagnostics-check-operational-detail = pass/warn here means operationally usable; strict parity may still fail on the same USB
# A row named after a playlist with missing tracks. $resolved: its rows whose track exists. $total: its rows. $percent: the share, such as 98.5.
diagnostics-check-playlist-resolution = { $resolved }/{ $total } entries ({ $percent }%)

# The player counter checks. t00, t08, t11, t12, t17, t18, t19, ec, nrs, num_rl, rowpf0 and
# tranrf0 are the PDB format's own names: keep them as they are.
diagnostics-check-history-page = PDB history page
diagnostics-check-history-page-mismatch = t19 does not match the track count or the eDB's property row — older players may show 0 or 1 tracks. Repair fixes it.
diagnostics-check-predicted-counter = Predicted player counter
# $playlists, $songs: what a player would count. $confidence: high, medium or low (not translated).
diagnostics-check-predicted-counter-detail = playlists { $playlists }, songs { $songs } (confidence: { $confidence })
diagnostics-check-shape-mode = Shape mode
# $mode: runtime, baseline-init or unknown (not translated). $baseline: diagnostics-check-yes or -no.
diagnostics-check-shape-mode-detail = { $mode } (baseline-init-like: { $baseline })
diagnostics-check-yes = yes
diagnostics-check-no = no
diagnostics-check-cross-check = Cross-check
# $tracks: the PDB's track rows. $entries: its playlist rows.
diagnostics-check-cross-check-detail = t00 tracks { $tracks }, t08 entries { $entries }
diagnostics-check-history-pointers = History pointers
diagnostics-check-primitive-signal = Primitive signal

diagnostics-check-edb-history = eDB history
# $playlists, $entries: the eDB's history rows.
diagnostics-check-edb-history-counts = { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $entries ->
    [one] { $entries } entry
   *[other] { $entries } entries
}
diagnostics-check-edb-history-empty = empty
diagnostics-check-string-alignment = Track/album string alignment
diagnostics-check-string-alignment-ok = All UTF-16 and long ASCII string slots are 4-byte aligned
# $count: the rows. $examples: up to three, such as "track #12 (title)", joined with ", ". The repair_* names are commands: keep them.
diagnostics-check-string-alignment-misaligned = { $count ->
    [one] { $count } row
   *[other] { $count } rows
} with a misaligned UTF-16 or long ASCII string slot — freezes/comm-errors CDJ hardware while browsing; e.g. { $examples }; run repair_pdb_track_string_alignment / repair_pdb_album_string_alignment / repair_pdb_artist_name_layout to fix
diagnostics-check-artist-layout = Artist name layout
diagnostics-check-artist-layout-ok = All artist names are where rekordbox puts them
# $count: the artist rows.
diagnostics-check-artist-layout-moved = { $count ->
    [one] { $count } artist row
   *[other] { $count } artist rows
} with the name at a different offset than rekordbox uses; an Engine DJ player importing export.pdb shows the artist's tracks without an artist; run repair_pdb_artist_name_layout to fix
diagnostics-check-bpm-key = BPM/key consistency
diagnostics-check-bpm-key-ok = export.pdb and exportLibrary.db agree on BPM and key for every track
# $count: the tracks. $examples: up to three of their paths, joined with ", ".
diagnostics-check-bpm-key-mismatch = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} whose BPM/key in exportLibrary.db differs from export.pdb -- CDJs may show a different BPM/key than the app; e.g. { $examples }; run the repair (fix_bpm_key_mismatch, which also checks the ANLZ beat grids) to fix
diagnostics-check-shared-analysis = Shared analysis files
diagnostics-check-shared-analysis-ok = every track has its own analysis file
# $tracks: the tracks. $files: the analysis files they share.
diagnostics-check-shared-analysis-shared = { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
} sharing { $files ->
    [one] { $files } analysis file
   *[other] { $files } analysis files
}, so players can show one track's waveform and beat grid for another; run the repair (separate_shared_analysis) to fix
# The Engine DJ library's checks.
diagnostics-check-engine-missing = Not on this USB
diagnostics-check-engine-missing-detail = Only needed for Denon Prime / Numark Mixstream players. Turn on Settings → Export → "Write Engine DJ library" before using this USB in one; otherwise the player imports export.pdb itself and analyzes every track again
diagnostics-check-engine-upkeep = Kept up to date
diagnostics-check-engine-upkeep-on = Rebuilt whenever this app changes the USB
diagnostics-check-engine-upkeep-off = "Write Engine DJ library" is off, so changes this app makes to the USB don't reach Denon Prime / Numark Mixstream players. Preview Fixes can turn it on
diagnostics-check-engine-sync = Matches export.pdb
# $pdb: the PDB's write counter.
diagnostics-check-engine-sync-ok = Written for export.pdb #{ $pdb }
# $library: the counter the library was written for. $pdb: the PDB's. Either can be "?".
diagnostics-check-engine-sync-behind = Written for export.pdb #{ $library }, the USB has #{ $pdb }. The player may import export.pdb again and analyze the tracks itself. Preview Fixes can rebuild it

# The parity report's checks.
diagnostics-check-parity-overall = Overall player parity status
# $playlists: the playlists checked. $failing: those that fail. $ids: those whose ids differ.
diagnostics-check-parity-overall-detail = playlists checked: { $playlists }, failing playlists: { $failing }, playlist id mismatches: { $ids }
diagnostics-check-parity-section = Parity-report section (required)
diagnostics-check-parity-section-detail = See parity summary rows for category counts.
diagnostics-check-parity-identity = Playlist identity parity
# $count: the playlists.
diagnostics-check-parity-identity-detail = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} with mismatched or unresolved playlist ids between PDB and eDB
diagnostics-check-parity-membership = Playlist membership parity
# $pdb: the playlist tracks only the PDB has. $edb: those only the eDB has.
diagnostics-check-parity-membership-detail = membership deltas: only-in-PDB={ $pdb }, only-in-eDB={ $edb }
diagnostics-check-parity-ordering = Playlist ordering parity
# $entries: the playlists whose tracks are in a different order. $playlists: those sorted differently.
diagnostics-check-parity-ordering-detail = entry-order mismatches={ $entries }, playlist-sort-order mismatches={ $playlists }
diagnostics-check-parity-files = Indexed audio file presence
# $total: the audio files the databases list.
diagnostics-check-parity-files-ok = all { $total } indexed audio files present, no unindexed files
# $missing: the listed files not on the USB. $total: the files the databases list.
diagnostics-check-parity-files-missing = { $missing } of { $total ->
    [one] { $total } indexed audio file
   *[other] { $total } indexed audio files
} missing from USB
# $extra: the audio files no playlist lists. $total: the files the databases list.
diagnostics-check-parity-files-extra = { $extra ->
    [one] { $extra } audio file
   *[other] { $extra } audio files
} on USB not referenced in any playlist (total indexed={ $total })
# $missing, $total, $extra: as above.
diagnostics-check-parity-files-both = { $missing } of { $total ->
    [one] { $total } indexed file
   *[other] { $total } indexed files
} missing; { $extra ->
    [one] { $extra } unindexed file
   *[other] { $extra } unindexed files
} on USB
diagnostics-check-parity-duplicates = Duplicate PDB entries
# $count: the duplicate playlist rows.
diagnostics-check-parity-duplicates-detail = { $count ->
    [one] { $count } duplicate PDB playlist entry
   *[other] { $count } duplicate PDB playlist entries
} detected
diagnostics-check-parity-pdb-metadata = PDB metadata completeness
# $count: the tracks.
diagnostics-check-parity-pdb-metadata-detail = { $count ->
    [one] { $count } playlist-linked PDB track
   *[other] { $count } playlist-linked PDB tracks
} missing required player metadata
diagnostics-check-parity-paths = Media and analysis path parity
# $count: the tracks.
diagnostics-check-parity-paths-detail = { $count ->
    [one] { $count } playlist-linked track
   *[other] { $count } playlist-linked tracks
} with media or analysis path mismatches
diagnostics-check-parity-artwork = Artwork presence parity
# $count: the tracks.
diagnostics-check-parity-artwork-detail = { $count ->
    [one] { $count } playlist-linked track
   *[other] { $count } playlist-linked tracks
} with artwork in one DB but not the other
diagnostics-check-parity-dictionary = PDB dictionary id resolution
# $count: the tracks.
diagnostics-check-parity-dictionary-detail = { $count ->
    [one] { $count } playlist-linked track
   *[other] { $count } playlist-linked tracks
} with unresolved required PDB dictionary ids
diagnostics-check-parity-edb-metadata = eDB source completeness
# $count: the tracks.
diagnostics-check-parity-edb-metadata-detail = { $count ->
    [one] { $count } playlist-linked eDB track
   *[other] { $count } playlist-linked eDB tracks
} missing metadata used by strict parity comparison
diagnostics-check-parity-rating-colour = Rating and colour parity
# $count: the tracks. $ratings, $colours: those whose rating or colour differs.
diagnostics-check-parity-rating-colour-detail = { $count ->
    [one] { $count } playlist-linked track
   *[other] { $count } playlist-linked tracks
} with a different rating or colour in the PDB than in the eDB (rating differs: { $ratings }, colour differs: { $colours })


# The progress bar while the job runs.
diagnostics-progress-resolving-root = USB: Resolving root
diagnostics-progress-checking-pdb-integrity = USB: Checking PDB integrity
diagnostics-progress-checking-database-access = USB: Checking database access
diagnostics-progress-checking-contents-integrity = USB: Checking contents integrity (DB-only)
diagnostics-progress-checking-analysis-references = USB: Checking analysis references
diagnostics-progress-checking-playlist-resolution = USB: Checking playlist resolution
diagnostics-progress-parsing-pdb = USB: Parsing PDB
diagnostics-progress-reading-edb = USB: Reading eDB
diagnostics-progress-checking-audio-files = USB: Checking indexed audio file presence
diagnostics-progress-comparing-playlist-sources = USB: Comparing playlist sources
diagnostics-progress-finalizing-parity-report = USB: Finalizing parity report
# $current: playlists done. $total: playlists in all. $playlist: its name.
diagnostics-progress-resolving-playlist = Resolving playlist { $current }/{ $total }: { $playlist }

# Event Log warnings from diagnostics and the parity report. $path: a file on the stick.
diagnostics-warning-missing-audio = indexed audio file missing from USB: { $path }
diagnostics-warning-unindexed-audio = unindexed audio file: { $path }
# $path: the analysis file. $tracks: the track files that name it, comma-separated.
diagnostics-warning-shared-analysis = shared analysis file: { $path } used by { $tracks }
# A track only a history session lists. $path: the track's file. $playlists: the sessions,
# comma-separated.
diagnostics-warning-history-only-track = history-only track: { $path } ({ $playlists })
diagnostics-warning-history-only-track-unknown = history-only track: { $path } (unknown history playlist)
# Event Log warnings for the stick's analysis folder. $path: the file or folder.
diagnostics-warning-anlz-malformed = analysis entry malformed: { $path }
diagnostics-warning-anlz-malformed-empty = analysis malformed entry is empty: { $path }
diagnostics-warning-anlz-empty = analysis file appears empty: { $path }
# $error: the reason, from the OS.
diagnostics-warning-anlz-unreadable = failed to read analysis file metadata { $path }: { $error }
diagnostics-warning-anlz-walk-failed = failed to walk USBANLZ analysis directory: { $error }

## repair

# The status line when the repair preview is ready. $ms: how long it took, in milliseconds.
repair-preview-ready = Repair preview ready ({ $ms }ms)
# The status line after applying fixes. $applied, $failed: the fixes. $ms: how long it took.
repair-apply-complete = Repair apply complete: { $applied } applied, { $failed } failed ({ $ms }ms)
# The same, when diagnostics ran again after the fixes.
repair-apply-complete-refreshed = Repair apply complete: { $applied } applied, { $failed } failed ({ $ms }ms). Diagnostics refreshed.

# The repair preview's overview: what Apply Fixes changes, one sentence per kind of fix, under the diagnostics-area-* rows.
repair-overview-interrupted-export = Repairs damage left in the player database by an interrupted export.
repair-overview-track-count = Fixes older players showing 0 or 1 tracks on this USB.
repair-overview-rekordbox-corrupted = Fixes errors that make rekordbox report the USB as corrupted.
repair-overview-header = Fixes a database setting some players don't accept.
repair-overview-string-alignment = Rewrites track and album text that can freeze players while browsing or stop the track list from opening.
repair-overview-artist-names = Rewrites artist names that can freeze players or go missing on Denon / Numark players.
repair-overview-history = Copies the play history so newer players show the same history as older ones.
repair-overview-strict-parity = Makes every playlist the same on older and newer players.
repair-overview-duplicate-entries = Removes tracks listed twice in a playlist by mistake.
repair-overview-relink = Reconnects tracks to music files that were renamed or moved on the USB.
repair-overview-unindexed = Puts music files that are in no playlist into a playlist called "Unindexed".
repair-overview-duplicate-copies = Deletes extra copies of music files that are in no playlist.
repair-overview-missing-audio = Removes tracks whose music file is no longer on the USB. Play history keeps them.
repair-overview-empty-analysis = Rebuilds missing waveforms and beat grids.
repair-overview-shared-analysis = Gives tracks that show another track's waveform their own.
repair-overview-analysis-path = Labels waveform files so players always pick the right one.
repair-overview-bpm-key = Makes every player show the same BPM and key.
repair-overview-beat-grid = Updates beat grids from older app versions so Beat Sync works on every player.
repair-overview-mp3-seek = Adds the data players need to jump to any point in MP3 tracks.
repair-overview-flac-seek = Adds the data players need to jump to any point in FLAC tracks.
repair-overview-engine = Rebuilds the Denon / Numark library and keeps it up to date from now on. Changes made on those players themselves are replaced.
repair-overview-other = Fixes other errors in the player database.
# The row for what the repair can't fix, and its text. $count: the problems.
repair-overview-needs-attention = Needs your attention
repair-overview-not-fixable = { $count ->
    [one] { $count } problem can't be fixed automatically
   *[other] { $count } problems can't be fixed automatically
} — Technical details says what to do.
# The headline above the repair overview.
repair-summary-no-issues = No issues found.
repair-summary-nothing-fixable = Nothing here can be fixed automatically.
# $count: the problems Apply Fixes repairs.
repair-summary = Apply Fixes repairs { $count ->
    [one] { $count } problem
   *[other] { $count } problems
} on this USB. The databases are backed up first.

# The repair preview's fix list: each fix's title, then what it does. The words in code style
# (DAT, EXT, 2EX, ANLZ, PPTH, PVBR, PVB2, tt=…, u5, num_rl, nrs, empty_candidate,
# next_unused_page, seqdb, playlist_entries, history_content) are file-format names: keep them.
repair-fix-empty-analysis = Fix Empty Analysis Files
repair-fix-empty-analysis-description = Regenerate missing/empty DAT/EXT/2EX bundles when source audio is resolvable.
repair-fix-bpm-key = Fix BPM/Key Mismatch
repair-fix-bpm-key-description = Rebuild the ANLZ beat grid and set the eDB BPM and key from each track's PDB row, so every player reads the same BPM and key. Waveforms, cues and the first beat are kept. See Event Log for the tracks.
repair-fix-shared-analysis = Give Tracks Their Own Analysis Files
# $count: the tracks.
repair-fix-shared-analysis-description = Tracks using another track's analysis file: { $count }. Players can show that track's waveform and beat grid instead. Each gets its own file in its folder (ANLZ0001 and on, as rekordbox names them), analysed from its audio with its BPM; the file's own track keeps it. Old cues stay with the other track. See Event Log for the list.
repair-fix-analysis-path = Add Track Path to Analysis Files
# $count: the analysis files.
repair-fix-analysis-path-description = Analysis files not naming their track (PPTH): { $count }. Players use that path to find a track's file when two tracks hash to the same folder. Writes the track path into each; nothing else changes. See Event Log for the list.
repair-fix-beat-grid = Fix Beat Grid
repair-fix-beat-grid-description = Rewrite beat grids from app versions before 0.3.7 in rekordbox's format: the .DAT header value two bytes off, and the .EXT extended grid without its checksum and sub-millisecond beat times. Some players (e.g. the XDJ-AZ) refuse Beat Sync on those. Beat positions stay within 1 ms; cues and waveforms are kept.
repair-fix-mp3-seek = Add Missing MP3 Seek Data
repair-fix-mp3-seek-description = Read each MP3 on the USB and write the seek index rekordbox writes for it (PVBR in the .DAT), byte for byte as rekordbox does. Beat grids, cues and waveforms are kept. Files whose index can't be reproduced exactly are skipped and named in the Event Log.
repair-fix-flac-seek = Add Missing FLAC Seek Data
repair-fix-flac-seek-description = Read each FLAC on the USB and write a seek index (PVB2 in the .EXT). For long tracks about 98% of its entries match rekordbox's; the rest are one frame later. Beat grids, cues and waveforms are kept. Files that don't parse cleanly are skipped and named in the Event Log.
repair-fix-history-shape = Repair PDB History/Columns Page Footer Shape
# $count: the pages.
repair-fix-history-shape-description = Patch u5/num_rl footer fields on { $count ->
    [one] { $count } tt=16/17/18 data page
   *[other] { $count } tt=16/17/18 data pages
} from the wrong (1, nrs-1) shape to the correct (nrs, 0) convention.
repair-fix-ec-conflict = Repair Table Write-Pointer Conflict
# $count: the tables.
repair-fix-ec-conflict-description = Tables whose empty_candidate pointer targets a page already used as data by a different table: { $count }. The DJ software validates this invariant and reports 'database corrupted' when it finds a conflict. The repair reassigns each conflicting empty_candidate to a free page beyond the physical file and updates next_unused_page in the header.
repair-fix-truncated = Repair Truncated Table Chain
# $count: the tables.
repair-fix-truncated-description = Tables declaring a last page that does not physically exist in the file: { $count } (an additive-growth write was interrupted after the table pointer was advanced but before the new pages were flushed to disk). The repair points the table's last/empty_candidate fields back at the real last written page; no page content is changed. This is a structural prerequisite and always runs before other repairs, including the strict-parity upgrade.
repair-fix-torn-growth = Repair Torn Growth Pages (Interrupted Export)
# $count: the pages. The second form is for a file also left longer than its pages.
repair-fix-torn-growth-description = The export was interrupted (e.g. USB disconnected) while rekordbox was growing the PDB: table empty_candidate pages earmarked as the next slot to write ({ $count }) never received data, leaving non-zero garbage instead of the blank page the writer requires before reuse. This zeroes each such page and recomputes seqdb. No table chain, page index, or transaction history is touched. This is a structural prerequisite and always runs before other repairs, including the strict-parity upgrade: parity's additive writes can grow the file, and this repair's tail-truncation boundary would otherwise go stale and cut off tracks/playlists parity had just written.
repair-fix-torn-growth-description-tail = The export was interrupted (e.g. USB disconnected) while rekordbox was growing the PDB: table empty_candidate pages earmarked as the next slot to write ({ $count }) never received data, leaving non-zero garbage instead of the blank page the writer requires before reuse and left the file extended past next_unused_page with unpopulated content. This zeroes each such page and truncates the never-populated tail and recomputes seqdb. No table chain, page index, or transaction history is touched. This is a structural prerequisite and always runs before other repairs, including the strict-parity upgrade: parity's additive writes can grow the file, and this repair's tail-truncation boundary would otherwise go stale and cut off tracks/playlists parity had just written.
repair-fix-t19 = Activate PDB History Page
repair-fix-t19-description = The tt=19 page still has the single sealed row of a never-exported USB, the fixed two rows an older version wrote, or no USB name although the USB has one, so older players count 0 or 1 tracks. Rewrite it as the ring of one row per track rekordbox keeps, with the USB's name, keeping the template row, and mark the page active. No other page is changed.
repair-fix-track-alignment = Repair Track String Alignment
# $count: the track rows.
repair-fix-track-alignment-description = Re-encode { $count ->
    [one] { $count } track row
   *[other] { $count } track rows
} whose title/filename/comment/other UTF-16 or long ASCII string slot doesn't start at a 4-byte aligned offset. MIPS-based CDJ hardware requires 4-byte aligned reads for that 4-byte string header; a misaligned slot freezes/comm-errors the player while browsing, or as soon as the track list opens.
repair-fix-album-alignment = Repair Album String Alignment
# $count: the album rows.
repair-fix-album-alignment-description = Re-encode { $count ->
    [one] { $count } album row
   *[other] { $count } album rows
} whose UTF-16 name slot doesn't start at a 4-byte aligned offset. Same class of issue as the track string alignment repair.
repair-fix-artist-layout = Repair Artist Name Layout
# $count: the artist rows.
repair-fix-artist-layout-description = Re-encode { $count ->
    [one] { $count } artist row
   *[other] { $count } artist rows
} the way rekordbox writes artist rows: a plain ASCII name at offset 10, a UTF-16 name at the 4-byte aligned offset 12. Older versions of this app put every artist name at 12, which an Engine DJ player's own import of export.pdb reads as no artist.
repair-fix-header = Repair PDB Header Compatibility Field
# $current: the value in the file. $target: the value the repair writes.
repair-fix-header-description = Patch only bytes 0x10..0x14 from { $current } to the built-in compatibility value { $target }.
repair-fix-duplicate-copies = Remove Duplicate Audio Copies
# $count: the copies.
repair-fix-duplicate-copies-description = Audio files on the USB in no playlist, each a byte-for-byte copy of a track in the same folder: { $count }. Deletes each copy; the original track keeps its playlists, cues and analysis. See Event Log for the file list.
repair-fix-unindexed = Add Unindexed Audio to a Playlist
# $count: the files. $playlist: the playlist's name on the USB (Unindexed; not translated, it's what the USB stores).
repair-fix-unindexed-description = Audio files on the USB in no playlist (e.g. after restoring the databases from a backup): { $count }. Adds each to a USB playlist "{ $playlist }", reusing its analysis files; nothing is copied or deleted. See Event Log for the file list.
repair-fix-relink = Relink Moved Audio Files
# $count: the tracks.
repair-fix-relink-description = Tracks pointing to a missing file while the same track is on the USB under another name or folder: { $count }. Points each to the file found; it keeps its playlists, cues and analysis. See Event Log for the list.
repair-fix-missing-audio = Remove Missing Audio References
# $count: the files.
repair-fix-missing-audio-description = Audio files no longer on the USB: { $count }. Removes their tracks from both databases (track and content rows, playlist entries). Tracks in a history playlist are kept. See Event Log for the file list.
repair-fix-history-sync = Sync eDB History Tables From PDB
repair-fix-history-sync-description = Optional fix: replace eDB history/history_content rows using current PDB history payload mapping.
repair-fix-strict-parity = Upgrade Export Data To Strict Parity
# $summary: repair-strict-parity-summary, what a dry run would change.
repair-fix-strict-parity-description = Collect the failing playlists from both eDB and PDB, merge metadata from both sides, and write both databases once through the export writers. { $summary }.
# $error: why the dry run failed, in English.
repair-fix-strict-parity-description-failed = Collect the failing playlists from both eDB and PDB, merge metadata from both sides, and write both databases once through the export writers. Dry run failed: { $error }.
repair-fix-strict-parity-pdb-missing = This USB has an eDB but no PDB, so older players show no music. Creates the PDB as Initialize USB does and writes every eDB playlist into it.
repair-fix-strict-parity-edb-missing = This USB has a PDB but no eDB, so newer players and rekordbox show no music. Creates the eDB as Initialize USB does and writes every PDB playlist into it.
# What the strict-parity upgrade changes. $playlists: the playlists merged. $edbadded, $edbchanged: eDB rows. $tracks, $trackschanged: PDB tracks. $pdbplaylists: PDB playlists added. $duplicates: duplicate entries removed.
repair-strict-parity-summary = merged { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}; eDB: { $edbadded ->
    [one] { $edbadded } row
   *[other] { $edbadded } rows
} added, { $edbchanged } changed; PDB: { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
} added, { $trackschanged } changed, { $pdbplaylists ->
    [one] { $pdbplaylists } playlist
   *[other] { $pdbplaylists } playlists
} added, { $duplicates ->
    [one] { $duplicates } duplicate entry
   *[other] { $duplicates } duplicate entries
} removed
# The same, when one database was missing. $summary: the line above.
repair-strict-parity-rebuilt-pdb = rebuilt the missing PDB from the eDB; { $summary }
repair-strict-parity-rebuilt-edb = rebuilt the missing eDB from the PDB; { $summary }
repair-fix-duplicate-entries = Remove Duplicate PDB Playlist Entries
# $count: the rows.
repair-fix-duplicate-entries-description = Remove { $count ->
    [one] { $count } stale duplicate playlist_entries row
   *[other] { $count } stale duplicate playlist_entries rows
} left behind on shared pages from repeated exports of the same playlist over time. This only cleans up duplicate rows already in the PDB — it does not merge playlists between eDB and PDB or otherwise rewrite the export. For that broader repair, use "{ repair-fix-strict-parity }" instead (or in addition).
repair-fix-engine = Keep Engine DJ Library Up to Date
repair-fix-engine-description = This USB has an Engine DJ library (Denon Prime, Numark Mixstream). Turns on "Write Engine DJ library" and rebuilds it now from this USB's rekordbox data; from then on it's rebuilt whenever this app changes a USB. Play history is kept, but changes made on the player itself (cues, loops, playlists created there) are replaced every time.

# The rows for what the repair can't fix: the problem, then what to do.
# $count: the entries.
repair-unsupported-malformed-analysis = { $count ->
    [one] { $count } malformed USBANLZ entry
   *[other] { $count } malformed USBANLZ entries
}
repair-unsupported-malformed-analysis-reason = These indicate USB analysis directory corruption or stray files. Inspect Event Log entries tagged 'analysis entry malformed' and re-export affected tracks (or rebuild USB analysis data).
# $count: the tracks. $format: MP3 or FLAC.
repair-unsupported-seek-data = { $count } { $format } { $count ->
    [one] track
   *[other] tracks
} without seek data that can't be added
repair-unsupported-seek-data-reason = The seek index can't be built from these files (Event Log names each one and why). A damaged file needs an intact copy.
# $count: the files.
repair-unsupported-no-audio = { $count ->
    [one] { $count } indexed audio file
   *[other] { $count } indexed audio files
} missing: the USB has no audio under Contents
repair-unsupported-no-audio-reason = Looks like a copy of the databases without the music. Copy the audio files back instead of removing every track.
# $count: the tracks.
repair-unsupported-history-audio = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} in a history playlist with missing audio files
repair-unsupported-history-audio-reason = Tracks in a history playlist are not removed. Copy the files back to the USB (Event Log lists them).

# A fix row's details line, its parts joined with " · ".
repair-meta-supported = ✓ supported
repair-meta-preview-only = ✗ preview-only
repair-meta-destructive = destructive
repair-meta-safe = safe
# $count: the files the fix writes or deletes.
repair-meta-writes = { $count ->
    [one] { $count } write
   *[other] { $count } writes
}
repair-meta-deletes = { $count ->
    [one] { $count } delete
   *[other] { $count } deletes
}
repair-meta-always-applied = always applied
# The tooltip on an always-applied fix's ticked checkbox.
repair-meta-always-applied-tooltip = Always applied — required for other repairs to complete safely


# The fixes rekordbox's bookkeeping needs, run with every repair and never listed: their titles show
# in the apply progress. Their descriptions are for the CLI tool only and stay English.
repair-fix-zero-tranrf = Repair Data Pages With Zero tranrf
repair-fix-page-flags = Repair PDB Data Page Flags
repair-fix-sentinel-u5 = Repair PDB Data Pages With Sentinel u5
repair-fix-track-shape = Repair PDB Track Page Footer Shape
repair-fix-t00-multipage = Repair Track Page Flag (Multi-Page Chain)
repair-fix-tombstoned = Repair Tombstoned Row IDs
repair-fix-stale-btree = Repair Stale Sentinel B-Tree Index
repair-fix-playlist-tree-shape = Repair PDB Playlist-Tree Page Footer Shape

# The progress bar while repairing.
repair-progress-baseline = USB: Running diagnostics baseline
repair-progress-parity = USB: Building parity guidance
repair-progress-collect = USB: Collecting repair opportunities
repair-progress-apply = USB: Applying selected fixes
repair-progress-backup = USB: Backing up databases before repair
repair-progress-open = USB: Opening repair databases
repair-progress-save = USB: Saving repaired databases
repair-progress-rerun = USB: Re-running diagnostics after repair
repair-progress-applied = USB: Repair apply complete
repair-progress-preview-ready = USB: Repair preview ready
# $fix: the fix's title (repair-fix-…), here and below.
repair-progress-applying = USB: Applying { $fix }
# $done, $total: the fix's items (files, tracks) done and in all.
repair-progress-applying-items = USB: Applying { $fix } ({ $done }/{ $total })
repair-progress-finished = USB: Finished { $fix }

# Event Log warnings from the repair preview, the details its rows link to. $path: a file on
# the stick, here and below.
# $values: the tempos and key each database and analysis file holds ("PDB 128.00, DAT grid
# 127.00, EXT grid -, eDB same"): data, not translated.
repair-warning-bpm-key-mismatch = BPM/key mismatch: { $path } ({ $values })
# $tracks: the track files that name it, comma-separated. $owner: the one that keeps it.
repair-warning-shared-analysis = shared analysis file: { $path } used by { $tracks }; kept by { $owner }
repair-warning-shared-analysis-no-owner = shared analysis file: { $path } used by { $tracks }; kept by no track (each gets its own)
# $track: the track file the analysis file belongs to.
repair-warning-analysis-without-track-path = analysis file without track path: { $path } ({ $track })
# $format: MP3 or FLAC. $reason: what in the audio file stops it (technical, not translated).
repair-warning-seek-data-unfixable = { $format } seek data can't be added ({ $reason }): { $path }
repair-warning-seek-data-unfixable-replace = { $format } seek data can't be added ({ $reason }; replace it with an intact copy): { $path }
repair-warning-seek-data-unfixable-report = { $format } seek data can't be added ({ $reason }; please report this file (format details, not the audio) so its seek data can be supported): { $path }
repair-warning-missing-audio-scan-skipped = missing-audio scan skipped: Contents directory is absent or empty (DB-only snapshot)
repair-warning-history-audio-missing = history track's audio file missing: { $path }
# $original: the indexed file it copies.
repair-warning-duplicate-audio = duplicate audio file: { $path } (copy of { $original })
# $old: where the databases point. $new: where the file is now.
repair-warning-moved-audio = moved audio: { $old } -> { $new }
repair-warning-missing-audio = missing-audio reference: { $path }
# $error: the reason, from the OS.
repair-warning-save-failed = Saving the repaired databases to the USB failed: { $error }

# Event Log notes while the fixes run.
repair-warning-duplicate-audio-removed = removed duplicate audio file: { $path }
repair-warning-beat-grid-length-unknown = beat grid not rebuilt: track length unknown for { $path }
# $track: the track's file. $path: its new analysis file.
repair-warning-analysis-separated = own analysis file for { $track }: { $path }
repair-warning-beat-grid-not-constant = beat grid not rewritten (not a constant-tempo grid): { $path }
# $format: MP3 or FLAC. $reason: what in the audio file stops it (technical, not translated).
repair-warning-seek-data-skipped-replace = { $format } seek data not added ({ $reason }): { $path }; replace it with an intact copy
repair-warning-seek-data-skipped-report = { $format } seek data not added ({ $reason }): { $path }; please report this file (format details, not the audio) so its seek data can be supported
# The empty analysis files fix. $path: the empty analysis file. $audio: the track's file.
repair-warning-empty-analysis-no-track = repair skipped (empty analysis): source audio mapping not found for { $path }
repair-warning-empty-analysis-audio-missing = repair skipped (empty analysis): source audio missing for { $path } -> { $audio }
repair-warning-empty-analysis-unreadable = repair failed (empty analysis): unable to analyze source audio { $audio }
repair-warning-empty-analysis-length-unknown = repair failed (empty analysis): track length unknown for { $audio }
# $error: the reason.
repair-warning-empty-analysis-write-failed = repair failed (empty analysis): { $path }: { $error }
repair-warning-relink-edb-unavailable = relink moved audio: eDB unavailable, only the PDB was updated
# The unindexed audio fix. $error: why the file couldn't be read.
repair-warning-unindexed-unreadable = unindexed audio skipped, unreadable: { $path }: { $error }
repair-warning-unindexed-no-analysis = unindexed audio has no analysis files on the USB: { $path }
repair-warning-unindexed-edb-unavailable = unindexed audio: eDB unavailable, only the PDB was updated
repair-warning-missing-audio-edb-unavailable = repair skipped (missing audio): unable to open eDB read-write

# What each fix did, one Event Log line per fix after an apply. $outcome: one of the repair-outcome-* lines
# below.
repair-log-applied = repair applied: { $outcome }
repair-log-skipped = repair skipped: { $outcome }
repair-log-failed = repair failed: { $outcome }
# $fix: the fix's title. $detail: one of the repair-outcome-detail-* lines.
repair-outcome-applied = { $fix }: { $detail }
repair-outcome-nothing-to-apply = { $fix }: nothing to apply
repair-outcome-not-selected = { $fix }: not selected
repair-outcome-pdb-unavailable = { $fix }: PDB unavailable
# $error: the reason.
repair-outcome-failed = { $fix } failed: { $error }
# $outcome: the fix's repair-outcome-applied line, which the USB never got.
repair-outcome-not-saved = { $outcome } (not saved to the USB)
repair-outcome-not-run = { $fix }: not run, the repaired databases were not saved
repair-outcome-read-failed = Reading the PDB failed, nothing was applied: { $error }
repair-outcome-save-failed = Saving the repaired databases to the USB failed, the USB's databases are unchanged: { $error }
# Analysis files (empty analysis files) or tracks (seek data).
repair-outcome-detail-fixed-skipped = fixed { $fixed }, skipped { $skipped }
repair-outcome-detail-fixed-skipped-failed = fixed { $fixed }, skipped { $skipped }, failed { $failed }
repair-outcome-detail-duplicate-entries = removed { $count ->
    [one] { $count } duplicate entry
   *[other] { $count } duplicate entries
}
repair-outcome-detail-relinked = relinked { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# $playlist: the USB playlist they were added to.
repair-outcome-detail-unindexed = added { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} to "{ $playlist }"
# $detail: repair-outcome-detail-unindexed. $notes: the lines below, joined with ", ".
repair-outcome-detail-unindexed-notes = { $detail } ({ $notes })
repair-outcome-detail-unindexed-analysed = { $count } analysed from the audio
repair-outcome-detail-unindexed-without-analysis = { $count } without analysis files
repair-outcome-detail-duplicate-copies = deleted { $count ->
    [one] { $count } file
   *[other] { $count } files
}
# $kept: tracks that keep the file they shared.
repair-outcome-detail-shared-analysis = gave { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} a new analysis file, { $kept } kept the original
repair-outcome-detail-analysis-path = named the track in { $count ->
    [one] { $count } analysis file
   *[other] { $count } analysis files
}
repair-outcome-detail-bpm-key = rebuilt { $grids ->
    [one] { $grids } beat grid
   *[other] { $grids } beat grids
}, updated { $rows ->
    [one] { $rows } eDB row
   *[other] { $rows } eDB rows
}
repair-outcome-detail-beat-grid = fixed { $count ->
    [one] { $count } bundle
   *[other] { $count } bundles
}
repair-outcome-detail-engine = setting on, rebuilt ({ $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
}, { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
})

## job

# The progress bar's texts while a backend job runs: job-<command>-started when it starts,
# -progress for a step, -completed when it ends. A failed job shows its error instead.

# When a job fails without a message.
job-failed-unknown = Operation failed
job-save-usb-track-metadata-edits-started = USB: Saving track details
job-save-usb-track-metadata-edits-completed = USB: Track details saved
job-save-usb-track-analysis-edits-started = USB: Saving cue edits
job-save-usb-track-analysis-edits-completed = USB: Cue edits saved
job-validate-usb-root-started = USB: Validating root
job-validate-usb-root-completed = USB: Root validation complete
job-validate-usb-root-progress = USB: Checking folders and permissions
job-migrate-started = Upgrading analysis data
job-migrate-completed = Analysis data upgraded
job-fetch-usb-playlists-started = USB: Reading playlists
job-fetch-usb-playlists-completed = USB: Playlist read complete
job-fetch-usb-histories-started = USB: Reading history
job-fetch-usb-histories-completed = USB: History read complete
job-get-usb-player-menu-config-started = USB: Loading player menu
job-get-usb-player-menu-config-completed = USB: Player menu loaded
job-get-usb-player-menu-config-progress = USB: Reading menu configuration
job-update-usb-player-menu-config-started = USB: Updating player menu
job-update-usb-player-menu-config-completed = USB: Player menu updated
job-update-usb-player-menu-config-progress = USB: Writing menu configuration
job-get-usb-background-color-started = USB: Loading background color
job-get-usb-background-color-completed = USB: Background color loaded
job-get-usb-background-color-progress = USB: Reading background color
job-set-usb-background-color-started = USB: Setting background color
job-set-usb-background-color-completed = USB: Background color set
job-set-usb-background-color-progress = USB: Writing background color
job-remove-usb-playlist-started = USB: Removing playlist
job-remove-usb-playlist-completed = USB: Playlist removal complete
job-reorder-usb-playlists-started = USB: Saving playlist order
job-reorder-usb-playlists-completed = USB: Playlist order saved
job-export-to-usb-started = USB: Exporting playlist
job-export-to-usb-completed = USB: Export complete
job-run-usb-diagnostics-started = USB: Running diagnostics
job-run-usb-diagnostics-completed = USB: Diagnostics complete
job-run-usb-parity-report-started = USB: Running parity report
job-run-usb-parity-report-completed = USB: Parity report complete
job-repair-usb-diagnostics-started = USB: Planning repair fixes
job-repair-usb-diagnostics-completed = USB: Repair planning complete
job-initialize-usb-started = USB: Initializing structure
job-initialize-usb-completed = USB: Initialization complete
job-initialize-usb-progress = USB: Creating External library directories
job-scan-library-started = Scanning media library
job-scan-library-progress = Discovering audio files
job-scan-library-completed = Library scan completed
# $indexed, $updated, $removed: track counts.
job-scan-library-completed-counts = Library scan completed: indexed { $indexed }, updated { $updated }, removed { $removed }
job-scan-rekordbox-db-started = Importing from desktop library
job-scan-rekordbox-db-progress = Reading desktop library database
job-scan-rekordbox-db-completed = Desktop library import completed
# $indexed, $updated: track counts.
job-scan-rekordbox-db-completed-counts = Desktop library import completed: indexed { $indexed }, updated { $updated }
job-scan-mixxx-db-started = Importing from Mixxx library
job-scan-mixxx-db-progress = Reading Mixxx library database
job-scan-mixxx-db-completed = Mixxx library import completed
# $indexed, $updated: track counts.
job-scan-mixxx-db-completed-counts = Mixxx library import completed: indexed { $indexed }, updated { $updated }
job-transcode-tracks-started = Converting selected tracks
# Convert…, one per finished track. $current: tracks done. $total: tracks in all. $file: the file's name.
job-transcode-tracks-progress = Converting { $current }/{ $total }: { $file }
# $converted, $skipped, $failed: track counts.
job-transcode-tracks-completed = Converting finished: { $converted } converted, { $skipped } skipped, { $failed } failed
# $converted: tracks converted before the cancel.
job-transcode-tracks-cancelled = Converting cancelled: { $converted } converted
job-install-update-started = Checking for update...
# $version: the new version (1.2.3), here and below.
job-install-update-downloading = Downloading update { $version }...
job-install-update-waiting = Update { $version } downloaded, waiting for the running job to finish...
job-install-update-installing = Installing update { $version } and restarting...
# $error: the updater's reason, in English.
job-install-update-failed = Update failed: { $error }

## migration

# The data upgrades run once after an app update. Their names, shown in the progress bar.
migration-usb-devices = Importing USB devices from old settings
migration-fingerprints = Filling track fingerprints
migration-merge-usb-tracks = Merging duplicate USB tracks
migration-beat-grids = Upgrading cached beat grids
migration-mp3-seek = Adding MP3 seek data
# The progress bar during an upgrade. $migration: one of the names above. $current, $total: items done and in all.
migration-progress = { $migration }: { $current }/{ $total }

# Event Log warnings when a data upgrade fails. $migration: the upgrade (migration-*). $error: the reason.
migration-warning-failed = { $migration } failed ({ $error }); it runs again next launch
migration-warning-startup-failed = Data upgrades at startup failed ({ $error }); they run again next launch

## transcode

# Convert…'s choices. $format: AIFF or WAV.
transcode-choice-lossless = { $format } (lossless)
transcode-choice-mp3 = MP3 320 kbps
# The status line after Convert…. $parts: the counts below, joined with ", ".
transcode-summary-done = Converting done: { $parts }
transcode-summary-cancelled = Converting cancelled: { $parts }
# $count: tracks, here and in the next two.
transcode-summary-converted = { $count ->
    [one] { $count } track converted
   *[other] { $count } tracks converted
}
transcode-summary-skipped = { $count } skipped
transcode-summary-failed = { $count } failed
# The status line after removing transcodes. $count: the transcodes removed.
transcode-removed = Removed { $count ->
    [one] { $count } transcode
   *[other] { $count } transcodes
}
# A transcode's tooltip in the track's details. $format: MP3 320, AIFF or WAV. $size: its size ("9 MB").
transcode-tooltip = { $format } transcode · { $size }
# $about: transcode-tooltip.
transcode-tooltip-outdated = { $about } — out of date: the source file changed since. Convert again to update it
# The FMT column's tooltip when a USB is set: which file goes to the USB.
# $format: the track's own file, as the FMT column describes it ("FLAC · 44.1 kHz · 16-bit").
transcode-export-source = source: { $format }
# $format: MP3 320, AIFF or WAV. $size: its size. $source: transcode-export-source.
transcode-export-exported = { $format } transcode · { $size }, exported to this USB — { $source }
# $format: what it's converted to. $source: transcode-export-source.
transcode-export-convert = Converted to { $format } on export — { $source }
# $file: the track's own file, as the FMT column describes it. $format: its format's name.
transcode-export-refused = { $file } — { $format } can't go on this USB: only lossless files can be converted

transcode-error-folder-in-source = the transcode folder can't be inside a source folder: the scan would list the transcodes as tracks

# Event Log notes per track. $file: the source file's name.
transcode-skipped-not-found = { $file }: file not found
transcode-skipped-already-format = { $file }: already this format
transcode-skipped-not-lossless = { $file }: not a lossless file
transcode-skipped-up-to-date = { $file }: already has an up-to-date transcode
# $reason: why the encoder failed (from the encoder, not translated).
transcode-failed = { $file }: { $reason }

## tracklist-export

# USB History's Export Tracklist, the status line after saving the file. $history: the history's name.
tracklist-export-done = Tracklist exported: { $history }
tracklist-export-cancelled = Tracklist export cancelled

## analysis

# The status line after asking for analysis. $count: the tracks queued.
analysis-submitted = Analyzing { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# $count: tracks asked for that are already being analyzed.
analysis-submitted-already = Already analyzing { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# $count: the tracks queued. $already: tracks asked for that are already being analyzed.
analysis-submitted-some-already = Analyzing { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}, { $already } already in progress
analysis-submitted-none = No tracks need analysis
# A queued track's tooltip.
analysis-waiting-tooltip = Waiting for its turn to be analyzed
# The progress bar.
analysis-started = Analyzing selected tracks
# $current: tracks done. $total: tracks in all. $file: the file's name.
analysis-progress = Analyzing { $current }/{ $total }: { $file }
# The status line when the job ends. $analyzed, $failed, $incomplete: track counts.
analysis-finished = Analysis finished: analyzed { $analyzed }, failed { $failed }
analysis-finished-incomplete = Analysis finished: analyzed { $analyzed }, failed { $failed }, incomplete { $incomplete }
# The Event Log's line when the job ends. $count: tracks processed. $elapsed: how long it took ("1:05").
# $analyzed, $failed, $incomplete: track counts.
analysis-log = Analysis: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} in { $elapsed } (analyzed { $analyzed }, failed { $failed })
analysis-log-incomplete = Analysis: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} in { $elapsed } (analyzed { $analyzed }, failed { $failed }, incomplete { $incomplete })

# Event Log notes from an analysis job.
# $analyzed: tracks analyzed before the cancel. $total: tracks the job had.
analysis-cancelled = Analysis cancelled: { $analyzed } of { $total ->
    [one] { $total } track
   *[other] { $total } tracks
} analyzed
analysis-no-eligible-tracks = No eligible tracks found for analysis
# $path: the track's file. $reasons: analysis-incomplete-* joined with ", ".
analysis-track-incomplete = { $path }: { $reasons }
analysis-incomplete-no-bpm = no BPM detected
analysis-incomplete-no-duration = no duration
analysis-incomplete-no-waveform = no waveform
# $error: why it failed.
analysis-track-failed = { $path }: { $error }
# An analysed MP3's seek data couldn't be made. $reason: what in the file stops it (technical, not
# translated). Same advice as the repair's repair-warning-seek-data-skipped-*.
analysis-warning-mp3-seek-skipped-replace = MP3 seek data not added ({ $reason }): { $path }; replace it with an intact copy
analysis-warning-mp3-seek-skipped-report = MP3 seek data not added ({ $reason }): { $path }; please report this file (format details, not the audio) so its seek data can be supported

## error

# Errors several pages show, in the status line after the command's own words ("Delete failed: …").
# $id: the track's or playlist's id in the app's library.
error-track-not-found = track not found: { $id }
error-playlist-not-found = playlist not found: { $id }
error-track-not-found-anywhere = track not found in Library or selected USB
# $playlist: the USB playlist's name or id.
error-usb-playlist-not-found = USB playlist not found: { $playlist }
# $history: the USB history's id.
error-usb-history-not-found = USB history not found: { $history }
# $path: the audio file, here and in the next one.
error-audio-file-not-found = audio file not found: { $path }
error-audio-path-not-file = audio path is not a file: { $path }
# The USB's eDB (exportLibrary.db is its file name on the stick).
error-usb-edb-unreadable = USB library database (exportLibrary.db) not found or unreadable
# $path: the track's analysis file as the USB lists it, here and in the next three.
error-usb-analysis-path-not-found = USB analysis path not found: { $path }
error-usb-analysis-unreadable = USB analysis bundle not readable: { $path }
error-usb-analysis-empty = USB analysis bundle is empty: { $path }
error-usb-analysis-missing = USB analysis bundle missing: { $path }
# $path: the USB folder.
error-usb-root-missing = requested usbRoot does not exist: { $path }
# Refused while the data upgrade after an app update finishes.
error-data-upgrade-running = Finishing a data upgrade, try again in a moment

## export

# The playlist's export button with a USB selected, and its tooltip. $dir: the drive's name.
export-button = Export to USB: { $dir }
export-button-no-dir = Export to USB
export-button-tooltip = Export current playlist to selected USB
# The same when the USB already has a playlist of this name, so export appends to it.
# $playlist: the playlist's name. $dir: the drive's name.
export-button-append = Append to ({ $playlist }) on USB: ({ $dir })
export-button-append-no-dir = Append to ({ $playlist }) on USB
export-button-append-tooltip = Append current playlist tracks to existing USB playlist "{ $playlist }"
# The export button with no USB selected, and its tooltip.
export-button-no-usb = Select USB first
export-button-no-usb-tooltip = Select a valid USB folder first
# The sidebar mark: with a USB selected that has this playlist, and with none selected for one exported before.
export-mark-on-usb = On this USB
export-mark-exported = Exported to USB
# The playlist heading's line about what an export would copy, its parts joined with " · ".
# $count: the tracks to copy. $total: the playlist's tracks.
export-space-to-copy = { $count } of { $total ->
    [one] { $total } track
   *[other] { $total } tracks
} to copy
export-space-all-on-usb = All tracks on USB
# The playlist heading's line about the USB audio formats, its parts joined with " · ".
# $count: the tracks the export converts.
export-format-convert = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} will be converted on export
# $count: the tracks the export converts. $format: the format they're converted to, such as MP3 320.
export-format-convert-to = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} will be converted to { $format } on export
# $count: the tracks the export refuses.
export-format-refused = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} can't go on this USB; export is blocked
# $count: the tracks the export refuses. $format: their format, such as FLAC.
export-format-refused-kind = { $count ->
    [one] { $count } { $format } track
   *[other] { $count } { $format } tracks
} can't go on this USB; export is blocked
# $size: the free space, such as 27.0 GB.
export-space-free = { $size } free on USB
# Before the line when the export doesn't fit. $line: the line.
export-space-does-not-fit = Doesn't fit: { $line }
# The status line after an export. $playlist: its name. $count: the tracks exported.
# $skipped: the tracks left out. $warnings: a warning count (" | (2 warnings)") or nothing.
# $mode: export-sync-mirror or export-sync-additive.
export-complete = Export complete: { $playlist } - { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}, { $skipped } skipped{ $warnings } [sync: { $mode }]
export-sync-mirror = mirror
export-sync-additive = additive
# Status lines when an export is refused.
# $required, $available: sizes, such as 1.2 GB.
export-refused-no-space = Export blocked: not enough free space on the USB drive — need about { $required }, { $available } free.
# $count: the tracks in a format the USB's formats don't allow.
export-refused-formats = Export blocked: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} in a format this USB's formats don't allow, and only lossless files can be converted. See Event Log for details.
# $missing: the tracks without analysis files. $total: the playlist's tracks.
export-refused-analysis-files = Export blocked: the analysis files of { $missing } of { $total ->
    [one] { $total } track
   *[other] { $total } tracks
} are missing. Select them and use Reanalyze.
# $missing: the tracks not analyzed. $total: the playlist's tracks.
export-refused-not-analyzed = Export blocked: { $missing } of { $total ->
    [one] { $total } track
   *[other] { $total } tracks
} not analyzed yet. Use Analyze Missing Tracks.
export-refused-read-only = USB is read-only. Remount as read-write before export.
# $folder: the missing source folder.
export-refused-source-missing = Export blocked: source folder is missing: { $folder }. Relocate or remove it first.
export-refused-manifest = Export blocked: manifest invariant check failed.
export-refused-manifest-after-prune = Export finished with errors: the stale-file prune removed a file the exported playlist still references — the USB databases were written; restore from the pre-export backup.


# The progress bar while the job runs.
export-progress-preparing = USB: Preparing export
export-progress-finalizing-export-metadata = USB: Finalizing export metadata
export-progress-writing-engine-dj-library = USB: Writing Engine DJ library
export-progress-waiting-for-writes = USB: Waiting for files to finish writing
# $current: tracks done. $total: tracks in all. $file: the track's file name. Here and below.
export-progress-copying = USB: Copying track { $current }/{ $total }: { $file }
# Lossless tracks converted for this USB's formats.
export-progress-converting = USB: Converting track { $current }/{ $total }: { $file }

# Why an export was refused: the error the Event Log shows (the status line has its own text above).
export-error-busy = Another export or change to the USB is still running. Export again once it has finished.
# $path: the USB folder.
export-error-usb-missing = USB root does not exist: { $path }
export-error-no-tracks = playlist has no tracks to export
export-error-none-playable = none of the playlist's tracks are in a format the players play (MP3, AAC, WAV, AIFF, FLAC, ALAC)
export-error-none-convertible = none of the playlist's tracks could be converted for this USB
export-error-manifest = Export blocked: manifest invariant check failed
export-error-manifest-after-prune = Export finished with errors: the stale-file prune removed a file the exported playlist still references — the USB databases were written; restore from the pre-export backup
# $needed, $free: sizes ("1.2 GB").
export-error-no-space = Export blocked: not enough free space on the USB drive — need about { $needed }, { $free } free
# $count: the tracks refused. $tracks: their files, joined with ", ".
export-error-formats = Export blocked: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} in a format this USB's formats don't allow, and only lossless files can be converted: { $tracks }
# $missing: tracks without analysis. $total: the playlist's tracks.
export-error-not-analyzed = Export blocked: { $missing }/{ $total } playlist tracks are missing required analysis (waveform, bpm, duration, DAT/EXT files); run analysis first
# $folder: the missing source folder.
export-error-source-missing = Export blocked: source folder is missing: { $folder }. Relocate or remove it first.
# $path: where the eDB should be.
export-error-edb-missing = eDB not found at { $path }

# Event Log warnings during an export. $path: a track's file.
export-warning-unplayable-format = not exported, players don't play its format: { $path }
export-warning-source-missing = missing source file: { $path }
# The playlist was exported to the PDB but not to the eDB. $error: the reason.
export-warning-edb-skipped = eDB update skipped: { $error }
# After copying: a file the export wrote is not on the USB. $track: the track's id. $path: the file.
export-warning-artwork-missing-after = export verification warning: artwork missing for track '{ $track }': { $path }
export-warning-analysis-missing-after = export verification warning: analysis file missing for track '{ $track }': { $path }
# A converted copy is exported without analysis. $path: the source track's file.
export-warning-transcode-no-analysis = no analysis for the transcode of { $path }
# A track left out because converting it failed. $path: its file. $reason: why, from the converter.
export-warning-transcode-failed = not exported, converting it failed: { $path }: { $reason }
export-warning-transcode-failed-unknown = not exported, converting it failed: { $path }
# The track's cover file. $path: the file. $error: the reason, from the image library.
export-warning-artwork-source-missing = artwork missing: { $path }
export-warning-artwork-unreadable = artwork decode failed for { $path }: { $error }
export-warning-artwork-write-failed = artwork encode failed for { $path }: { $error }
# The track's analysis files aren't in the app's data. $track: the track's id. $path: the file looked for.
export-warning-analysis-not-made = analysis bundle missing for track { $track } (no DAT path)
export-warning-analysis-files-missing = analysis bundle missing for track { $track }: { $path }

# Event Log notes after writing the Engine DJ library to the USB.
# $analyzed: tracks with a beat grid and waveform.
export-engine-written = Engine DJ library written (tracks: { $tracks }, analyzed: { $analyzed }, playlists: { $playlists })
# $error: the reason.
export-engine-not-written = Engine DJ library not written: { $error }
# $missing: tracks without analysis. $total: all tracks.
export-engine-tracks-without-analysis = { $missing } of { $total ->
    [one] { $total } track
   *[other] { $total } tracks
} without a usable beat grid or waveform on the USB; the Engine player analyzes those itself on load

## track-list

# A track list's heading: its tracks, and their size when known.
# $count: the tracks. $size: their size, such as 412 MB.
track-list-summary = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
track-list-summary-size = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} · { $size }

## update

# The settings update line. $version: the newest release's version.
update-available = Version { $version } is available.
update-check-failed = Couldn't check for updates.
update-up-to-date = Up to date.
# The banner for a release that fixes something important, or brings notable features.
# $version: the newest release's version.
update-banner-critical = Critical update available: { $version }
update-banner-feature = New features available: { $version }

## settings

# The error when a language that has no catalog is saved as the app's language.
# $language: the language code that was sent.
settings-language-unknown = Unknown language: { $language }
# The Event Log warning at startup when the saved language has no catalog any more.
settings-warning-language-unavailable = The saved language { $language } isn't available; using English

## library

# The status line after a source chip's switch changes.
# $enabled: the folders and imported libraries switched on (a missing folder counts as off).
# $total: all folders plus the imported libraries still found.
library-sources-filter-status = Source filters: { $enabled }/{ $total } enabled

# The status line after adding source folders and indexing them.
# $folders: how many source folders the library has now. $count: the tracks the scan found new.
library-folders-indexed = Source folders: { $folders } | indexed { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}

# The status line when every folder picked was already a source folder.
# $folders: how many source folders the library has.
library-folders-unchanged = Source folders unchanged: { $folders }

# The status line after removing a source folder.
# $folders: how many source folders are left. $count: the tracks that left the library with it.
library-folder-removed = Source folders: { $folders } | removed { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} from library

# The status line after relocating a source folder, when every track found its file.
# $count: the tracks whose path now points into the new folder.
library-relocated = Source relocated: { $count ->
    [one] { $count } track path
   *[other] { $count } track paths
} updated

# The same, when some tracks still point at the old folder.
# $count: the tracks whose path was updated. $unresolved: the tracks left pointing at the old folder.
library-relocated-unresolved = Source relocated: { $count ->
    [one] { $count } track path
   *[other] { $count } track paths
} updated | { $unresolved ->
    [one] { $unresolved } track
   *[other] { $unresolved } tracks
} still unresolved

# The status line after importing the tracks of rekordbox's desktop library (master.db).
# $new: tracks new to the library. $updated: tracks already in it that were updated.
library-import-done-rekordbox = Desktop library import done: { $new } new, { $updated } updated

# The same, when some of the library's files weren't found; the Event Log lists them.
# $new, $updated: as above. $count: the files not found.
library-import-done-rekordbox-not-found = Desktop library import done: { $new } new, { $updated } updated | { $count ->
    [one] { $count } file
   *[other] { $count } files
} not found (see event log)

# The status line after importing the tracks of a Mixxx library.
# $new: tracks new to the library. $updated: tracks already in it that were updated.
library-import-done-mixxx = Mixxx library import done: { $new } new, { $updated } updated

# The same, when some of the library's files weren't found; the Event Log lists them.
# $new, $updated: as above. $count: the files not found.
library-import-done-mixxx-not-found = Mixxx library import done: { $new } new, { $updated } updated | { $count ->
    [one] { $count } file
   *[other] { $count } files
} not found (see event log)

# Scanning and importing refused.
library-error-no-source-folder = Add a source folder before scanning
library-error-no-source-enabled = Switch on at least one source folder before scanning
# $path: the file.
library-error-source-file-missing = source file does not exist: { $path }
# $folder: the source folder, here and below.
library-error-folder-not-configured = source folder not configured: { $folder }
# Relocate: the folder picked is the one the tracks are in already.
library-error-relocate-same = The new folder must be different from the old one
library-error-relocate-missing = The new folder does not exist or is not a folder: { $folder }
# The Mixxx and rekordbox imports. $error: SQLite's reason, in English. $path: the library's file. $id: the playlist's id there.
library-error-mixxx-not-found = Mixxx library not found
library-error-mixxx-not-found-at = Mixxx library not found: { $path }
library-error-mixxx-open = could not open Mixxx library: { $error }
library-error-mixxx-tables = Mixxx library opened but library/track_locations tables not found
library-error-mixxx-query = Mixxx library query failed: { $error }
library-error-mixxx-row = Mixxx library row error: { $error }
library-error-mixxx-playlist-query = Mixxx playlist query failed: { $error }
library-error-mixxx-playlist-not-found = Mixxx playlist not found: { $id }
library-error-rekordbox-not-found = master.db not found
library-error-rekordbox-not-found-at = master.db not found: { $path }
library-error-rekordbox-open = could not open master.db: { $error }
library-error-rekordbox-key = master.db key failed: { $error }
library-error-rekordbox-tables = master.db opened but djmdContent table not found (wrong key or version)
library-error-rekordbox-query = master.db query failed: { $error }
library-error-rekordbox-row = master.db row error: { $error }
library-error-rekordbox-playlist-not-found = rekordbox playlist not found: { $id }

# Event Log warnings from a scan. $duplicate: the file left out. $kept: the identical file the library keeps.
library-warning-duplicate-file = Same file in two places: { $duplicate } is identical to { $kept }. The library keeps one; delete the extra copy.
# $folders: the folders, comma-separated.
library-warning-source-folders-missing = { $count ->
    [one] { $count } source folder
   *[other] { $count } source folders
} missing or not directories: { $folders }. Relocate or remove { $count ->
    [one] it
   *[other] them
}.

## scan

# Event Log notes after importing from rekordbox's or Mixxx's library. $library: rekordbox or Mixxx.
# $path: the file the library names.
scan-warning-analysis-not-found = analysis file not found: { $path }
scan-warning-artwork-not-found = artwork not found: { $path }
# $track: the track's file.
scan-warning-cover-not-found = cover file not found: { $path } (for { $track })
# $error: the reason, from the OS.
scan-warning-artwork-copy-failed = artwork copy failed: { $path }: { $error }
scan-note-cues-imported = cues imported for { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
scan-note-hot-cues-imported = hot cues imported for { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
scan-note-cues-kept-local = existing cues kept for { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}; their { $library } cues were not imported
scan-note-unsupported-skipped = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} skipped: not a supported audio file
scan-warning-analysis-missing = { $count ->
    [one] { $count } analysis file
   *[other] { $count } analysis files
} not found on disk
scan-warning-artwork-missing = { $count ->
    [one] { $count } artwork file
   *[other] { $count } artwork files
} not found or copy failed
scan-warning-cover-missing = { $count ->
    [one] { $count } cover file
   *[other] { $count } cover files
} not found or copy failed
# $playlist: the imported playlist.
scan-note-playlist-duplicates = { $count ->
    [one] { $count } repeated track
   *[other] { $count } repeated tracks
} in "{ $playlist }" added once
scan-warning-playlist-tracks-deleted = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} in "{ $playlist }" no longer in the { $library } library

## playlist

# The status lines after creating and deleting a playlist. $name: the playlist's name.
playlist-created = Playlist created: { $name }
playlist-deleted = Playlist deleted: { $name }

# What the dialog asks before deleting a playlist. $name: the playlist's name.
playlist-delete-confirm = Delete "{ $name }"?

    This removes the app playlist and its app playlist-track links.
# The same, for a playlist that was exported to a USB before.
playlist-delete-confirm-exported = Delete "{ $name }"?

    This removes the app playlist and its app playlist-track links.

    This playlist was exported before. It is easy to recreate by importing playlists from USB.

# The status line after adding tracks to a playlist.
# $count: the tracks added. $skipped: the tracks already in it. $name: the playlist's name.
playlist-tracks-added = Added { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} (skipped { $skipped }) to { $name }

# The status line after removing tracks from a playlist.
# $count: the tracks removed. $name: the playlist's name.
playlist-tracks-removed = Removed { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} from { $name }

# The playlist page's button that analyzes its tracks that still need it, and its tooltip.
# $count: how many tracks of the playlist need analysis.
playlist-analyze-missing = Analyze Missing Tracks ({ $count })
playlist-analyze-missing-tooltip = Add the missing waveform, BPM, key and first beat to this playlist's unanalyzed tracks; a BPM or key you edited or imported is kept
# The same button (hidden) when no track needs analysis.
playlist-analyze-missing-none = Analyze Missing Tracks
playlist-analyze-missing-none-tooltip = No tracks in this playlist need analysis

# The Add to… dialog's note beside a playlist that holds all of the selected tracks.
playlist-membership-all = Already in
# The same, for one holding some of them.
# $contained: the selected tracks it holds. $selected: how many tracks are selected.
playlist-membership-some = { $contained } of { $selected } in

# The tooltip on a playlist's sortable columns when exporting it can't change the order on the USB.
# $name: the playlist's name.
playlist-reorder-lock-tooltip = Won't reorder on USB — "{ $name }" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order.

playlist-error-name-empty = playlist name must not be empty

## playlist-import

# The import dialog's note on a list. $count: its tracks.
playlist-import-note = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# The same, for a list imported before. $count: its tracks. $name: the playlist importing it again updates.
playlist-import-note-updates = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} · updates "{ $name }"

# The status line after importing one list. $name: the playlist. $count: the tracks placed in it.
playlist-import-one-imported = Imported playlist { $name }: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# The same, when the import updated the playlist an earlier import made.
playlist-import-one-updated = Updated playlist { $name }: { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# The status line after importing several lists.
# $imported: the lists imported. $count: the lists picked. $tracks: the tracks placed over all of them.
playlist-import-many = Imported { $imported } of { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
}: { $tracks ->
    [one] { $tracks } track
   *[other] { $tracks } tracks
}
# Added to an import's status line after " | ". $count: the list entries not placed.
playlist-import-skipped = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} skipped (see event log)
# Added to an import's status line after " | ". $count: the lists that failed. $names: their names, comma-separated.
playlist-import-failed-lists = { $count } failed: { $names } (see event log)

# An imported list none of whose tracks could be added. $count: the tracks it lists. $playlist: its name.
playlist-import-error-nothing-imported = { $count ->
    [one] the only track in "{ $playlist }" could not be imported
   *[other] none of the { $count } tracks in "{ $playlist }" could be imported
}: files not found on this computer or not supported

## selection

# The tooltip of the selection bar's time when some selected tracks have no length yet.
# $count: how many of them.
selection-duration-unknown = { $count ->
    [one] { $count } of the selected tracks has no length yet
   *[other] { $count } of the selected tracks have no length yet
}

## track-details

# The status line after saving genre, rating or release date of tracks.
# $count: the tracks saved.
track-details-saved = Saved track details for { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# The same, for tracks on the USB.
# $count: the tracks saved.
track-details-saved-usb = Saved track details for { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} to USB


# The progress bar while the job runs.
track-details-progress-checking-tracks = USB: Checking tracks
track-details-progress-writing-pdb = USB: Writing export.pdb
track-details-progress-writing-edb = USB: Writing exportLibrary.db
track-details-progress-saved = USB: Track details saved

# $min, $max: the earliest and latest year allowed.
track-details-error-release-date = release date must be YYYY, YYYY-MM or YYYY-MM-DD, from { $min } to { $max }

## cues

# The status line after saving a track's cues. $count: the cues saved.
cues-saved = Saved { $count ->
    [one] { $count } cue
   *[other] { $count } cues
}
# The same, when the track's analysis cache couldn't be rewritten yet.
# $count: the cues saved.
cues-saved-no-cache = Saved { $count ->
    [one] { $count } cue
   *[other] { $count } cues
} (analysis cache not updated yet)
# The same, for a track on the USB. $count: the cues saved.
cues-saved-usb = Saved { $count ->
    [one] { $count } cue
   *[other] { $count } cues
} to USB


# The progress bar while the job runs.
cues-progress-checking-track = USB: Checking track
cues-progress-writing-analysis-files = USB: Writing analysis files
cues-progress-writing-pdb = USB: Writing export.pdb
cues-progress-updating-library-track = Updating library track
cues-progress-writing-edb = USB: Writing exportLibrary.db
cues-progress-saved = USB: Cue edits saved

# Saving the cue editor. $path: the track's file on the USB.
cues-error-not-in-usb-database = this track is missing from one of the USB's two databases, so it can't be edited on the USB ({ $path }). Preview Fixes shows what can be repaired.
# $key: the key as typed.
cues-error-unknown-key = unknown key: { $key }
# $max: the most cue points a track can have (8).
cues-error-too-many = at most { $max } cue points are allowed
cues-error-bpm-range = bpm must be greater than 0 and at most 999

## playback

# Where a track plays from, in the status line below.
playback-source-library = Library
playback-source-usb = USB
# The status line while a track plays.
# $source: where it plays from (above). $track: "Artist - Title", or the title alone.
playback-playing = Playing from { $source }: { $track }

## track-table

# The BPM pill's tooltip: where the track's BPM came from, when not from this app's analysis.
track-table-bpm-source-user = Manually set
track-table-bpm-source-mixxx = From Mixxx
track-table-bpm-source-rekordbox = From rekordbox
# The same, for a BPM this app's analysis found. $analyzer: the analyzer's id, such as stratum-v2 or essentia.
track-table-bpm-source-analyzer = Analyzed with: { $analyzer }

# The FMT badge for a file with no extension.
track-table-format-unknown = Unknown
# Parts of the FMT badge's tooltip ("44.1 kHz · 16-bit").
# $khz: the sample rate in kHz, formatted. $bits: the bit depth. $kbps: the bitrate.
track-table-format-sample-rate = { $khz } kHz
track-table-format-bit-depth = { $bits }-bit
track-table-format-bitrate = { $kbps } kbps

## track-row

# A track row's title and artist when the file has none.
track-row-unknown-title = Unknown Title
track-row-unknown-artist = Unknown Artist

## ui-add-to

ui-add-to-apply = Add
ui-add-to-create = Create
ui-add-to-list-aria-label = Playlists
ui-add-to-new-name-aria-label = New playlist name
ui-add-to-new-name-placeholder = New playlist
ui-add-to-title = Add to playlists
# The dialog's title. $count: how many tracks are selected.
ui-add-to-title-count = { $count ->
    [one] Add { $count } track to playlists
   *[other] Add { $count } tracks to playlists
}
# The list when there are no playlists.
ui-add-to-empty = No playlists yet: create one below.
# The status line when creating a playlist fails. $error: the backend's message.
ui-add-to-create-failed = Could not create playlist: { $error }

## ui-backups

ui-backups-delete = Delete
# The status line when deleting a backup fails.
# $error: why it failed.
ui-backups-delete-failed = Delete failed: { $error }
ui-backups-delete-title = Delete Backup
ui-backups-intro = Each backup holds both of the USB's databases: legacy (PDB) and new (eDB).
ui-backups-load-failed = Failed to load backups
ui-backups-refresh = Refresh
ui-backups-restore = Restore
# The status line when restoring a backup fails.
# $error: why it failed.
ui-backups-restore-failed = Restore failed: { $error }
ui-backups-restore-title = Restore Backup
ui-backups-summary = No entries
ui-backups-title = Backups

## ui-common

ui-common-cancel = Cancel
ui-common-save = Save

## ui-convert

# Under the Convert… dialog's choices.
# $folder: the folder the converted files are saved in.
ui-convert-folder-hint = Saved in { $folder } (change it in Settings).
ui-convert-apply = Convert
ui-convert-format = Format
ui-convert-intro = Makes another file of each selected lossless track, for players that can't read FLAC or ALAC. It is still the same track: no new library row, and cues and edits stay on the track.
ui-convert-remove = Remove transcodes
ui-convert-remove-tooltip = Delete the selected tracks' transcode files
ui-convert-title = Convert tracks
# The dialog's title. $count: how many tracks are selected.
ui-convert-title-count = { $count ->
    [one] Convert { $count } track
   *[other] Convert { $count } tracks
}

## ui-cover-img

ui-cover-img-alt = cover

## ui-drive-name

# The name dialog, when the connected drive carries a name another drive here already has.
# $name: the name it carries.
ui-drive-name-conflict = This drive carries the name “{ $name }”, but it isn't the drive this computer knows by that name — most likely it's a copy of it. Give this one a name of its own so the two don't share backups.
ui-drive-name-input-placeholder = e.g. Club Stick
ui-drive-name-intro = Give this USB a name so backups and local caching can reliably follow it, even on a different computer. If this is a real USB drive with a volume label, it's pre-filled below — edit it or replace it with your own. Each drive needs its own name, even when several have the same volume label.
ui-drive-name-rename-note = The drive's backups and local cache stay with it under the new name.
ui-drive-name-skip = Not now
ui-drive-name-title-name = Name This Drive
ui-drive-name-title-rename = Rename This Drive

## ui-event-log

ui-event-log-clear = Clear
ui-event-log-coalesced-occurrences-tooltip = Coalesced occurrences
ui-event-log-level = Level
ui-event-log-level-filter-all = All
ui-event-log-level-filter-error = Error
ui-event-log-level-filter-info = Info
ui-event-log-level-filter-log = Log
ui-event-log-level-filter-warn = Warn
ui-event-log-open-file = Open log file
ui-event-log-open-file-tooltip = Open this session's full log in a text viewer -- the list here keeps only the newest entries
ui-event-log-source = Source
ui-event-log-summary = No entries
ui-event-log-title = Event Log
# The header Event Log button's label with errors logged while it was closed.
# $count: how many.
ui-event-log-button-new-errors = { $count ->
    [one] Event Log, { $count } new error
   *[other] Event Log, { $count } new errors
}
# The same, when a USB report links to details in the log.
ui-event-log-button-linked-details = Event Log, has details for the USB report
# The count above the list. $count: the entries shown.
ui-event-log-summary-count = { $count ->
    [one] { $count } event
   *[other] { $count } events
}
# The same, when some entries repeat. $count: the entries shown.
# $occurrences: how many times they happened in all.
ui-event-log-summary-count-repeats = { $count ->
    [one] { $count } event ({ $occurrences } occurrences)
   *[other] { $count } events ({ $occurrences } occurrences)
}
# An entry's code when it has none, shown in brackets.
ui-event-log-unknown-code = unknown
# The list when no entry matches the filters.
ui-event-log-empty = No events
# The status line when the log file can't be opened. $error: the error's message.
ui-event-log-open-file-failed = Could not open the log file: { $error }

## ui-group-edit

ui-group-edit-apply = Apply
ui-group-edit-genre = Genre
ui-group-edit-intro = Only the fields you change are written; the rest stay as they are on each track. An empty field clears it.
ui-group-edit-rating = Rating
# A field's placeholder, or beside the rating, when the selected tracks differ.
ui-group-edit-mixed = Mixed
ui-group-edit-release-date = Release date
ui-group-edit-release-date-tooltip = YYYY-MM-DD, or just YYYY-MM or YYYY: export completes a month or year
# The release date field's placeholder when the selected tracks share no date.
ui-group-edit-release-date-placeholder = YYYY-MM-DD
ui-group-edit-title = Edit tracks
# The dialog's title. $count: how many tracks are selected.
ui-group-edit-title-count = { $count ->
    [one] Edit { $count } track
   *[other] Edit { $count } tracks
}
# The status line while a USB save waits for another USB job.
ui-group-edit-waiting-for-usb-job = Waiting for the running USB job to finish before saving...
# The status line when the save fails. $error: the backend's message.
ui-group-edit-save-failed = Could not save track details: { $error }

## ui-header

# The header's drive badge, when a drive is connected but has no name yet.
ui-header-usb-name-badge-unnamed = Unnamed
# The header's drive badge, when no drive is connected.
ui-header-usb-name-badge-not-connected = Not connected
ui-header-usb-name-badge-name-tooltip = Name this drive
ui-header-usb-name-badge-rename-tooltip = Rename this drive

ui-header-badge-none = No active playlist
ui-header-event-log = Event Log
ui-header-help = Help
ui-header-playlist-badge = Playlist
ui-header-settings = Settings
# The Settings button's label when an update is available.
ui-header-settings-update-available = Settings (update available)
# The confirm dialog's title and button when the asker names none.
ui-confirm-default = Confirm
# The status line's text when nothing is going on: at startup, and once playback stops.
ui-header-status = Idle
ui-header-update-banner-dismiss-aria-label = Dismiss
ui-header-update-banner-download = Download
ui-header-update-banner-restart = Update & restart
ui-header-update-banner-view-release = view release
ui-header-usb-name-badge = USB

## ui-help

ui-help-aria-label = Quick Start Help
ui-help-close-aria-label = Close help
ui-help-donate-heading = Support DJ USB Tkit
ui-help-donate-open = Open support page
ui-help-donate-text = Support options are maintained on chiph.art so payment links can stay current.
ui-help-step-analyze = Analyze:
ui-help-step-analyze-text = Click "Analyze Library" to check your folders for changes and analyze their tracks.
ui-help-step-export = Export:
ui-help-step-export-text = Open your playlist and click "Export to USB" to write it to your drive.
ui-help-step-folders = Add folders:
ui-help-step-folders-text = Click "+ Add Folder" in Library to add your music directories.
ui-help-step-playlist = Create playlist:
ui-help-step-playlist-text = Click "+ New" in the sidebar to create a playlist, then add tracks from Library or USB.
ui-help-step-usb = Connect USB:
ui-help-step-usb-text = Go to USB and select your External library USB drive.
ui-help-title = Quick Start

## ui-job

ui-job-cancel-analysis = Cancel analysis
ui-job-cancel-transcode-aria-label = Cancel converting
ui-job-cancel-transcode-title = Cancel converting (finished files are kept)
ui-job-dismiss-aria-label = Dismiss
ui-job-idle = Idle
# The progress bar's text while a job runs with no step of its own.
ui-job-working = Working...
# After the elapsed time while analysis is paused.
ui-job-paused = (paused)
# A job's progress bar: running, finished, failed. $label: the job's title, such as "Adding tracks".
ui-job-label-working = { $label }...
ui-job-label-done = { $label } done
ui-job-label-failed = { $label } failed
# The same, for a backend job that sent no message.
ui-job-done = Done
ui-job-failed = Failed
# The analysis pause button's label while paused.
ui-job-resume-analysis = Resume analysis
# The Event Log entry when a track's analysis fails. $track: the track. $error: why.
ui-job-track-analysis-failed = Track analysis failed: { $track } - { $error }
# Stands in for $error above when the analysis gave no reason.
ui-job-track-analysis-failed-unknown = unknown analysis error
ui-job-pause-analysis = Pause analysis
ui-job-progress = Progress
ui-job-progress-aria-label = Operation progress

## ui-library

ui-library-add-source = + Add Folder
ui-library-analyze-library = Analyze Library
ui-library-external-rekordbox-db = External Library
ui-library-filter-source-aria-label = Filter source
ui-library-folders = Folders
ui-library-import-from-mixxx = Import from Mixxx
ui-library-import-from-rekordbox = Import from rekordbox
ui-library-libraries = Libraries
ui-library-remove-aria-label = Remove
ui-library-search-placeholder = Search title, artist, album, genre, year
ui-library-source-filter-indicator = Filtered
ui-library-sources = Sources
ui-library-title = Media Library
ui-library-toggle-mixxx-library-aria-label = Toggle Mixxx library
ui-library-toggle-rekordbox-library-aria-label = Toggle rekordbox library
# The tooltip of an imported library's chip before its tracks are imported.
ui-library-chip-not-imported-rekordbox-tooltip = Not imported yet: click ↻ to import from rekordbox
ui-library-chip-not-imported-mixxx-tooltip = Not imported yet: click ↻ to import from Mixxx
# A source folder chip whose folder is gone: the chip's tooltip, its switch's label,
# and its name's tooltip. $path: the folder.
ui-library-folder-missing-tooltip = Source folder missing. Click to relocate.
ui-library-folder-missing-aria-label = Source folder missing
ui-library-folder-missing-path-tooltip = Folder missing. Click to relocate: { $path }
# The status line while picking a source folder's new place. $path: the folder that's missing.
ui-library-relocate-starting = Relocate source folder: { $path }
# The library before any source is set up: its heading and buttons.
ui-library-empty-heading = Add a music folder to get started
ui-library-empty-add-folder = Add Folder
ui-library-empty-import-rekordbox = rekordbox library
ui-library-empty-import-mixxx = Mixxx library
# The progress bar's title and steps while adding source folders.
ui-library-add-folders-title = Loading media library
ui-library-add-folders-saving = Saving source folders...
ui-library-add-folders-indexing = Indexing new files...
ui-library-add-folders-building = Building track list...
ui-library-add-folders-refreshing = Refreshing playlist views...
# Asked before removing a source folder: the dialog's title, its text and its button.
# $path: the folder.
ui-library-remove-folder-title = Remove Source Folder
ui-library-remove-folder-confirm = Remove source folder?

    { $path }

    All tracks from this folder will be removed from the library. Tracks that are in a local playlist will also be removed from those playlists. Tracks that are also inside another source folder stay in the library.
ui-library-remove-folder = Remove
# Where an Event Log entry came from, in its details ("context: Mixxx library import").
ui-library-import-rekordbox-log-context = desktop library import
ui-library-import-mixxx-log-context = Mixxx library import
# Importing the tracks of rekordbox's desktop library (master.db) or of Mixxx's library:
# the status line while it runs, and when it fails. $error: the backend's message.
ui-library-import-rekordbox-starting = Importing from desktop library...
ui-library-import-rekordbox-failed = Desktop library import failed: { $error }
ui-library-import-mixxx-starting = Importing from Mixxx library...
ui-library-import-mixxx-failed = Mixxx library import failed: { $error }

## ui-playlist

ui-playlist-analyze-missing = Analyze Missing Tracks
ui-playlist-delete-playlist = Delete playlist
ui-playlist-drag-to-reorder = Drag to reorder
ui-playlist-export = Export to USB
ui-playlist-export-space-tooltip = The size to copy includes the tracks' analysis files and artwork, so it can be larger than the audio size in the title
ui-playlist-export-status = Not exported yet.
ui-playlist-format-warning-tooltip = Follows the USB's Audio formats (USB view). Only lossless tracks can be converted
ui-playlist-name-placeholder = Playlist name...
ui-playlist-rename-aria-label = Rename playlist
ui-playlist-rename-tooltip = Rename playlist (or double-click the name)
ui-playlist-search-placeholder = Search playlist tracks
ui-playlist-title = Playlist
# The tooltip of the active playlist (the one tracks are added to) in the sidebar.
ui-playlist-active-tooltip = Active playlist
# The status line when there's no active playlist to add tracks to.
ui-playlist-none-active = Create and activate a playlist first
# After the title, for a playlist imported from another library. $library: rekordbox or Mixxx.
ui-playlist-imported-from = · Imported from { $library }
# Status lines when an action fails. $error: the backend's message.
ui-playlist-rename-failed = Rename failed: { $error }
ui-playlist-order-failed = Save playlist order failed: { $error }
ui-playlist-remove-failed = Remove failed: { $error }
ui-playlist-export-failed = Export failed: { $error }
# Stands in for $error above when the failure has no message.
ui-playlist-export-failed-unknown = unknown error
ui-playlist-analyze-failed = Analyze failed: { $error }
ui-playlist-track-order-failed = Save track order failed: { $error }
ui-playlist-track-order-still-loading = Track order not saved: the list was still loading. Try again.
# Creating a playlist: the status line without a name, then the progress bar's title and steps.
ui-playlist-name-required = Playlist name is required
ui-playlist-create-title = Creating playlist
ui-playlist-create-saving = Saving playlist...
ui-playlist-refreshing-playlists = Refreshing playlists...
# Asked before deleting a playlist: the dialog's title and its button.
ui-playlist-delete-title = Delete App Playlist
ui-playlist-delete = Delete
# The status line when the backend didn't delete the playlist. $name: the playlist's name.
ui-playlist-delete-failed = Delete failed: { $name }
# Adding tracks to the active playlist: the progress bar's title and steps, and the status
# line when none of the tracks could be found or none is ticked.
ui-playlist-add-title = Adding tracks
ui-playlist-add-resolving = Resolving tracks...
ui-playlist-add-adding = Adding tracks...
ui-playlist-add-refreshing = Refreshing playlist...
ui-playlist-add-none-resolved = No imported track IDs found to add
ui-playlist-add-select-first = Select at least one track to add
# The Event Log entry as the add starts. $count: the tracks sent. $playlist: the playlist's name.
ui-playlist-adding-tracks = { $count ->
    [one] Adding { $count } track to { $playlist }
   *[other] Adding { $count } tracks to { $playlist }
}
# An empty playlist: its heading and the buttons that lead to tracks to add.
ui-playlist-empty-heading = Browse Library or USB to add tracks
ui-playlist-empty-library = Media Library
ui-playlist-empty-usb-playlists = USB Playlists
ui-playlist-empty-usb = USB

## ui-playlist-import

ui-playlist-import-force = Force update track data
ui-playlist-import-force-note = Replaces the BPM, key and cues of the playlists' tracks, including your own edits.
ui-playlist-import-intro = Each is imported as a new playlist. Tracks not yet in the library are imported with it.
ui-playlist-import-ok = Import
# The import button with more than one list ticked. $count: how many.
ui-playlist-import-ok-count = { $count ->
    [one] Import { $count }
   *[other] Import { $count }
}
# The force option when every ticked list comes from one library. $library: rekordbox or Mixxx.
ui-playlist-import-force-from = Force update track data from { $library }
# A library's list of playlists. $library: rekordbox or Mixxx.
ui-playlist-import-column-aria-label = { $library } lists
ui-playlist-import-title = Import Playlists
# The dialog's groups of lists in each library's column.
ui-playlist-import-group-playlists = Playlists
ui-playlist-import-group-crates = Crates
ui-playlist-import-group-history = History
# The status line when a library's lists can't be read. $library: rekordbox or Mixxx. $error: the backend's message.
ui-playlist-import-list-failed = Could not read { $library } playlists: { $error }
# The status line when no library has a list with tracks.
ui-playlist-import-none-found = No playlists with tracks found
# Where an Event Log entry came from, in its details. $library: rekordbox or Mixxx.
ui-playlist-import-log-context = { $library } playlist import
# The progress bar while importing one list. $library: rekordbox or Mixxx. $name: the list's name.
ui-playlist-import-one-title = Importing { $library } playlist
ui-playlist-import-one-starting = Importing { $name }...
# The same for several lists. $count: how many.
ui-playlist-import-many-title = { $count ->
    [one] Importing { $count } playlist
   *[other] Importing { $count } playlists
}
ui-playlist-import-many-starting = Importing playlists...
# Stands in for a failed list's error when the backend gave none.
ui-playlist-import-failed-unknown = import failed
# The status line when no list could be imported. $library: rekordbox or Mixxx. $error: the first failure's message.
ui-playlist-import-one-failed = { $library } playlist import failed: { $error }
ui-playlist-import-many-failed = Playlist import failed: { $error }

## ui-playback

# The status line while playback is paused.
ui-playback-paused = Paused
# The status line when the track to play is in neither the library nor the selected USB.
ui-playback-not-found = Cannot play: track not found in Library or selected USB.
# The status line when playback fails. $error: the error's message.
ui-playback-failed = Playback failed: { $error }
# The same, when the player gave no message.
ui-playback-failed-no-message = Playback failed

## ui-startup

# The status line when the app couldn't start. $error: why.
ui-startup-init-failed = Initialization failed: { $error }
# The progress bar while the first lists load: its title and steps.
ui-startup-initializing = Initializing
ui-startup-loading-playlists = Loading playlists...
ui-startup-loading-tracks = Loading tracks...
# The Event Log entry when a data upgrade fails at startup. $error: why.
ui-startup-data-upgrade-failed = Data upgrade failed: { $error }

## ui-update

# The settings update line after the in-app install failed. $version: the version it tried.
ui-update-install-failed = Couldn't install version { $version }.
# The status line when an update is started during another job.
ui-update-busy = Finish the running job before updating.

## ui-rating

ui-rating-1-star-aria-label = 1 star
ui-rating-2-stars-aria-label = 2 stars
ui-rating-3-stars-aria-label = 3 stars
ui-rating-4-stars-aria-label = 4 stars
ui-rating-5-stars-aria-label = 5 stars
ui-rating-aria-label = Rating
ui-rating-tooltip = Click a star to rate; click the same star again to clear

## ui-selection-bar

# The selection bar's count of ticked tracks, on every track list.
# $count: how many tracks are ticked.
ui-selection-bar-count = { $count ->
    [one] { $count } selected
   *[other] { $count } selected
}

ui-selection-bar-add-to = Add to…
ui-selection-bar-add-to-target = Add to
ui-selection-bar-add-to-tooltip = Pick one or more playlists to add the selected tracks to
# The tooltip of "+ Add to" the active playlist. $playlist: its name.
ui-selection-bar-add-tooltip = Add the selected tracks to { $playlist }
# Selecting every track of the list: the progress bar's title and step.
ui-selection-bar-selecting-all = Selecting all matching tracks
ui-selection-bar-enumerating = Enumerating tracks...
ui-selection-bar-clear-selection = Clear selection
ui-selection-bar-convert = Convert…
ui-selection-bar-convert-tooltip = Make AIFF, WAV or MP3 files of the selected lossless tracks for players that can't read FLAC or ALAC, or remove them
ui-selection-bar-count-none = 0 selected
ui-selection-bar-edit-details = Edit details…
ui-selection-bar-edit-short = Edit…
ui-selection-bar-edit-tooltip = Edit genre, release date and rating of the selected tracks
ui-selection-bar-hint-library = Select tracks to add, reanalyze, convert or edit them
ui-selection-bar-hint-playlist = Select tracks to add, remove, reanalyze, convert or edit them
ui-selection-bar-hint-usb = Select tracks to add to a playlist or edit them
ui-selection-bar-reanalyze = Reanalyze
ui-selection-bar-reanalyze-tooltip = Reset waveform, BPM, key and first beat of the selected tracks to a fresh analysis, replacing imported or edited values (cues are kept)
ui-selection-bar-remove = Remove
ui-selection-bar-remove-from-playlist = Remove from playlist
ui-selection-bar-remove-tooltip = Remove the selected tracks from this playlist (they stay in the library)

## ui-settings

ui-settings-accent-reset = Reset
ui-settings-analysis = Analysis
ui-settings-analysis-engine = BPM/Key engine
ui-settings-analysis-engine-stratum = Stratum (built-in)
ui-settings-backup-before-export = Back up PDB and eDB before export
ui-settings-backup-before-export-hint = Before each export, copies the PDB and eDB into a timestamped backups folder alongside the originals.
ui-settings-backups = Backups
ui-settings-backups-to-keep = Backups to keep per file
ui-settings-backups-to-keep-hint = The newest backup always stays on the USB; older ones are archived on this computer up to this count, then pruned.
ui-settings-bpm-range = BPM range
ui-settings-close-aria-label = Close settings
ui-settings-engine-library = Write Engine DJ library
ui-settings-engine-library-hint = Whenever this app changes a USB, rebuilds the library Denon Prime and Numark Mixstream players read, so they use its beat grids, cues and playlists without importing and analyzing again. Play history is kept; changes made on the player itself (cues, loops, playlists) are replaced.
ui-settings-essentia-download = Download ~5 MB
ui-settings-essentia-remove = Remove
# The Essentia row's status.
ui-settings-essentia-ready = ✓ Essentia ready
ui-settings-essentia-downloading = Downloading...
ui-settings-essentia-not-installed = Essentia files not installed
# $error: why the download failed.
ui-settings-essentia-download-failed = Download failed: { $error }
# The progress bar while Essentia downloads.
ui-settings-essentia-downloading-progress = Downloading Essentia...
# Status lines after installing or removing Essentia.
ui-settings-essentia-installed = Essentia installed
ui-settings-essentia-removed = Essentia removed
# $error: the backend's message.
ui-settings-essentia-remove-failed = Remove failed: { $error }
# The status line when the transcode folder can't be read or changed. $error: the backend's message.
ui-settings-transcode-folder-failed = Transcode folder: { $error }
# Status lines after changing the export sync mode.
ui-settings-sync-mode-mirror-saved = Export sync mode: mirror (exact match)
ui-settings-sync-mode-additive-saved = Export sync mode: additive
# The same, when the open playlist is already on the USB, so additive export locks its order.
# $playlist: the open playlist's name.
ui-settings-sync-mode-additive-locked = Export sync mode: additive — "{ $playlist }" already exists on USB, so its track order is locked here (current order kept).
# The same, when its column sort couldn't be saved first. $playlist: the open playlist's name.
ui-settings-sync-mode-additive-locked-sort-failed = Export sync mode: additive — "{ $playlist }" already exists on USB, so its track order is locked here (couldn't save the current sort first).
# Status lines after the export checkboxes change.
ui-settings-export-backup-enabled = Export backup: enabled
ui-settings-export-backup-disabled = Export backup: disabled
ui-settings-engine-library-enabled = Engine DJ library on export: enabled
ui-settings-engine-library-disabled = Engine DJ library on export: disabled
# Status lines after a setting changes. $value: the backups kept. $range: the BPM range, such as 70-180.
ui-settings-backup-retention-saved = Backups to keep per file: { $value }
ui-settings-bpm-range-saved = Analysis BPM range: { $range }
# $notation: ui-settings-key-notation-camelot or -classic.
ui-settings-key-notation-saved = Key notation: { $notation }
ui-settings-key-notation-failed = Could not save key notation
# $engine: ui-settings-analysis-engine-stratum, or Essentia. The second is the Event Log entry.
ui-settings-analysis-engine-saved = Analysis engine: { $engine }
ui-settings-analysis-engine-changed = Analysis engine changed to { $engine }
# The app's version in the settings header. $version: the version.
ui-settings-version = Version { $version }
# Stands in for $version above outside the desktop app.
ui-settings-version-not-set = Not set
ui-settings-export = Export
ui-settings-get-node-js = Get Node.js
ui-settings-key-notation = Key notation
ui-settings-key-notation-camelot = Camelot (8A)
ui-settings-key-notation-classic = Classic (Am)
ui-settings-node-js-missing = Node.js not found —
ui-settings-sync-mode = Sync Mode
ui-settings-sync-mode-additive = Additive playlist update
ui-settings-sync-mode-additive-hint = Adds missing tracks to same-name USB playlists, but keeps existing USB tracks.
ui-settings-sync-mode-mirror = Mirror playlist (exact match)
ui-settings-sync-mode-mirror-hint = Same-name playlists on USB are rewritten to exactly match this app playlist (old tracks removed).
ui-settings-theme = Theme
ui-settings-theme-auto = Auto
ui-settings-theme-dark = Dark
ui-settings-theme-light = Light
ui-settings-title = Settings
ui-settings-transcode-folder-change = Change…
ui-settings-transcode-folder-default-tag = Default folder
ui-settings-transcode-folder-hint = Where Convert… saves the AIFF, WAV and MP3 files it makes. Keep it outside your music folders, so other music apps don't list the files twice. A new folder only applies to new files.
ui-settings-transcode-folder-reset = Use default
ui-settings-transcodes = Transcodes
ui-settings-update-check = Check for updates
ui-settings-update-download = Download
ui-settings-update-release-link = What's new ↗
ui-settings-update-restart = Update & restart

## ui-sidebar

ui-sidebar-add-playlist = New
ui-sidebar-collapse = Collapse sidebar
ui-sidebar-donate = Donate
ui-sidebar-donate-tooltip = Please consider donating
ui-sidebar-expand = Expand sidebar
ui-sidebar-history = History
ui-sidebar-import-playlist = Import
ui-sidebar-import-playlist-aria-label = Import playlists
ui-sidebar-import-playlist-tooltip = Import playlists from rekordbox or Mixxx
ui-sidebar-library = Library
ui-sidebar-manage = Manage
ui-sidebar-nav-aria-label = App sections
ui-sidebar-new-playlist = New playlist
ui-sidebar-player-menu = Player Menu
ui-sidebar-playlists = Playlists
ui-sidebar-resizer-tooltip = Drag to resize · double-click to reset
ui-sidebar-usb = USB
ui-sidebar-website = Website

## ui-track-detail

ui-track-detail-add-cue = + Hot cue
ui-track-detail-add-memory-cue-aria-label = Add a memory cue
ui-track-detail-add-memory-cue-tooltip = Add a memory cue at the playhead (M)
ui-track-detail-beat-grid = Beat grid
ui-track-detail-bpm = BPM
ui-track-detail-bpm-double-tooltip = Double the BPM (analysis caught half tempo)
ui-track-detail-bpm-half-tooltip = Halve the BPM (analysis caught double tempo)
ui-track-detail-bpm-minus-tooltip = -0.01 BPM
ui-track-detail-bpm-plus-tooltip = +0.01 BPM
ui-track-detail-close-aria-label = Close
ui-track-detail-cues = Cues
ui-track-detail-delete-cue-aria-label = Delete cue
ui-track-detail-delete-memory-cue-aria-label = Delete memory cue
ui-track-detail-first-beat = First beat
ui-track-detail-first-beat-minus-tooltip = Back one beat
ui-track-detail-first-beat-plus-tooltip = Forward one beat
ui-track-detail-follow-grid-aria-label = Cues follow grid
ui-track-detail-follow-grid-tooltip = Cues follow grid: changing BPM or the first beat moves every cue so it stays on its beat. Off, cues keep their place in the audio, as in rekordbox
ui-track-detail-genre = Genre
ui-track-detail-genre-tooltip = Genre: type one or pick one the library already has
ui-track-detail-grid-level-tooltip = How strongly the beat grid shows over the waveform
ui-track-detail-hint-aria-label = How to use the waveform
ui-track-detail-hint-cue-markers = Cue markers
ui-track-detail-hint-cue-markers-drag = drag to move ·
ui-track-detail-hint-cue-markers-free = places freely · click to play from the cue ·
ui-track-detail-hint-cue-markers-memory = adds a memory cue (▼)
ui-track-detail-hint-cue-markers-quantize = on: snaps to beats,
ui-track-detail-hint-shift-key = Shift
ui-track-detail-hint-tooltip = Waveform: click to move the playhead, drag the playhead to listen, double-click to add a cue, scroll to zoom, drag to pan. Cue markers: drag to move, click to play from the cue. With Q on, cues snap to beats; hold Shift to place freely. Keys: Space play/pause · C add cue · M add memory cue · 1–8 jump to cue · ←/→ move the selected cue a beat (Shift: 10 ms) · Ctrl+Z undo · Ctrl+Shift+Z redo.
ui-track-detail-hint-waveform = Waveform
ui-track-detail-hint-waveform-text = click to move the playhead · drag the playhead to listen · double-click to add a cue · scroll to zoom · drag to pan
ui-track-detail-key = Key
ui-track-detail-key-minus-tooltip = Previous key
ui-track-detail-key-plus-tooltip = Next key
ui-track-detail-memory-cue-badge = memory cue
ui-track-detail-memory-cue-colour-aria-label = Memory cue colour
ui-track-detail-memory-cue-only-tooltip = Memory cue only (no hot cue)
ui-track-detail-memory-cues = Memory cues:
ui-track-detail-metronome-aria-label = Metronome
ui-track-detail-metronome-mix = Mix
ui-track-detail-metronome-mix-tooltip = Metronome mix: left favours the music, right the clicks; in the middle both play at full level
ui-track-detail-metronome-tooltip = Metronome: click on every beat while playing, to hear whether the grid lines up
ui-track-detail-ms = ms
ui-track-detail-name-placeholder = Name
ui-track-detail-next-memory-cue = Next memory cue
ui-track-detail-no-colour = No colour
ui-track-detail-overview-aria-label = Whole track: click or drag to move the view
ui-track-detail-play-from-cue-aria-label = Play from this cue
ui-track-detail-play-from-here-tooltip = Play from here
ui-track-detail-play-from-memory-cue-aria-label = Play from this memory cue
ui-track-detail-play-pause = Play
ui-track-detail-playback-start = Playback start
ui-track-detail-previous-memory-cue = Previous memory cue
ui-track-detail-quantize = Q
ui-track-detail-quantize-tooltip = Quantize: new and dragged cues snap to the nearest beat (hold Shift to place freely)
ui-track-detail-redo = Redo
ui-track-detail-redo-tooltip = Redo (Ctrl+Shift+Z)
ui-track-detail-release-date = Release date
ui-track-detail-release-date-tooltip = Release date: YYYY-MM-DD, or just YYYY-MM or YYYY (export completes a month or year); empty for none
ui-track-detail-reveal-aria-label = Show in folder
ui-track-detail-start = Playback starts at
ui-track-detail-start-first-cue = First cue
ui-track-detail-start-first-cue-tooltip = The CDJ loads on its first cue: cue A, or a memory cue before it
ui-track-detail-start-note = No cue points yet: the CDJ starts at the first audio
ui-track-detail-title = Cue Points
ui-track-detail-undo = Undo
ui-track-detail-undo-tooltip = Undo (Ctrl+Z)
ui-track-detail-zoom-fit = Fit
ui-track-detail-zoom-fit-tooltip = Fit whole track
ui-track-detail-zoom-in = Zoom in
ui-track-detail-zoom-out = Zoom out
# The play button's label and tooltip while playing.
ui-track-detail-pause = Pause
# Starts a memory cue marker's tooltip, before its name and position.
ui-track-detail-memory-cue = Memory cue
# A hot cue's colour button. $cue: the cue's letter (A to H).
ui-track-detail-cue-colour-aria-label = Cue { $cue } colour
# "Playback starts at": the button that adds a start marker on the first beat, and its tooltip.
ui-track-detail-start-first-beat-tooltip = Adds a ▶ memory cue (no hot cue) on the first beat, so the CDJ loads there instead of on cue A
# The same button once the start marker is moved off the first beat, and its tooltip.
ui-track-detail-start-marker = Start marker
ui-track-detail-start-marker-tooltip = The CDJ loads on the ▶ start marker you placed; drag it on the waveform to move it
# Beside the zoom buttons when the view shows the whole track.
ui-track-detail-whole-track = Whole track
# The Show in folder button's tooltip. $path: the track's file.
ui-track-detail-reveal-tooltip = Show in folder: { $path }
# The status line when Show in folder fails. $error: the error's message.
ui-track-detail-reveal-failed = Could not show the file: { $error }
# Status lines when the cue editor can't open.
ui-track-detail-connect-usb-first = Connect the USB this track is on before editing its cues.
ui-track-detail-analyze-first = Analyze this track first to edit its cues.
# $error: the backend's message.
ui-track-detail-open-failed = Could not open cue editor: { $error }
# The status line while a USB save waits for another USB job.
ui-track-detail-waiting-for-usb-job = Waiting for the running USB job to finish before saving cues...
# The status line when saving the cues fails. $error: the backend's message.
ui-track-detail-save-cues-failed = Could not save cues: { $error }
# Status lines when a cue can't be added: all hot cue pads are taken, or a cue is at that spot.
ui-track-detail-max-cues = Maximum 8 cue points.
ui-track-detail-cue-already-there = A cue is already there.

## ui-track-actions

# Status lines when a row's action fails. $error: the error's message.
ui-track-actions-analyze-failed = Analyze failed: { $error }
ui-track-actions-stop-failed = Stop failed: { $error }
ui-track-actions-playback-failed = Playback failed: { $error }

## ui-track-row

ui-track-row-drag-to-reorder = Drag to reorder
ui-track-row-edit-cues-aria-label = Cue points & beat grid
ui-track-row-empty = No tracks available.
ui-track-row-play = Play
ui-track-row-stop = Stop
ui-track-row-analyze = Analyze
ui-track-row-reanalyze = Reanalyze
# The Analyze and Reanalyze buttons' tooltips.
ui-track-row-analyze-tooltip = Add the missing waveform, BPM, key and first beat; a BPM or key you edited or imported is kept
ui-track-row-reanalyze-tooltip = Reset waveform, BPM, key and first beat to a fresh analysis, replacing imported or edited values (cues are kept)
# The cue editor button's tooltip: enabled, and before the track is analyzed.
ui-track-row-edit-cues-tooltip = Edit cue points & beat grid
ui-track-row-edit-cues-analyze-first-tooltip = Analyze this track first
# The row's "+" button. $playlist: the active playlist's name.
ui-track-row-add-to-tooltip = Add to { $playlist }
ui-track-row-add-no-playlist-tooltip = Create and activate a playlist first, then add tracks to it.
# The row's remove button in a playlist. $playlist: the playlist's name.
ui-track-row-remove-from-tooltip = Remove from { $playlist }
ui-track-row-remove-from-playlist-tooltip = Remove from playlist
ui-track-row-play-from-here-tooltip = Play from here
ui-track-row-waveform-preview-aria-label = waveform preview

## ui-track-table

ui-track-table-album = Album
ui-track-table-bpm = BPM
ui-track-table-fmt = Fmt
ui-track-table-key = Key
ui-track-table-length = Length
ui-track-table-library-tracks-aria-label = Library tracks
ui-track-table-loading-tracks = Loading tracks…
ui-track-table-playlist-tracks-aria-label = Playlist tracks
ui-track-table-select-all-aria-label = Select all
ui-track-table-sort-hint = sorted by →
# The Track column's label while the list is sorted by artist, then by title.
ui-track-table-sort-artist = Artist
ui-track-table-sort-title = Title
ui-track-table-total-time = Total time: 0:00
ui-track-table-track = Track
ui-track-table-usb-history-tracks-aria-label = USB history tracks
ui-track-table-usb-playlist-tracks-aria-label = USB playlist tracks
ui-track-table-year = Year

## ui-tracklist-export

ui-tracklist-export-estimated-note = Times are estimated by summing track lengths in play order — CDJs do not record per-track playback times in the USB.
ui-tracklist-export-include-estimated-times = Include estimated times
ui-tracklist-export-ok = Export…
ui-tracklist-export-placement = Time placement
ui-tracklist-export-placement-after = After track (Artist - Title - 0:00)
ui-tracklist-export-placement-before = Before track (0:00 Artist - Title)
ui-tracklist-export-start-from = Start from track
ui-tracklist-export-title = Export Tracklist

## ui-usb

ui-usb-apply-repairs = Apply Fixes
ui-usb-audio-formats = Audio formats
ui-usb-backups = Backups
ui-usb-backups-tooltip = List, restore and delete backups of this USB's databases
ui-usb-diag-back-to-report = Back to Report
ui-usb-diag-log-link = → event log
ui-usb-drag-to-reorder = Drag to reorder
ui-usb-drag-to-set-tooltip = Drag to set the order of preference
ui-usb-edb = eDB
ui-usb-forget-tooltip = Forget this USB device
ui-usb-health-card = Health & Diagnostics
# The USB health dots' tooltip before diagnostics ran (after, the diagnostics send theirs).
ui-usb-health-unknown = USB health: unknown
# The USB page with no USB chosen: its heading (its button is ui-usb-select-usb-folder).
ui-usb-empty-heading = Connect a USB drive to browse and export
ui-usb-init-hint = No External library structure found.
ui-usb-initialize-usb = Initialize USB Structure
ui-usb-issues = Issues
ui-usb-matched = Matched
ui-usb-parity-intro = Checks that older and newer Pioneer players see the same playlists and tracks on this USB.
ui-usb-parity-report = Parity Report
ui-usb-parity-report-tooltip = Check that the USB's legacy (PDB) and new (eDB) databases agree
ui-usb-pdb = PDB
ui-usb-playlist = Playlist
ui-usb-preview-repairs = Preview Fixes
ui-usb-re-diagnose = Rediagnose USB
ui-usb-recent = Recent USBs
ui-usb-remove = Remove
ui-usb-repair-technical-details = Technical details
ui-usb-root-path = No USB selected
ui-usb-select-usb-folder = Select USB Folder
ui-usb-status = Status
ui-usb-title = USB Browser

# The status line when the automatic diagnostics after selecting a USB fail.
# $error: why they failed.
ui-usb-auto-diagnostics-failed = Auto-diagnostics failed: { $error }
# The status line when an audio format ticked or unticked for the USB can't be saved.
# $error: why it failed.
ui-usb-audio-formats-save-failed = Save USB audio formats failed: { $error }
ui-usb-diagnostics-running = Running USB diagnostics...
# The status line when a playlist export can't start because the playlist's sort order wasn't saved.
# $error: why saving it failed.
ui-usb-export-sort-failed = Export blocked: couldn't save the current sort order ({ $error })
# The status line while a playlist is exported.
# $name: the playlist's name.
ui-usb-export-running = Exporting { $name } to USB...
ui-usb-export-starting = Starting USB export...
# The status line when an export fails and the backend sent no status line of its own.
# $error: why it failed, or ui-usb-export-failed-unknown when nothing says.
ui-usb-export-failed = Export failed: { $error }. See Event Log for details.
ui-usb-export-failed-unknown = USB export failed
# The aria-label of the button that forgets a recent USB.
# $path: the USB's folder.
ui-usb-forget-path-aria-label = Forget { $path }
# The status line when initializing the USB's folder structure fails.
# $error: why it failed.
ui-usb-initialize-failed = Initialize failed: { $error }
# The status line when the connected drive's name can't be read.
# $error: why it failed.
ui-usb-name-check-failed = Could not check this drive's name: { $error }
ui-usb-parity-running = Running USB parity report...
ui-usb-repair-applying = Applying supported USB repair fixes...
ui-usb-repair-previewing = Previewing USB repair fixes...
ui-usb-root-cleared = USB root cleared
# The Select USB Folder button's title while a USB operation runs.
ui-usb-select-usb-folder-busy-title = Please wait for the current USB operation to finish
# The status line when another USB is picked while a USB operation runs.
ui-usb-switch-busy = Please wait for the current USB operation to finish before switching drives

## ui-usb-history

ui-usb-history-export-tracklist = Export Tracklist to Text File
# The status line when a history session's tracks can't be loaded.
# $error: why it failed.
ui-usb-history-load-failed = Failed to load USB history: { $error }
ui-usb-history-loading = Loading USB history...
ui-usb-history-search-placeholder = Search history tracks
ui-usb-history-title = USB History

## ui-usb-player-menu

ui-usb-player-menu-add = Add →
ui-usb-player-menu-available = Available (hidden)
ui-usb-player-menu-available-aria-label = Available hidden player menu items
ui-usb-player-menu-background-colour = Background colour
ui-usb-player-menu-background-colour-aria-label = Player background colour
# The status line when the USB's player background colour can't be read.
# $error: why it failed.
ui-usb-player-menu-background-colour-read-failed = Failed to read the background colour: { $error }
ui-usb-player-menu-current = Current player menu
ui-usb-player-menu-current-aria-label = Current visible player menu items
ui-usb-player-menu-down = Move Down
ui-usb-player-menu-loading = Loading player menu configuration...
ui-usb-player-menu-remove = ← Remove
ui-usb-player-menu-title = Player Menu Editor
ui-usb-player-menu-up = Move Up

## ui-usb-playlists

ui-usb-playlists-compared = Playlists compared
# The status line when a USB playlist's tracks can't be loaded.
# $error: why it failed.
ui-usb-playlists-load-failed = Failed to load USB playlist: { $error }
ui-usb-playlists-loading = Loading USB playlists...
# The status line when the new order of the USB's playlists can't be saved.
# $error: why it failed.
ui-usb-playlists-order-failed = Failed to save playlist order: { $error }
# The progress bar when reading the USB's playlists fails.
ui-usb-playlists-read-failed = USB load failed
ui-usb-playlists-reading = Reading USB database...
ui-usb-playlists-remove = Remove
# The status line when removing a playlist from the USB fails.
# $error: why it failed.
ui-usb-playlists-remove-failed = Remove USB playlist failed: { $error }
ui-usb-playlists-remove-title = Remove USB Playlist
ui-usb-playlists-search-placeholder = Search USB playlist tracks
ui-usb-playlists-title = USB Playlists
