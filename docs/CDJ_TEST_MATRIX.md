# CDJ Hardware Test Matrix

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

Overall status: everything is working. There are no open `warn`/`fail` outcomes
on any device, and all previously known issues are fixed and hardware-confirmed
on the hardware that originally showed them. The latest run (app 0.1.36,
2026-09-02) re-validated every scenario on CDJ-2000NXS2, CDJ-3000, and the newly
added CDJ-3000X; CDJ-2000NXS remains green on its last-tested versions with no
regression reported. `cue-points-and-edited-beatgrid` initially failed on
CDJ-2000NX and CDJ-2000NXS2 (2026-09-11) and is now hardware-confirmed fixed
on both — see Known Issues: "Unreleased — `cue-points-wrong-seek-position`".

Additive export — adding tracks to a USB that was initialized by rekordbox,
without wiping the existing library — has worked on hardware since the first
release (0.1.0) and has stayed working through every version since.

| Device model | Test scenario | Status | Last tested app version | Last validated date | Notes |
|---|---|---|---|---|---|
| CDJ-2000NXS2 | `normal-export` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-2000NXS2 | `strict-parity-repair` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-2000NXS2 | `non-ascii-track-string-alignment` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-2000NXS2 | `more-than-16-tracks-fresh-usb-init` | pass | 0.1.36 | 2026-09-02 | Was `fail` at <=0.1.30; fixed in 0.1.31, hardware-confirmed on 0.1.36. |
| CDJ-2000NXS | `normal-export` | pass | 0.1.16 | 2026-08-14 | |
| CDJ-2000NXS | `strict-parity-repair` | pass | 0.1.16 | 2026-08-14 | |
| CDJ-2000NXS | `non-ascii-track-string-alignment` | pass | 0.1.11 | 2026-08-07 | Not retested since; app version has moved on but no regression reported. |
| CDJ-3000 | `normal-export` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-3000 | `strict-parity-repair` | pass | 0.1.36 | 2026-09-02 | |
| CDJ-3000 | `non-ascii-track-string-alignment` | pass | 0.1.36 | 2026-09-02 | First direct test of this scenario on CDJ-3000. |
| CDJ-3000 | `more-than-16-tracks-fresh-usb-init` | pass | 0.1.36 | 2026-09-02 | First direct test of this scenario on CDJ-3000. |
| CDJ-3000X | `normal-export` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-3000X | `strict-parity-repair` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-3000X | `non-ascii-track-string-alignment` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-3000X | `more-than-16-tracks-fresh-usb-init` | pass | 0.1.36 | 2026-09-02 | First validation on this device (fw 1.31). |
| CDJ-2000NX | `cue-points-and-edited-beatgrid` | pass | Unreleased | 2026-09-11 | Was `fail` before the ANLZ cue-encoder byte-layout fix in this version — see `cue-points-wrong-seek-position` below. |
| CDJ-2000NXS2 | `cue-points-and-edited-beatgrid` | pass | Unreleased | 2026-09-11 | Was `fail` before the ANLZ cue-encoder byte-layout fix in this version — see `cue-points-wrong-seek-position` below. |

## Validation History

Append-only log of every hardware test run. This is the source of truth; the
Current Status table above is a summary of its latest rows.

| Device model | Firmware version | App version | Test scenario | Operations tested | Result | Validation source | Last validated date | Tester | Notes |
|---|---|---|---|---|---|---|---|---|---|
| CDJ-2000NXS2 | 1.87 | 0.1.0 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-06-28 | maintainer | Exported USB is accepted and playable. |
| CDJ-3000 | 3.20 | 0.1.0 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-06-28 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.87 | 0.1.0 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-06-28 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-3000 | 3.20 | 0.1.0 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-06-28 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.4 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-07-24 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.4 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-07-24 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.4 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-07-24 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.4 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-07-24 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | <=0.1.10 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding | fail | hardware | 2026-08-06 | maintainer | See Known Issues: "0.1.10 and earlier — `non-ascii-track-string-alignment`". |
| CDJ-2000NXS | 1.44 | <=0.1.10 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding | fail | hardware | 2026-08-06 | maintainer | Same fixture, root cause, and fix as the CDJ-2000NXS2 row above. See Known Issues: "0.1.10 and earlier — `non-ascii-track-string-alignment`". |
| CDJ-2000NXS2 | 1.82 | 0.1.11 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-08-07 | maintainer | Previously comm-errored/froze in Albums browse on a library containing pathological non-ASCII track titles/filenames (traced to unaligned UTF-16 string slots 17/19; slot 20 alone was previously padded). Re-exported after removing the affected tracks so they were re-encoded through the fixed writer (additive export's semantic diff does not rewrite unchanged content, so already-exported tracks needed to be removed and re-added). Confirmed working after re-export. |
| CDJ-2000NXS | 1.44 | 0.1.11 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-08-07 | maintainer | Same fixture and fix as the CDJ-2000NXS2 row above. |
| CDJ-2000NXS2 | 1.82 | 0.1.16 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-08-14 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.16 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-08-14 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.16 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-08-14 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS | 1.44 | 0.1.16 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-08-14 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | <=0.1.30 | `more-than-16-tracks-fresh-usb-init` | Initialize a fresh USB, export a playlist with more than 16 tracks, insert USB, database mount | fail | hardware | 2026-08-29 | maintainer | See Known Issues: "0.1.30 and earlier — `more-than-16-tracks-fresh-usb-init`". |
| CDJ-2000NXS2 | 1.82 | 0.1.33 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-08-29 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.33 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-08-29 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.33 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-08-29 | maintainer | Confirmed still working on this version. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | Exported USB is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | Strict parity repair output is accepted and playable. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-09-02 | maintainer | Confirmed still working on this version. |
| CDJ-2000NXS2 | 1.82 | 0.1.36 | `more-than-16-tracks-fresh-usb-init` | Initialize a fresh USB, export a playlist with more than 16 tracks, insert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | First hardware retest on this device since the 0.1.31 fix; previously `fail` at <=0.1.30 (see Known Issues). No longer ejects in a loop; database mounts and plays. |
| CDJ-3000 | 3.20 | 0.1.36 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | Exported USB is accepted and playable. First retest since 0.1.0. |
| CDJ-3000 | 3.20 | 0.1.36 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | Strict parity repair output is accepted and playable. First retest since 0.1.0. |
| CDJ-3000 | 3.20 | 0.1.36 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-09-02 | maintainer | First direct test of this scenario on CDJ-3000. |
| CDJ-3000 | 3.20 | 0.1.36 | `more-than-16-tracks-fresh-usb-init` | Initialize a fresh USB, export a playlist with more than 16 tracks, insert USB, database mount | pass | hardware | 2026-09-02 | maintainer | First direct test of this scenario on CDJ-3000. |
| CDJ-3000X | 1.31 | 0.1.36 | `normal-export` | USB insert, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | First validation on CDJ-3000X. Exported USB is accepted and playable. |
| CDJ-3000X | 1.31 | 0.1.36 | `strict-parity-repair` | Apply strict parity repair, reinsert USB, database mount, playlist browse, track load, playback start | pass | hardware | 2026-09-02 | maintainer | First validation on CDJ-3000X. Strict parity repair output is accepted and playable. |
| CDJ-3000X | 1.31 | 0.1.36 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-09-02 | maintainer | First validation on CDJ-3000X. |
| CDJ-3000X | 1.31 | 0.1.36 | `more-than-16-tracks-fresh-usb-init` | Initialize a fresh USB, export a playlist with more than 16 tracks, insert USB, database mount | pass | hardware | 2026-09-02 | maintainer | First validation on CDJ-3000X. |
| CDJ-2000NX | unspecified | Unreleased | `cue-points-and-edited-beatgrid` | USB insert, database mount, track load, trigger each saved memory/hot cue | fail | hardware | 2026-09-11 | maintainer | Every cue jumped to roughly the same spot near track start instead of its saved position. See Known Issues: "Unreleased — `cue-points-wrong-seek-position`". |
| CDJ-2000NXS2 | unspecified | Unreleased | `cue-points-and-edited-beatgrid` | USB insert, database mount, track load, trigger each saved memory/hot cue | fail | hardware | 2026-09-11 | maintainer | Same symptom as the CDJ-2000NX row above. See Known Issues: "Unreleased — `cue-points-wrong-seek-position`". |
| CDJ-2000NX | unspecified | Unreleased | `cue-points-and-edited-beatgrid` | USB insert, database mount, track load, trigger each saved memory/hot cue | pass | hardware | 2026-09-11 | maintainer | Re-tested after the ANLZ cue-encoder byte-layout fix (re-saved the track's cues, no re-analysis). Cues now trigger at their saved positions. |
| CDJ-2000NXS2 | unspecified | Unreleased | `cue-points-and-edited-beatgrid` | USB insert, database mount, track load, trigger each saved memory/hot cue | pass | hardware | 2026-09-11 | maintainer | Same fix and retest as the CDJ-2000NX row above. |

## Known Issues

Full write-ups for every `fail`/`warn` row in Validation History, headed by the
affected app version range — that's the first thing anyone checking this file
wants to know. Referenced from the table's Notes column by heading text.

### Unreleased — `cue-points-wrong-seek-position` (fixed same cycle)

**Devices:** CDJ-2000NX, CDJ-2000NXS2 (firmware not recorded)

Symptoms:
- Every saved cue point (memory point or hot-cue pad) jumped playback to
  roughly the same position near the start of the track instead of its own
  saved position, on every cue tested. The app's own cue editor showed the
  correct times for the same track — this was purely an on-device playback
  bug, not a data-storage or UI bug.

Reproduction:
1. Save two or more cue points at distinct positions on a track (via the
   local cue editor or the USB-native cue editor).
2. Export (or, for a USB-native edit, save in place) to a USB stick.
3. Insert into a CDJ-2000NX or CDJ-2000NXS2, load the track, trigger each
   memory/hot cue.
4. Every cue lands at (approximately) the same spot near track start.

Context:
- The eDB `cue` table's ten seek-anchor columns (`inMpegFrameNumber`,
  `inDecodingStartFramePosition`, `inFileOffsetInBlock`, etc.) were initially
  suspected, since this app leaves them `NULL`. Ruled out: a genuine
  Rekordbox-exported reference USB (with real hot cues on real tracks)
  showed the `cue` table has **zero rows** even for tracks with 8 real hot
  cues — real Rekordbox doesn't populate that table for on-device playback
  either, so CDJs must read cue positions from the ANLZ files alone.
- The actual bug: byte-for-byte diffing this app's own exported
  `ANLZ0000.DAT`/`.EXT` cue chunks (`PCOB`/`PCPT` basic, `PCO2`/`PCP2`
  extended) against the genuine Rekordbox reference found this app's
  encoder was writing wrong values in three places every real Rekordbox
  export gets consistently right: the `unknown1` constant in `PCPT`
  (`0x00100000` instead of `0x00010000`), the `status` byte (`1` for hot
  cues instead of always `0`), and a reserved 3-byte field immediately after
  the `kind`/`type` byte in both `PCPT` and `PCP2` (left `0` instead of the
  constant `0x0003E8`). All three were confirmed wrong on every one of 13
  real cue entries sampled across two genuine Rekordbox tracks. A firmware
  parser sanity-checking any of these fixed/magic fields and rejecting a
  non-matching entry (falling back to a default position) fits the observed
  symptom exactly.

Artifacts:
- Fixed in `backend/src/service/anlz.rs` (`build_pcpt_entry`,
  `build_pcp2_entry`, `append_pcob_chunk`) — see the CHANGELOG entry in this
  same Unreleased cycle.

Validation questions:
- None outstanding for the fields above — hardware-confirmed fixed on both
  devices. Not yet verified: the `.DAT` file's basic `PCOB` caps hot cues at
  3 (slots 4-8 go into the `.EXT`'s basic `PCOB` instead) in genuine
  Rekordbox output; this app still writes the full list (up to 8) into both
  files. Untested because the reproduction track only had 2 hot cues — worth
  a follow-up hardware pass with a track that has 4+ hot cues.

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

## Required Operations

Each passing row must cover the operations listed in the row. At minimum, a USB
export is considered hardware-validated only after:

- the player recognizes the USB;
- the player mounts the database without corruption or communication errors;
- exported playlists are visible;
- playlist tracks load;
- playback starts.

For strict parity repair validation, the test USB must first be repaired through
the app's explicit strict parity repair flow, then validated on hardware after
the repaired database files are written.

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
