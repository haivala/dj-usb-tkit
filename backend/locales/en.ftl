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
#   Rust:     export, diagnostics, repair, analysis, scan, job, error
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

## usb-playlists

# The status line when a USB playlist is clicked.
# $name: the playlist's name. $count: how many tracks it has.
usb-playlists-selected = USB playlist selected: { $name } ({ $count ->
    [one] { $count } track
   *[other] { $count } tracks
})

# What the dialog asks before removing a playlist from the stick.
# $name: the playlist's name.
usb-playlists-remove-confirm = Remove USB playlist "{ $name }" from the stick?

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

## diagnostics

# The badge on the USB diagnostics report, for its overall result.
diagnostics-status-pass = PASS
diagnostics-status-warn = WARN
diagnostics-status-fail = FAIL
# The USB health dots' tooltip after diagnostics ran, for the same results.
diagnostics-health-good = USB health: good
diagnostics-health-warnings = USB health: warnings
diagnostics-health-issues = USB health: issues found

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
# The context in the details of analysis warnings in the Event Log.
ui-job-analysis-context = Analysis
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
# Where an Event Log entry came from, in its details ("context: Library scan").
ui-library-scan-log-context = Library scan
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
