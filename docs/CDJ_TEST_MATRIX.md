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

## Current Status

Latest hardware-validated result per device × scenario. This is a derived view for
"is X known-good right now" — it is not itself a source of truth. When adding a row
to the Validation History below that changes an outcome, update the matching row
here too.

| Device model | Test scenario | Status | Last tested app version | Last validated date | Notes |
|---|---|---|---|---|---|
| CDJ-2000NXS2 | `normal-export` | pass | 0.1.33 | 2026-08-29 | |
| CDJ-2000NXS2 | `strict-parity-repair` | pass | 0.1.33 | 2026-08-29 | |
| CDJ-2000NXS2 | `non-ascii-track-string-alignment` | pass | 0.1.33 | 2026-08-29 | |
| CDJ-2000NXS2 | `more-than-16-tracks-fresh-usb-init` | fail (last direct test) | <=0.1.30 | 2026-08-29 | Fixed in 0.1.31 per `CHANGELOG.md`. |
| CDJ-2000NXS | `normal-export` | pass | 0.1.16 | 2026-08-14 | |
| CDJ-2000NXS | `strict-parity-repair` | pass | 0.1.16 | 2026-08-14 | |
| CDJ-2000NXS | `non-ascii-track-string-alignment` | pass | 0.1.11 | 2026-08-07 | Not retested since; app version has moved on but no regression reported. |
| CDJ-3000 | `normal-export` | pass (stale) | 0.1.0 | 2026-06-28 | No retest since the first release; many app versions have shipped since. |
| CDJ-3000 | `strict-parity-repair` | pass (stale) | 0.1.0 | 2026-06-28 | No retest since the first release; many app versions have shipped since. |

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
- Root cause fixed in 0.1.31 per `CHANGELOG.md`, but this specific scenario
  has not been independently hardware-retested since — the 0.1.33 `pass` rows
  in Validation History cover other scenarios only.

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
