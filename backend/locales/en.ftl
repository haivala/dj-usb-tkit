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
#             event-log, backups, track-detail, settings, header
#   JS only:  job, update, playback
#   Rust:     export, diagnostics, repair, analysis, scan, job, error
#   shared:   common (generic dialog buttons and words only), and one area per
#             <template> used on more than one page, named from its id
#             (tplTrackRow is track-row)
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

## settings

# The error when a language that has no catalog is saved as the app's language.
# $language: the language code that was sent.
settings-language-unknown = Unknown language: { $language }

## ui-header

# The header's drive badge, when a drive is connected but has no name yet.
ui-header-usb-name-badge-unnamed = Unnamed
# The header's drive badge, when no drive is connected.
ui-header-usb-name-badge-not-connected = Not connected
ui-header-usb-name-badge-name-tooltip = Name this drive
ui-header-usb-name-badge-rename-tooltip = Rename this drive

## ui-common

# The selection bar's count of ticked tracks, on every track list.
# $count: how many tracks are ticked.
ui-common-selection-count = { $count ->
    [one] { $count } selected
   *[other] { $count } selected
}
