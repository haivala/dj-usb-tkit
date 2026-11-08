# The text catalog: every user-facing text, in English. Other languages are
# locales/<language>/backend.ftl with the same keys; a key missing there falls back
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
