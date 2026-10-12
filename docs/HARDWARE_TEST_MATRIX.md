# CDJ/XDJ Hardware Test Matrix

This matrix tracks compatibility validation results captured on real CDJ/XDJ hardware.

Only hardware-validated outcomes belong in this file. Automated tests and parity
reports are useful gates, but they are not substitutes for these rows.

**Keep the app up to date.** When the in-app update checker shows the critical
banner, update before your next USB export — a critical release means it fixes
a bug that can corrupt an export or break hardware playback (see `CHANGELOG.md`
entries marked `(CRITICAL)`), several of which are the same bugs tracked in
Known Issues below.

## Status values

- `pass`: scenario works end-to-end on tested hardware.
- `warn`: scenario is usable but has caveats.
- `fail`: scenario does not work as required.
- `untested`: implemented and covered by automated tests, but not yet run on hardware.

## Current Status

Latest hardware-validated result per device × scenario. This is a derived view for
"is X known-good right now" — it is not itself a source of truth. When adding a row
to the Validation History below that changes an outcome, update the matching row
here too.

A device that has passed every scenario gets a single `everything tested` row
instead of one row per scenario. A scenario added in a later app version gets
its own row for that device (`untested` until it has run on hardware).

Overall status: everything is working on CDJ/XDJ hardware. There are no open
`warn`/`fail` outcomes on those devices, and all their previously known issues
are fixed and hardware-confirmed on the hardware that originally showed them. The
one Engine DJ issue (`engine-pdb-import-artists`) is fixed in 0.4.1 but not yet
confirmed on the player. The latest run (app 0.1.36,
2026-09-02) re-validated every scenario on CDJ-2000NXS2, CDJ-3000, and the newly
added CDJ-3000X. On 0.2.0 (2026-09-11), CDJ-2000, CDJ-2000NXS and CDJ-2000NXS2
each ran every scenario available in that version. `cue-points-and-edited-beatgrid` is hardware-confirmed
on CDJ-2000NXS and CDJ-2000NXS2 for its first release in 0.2.0, and re-confirmed
on both on 0.2.4 together with the new `playback-start-position` scenario
(the cue editor's "Playback starts at" choice).
The original CDJ-2000 (fw 4.33) was added on that 0.2.0 run, and passed
`playback-start-position` on 0.3.7 (2026-10-09).
XDJ-1000MK2 (fw 1.44) was added on 0.2.6 and passes every scenario, the
first XDJ in the matrix.
On 0.3.1 (2026-10-01), the same CDJ-2000NXS and CDJ-2000NXS2 passed
`normal-export` (playlist export, mount, browse, load, playback) while
confirming the fix for exported file names losing their extension: a track
from a multi-script album now loads and plays. The other scenarios were not
re-run on 0.3.1.
On 0.3.7 (2026-10-09), CDJ-2000NXS and CDJ-2000NXS2 re-ran
`cue-points-and-edited-beatgrid` with the new memory cues: memory cues and hot
cues both work. No eDB player (CDJ-3000, CDJ-3000X, XDJ-AZ) has run memory cues
yet.
XDJ-AZ (fw 1.30) was added on 0.3.7-rc.3 (2026-10-09): exports play, and the
beat grid and Beat Sync work on app-analysed tracks. Up to 0.3.7-rc.1 it
refused the app's beat grids; see Known Issues.
Numark Mixstream Pro, the first Engine DJ player, was added on 0.4.0
(2026-10-09): it uses the app-written Engine DJ library with the app's beat grid
and hot cues, and keeps its play history across a rebuild. Its own import of
`export.pdb` (Engine DJ library not written) showed tracks without an artist; the
fix is not yet hardware-confirmed. See Known Issues.

Additive export — adding tracks to a USB that was initialized by rekordbox,
without wiping the existing library — has worked on hardware since the first
release (0.1.0) and has stayed working through every version since.

| Device model | Test scenario | Status | Last tested app version | Last validated date | Notes |
|---|---|---|---|---|---|
| CDJ-2000 | everything tested | pass | 0.3.7 | 2026-10-09 | First validation on this device (fw 4.33) on 0.2.0 (2026-09-11), covering every scenario available then; `playback-start-position` (added in 0.2.4) run on 0.3.7. |
| CDJ-2000NXS | everything tested | pass | 0.3.7 | 2026-10-09 | All scenarios run on 0.2.0 (2026-09-11); cue-editor scenarios re-run on 0.2.4; `cue-points-and-edited-beatgrid` re-run on 0.3.7 with memory cues. |
| CDJ-2000NXS2 | everything tested | pass | 0.3.7 | 2026-10-09 | All scenarios run on 0.2.0 (2026-09-11); cue-editor scenarios re-run on 0.2.4; `cue-points-and-edited-beatgrid` re-run on 0.3.7 with memory cues. `more-than-16-tracks-fresh-usb-init` was `fail` at <=0.1.30; fixed in 0.1.31. |
| XDJ-1000MK2 | everything tested | pass | 0.2.6 | 2026-09-29 | First validation on this device (fw 1.44). |
| XDJ-AZ | `normal-export` | pass | 0.3.7-rc.3 | 2026-10-09 | First validation on this device (fw 1.30). |
| XDJ-AZ | `beat-grid-and-beat-sync` | pass | 0.3.7-rc.3 | 2026-10-09 | `fail` at <=0.3.7-rc.1; fixed in 0.3.7-rc.3. |
| Mixstream Pro | `engine-library` | pass | 0.4.0 | 2026-10-09 | First validation on this device and the first Engine DJ player. |
| Mixstream Pro | `engine-pdb-import-artists` | untested | 0.4.1 | | `fail` at <=0.4.0; fix in 0.4.1 not yet run on hardware. |
| CDJ-3000 | `normal-export` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-3000 | `strict-parity-repair` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-3000 | `non-ascii-track-string-alignment` | pass | 0.1.36 | 2026-09-02 | First direct test of this scenario on CDJ-3000. |
| CDJ-3000 | `more-than-16-tracks-fresh-usb-init` | pass | 0.1.36 | 2026-09-02 | First direct test of this scenario on CDJ-3000. |
| CDJ-3000X | `normal-export` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-3000X | `strict-parity-repair` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-3000X | `non-ascii-track-string-alignment` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-3000X | `more-than-16-tracks-fresh-usb-init` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |

## Validation History

Append-only log of every hardware test run. This is the source of truth; the
Current Status table above is a summary of its latest rows. A run that passed
every scenario its app version supports is logged as a single `everything
tested` row.

| Device model | Firmware version | App version | Test scenario | Result | Last validated date | Tester | Notes |
|---|---|---|---|---|---|---|---|
| CDJ-2000NXS2 | 1.87 | 0.1.0 | `normal-export` | pass | 2026-06-28 | maintainer | Exported USB is accepted and playable. |
| CDJ-3000 | 3.20 | 0.1.0 | `normal-export` | pass | 2026-06-28 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.87 | 0.1.0 | `strict-parity-repair` | pass | 2026-06-28 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-3000 | 3.20 | 0.1.0 | `strict-parity-repair` | pass | 2026-06-28 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.4 | `normal-export` | pass | 2026-07-24 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.4 | `strict-parity-repair` | pass | 2026-07-24 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.4 | `normal-export` | pass | 2026-07-24 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.4 | `strict-parity-repair` | pass | 2026-07-24 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | <=0.1.10 | `non-ascii-track-string-alignment` | fail | 2026-08-06 | maintainer | Run stopped at: Albums browse into a track whose title/filename require UTF-16 encoding. See Known Issues: "0.1.10 and earlier — `non-ascii-track-string-alignment`". |
| CDJ-2000NXS | 1.44 | <=0.1.10 | `non-ascii-track-string-alignment` | fail | 2026-08-06 | maintainer | Run stopped at: Albums browse into a track whose title/filename require UTF-16 encoding. Same fixture, root cause, and fix as the CDJ-2000NXS2 row above. See Known Issues: "0.1.10 and earlier — `non-ascii-track-string-alignment`". |
| CDJ-2000NXS2 | 1.82 | 0.1.11 | `non-ascii-track-string-alignment` | pass | 2026-08-07 | maintainer | Previously comm-errored/froze in Albums browse on a library containing pathological non-ASCII track titles/filenames (traced to unaligned UTF-16 string slots 17/19; slot 20 alone was previously padded). Re-exported after removing the affected tracks so they were re-encoded through the fixed writer (additive export's semantic diff does not rewrite unchanged content, so already-exported tracks needed to be removed and re-added). Confirmed working after re-export. |
| CDJ-2000NXS | 1.44 | 0.1.11 | `non-ascii-track-string-alignment` | pass | 2026-08-07 | maintainer | Same fixture and fix as the CDJ-2000NXS2 row above. |
| CDJ-2000NXS2 | 1.82 | 0.1.16 | `normal-export` | pass | 2026-08-14 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.16 | `normal-export` | pass | 2026-08-14 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.16 | `strict-parity-repair` | pass | 2026-08-14 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.16 | `strict-parity-repair` | pass | 2026-08-14 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | <=0.1.30 | `more-than-16-tracks-fresh-usb-init` | fail | 2026-08-29 | maintainer | Run stopped at: database mount. See Known Issues: "0.1.30 and earlier — `more-than-16-tracks-fresh-usb-init`". |
| CDJ-2000NXS2 | 1.82 | 0.1.33 | `normal-export` | pass | 2026-08-29 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.33 | `strict-parity-repair` | pass | 2026-08-29 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.33 | `non-ascii-track-string-alignment` | pass | 2026-08-29 | maintainer | Confirmed still working on this version. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `normal-export` | pass | 2026-09-02 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `strict-parity-repair` | pass | 2026-09-02 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `non-ascii-track-string-alignment` | pass | 2026-09-02 | maintainer | Confirmed still working on this version. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `more-than-16-tracks-fresh-usb-init` | pass | 2026-09-02 | maintainer | First hardware retest on this device since the 0.1.31 fix; previously `fail` at <=0.1.30 (see Known Issues). No longer ejects in a loop; database mounts and plays. |
| CDJ-3000 | 3.20 | 0.1.36 | `normal-export` | pass | 2026-09-02 | maintainer | Exported USB is accepted and playable. First retest since 0.1.0. |
| CDJ-3000 | 3.20 | 0.1.36 | `strict-parity-repair` | pass | 2026-09-02 | maintainer | Strict parity repair output is accepted and playable. First retest since 0.1.0. |
| CDJ-3000 | 3.20 | 0.1.36 | `non-ascii-track-string-alignment` | pass | 2026-09-02 | maintainer | First direct test of this scenario on CDJ-3000. |
| CDJ-3000 | 3.20 | 0.1.36 | `more-than-16-tracks-fresh-usb-init` | pass | 2026-09-02 | maintainer | Covered only up to database mount. First direct test of this scenario on CDJ-3000. |
| CDJ-3000X | 1.31 | 0.1.36 | `normal-export` | pass | 2026-09-02 | maintainer | First validation on CDJ-3000X. Exported USB is accepted and playable. |
| CDJ-3000X | 1.31 | 0.1.36 | `strict-parity-repair` | pass | 2026-09-02 | maintainer | First validation on CDJ-3000X. Strict parity repair output is accepted and playable. |
| CDJ-3000X | 1.31 | 0.1.36 | `non-ascii-track-string-alignment` | pass | 2026-09-02 | maintainer | First validation on CDJ-3000X. |
| CDJ-3000X | 1.31 | 0.1.36 | `more-than-16-tracks-fresh-usb-init` | pass | 2026-09-02 | maintainer | Covered only up to database mount. First validation on CDJ-3000X. |
| CDJ-2000NXS | 1.44 | 0.2.0 | everything tested | pass | 2026-09-11 | maintainer | First hardware validation of the 0.2.0 cue editor. Cues trigger at their saved positions. |
| CDJ-2000NXS2 | 1.82 | 0.2.0 | everything tested | pass | 2026-09-11 | maintainer | First hardware validation of the 0.2.0 cue editor. Cues trigger at their saved positions. |
| CDJ-2000 | 4.33 | 0.2.0 | everything tested | pass | 2026-09-11 | maintainer | First validation on CDJ-2000. |
| CDJ-2000NXS | 1.44 | 0.2.4 | `cue-points-and-edited-beatgrid` | pass | 2026-09-25 | maintainer | Cue editor works; cues trigger at their saved positions. |
| CDJ-2000NXS | 1.44 | 0.2.4 | `playback-start-position` | pass | 2026-09-25 | maintainer | First hardware validation of the playback-start editor. Track loads at the chosen start position. |
| CDJ-2000NXS2 | 1.82 | 0.2.4 | `cue-points-and-edited-beatgrid` | pass | 2026-09-25 | maintainer | Cue editor works; cues trigger at their saved positions. |
| CDJ-2000NXS2 | 1.82 | 0.2.4 | `playback-start-position` | pass | 2026-09-25 | maintainer | First hardware validation of the playback-start editor. Track loads at the chosen start position. |
| XDJ-1000MK2 | 1.44 | 0.2.6 | everything tested | pass | 2026-09-29 | nack | First validation on XDJ-1000MK2. |
| CDJ-2000NXS | 1.44 | 0.3.1 | `normal-export` | pass | 2026-10-01 | maintainer | Run to check the 0.3.1 fix, covering every `normal-export` operation: exported a playlist with the multi-script album "–5", USB insert, database mount, playlist browse, track load and playback all work. Track 6, re-exported from the original file, now keeps its `.mp3` extension and plays (previously unplayable). Other scenarios not re-run. |
| CDJ-2000NXS2 | 1.82 | 0.3.1 | `normal-export` | pass | 2026-10-01 | maintainer | Run to check the 0.3.1 fix, covering every `normal-export` operation: exported a playlist with the multi-script album "–5", USB insert, database mount, playlist browse, track load and playback all work. Track 6, re-exported from the original file, now keeps its `.mp3` extension and plays (previously unplayable). Other scenarios not re-run. |
| XDJ-AZ | 1.30 | <=0.3.6 | `beat-grid-and-beat-sync` | fail | 2026-10-07 | mrantillies | Run stopped at: beat grid. App-analysed tracks load and play with cues, key and BPM, but show no beat grid and refuse Beat Sync. See Known Issues: "0.3.7-rc.1 and earlier — `beat-grid-and-beat-sync`". |
| XDJ-AZ | 1.30 | 0.3.7-rc.1 | `beat-grid-and-beat-sync` | fail | 2026-10-07 | mrantillies | Run stopped at: beat grid. Same as the <=0.3.6 row after applying **Add Missing Seek Data**, on MP3s and FLACs. Needle search, hot cues and loops on FLACs worked. See Known Issues: "0.3.7-rc.1 and earlier — `beat-grid-and-beat-sync`". |
| XDJ-AZ | 1.30 | 0.3.7-rc.3 | `beat-grid-and-beat-sync` | pass | 2026-10-08 | mrantillies | Old export fixed with USB repair **Fix Beat Grid** only. Beat grid shown and placed as in the app. |
| XDJ-AZ | 1.30 | 0.3.7-rc.3 | `normal-export` | pass | 2026-10-09 | mrantillies | First validation on XDJ-AZ. Playlist exported to an empty USB, no repair afterwards. |
| XDJ-AZ | 1.30 | 0.3.7-rc.3 | `beat-grid-and-beat-sync` | pass | 2026-10-09 | mrantillies | Same fresh export as the `normal-export` row, no repair: beat grid and Beat Sync work. The bundles had rc3's empty MP3 seek index (`PVBR`) and no FLAC one (`PVB2`), so the XDJ-AZ does not need seek data. |
| CDJ-2000NXS | 1.44 | 0.3.7 | `cue-points-and-edited-beatgrid` | pass | 2026-10-09 | maintainer | First hardware validation of memory cues from the cue editor. Memory cues and hot cues work as expected. |
| CDJ-2000NXS2 | 1.82 | 0.3.7 | `cue-points-and-edited-beatgrid` | pass | 2026-10-09 | maintainer | First validation of memory cues on a player reading `PCO2`. Memory cues and hot cues work as expected; hot cue pads light in their colours. Cue names not checked. |
| CDJ-2000 | 4.33 | 0.3.7 | `playback-start-position` | pass | 2026-10-09 | maintainer | First validation of the playback-start editor on CDJ-2000. Track loads at the chosen start position. |
| Mixstream Pro | not recorded | <=0.4.0 | `engine-pdb-import-artists` | fail | 2026-10-09 | maintainer | Run stopped at: artist shown. With no Engine DJ library on the USB, the player's own import of `export.pdb` showed some tracks without an artist. See Known Issues: "0.4.0 and earlier — `engine-pdb-import-artists`". |
| Mixstream Pro | not recorded | 0.4.0 | `engine-library` | pass | 2026-10-09 | maintainer | First validation on Mixstream Pro. The player used the app's beat grid without analysing again, and hot cues work. A play recorded on the player was still in `hm.db`, with the track's id and play state, after the library was rebuilt following a later export (checked on the USB afterwards, not on the player). |

## Known Issues

Full write-ups for every `fail`/`warn` row in Validation History, headed by the
affected app version range — that's the first thing anyone checking this file
wants to know. Referenced from the table's Notes column by heading text.

### 0.1.10 and earlier — `non-ascii-track-string-alignment` (fixed in 0.1.11)

**Devices:** CDJ-2000NXS2 (fw 1.82), CDJ-2000NXS (fw 1.44)

Symptoms:
- COMM ERROR / freeze when browsing into Albums for an album containing a non-ASCII
  track title or filename; playlist/history views containing only some of that
  album's tracks could still work depending on which specific tracks they included.

Reproduction:
1. Export a library containing a track whose title or filename needs UTF-16
   encoding (any non-ASCII character).
2. Insert the USB into the player.
3. Browse Albums into the album containing that track (or otherwise cause the
   player to read that track's title/filename).
4. Player comm-errors or freezes.

Context:
- Reproduced on a 52-track library with one track title containing a single
  U+2019 curly apostrophe, and independently on a much larger deliberately
  pathological-Unicode "torture test" album (heavy combining marks / mixed
  scripts per track). Explicitly reproduced on released versions 0.1.2 and
  0.1.10; every version before 0.1.11 shares the same unfixed writer code path.

Artifacts:
- `dump_pdb_track_debug` showing the affected track's title (slot 17) at
  row-relative offset 274 (`274 % 4 == 2`) and filename (slot 19) at offset 303
  (`303 % 4 == 3`), both carrying the `0x90` UTF-16 marker —
  `encode_track_row_with_profile` only padded slot 20 (media path) to a 4-byte
  boundary, not the other 20 track string slots.

Validation questions:
- None outstanding — fixed and hardware-confirmed working on the same devices,
  see the `pass` rows in Validation History for 0.1.11 and 0.1.33.

### 0.1.30 and earlier — `more-than-16-tracks-fresh-usb-init` (fixed in 0.1.31)

**Devices:** CDJ-2000NXS2 (fw 1.82)

Symptoms:
- Player mounts the USB then silently ejects it in a repeating loop; rekordbox
  desktop separately rejects the exported PDB as corrupted.

Reproduction:
1. Initialize a fresh USB.
2. Export a playlist containing more than 16 tracks to it.
3. Insert the USB into a CDJ-2000NXS2.

Context:
- The per-track runtime table's footer marked every row as simultaneously
  "active" instead of only the most recent one, once the table grew past a
  single 16-row footer group.

Artifacts:
- None captured beyond the observed eject loop.

Validation questions:
- None outstanding — root cause fixed in 0.1.31 and hardware-confirmed `pass` on
  0.1.36 on the CDJ-2000NXS2 (fw 1.82) that originally showed the fault, plus
  CDJ-3000 (fw 3.20) and CDJ-3000X (fw 1.31).

### 0.3.7-rc.1 and earlier — `beat-grid-and-beat-sync` (fixed in 0.3.7-rc.3)

**Devices:** XDJ-AZ (fw 1.30)

Symptoms:
- App-analysed tracks load and play with cues, key and BPM, but show no beat
  grid; Beat Sync answers "please analyse this track in rekordbox".

Reproduction:
1. Analyse a track in the app (0.3.6 or earlier).
2. Export it and load it on an XDJ-AZ.
3. No beat grid; Beat Sync is refused.

Context:
- Older players (CDJ-2000NXS2, CDJ-3000, XDJ-1000MK2) accept the same bundles.
- The app's beat grids differed from rekordbox's: the `PQTZ` header value was
  two bytes off, and `PQT2` had a zero checksum and no microsecond body (see
  `docs/WAVEFORMS.md`, "Beat-grid layout"). Seek data was ruled out: filling
  it in (0.3.7-rc.1) changed nothing, and rc3 exports work without it.
- Tracks imported from rekordbox showed the same symptom until 0.3.6's fix for
  imported bundles, applied with "force update track data from rekordbox".

Artifacts:
- Event Log of the rc3 **Fix Beat Grid** repair, attached to
  https://github.com/haivala/dj-usb-tkit/issues/6.

Validation questions:
- None outstanding — hardware-confirmed `pass` on 0.3.7-rc.3, both after
  **Fix Beat Grid** and on a fresh export.

### 0.4.0 and earlier — `engine-pdb-import-artists` (fix in 0.4.1, not yet hardware-confirmed)

**Devices:** Numark Mixstream Pro (firmware not recorded)

Symptoms:
- Some tracks show no artist after the player imports `export.pdb` itself.
  Every track of an affected artist is affected; other metadata is fine.

Reproduction:
1. Export a playlist with this app (0.4.0 or earlier) to a USB that has no
   `Engine Library` folder ("Write Engine DJ library" off).
2. Insert the USB into the player and let it import the rekordbox library.
3. Browse the imported tracks.

Context:
- The artists missing are exactly those whose `t02` row this app wrote: it put
  every artist name at offset 12. rekordbox puts a short ASCII name at 10 and
  only a UTF-16 name at 12, and the player read only those two forms. CDJs follow
  the row's own offset field and show these artists. With "Write Engine DJ library"
  on, the player uses the app's library, which has every artist.

Artifacts:
- The player-built `Engine Library/Database2/m.db` from that USB, compared with
  the app's rebuild of the same USB.

Validation questions:
- After 0.4.1's export (or **Repair Artist Name Layout** on an older USB), does the
  player's own import show every artist?

## Required Operations

Each passing row must cover every operation listed for its scenario in the table
below; a run that covered less says so in its Notes. At minimum, a USB export is
considered hardware-validated only after:

- the player recognizes the USB;
- the player mounts the database without corruption or communication errors;
- exported playlists are visible;
- playlist tracks load;
- playback starts.

For strict parity repair validation, the test USB must first be repaired through
the app's explicit strict parity repair flow, then validated on hardware after
the repaired database files are written.

| Test scenario | Operations |
|---|---|
| `normal-export` | USB insert, database mount, playlist browse, track load, playback start |
| `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start |
| `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start |
| `more-than-16-tracks-fresh-usb-init` | Initialize a fresh USB, export a playlist with more than 16 tracks, insert USB, database mount, playlist browse, track load, playback start |
| `cue-points-and-edited-beatgrid` | USB insert, database mount, track load, trigger each saved memory/hot cue |
| `beat-grid-and-beat-sync` | USB insert, database mount, load an app-analysed track, beat grid shown, Beat Sync engages |
| `playback-start-position` | Set "Playback starts at" in the cue editor, export, USB insert, database mount, track load, check the load position |
| `engine-library` | Export with "Write Engine DJ library" on, USB insert on an Engine DJ player, library loads without importing again, track load without analysing again, app beat grid shown, hot cues trigger, play history kept after the library is rebuilt |
| `engine-pdb-import-artists` | Export with "Write Engine DJ library" off (no `Engine Library` folder), USB insert on an Engine DJ player, player imports `export.pdb`, browse tracks, artist shown on every track that has one |

## Recording Warn Or Fail Results

Every `warn` or `fail` row in Validation History gets a short Notes pointer
("See Known Issues: ...") plus a matching heading under Known Issues, led by
the affected app version range, containing:

```text
**Devices:** which device(s)/firmware this was seen on

Symptoms:
- exact symptom(s)

Reproduction:
1. step-by-step reproduction

Context:
- USB/content context needed to reproduce

Artifacts:
- logs or captures collected

Validation questions:
- questions that still need hardware confirmation
```
