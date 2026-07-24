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
