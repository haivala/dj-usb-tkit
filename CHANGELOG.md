# Changelog

## Unreleased

- Fix USB export becoming permanently blocked with a "PDB export blocked"
  error once a USB's playlist list had grown large enough to need extra
  internal storage space for a new playlist — affected any USB that had
  accumulated enough playlists over time, making it impossible to export any
  further new playlist to it.
- Fix USB diagnostics permanently reporting one unfixable "history page
  shape" issue on USBs with a single-entry history-menu page; repair now
  correctly recognizes it as already valid.
- Detect WAV files using the `WAVE_FORMAT_EXTENSIBLE` header, which some
  Pioneer CDJs reject even when the underlying audio is otherwise within
  spec. Flagged during library scan with a format-badge tooltip; when the
  extensible header wraps plain PCM/IEEE-float data, export automatically
  rewrites it to a standard header (lossless, no re-encoding) so the file
  plays on CDJ hardware.

## 0.1.2

- Fix CDJ hardware hangs on pathological Unicode metadata (long
  combining-mark "zalgo" stacks and names mixing many unrelated Unicode
  scripts) in exported titles, artist, and album names, and in on-disk
  export file/folder names. Thanks to 00000ooooo's album "–5"
  (https://00000ooooo.bandcamp.com/album/--5) for the real-world torture
  test.
- Fix a MIPS unaligned-read hardware freeze for non-ASCII album names.

Note: while testing against that album, rekordbox threw an
"unexpected error" and broke playlists on export of the same USB.

## 0.1.1

- Fix macOS master.db detection for current rekordbox installs, which store
  the database directly under `~/Library/Pioneer/rekordbox/master.db` rather
  than under `Application Support`.

## 0.1.0

- Initial public release.
- Local-first library scanning, playlist management, native playback, and
  frontend source-folder workflows.
- USB import/export with mirror and additive playlist sync modes.
- USB diagnostics, strict parity reporting, preview-first repair actions, and
  timestamped database backups before write operations.
- Local BPM, key, waveform, and artwork analysis for export preparation.
- Release packaging workflow for Tauri desktop builds.
