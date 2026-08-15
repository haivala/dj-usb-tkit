# Changelog

<!--
  The in-app update checker reads this file's content via the GitHub Release
  body (release.yml copies each `## <version>` section verbatim into the
  release notes). To flag a release as critical — shown as a prominent
  in-app banner instead of the quiet default notice — add a line right
  under the version heading:

  **Severity:** critical

  Every entry is prefixed with its category, bolded, followed by a colon:

  - **New feature:** user-facing capability that didn't exist before
  - **Fix:** corrects incorrect behavior
  - **Improvement:** changes/refactors existing behavior without fixing a bug
    or adding a new capability
  - **Chore:** tests, docs, tooling, or other non-user-facing changes

  When a release is flagged critical, the Severity line must point at which
  item(s) below are the critical ones — not every entry in a critical release
  is necessarily itself critical. Do this by appending `(CRITICAL)` to the
  category prefix of the affected entry/entries, then referencing them from
  the Severity line, e.g.:

  **Severity:** critical — see item(s) marked **(CRITICAL)** below.
  ...
  - **Fix (CRITICAL):** ...
-->

## Unreleased

- **Fix:** additive USB export could create a duplicate track — a second PDB row, a second
  copy of the audio file, and a second playlist entry — when re-exporting a track that was
  already physically present on the USB under a path this app hadn't itself written (e.g.
  copied there originally by rekordbox, or by an earlier export run using a different
  file-naming scheme). The "is this track already on the USB" check only recognized a match
  when the freshly computed destination path exactly matched an existing PDB row's path;
  anything already on the stick under a different layout was invisible to it, so the
  additive-export resolver minted a brand-new track id, copied the audio again, and appended
  a duplicate playlist entry instead of reusing the existing one. Additive export now also
  matches against a content fingerprint (file size plus normalized title and artist) built
  from the USB's existing PDB, and reuses the matched row's identity, media, artwork, and
  analysis bundle instead of duplicating them. This does not retroactively clean up
  duplicates a USB already accumulated before this fix — only diagnostics/repair tooling can
  do that — it only stops additive export from creating new ones going forward.
- **Fix:** the album string-alignment repair (`repair_pdb_album_string_alignment`) could silently
  fail to fix rows that USB diagnostics flagged, when the affected album's original data page had
  since been orphaned from the live PDB table chain (e.g. a track exported by an older,
  pre-alignment-fix build was played on a CDJ — recording it into a PDB history playlist — and its
  regular playlist was later removed). The repair only walked the live chain, while detection scans
  every page in the file, so it could report success without touching the flagged row and USB
  diagnostics would keep reporting the same misaligned album row after every repair attempt. The
  repair now scans the same page range diagnostics does.

## 0.1.16

- **New feature:** the top status line now shows the active playlist and the connected USB drive's
  name stacked together, each with a status dot (accent-colored when a playlist is active; health-
  colored — pass/warn/fail — for the USB drive, from the last diagnostics run). Both are always
  visible, showing "No active playlist" / "Not connected" as placeholders rather than disappearing
  when there's nothing to show.
- **New feature:** USB DB backups (`export.pdb`/`exportLibrary.db` snapshots, taken before every
  export/reorder/remove-playlist/repair/menu-config change) now have a dedicated **Backups** panel
  (Settings → Open Backups) to list, restore, and delete snapshots. Since the PDB and eDB are
  always backed up together as a pair, each backup event is listed as a single "eDB and PDB" entry
  rather than two separate rows, and restoring or deleting one acts on both files at once. The
  newest backup per file always stays on the USB stick, so it travels with the drive to any
  computer; older backups are moved to the local HDD staging cache instead of accumulating forever
  on the (often space-constrained) USB drive. How many backups to keep in total is configurable in
  Settings (default 10) instead of growing unbounded.
- **New feature:** USB drives can now be given a name (prompted once, the first time a valid,
  unnamed drive is connected). The name is stored both in a small marker file on the drive itself
  and locally, so the same physical drive keeps its identity across replugs, different USB ports,
  and even different computers — something the previous mount-path-based identity couldn't do.
  Naming also fixes a pre-existing bug where the local HDD staging cache could pick a new,
  unrelated folder for the same drive after a replug (its cache key was derived from the OS-assigned
  mount path, which isn't guaranteed to stay the same). If the selected folder is on an actual
  removable USB device, the drive's own filesystem volume label is read (via `GetVolumeInformationW`
  on Windows, DiskArbitration on macOS, `/dev/disk/by-label` on Linux) and pre-filled into the
  naming prompt as a suggestion — the user can still edit or replace it before saving.
- **Chore:** removed the dead `patch_pdb_columns_menu_order_by_kind` / `patch_pdb_columns_playlist_first`
  helpers from `pdb_menu.rs`. They had no callers and bypassed the staging cache, unlike the rest of
  the PDB write path.
- **Improvement:** the USB-resident `export.pdb` and `exportLibrary.db` are now staged to a
  local HDD working copy the first time each is read, instead of being re-read from the USB
  mount on every call. The local copy is reused until the USB file's size/mtime changes, so
  browsing playlists, exporting, running diagnostics, and USB repair all hit local disk instead
  of the (slower, more failure-prone) USB mount after the first touch. Writes go to the local
  copy first and are only committed back to the USB drive — atomically, via a temp-file rename —
  if the local copy actually changed; an unchanged write no longer touches the drive at all. USB
  repair batches every fix selected in one apply request against the local copy and flushes to
  the drive once at the end, instead of once per fix. This only applies to the desktop app, which
  is the only context that enables staging (see `usb_staging::init_cache_root`); CLI dev tools
  and the test suite keep reading/writing the USB path directly, unchanged.
- **Improvement:** the Event Log now shows where the local USB staging cache lives (once per
  device, the first time it's created) plus each time a file is staged locally or written back to
  the USB drive, so the working folder is easy to find if you want to inspect it directly.
- **Improvement:** selecting a USB playlist or history session now hydrates all of its
  tracks' waveform/BPM/key/artwork metadata through a handful of batched backend calls
  instead of one call per track. Each `inspect_usb_track` call used to re-parse the PDB
  and re-open/re-key the SQLCipher eDB connection from scratch, so large playlists opened
  dozens of fresh database connections in a row; the new `inspect_usb_tracks` batch command
  parses the PDB and opens the eDB once per chunk of tracks. Hydration still stops issuing
  further chunks and patching rows the moment the user selects a different playlist/history,
  so switching away mid-load no longer wastes backend work on a stale selection.
- **Chore:** update deps

## 0.1.15

**Severity:** critical — see item(s) marked **(CRITICAL)** below.

- **Fix (CRITICAL):** stop the torn-growth-pages repair (added in 0.1.13) from silently
  dropping tracks/playlists that a strict-parity upgrade had just written in the same repair
  run. That repair truncates the PDB back to a "never-populated file tail" boundary computed
  once, up front, before any other repair writes; strict parity's additive writer legitimately
  grows the file past that boundary while merging playlists and tracks, so running the
  truncation afterward (the previous apply order) cut off everything strict parity had just
  added. Torn-growth-pages now runs before strict parity, alongside the truncated-table-chain
  repair, so its truncation boundary is always computed and applied before anything grows the
  file.
- **Improvement:** the USB Diagnostics repair preview now lists proposed fixes in the order
  they're actually applied — the truncated-table-chain and torn-growth-pages structural
  prerequisites first, then the strict-parity upgrade, then the remaining structural repairs —
  instead of strict parity always being pinned to the top of the list regardless of apply
  order.
- **Improvement:** the truncated-table-chain and torn-growth-pages fix checkboxes in the repair
  preview are now locked checked and cannot be deselected while they're proposed, since leaving
  either unchecked while strict parity runs has no safe outcome.
- **Chore:** cache Rust build artifacts (`Swatinem/rust-cache` for macOS/Windows, a host-mounted
  Cargo registry/git cache for the Docker-based Linux build) and the Linux build image's Docker
  layers (`docker/build-push-action` with the GitHub Actions cache backend) in the release
  workflow, so `cargo build --release` and the Linux toolchain image don't rebuild from scratch
  on every tagged release.

## 0.1.14

**Severity:** critical — see item(s) marked **(CRITICAL)** below.

- **New feature:** detect and repair a PDB table whose declared last page is beyond the
  physical end of the file — a more severe variant of the interrupted-export ("USB
  disconnected mid-write") signature, where the table pointer's `last_page`/`empty_candidate`
  were advanced to claim new pages that never actually got flushed to disk. Unlike
  `empty_candidate` (which healthy exports routinely set beyond the current file length as
  reserved growth headroom), a table's `last_page` must always physically exist; when it
  doesn't, every additive track append needed by strict parity hard-fails, and USB Diagnostics
  previously only reported this indirectly as missing playlists and generic strict-parity
  failures. Repair points the table's `last`/`empty_candidate` fields back at the real last
  written page — no page content changes — and runs before the strict-parity upgrade fix,
  since strict parity's additive writes depend on the chain already being walkable.
- **Fix:** stop the strict-parity upgrade's playlist write from failing with `UNIQUE
  constraint failed: playlist.playlist_id` when a target playlist id happened to already be
  occupied by a folder row in eDB. `replace_export_playlist_row_with_identity`'s collision
  check and its reassignment update were both scoped to `attribute = 0` (playlist rows), but
  the `playlist_id` UNIQUE constraint applies to the whole `playlist` table regardless of
  `attribute` — folders share the same id space. This let a same-id folder go undetected (and
  even when detected, left unmoved), aborting the entire strict-parity upgrade on the first
  playlist that needed a fresh PDB-side id landing on an already-occupied eDB folder id.
- **Chore:** `run_usb_diagnostics` dev CLI (`backend/src/bin/run_usb_diagnostics.rs`) now
  prints each diagnostics section's individual status/checks, not just the overall status and
  flattened warning log — needed to tell which section was failing when the overall status and
  warning log alone didn't make it obvious.
- **Fix (CRITICAL):** stop the Windows build from failing to start with a missing
  `libcrypto-3-x64.dll`, and from failing to build at all on CI. SQLCipher (used to decrypt
  `exportLibrary.db`) was built with `rusqlite`'s `bundled-sqlcipher` feature, which links
  against a dynamically-found system OpenSSL instead of bundling one — on Windows this meant
  the installed app needed `libcrypto-3-x64.dll` present alongside it, which the installer
  never shipped. Statically linking against a vendored, from-source OpenSSL build
  (`bundled-sqlcipher-vendored-openssl`) fixed that but broke Windows CI itself, since building
  OpenSSL from source shells out to Perl's `Configure` script, which failed on the Windows
  runner (`Locale::Maketext::Simple` missing from Git Bash's bundled Perl). Settled on
  `OPENSSL_NO_VENDOR=1` pointing at the full slproweb Win64 OpenSSL package's static libraries
  instead — same static-linked, no-DLL-at-runtime result, without compiling OpenSSL at all.
  Also switched the Tauri CLI (`cargo install tauri-cli`, which hit the same
  `openssl-sys`/Perl problem building itself) to npm's `@tauri-apps/cli`, which ships prebuilt
  native binaries per platform.
- **Chore:** update deps

## 0.1.13

- **New feature:** detect and repair a PDB left mid-write by an interrupted export (e.g. the
  USB was disconnected while rekordbox was growing the database). One or more tables'
  `empty_candidate` page — the pre-reserved slot for the next batch of rows — was left holding
  leftover disk content instead of the blank page the writer requires before reusing it, so the
  database never self-healed on a later export and could be rejected as corrupted. USB
  Diagnostics now flags this distinctly (rather than as generic PDB corruption), and Repair
  zeroes the affected page(s), truncates any never-populated file tail left past the header's
  allocation boundary, and recomputes the transaction sequence counter — without touching any
  table's page chain or transaction history.

## 0.1.12

- **New feature:** detect and repair track/album rows with a misaligned UTF-16 string slot —
  MIPS-based CDJ hardware requires 4-byte aligned reads for the UTF-16 string header, and a
  misaligned slot freezes/comm-errors the player while browsing. USB Diagnostics' Contents
  Integrity section now flags any track title/filename/etc. or album name slot that isn't
  4-byte aligned, and Repair can re-encode and relocate the affected rows in place.
- **Chore:** Up the coverage on multiple files

## 0.1.11

**Severity:** critical — see item(s) marked **(CRITICAL)** below.

- **Fix:** restore waveform color refresh on USB/history track rows after
  analysis completes. `patchUsbTrackRow` and `patchHistoryTrackRow` were
  missing `setWaveformColorData` from the deps passed to the shared row-patch
  helper (the library and playlist row-patch call sites had it, these two
  didn't), so a freshly-analyzed track's waveform kept its stale color on
  those two views until the row was fully re-rendered some other way.
- **Chore:** de-duplicate frontend JS boilerplate in `vanilla-ui`
- **Chore:** trim unit tests that duplicated existing e2e coverage in `vanilla-ui`, consolidate a shared test DOM fixture, add an opt-in e2e coverage report (`npm run test:e2e:coverage`)
- **Fix:** determine track USB-origin authoritatively on the backend (`Track.isUsbPath`, matched against the full `usb_devices` registry) instead of a frontend heuristic that only checked whichever USB root happened to be selected in the current session
- **Fix:** stop additive USB export from desyncing every *other* playlist's sibling order. `move_export_playlist_row_to_front` (eDB) bumped every sibling's `sequenceNo` by +1 unconditionally on each export, while the PDB writer always recomputes a fresh contiguous `sort_order` from current relative order. The two matched by luck on a playlist's first export but diverged for any sibling sorting after the exported playlist, then drifted further on every repeat additive export — surfacing as "Playlist ordering parity" failures on every playlist except the one just exported.
- **Fix:** when an existing USB track is matched by alias-normalized path identity, update the eDB content row in place and rewrite the PDB indexed media path/filename slot from the manifest exactly. This prevents exports from leaving stale `Track - .mp3` DB references behind while the on-disk file is `Track -.mp3`, which showed up as an unindexed audio file in USB diagnostics.
- **Fix:** stop deleting a USB playlist from corrupting sibling `playlist_tree` (`t07`) page shape. `remove_rows_inplace` applied the `(1, last removed slot)` tombstone footer convention to every table it tombstones, but `t06`/`t07`/`t16`/`t17`/`t18` use a `(trc, 0)` convention instead (see `docs/PDB.md` "Page Footer Conventions") and never change `u5`/`num_rl` on removal — only `t00`/`t01`–`t05`/`t08`/`t13` do. This produced the "PDB structural integrity: playlist_tree page(s) wrong shape" diagnostic whenever a playlist was removed from a USB that had other playlists sharing its `t07` page.
- **Fix:** stamp `dj_usb_tkit_export_log.v1.json`'s `exportedAt`/`exportDate` fields in local time instead of UTC, matching the timestamp already used for the PDB/eDB backup filenames (`export_2025-04-23_14-32-01.pdb`) written next to it on the USB — the two USB-visible timestamps were previously in different time zones.
- **Fix (CRITICAL):** stop routine additive exports from leaving duplicate-id tombstones in the PDB. When a track's row couldn't be patched in place during an export (e.g. its rewritten `file_name`/`file_path` no longer fit the original row's byte length) and had to be relocated, `mark_track_slot_inactive` vacated the old slot without zeroing its id field, leaving it as a live duplicate of the relocated row's id — the "PDB structural integrity: tombstoned row(s) non-zero id" diagnostic. Because tracks are shared across playlists, exporting one playlist could corrupt rows belonging to playlists untouched by that export, and the corruption recurred on every subsequent export even after running the repair.
- **Fix:** stop the "Indexed audio file presence" strict-parity check from double-counting a single on-disk file as both "missing" and "unindexed". It compared DB-indexed paths against real on-disk filenames using unnormalized indexed paths against raw on-disk paths, so any track whose DB row still carried a trailing-space filename alias (`Track - .mp3` vs `Track -.mp3`) was flagged as one missing file and one unindexed file instead of being recognized as the same file. Fixed by normalizing both sides consistently through a new, narrowly-scoped `normalize_path_for_strict_presence_match` that trims only that trailing-space alias — reusing the existing (broader) `normalize_path_for_contents_match`, which also collapses a trailing `-<digits>` filename suffix, was tried first and made things worse: it collapsed genuinely distinct tracks whose real filenames end in a numeric suffix (release years, remix/version numbers, catalog codes) into the same key, turning a 1-file false positive into thousands of false missing/unindexed results on a large library.
- **Fix (CRITICAL):** stop exported tracks with a non-ASCII title or filename from freezing/comm-erroring CDJ hardware while browsing. The 4-byte MIPS alignment padding required before any UTF-16-encoded (`0x90`) track string was only ever applied to slot 20 (media path); slots 17 (title), 19 (filename), and any other slot that happens to need UTF-16 encoding had no such guarantee, since each slot's row offset depends on the cumulative length of every slot before it. A single track with one non-ASCII character in its title (a curly apostrophe was enough) could land that slot at a misaligned offset, and the CDJ raised a MIPS Address Error and comm-errored while listing the containing playlist — invisible to `dump_pdb_anomalies`, since it checks page/table shape, not per-row string alignment. The padding now applies to every track string slot whose encoded bytes start with the `0x90` marker, not just slot 20.
- **Fix:** stop the "Indexed audio file presence" strict-parity check from false-flagging a track as both missing and unindexed when its real on-disk filename ends in a bare hyphen with nothing after it (a 48-char truncation cutoff landing exactly on a `-`, e.g. `Track -.mp3`). `normalize_path_for_contents_match`'s trailing `-<digits>` dedup-suffix collapse checked `suffix.chars().all(is_ascii_digit)`, which is vacuously true for an *empty* suffix, so the trailing hyphen was silently stripped as if it were a real `-1`/`-2` dedup marker, producing a normalized path that matched nothing.
- **Fix:** stop the same check from false-flagging a track as missing/unindexed when its on-disk parent folder's casing has drifted from what's currently recorded in the PDB/eDB (e.g. `Artist Name` on disk vs `Artist name` indexed) — common on a long-lived library after an artist tag's casing gets corrected, since FAT32/exFAT doesn't rename an existing directory entry just because a later export used different casing for the same (case-insensitively) path. The file was always perfectly reachable on the case-insensitive filesystem; only the diagnostic's case-sensitive path-set comparison was wrong. Presence/coverage comparisons are now case-insensitive.

## 0.1.10

**Severity:** critical — see item(s) marked **(CRITICAL)** below.

- **Fix (CRITICAL):** stop USB-vs-local playback and materialization from
  silently diverging. Browsing a USB playlist/history that contained a song
  the user already had locally always created a second, disconnected
  `tracks` row instead of recognizing the existing one — which could let
  `resolve_playback_source` return the USB-mounted copy instead of the local
  disk copy (unnecessary USB read wear, and playback breaking entirely once
  the drive is unmounted), and could silently overwrite a correct local
  `waveform_peaks_path` with a USB-sourced one. Fixed by matching incoming
  USB tracks against existing local tracks by fingerprint (title/artist/album)
  plus a duration + exact file-size confidence gate before ever creating a
  placeholder row, by excluding any known USB-device root from playback
  resolution, and by giving every playback origin (library, playlist, USB,
  history) a `track_id` fast path so an already-saved playlist entry
  referencing a stale USB placeholder self-heals the next time it's played,
  with no migration required. A one-time startup pass also merges USB
  placeholder rows left behind by the old behavior back into their genuine
  local counterpart.
- **New feature:** add a `usb_devices` registry (recent USB roots with mount
  state, replacing the old localStorage-based recent-roots list) and a
  `usb_device_exports` table recording export history per device, queryable
  even when that exact drive isn't currently mounted.
- **Fix:** prevent switching the selected USB drive while a USB-scoped job
  (parity report, diagnostics, repair, playlist read/write, or export) is
  still running, so a slow response for the old drive can no longer land
  after the UI has already moved on to a different one and get rendered as
  if it belonged to the new drive.
- **Fix:** clear a playlist's "exported to USB" checkmark immediately when
  tracks are added to it after export, not just when tracks are removed.
  The backend already nulled the playlist's export-status fields on every
  add, but the frontend only refreshed the track list afterward and never
  updated its cached copy of that status, so the checkmark kept showing
  "exported" until an unrelated action happened to refetch the playlist
  list.
- **Fix:** stop the "Repair PDB Header Compatibility Field" diagnostic from
  perpetually re-flagging itself. It compared the PDB file header's
  compatibility byte against the most recent local backup snapshot and
  proposed patching toward that snapshot's value on any difference — but the
  writer always emits `5` on a fresh export while applying the repair patches
  to whatever the snapshot held (often `1`), so every export/repair cycle
  flipped the value and re-triggered the opposite-direction "fix" against the
  next snapshot, never converging. Both `1` and `5` are confirmed accepted by
  every tested validator, so a differing backup snapshot is no longer treated
  as an issue; only a genuinely unrecognized value is repaired now.
- **New feature:** add an "Export Tracklist" button to the USB History panel.
  Exports the selected history session's tracks as a plain `.txt` file
  (`Artist - Title` per line), with a choice of which track the list starts
  from and an option to include estimated cumulative times before or after
  each track. Times are always an estimate — summed from track length in
  play order starting from the chosen track — since CDJs never record a
  per-track playback timestamp in the USB, and often not even a reliable
  session date; this app estimates the latter from track metadata when the
  USB doesn't provide one directly.
- **Chore:** cargo fmt,clippy,update

## 0.1.9

**Severity:** critical

- Fix history import and diagnostics treating rekordbox's own blank
  template history-menu rows as real history. Every fresh or never-played
  rekordbox export ships a fixed block of empty, unnamed history-playlist
  placeholder rows; a fallback path added while hardening history import
  against hardware-recorded history started surfacing these as a batch of
  phantom "History N" playlists with fabricated track entries, and
  diagnostics reported misleading history counts for drives that had never
  actually recorded any DJ history. Only rekordbox-recorded sessions (named
  `HISTORY NNN`) are now surfaced as history.
- Remove frontend heuristic local-file matching from USB/imported track
  playback. Playback now substitutes an HDD/library file only when backend
  `resolve_playback_source` returns a verified match; if no verified HDD match
  exists, playback can still use the selected USB source path.
- Add test coverage for the USB repair engine (`service::repair`), raising its
  line coverage from ~63% to ~85%. Most of the gap was in the individual PDB
  byte-level repair functions (wrong page flags, sentinel u5, zero tranrf,
  wrong track/history/playlist-tree footer shape, stale sentinel B-tree,
  tombstoned row ids, EC data-page conflicts) and in the repair orchestrator's
  per-fix proposal/apply/skip wiring, none of which were previously exercised
  because no existing test constructed the specific corrupted-page byte
  patterns each one detects. No behavior change — tests only.
- Unify all backend logging into the Event Log. Previously there were four
  disconnected paths: per-command warning lists classified after the fact by
  four independently-hand-rolled guessers (two of which, in the diagnostics
  and repair modules, disagreed with each other on identical conditions — the
  same corruption could show up as `warn` from one command and `error` from
  another); a separate live-event path used by only two files; a handful of
  `eprintln!`-only messages that never reached the UI at all (notably
  `scan_master_db`'s ANLZ/artwork-miss diagnostics); and command failures,
  which never reached the Event Log at all. Every message now states its own
  level/code at the point it's created and is emitted through one function
  that both returns it in the command's response and pushes it live — so a
  failed command, a repair finding, or an import warning all land in the
  Event Log the same way, immediately. `error.rs` also gained a distinct
  `DbError` code (previously folded into the generic `InternalError`).
- Fix USB status/hint text rendering `[object Object]` and the "Auto analysis
  limit reached" notice silently disappearing after the Event Log unification
  above changed several commands' `warnings` field from plain strings to
  structured entries (`{level, code, message, source}`). `validate_usb_root`,
  `analyze_new_tracks`, and `scan_master_db` warning consumers in the frontend
  now read the `message` field instead of stringifying or re-joining the
  whole entry.

## 0.1.8

- Fix the play/pause transport button needing multiple clicks — sometimes to
  stop a playing track, sometimes to switch to a different track, sometimes
  silently doing nothing at all. Root cause was a single non-keyed "is a
  request in flight" latch: while one play/stop request was still resolving,
  the next click for a _different_ track was silently dropped instead of
  either queuing or superseding it. Play/stop now use a generation counter and
  a synchronous pending-intent so a new click always immediately reflects the
  user's latest intent, and a small backend-call queue guarantees whichever
  click was truly last is the one that wins. Also fixed: a global "click away
  from the playing row stops playback" listener that fired _before_ the
  clicked button's own play/stop logic on every track switch, effectively
  turning every switch into two competing backend commands instead of one; a
  dead-code fast path meant every single play — even a track already known
  locally — paid for an unnecessary database round-trip; and a stale
  "still playing" snapshot from a backend status-poll thread could arrive
  after an explicit stop and silently revive the "playing" state.
- Remove the backend's 250ms playback-status poll thread entirely. It existed
  to detect a track finishing on its own and to push waveform progress
  updates, but pushed events over the same channel as direct play/stop
  replies with no ordering guarantee between the two — the source of the
  stale-snapshot bug above. Natural end-of-track detection now happens inside
  the same serialized thread that already handles explicit play/stop
  commands (via a conditional receive timeout), so it can't race them by
  construction; the frontend now computes the waveform playhead position
  itself from a wall-clock timer instead of depending on a stream of pushed
  progress ticks.
- Fix seeking/scrubbing to a point in a track being slow — sometimes slow
  enough to hit a 5-second internal timeout and fail outright — because the
  backend's fallback seek implementation decoded and discarded every sample
  from the start of the track up to the target position rather than actually
  seeking. Playback now decodes and seeks through a small custom audio source
  built directly on the same `symphonia` decoding library already bundled for
  format support, replacing reliance on the `rodio` crate's own decoder (whose
  seek implementation couldn't report a stream length to the FLAC/MP3 seek
  logic, silently forcing the slow fallback for compressed formats). Seeking
  to any point in an MP3, FLAC, WAV, AAC/M4A, MP4, or Opus/OGG file is now
  effectively instant regardless of file length or seek position. `rodio` is
  still used for actual audio output (device I/O, buffering, mixing) — only
  its decoding path was replaced.
- Fix playback errors/warnings (e.g. a failed play or an unsupported file)
  only ever flashing briefly in the status bar with no way to review them
  afterward. Any status update tagged as a warning or error now also persists
  to the Event Log, the same as backend-reported issues already did.
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
  end — any commit interval tied to a track _count_, even a small one, can
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
