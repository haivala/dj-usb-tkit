# CDJ Hardware Test Matrix

This matrix tracks compatibility validation results captured on real CDJ/XDJ hardware.

Only hardware-validated outcomes belong in this file. Automated tests and parity
reports are useful gates, but they are not substitutes for these rows.

## Status values

- `pass`: scenario works end-to-end on tested hardware.
- `warn`: scenario is usable but has caveats.
- `fail`: scenario does not work as required.

## Matrix

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
| CDJ-2000NXS2 | 1.82 | <=0.1.10 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding | fail | hardware | 2026-08-06 | maintainer | Symptoms: COMM ERROR / freeze when browsing into Albums for an album containing a non-ASCII track title or filename; playlist/history views containing only some of that album's tracks could still work depending on which specific tracks they included.<br>Reproduction: 1) export a library containing a track whose title or filename needs UTF-16 encoding (any non-ASCII character); 2) insert USB into player; 3) browse Albums into the album containing that track (or otherwise cause the player to read that track's title/filename); 4) player comm-errors or freezes.<br>Context: reproduced on a 52-track library with one track title containing a single U+2019 curly apostrophe, and independently on a much larger deliberately pathological-Unicode "torture test" album (heavy combining marks / mixed scripts per track). Explicitly reproduced on released versions 0.1.2 and 0.1.10; every version before 0.1.11 shares the same unfixed writer code path.<br>Artifacts: `dump_pdb_track_debug` showing the affected track's title (slot 17) at row-relative offset 274 (`274 % 4 == 2`) and filename (slot 19) at offset 303 (`303 % 4 == 3`), both carrying the `0x90` UTF-16 marker — `encode_track_row_with_profile` only padded slot 20 (media path) to a 4-byte boundary, not the other 20 track string slots.<br>Validation questions: none outstanding — fixed and hardware-confirmed working on the same devices, see the `pass` rows below. |
| CDJ-2000NXS | 1.44 | <=0.1.10 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding | fail | hardware | 2026-08-06 | maintainer | Same fixture, root cause, and fix as the CDJ-2000NXS2 row above. |
| CDJ-2000NXS2 | 1.82 | 0.1.11 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-08-07 | maintainer | Previously comm-errored/froze in Albums browse on a library containing pathological non-ASCII track titles/filenames (traced to unaligned UTF-16 string slots 17/19; slot 20 alone was previously padded). Re-exported after removing the affected tracks so they were re-encoded through the fixed writer (additive export's semantic diff does not rewrite unchanged content, so already-exported tracks needed to be removed and re-added). Confirmed working after re-export. |
| CDJ-2000NXS | 1.44 | 0.1.11 | `non-ascii-track-string-alignment` | USB insert, database mount, Albums browse into a track whose title/filename require UTF-16 encoding, track listing, track load, playback start | pass | hardware | 2026-08-07 | maintainer | Same fixture and fix as the CDJ-2000NXS2 row above. |

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

Every `warn` or `fail` entry must include a detail block with:

```text
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
