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

## ui-group-edit

ui-group-edit-apply = Apply
ui-group-edit-genre = Genre
ui-group-edit-intro = Only the fields you change are written; the rest stay as they are on each track. An empty field clears it.
ui-group-edit-rating = Rating
ui-group-edit-rating-mixed = Mixed
ui-group-edit-release-date = Release date
ui-group-edit-release-date-tooltip = YYYY-MM-DD, or just YYYY-MM or YYYY: export completes a month or year
ui-group-edit-title = Edit tracks

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
ui-header-status = Idle
ui-header-update-banner-dismiss-aria-label = Dismiss
ui-header-update-banner-download = Download
ui-header-update-banner-restart = Update & restart
ui-header-update-banner-view-release = view release
ui-header-usb-health-dot = USB health unknown
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

## ui-playlist-import

ui-playlist-import-force = Force update track data
ui-playlist-import-force-note = Replaces the BPM, key and cues of the playlists' tracks, including your own edits.
ui-playlist-import-intro = Each is imported as a new playlist. Tracks not yet in the library are imported with it.
ui-playlist-import-ok = Import
ui-playlist-import-title = Import Playlists

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

## ui-track-row

ui-track-row-drag-to-reorder = Drag to reorder
ui-track-row-edit-cues-aria-label = Cue points & beat grid
ui-track-row-empty = No tracks available.
ui-track-row-play = Play
ui-track-row-stop = Stop
ui-track-row-analyze = Analyze
ui-track-row-reanalyze = Reanalyze
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
ui-usb-health-dot = USB health unknown
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
