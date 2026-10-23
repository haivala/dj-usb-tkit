# TODO

## Monitor: track rows outside the media library

The library database can collect track rows that no longer belong to the
media library. Every library listing reads past them, so they slow down each
page, search and select-all as they pile up. Known sources:

- tracks under a source folder that was removed before removing a folder also
  removed its tracks;
- USB placeholder rows (`materialize_usb_track_row`) for USB tracks that
  matched no library file, including some stored with a path relative to the
  USB root instead of the mount point;
- tracks outside every source folder whose files still exist.

Watch whether rows outside every source folder keep growing with normal use
(USB imports, history, folder changes). If they do, the app should prune them
itself: a row outside every source folder, not from the Mixxx or rekordbox
library, and in no playlist or history goes. Tracks whose files are in the
media library stay, also when they are on a USB too.
