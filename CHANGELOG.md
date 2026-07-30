# Changelog

<!--
  The in-app update checker reads this file's content via the GitHub Release
  body (release.yml copies each `## <version>` section verbatim into the
  release notes). To flag a release as critical — shown as a prominent
  in-app banner instead of the quiet default notice — add a line right
  under the version heading:

  **Severity:** critical
-->

## Unreleased

- Replace native browser `title=""` tooltips throughout the app with a custom
  tooltip that appears in ~150ms instead of the browser's native ~1s hover
  delay. This was most noticeable on the source-folder chips: the full path
  was already exposed on hover (visually truncated with an ellipsis), but the
  slow native delay made it read as "the feature is missing." Format badges
  in the library table now also show sample-rate/bit-depth (or bitrate)
  detail on hover for every track, not just ones with a compatibility warning.
- Remove the `analyze_track_piece` backend command. It was a per-piece
  (duration/artwork/waveform/bpm-key) analysis endpoint left over from an
  older frontend dispatch loop; that loop was already deleted in favor of the
  single `analyze_new_tracks` batch call, leaving `analyze_track_piece`
  unreachable from the app. Also reorder per-track analysis progress
  (duration/artwork/waveform/bpm-key) to artwork, then waveform, then
  duration, then bpm/key, matching the track row's left-to-right column
  order; audio is still decoded only once per track.
- Fix single-track analyze (clicking "Analyze"/"Reanalyze" on one track row)
  having no memory check at all — unlike a multi-track batch, it has no
  worker pool to size against available RAM, so clicking through many track
  rows in a row could pile up concurrent analysis jobs with no memory
  awareness and crash the app. It now checks for available memory headroom
  before starting and fails that track with a clear message instead of
  proceeding when memory is too tight.

## 0.1.7

**Severity:** critical

- Fix the memory-aware analysis worker cap (0.1.5/0.1.6) not applying to the
  most common analysis trigger. Clicking "Scan Library" ran its automatic
  post-scan analysis through a second, separate concurrency mechanism — a
  frontend-driven per-track dispatch loop sized only by CPU core count, with
  no awareness of available memory or `DJTKIT_ANALYSIS_MAX_WORKERS` — so the
  memory cap never actually applied to it. All analysis (Scan Library,
  Analyze Missing Tracks, manual reanalyze) now goes through the single
  memory/CPU/env-capped backend pipeline; the separate uncapped mechanism has
  been removed entirely, along with the now-unused `get_system_parallelism`
  command and the `benchmark_analyze_tracks` dev tool.
- Fix a custom BPM search range (set in Settings) being silently ignored by
  any analysis that went through the batch backend path — it always used the
  default 70-180 range regardless of the configured setting. Now that all
  analysis is consolidated onto this path, this applied everywhere; the
  batch command now honors the configured range.
- Fix the batch analysis worker pool using every available CPU core on
  high-core-count machines, starving the OS/UI thread and making the app
  unresponsive during a scan. The worker pool now reserves ~2 cores for the
  OS/UI, matching the budget the opt-in `essentia` engine's pool already
  used — a regression from consolidating all analysis onto the single capped
  pipeline above, which used a cap that bounded worker count by memory and
  CPU count but never reserved this headroom.
- Fix the library view (including scrolling) becoming unresponsive during a
  large batch analysis, independent of CPU/worker count — even a single
  worker triggered it. Live analysis progress was rebuilding the entire
  source-folder chip row from scratch and rescanning the whole visible track
  list on every single track completion; for a batch of hundreds of tracks
  this pinned the JS main thread almost continuously. Source chips now only
  refresh at natural batch checkpoints (they don't need to reflect
  per-track state, only whether a folder is fully analyzed), and the running
  duration total is updated with a fixed-cost increment per track instead of
  a full rescan.
- Fix batch analysis holding a single database write transaction open for
  the entire batch, committing only once at the very end. SQLite allows only
  one write transaction at a time even in WAL mode, so any other write
  attempted while a batch was running (such as saving a settings change)
  would wait up to 5 seconds and then fail, surfacing as "database is
  locked" (shown generically as "an internal database error" in the Event
  Log), and could make the whole app feel stuck until the batch finished.
  The batch now commits after every individual track instead of only at the
  end — any commit interval tied to a track *count*, even a small one, can
  still hold the lock for an unbounded amount of wall-clock time if
  individual tracks are slow to analyze, so track count isn't a reliable
  basis for the interval; only "after every track" is actually independent
  of hardware, worker count, and per-track duration.
- Add Pause/Resume and Cancel controls to the analysis progress bar. Pausing
  lets any tracks currently being analyzed finish, but holds off starting new
  ones until resumed; cancelling does the same but permanently, ending the
  batch early with a status noting how many tracks were analyzed before it
  stopped. The progress bar's elapsed-time counter keeps ticking for
  whichever track(s) were already in flight when Pause was pressed, and only
  switches to "paused" once they actually finish and nothing new has
  started; resuming picks the counter back up from where it left off
  (excluding the paused duration) instead of restarting it.

## 0.1.6

**Severity:** critical

- Fix analysis still crashing/hanging on lower-RAM, high-core-count machines
  after 0.1.5's memory-aware worker cap. That fix's per-worker memory budget
  for the `stratum` engine was based on a fixed decode-buffer estimate and
  didn't account for `stratum-dsp`'s own internal analysis buffers; measured
  under realistic concurrent load, actual usage was several times higher, so
  the cap rarely bound tighter than CPU count on affected machines. The
  budget is now based on real measurement and worker count is capped
  accordingly, while high-RAM machines see no change in behavior.
- Fix the opt-in `essentia` engine failing to start from the AppImage build
  with an `OPENSSL_3.x not found` error. The AppImage's own bundled
  `libcrypto.so.3` (needed by our Rust dependencies) was shadowing the
  system's newer OpenSSL via `LD_LIBRARY_PATH` when we shelled out to the
  system's Node runtime, which is linked against the newer version. The
  AppImage's own library path is no longer passed through to spawned Node
  processes.
- Fix `DJTKIT_ANALYSIS_DEBUG_WORKERS=1` diagnostic output only reaching the
  in-app Event Log; it now also prints to the terminal.
- Fix source folder chips not turning green after analysis finishes on
  large libraries — they only updated when toggling a folder's filter
  checkbox, and could stay stale on app start. Folder analysis status is
  now refreshed from the backend on startup and whenever an analysis job
  completes.
- When a status message reports warnings or errors (USB scans/exports,
  library scans/analysis), that part of the status line is now a link
  that jumps straight to the Event Log instead of being inert text.
- Fix the "Get Node.js" link in the Essentia setup panel not going
  anywhere — it pointed at a placeholder `#` href instead of the
  download page.

## 0.1.5

**Severity:** critical

- Fix analysis crashing on lower-RAM, high-core-count machines (reported on
  a 12-thread/16GB AppImage install). The analysis worker pool
  previously sized itself from CPU core count alone; it now also caps
  worker count based on available system memory, using a conservative
  per-worker budget (higher for the opt-in `essentia` engine, which runs an
  extra Node.js/WASM process per worker) so a busy or memory-constrained
  host no longer overcommits RAM during batch analysis.
- Add drag-and-drop reordering for USB playlists. Grab the handle on a
  playlist row in the USB Playlists panel to reorder it; the new order is
  written back to the USB's PDB and eDB, so it's reflected on CDJs and in
  rekordbox, not just in the app.
- Remove a dead `master.db` autodetection candidate path under
  `~/.local/share/Pioneer/rekordbox/master.db`. rekordbox has no official
  Linux install, so this path could never match a real installation.

## 0.1.4

- Missing library source folders are now surfaced as recoverable UI state
  instead of being treated as deleted media during scan. Missing source chips
  render as warning chips, scan pruning ignores missing roots, and affected
  exports are blocked until the source is relocated or explicitly removed.
- Add source-root relocation for moved music folders. Relocation rewrites
  indexed track paths to the new folder while preserving track IDs and local
  playlist membership.
- Fix "Duplicate PDB entries" diagnostics failures that the offered strict
  parity repair could not actually resolve. Some playlists on
  long-lived, heavily-edited USBs had accumulated stale duplicate copies of
  a track's playlist membership in the device-side database; running the
  repair reported success without removing them. The repair now finds and
  removes the stale duplicates so affected playlists pass diagnostics.
- Fix strict parity reporting for playlist sort-order drift. eDB playlist
  sequence numbers that no longer match the device-facing PDB playlist order
  are now reported as ordering parity failures and routed through strict
  repair, which syncs eDB order back from PDB.
- Clicking a track row in a USB playlist or history list to load its
  waveform/BPM/key no longer rebuilds and repaints the entire visible track
  list — only the clicked row updates now, matching how the library view
  already behaves.

## 0.1.3

- Fix USB export becoming permanently blocked with a "PDB export blocked"
  error once a USB's playlist list had grown large enough to need extra
  internal storage space for a new playlist — affected any USB that had
  accumulated enough playlists over time, making it impossible to export any
  further new playlist to it.
- Fix USB diagnostics permanently reporting one unfixable "history page
  shape" issue on USBs with a single-entry history-menu page; repair now
  correctly recognizes it as already valid.
- Internal: clean up clippy lint warnings and apply `cargo fmt` across the
  backend and desktop crates (no functional changes).
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
