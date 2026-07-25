# DJ USB Tkit vs. rekordbox

This page summarizes where DJ USB Tkit differs from Pioneer/AlphaTheta's
rekordbox for the specific job both tools do: getting a library onto a USB
drive a CDJ/XDJ can play, and keeping that USB drive healthy.

This is not a full feature comparison — rekordbox is a much larger application
(streaming integrations, DVS, video, Lighting, cloud library sync, and more).
The comparison below is scoped to library management, USB export, and USB
database health, which is what this project actually implements.

## Summary

| | DJ USB Tkit | rekordbox |
| --- | --- | --- |
| Platform | Windows, macOS, Linux | Windows, macOS only ([no official Linux build](../docs/USB_IMPORT.md)) |
| License | Open source (MIT) | Closed source, proprietary |
| Runs fully offline | Yes — local-first, no account or cloud dependency required | Has optional cloud/account-linked features |
| USB export sync control | Explicit `mirror` or `additive` mode per export (see below) | Playlist re-export behavior is not user-selectable the same way |
| USB diagnostics | Dedicated operational + strict-parity reports, read-only | Not exposed as a standalone diagnostic tool |
| USB repair | Preview-first repair catalog for a defined set of PDB/eDB structural issues, hardware-validated | No equivalent user-facing repair tooling |
| Pre-export backups | Automatic timestamped PDB/eDB backups before every export and repair write | Not automatic in the same way |
| Analysis scope | Analyze only the tracks a target playlist needs, on demand | Typically analyzes more broadly as tracks are added |

## Why this exists

rekordbox owns the reference database format (PDB/eDB), and DJs are stuck
with it if they want to play off USB on Pioneer hardware. But rekordbox
itself is a large, closed-source, Windows/macOS-only application, and the
parts of it that matter for USB prep — indexing, analysis, export, and
recovering a broken USB — are not independently inspectable, scriptable, or
fixable when something goes wrong. DJ USB Tkit re-implements just that slice,
openly, and adds tooling rekordbox doesn't expose to users at all.

## Cross-platform, including Linux

rekordbox has no official Linux release; DJ USB Tkit builds and runs on
Linux (Arch, Ubuntu/Debian, Fedora — see the main [README](../README.md)),
in addition to Windows and macOS.

## Open source

The full application — indexing, analysis, PDB/eDB writers, diagnostics, and
repair logic — is MIT-licensed and readable. See [LICENSE](../LICENSE) and
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Export you can control: mirror vs. additive

Every export chooses one of two explicit sync modes
(see [`docs/USB_EXPORT.md`](USB_EXPORT.md)):

- **`mirror`** — the USB playlist is replaced to exactly match the current
  local playlist.
- **`additive`** — new local tracks are appended to the USB playlist without
  touching or removing existing USB-side members.

This makes it possible to build up a playlist directly on a USB stick from
multiple sessions/machines without accidentally wiping prior additions, or to
force a clean resync when that's what's wanted instead.

## Diagnostics and repair for broken USB databases

DJ USB Tkit ships dedicated, read-only diagnostics
(`run_usb_diagnostics`, `run_usb_parity_report`) and a separate,
preview-first repair flow for a defined catalog of PDB/eDB structural
problems — corrupted page footers, tombstoned rows, stale sentinel B-trees,
playlist parity mismatches between PDB and eDB, and more. See
[`docs/DIAGNOSTICS_REPAIRS.md`](DIAGNOSTICS_REPAIRS.md) for the full repair
catalog. Repairs default to preview mode (nothing is written until you
apply), and repaired output has been validated on real CDJ-2000NXS,
CDJ-2000NXS2, and CDJ-3000 hardware (see
[`docs/CDJ_TEST_MATRIX.md`](CDJ_TEST_MATRIX.md)). rekordbox does not expose
an equivalent repair surface for a broken export database — the common
workaround is deleting the `PIONEER` folder and re-exporting from scratch,
which loses history and any USB-only playlist state.

`run_usb_parity_report` is the *strict* check in that catalog: it verifies
that PDB (the binary database older/PDB-primary units rely on) and eDB (the
newer relational database that newer units also read from) tell an
identical story, not just that the USB works on the one player you tested.
Different CDJ generations and firmware versions lean on PDB and eDB to
different degrees — a USB can play fine on the unit in front of you while
still disagreeing between PDB and eDB on playlist membership/order, track
metadata, dictionary IDs, or artwork, and surface that disagreement as a
problem on a different player. Strict parity checks both surfaces
field-by-field so the export is consistent across the CDJ lineup, not just
whichever hardware happened to be used for validation. rekordbox's own
reference exporter produces this consistency by construction; DJ USB Tkit
verifies and repairs it explicitly since it's rebuilding both database
writers independently.

## Automatic backups before writes

Before every export and every applied repair, existing PDB/eDB files are
copied to a timestamped backup under `PIONEER/rekordbox/backups/` on the
drive itself, so a prior database state can potentially be restored. See the
Backup section of [`docs/USB_EXPORT.md`](USB_EXPORT.md).

## Hardware bugs found and worked around

Two CDJ firmware hang conditions were found through real hardware testing and
are actively defended against on export: track/playlist text with excessive
stacked Unicode combining marks ("zalgo" text), and text mixing too many
distinct Unicode scripts in a short string. Both have been observed to hang
CDJ browse/track-load screens; DJ USB Tkit caps and sanitizes text at export
time to avoid producing them. See the sanitizer notes in
[`docs/USB_EXPORT.md`](USB_EXPORT.md).

## Fast prep flow

Library scanning (indexing) and analysis (BPM/key/waveform/artwork) are
separate stages. Tracks are usable in playlists as soon as they're indexed;
analysis can then be run only for the tracks a specific target playlist
actually needs, instead of requiring broader up-front analysis. See
[`docs/LIBRARY_ANALYSIS.md`](LIBRARY_ANALYSIS.md).

## Tolerant USB import

Import reads PDB, eDB, and (optionally) rekordbox's own local `master.db` as
fallback sources, resolves conflicts across them, and returns partial-but-usable
data with warnings when metadata is incomplete or corrupted rather than
failing the whole import. See [`docs/USB_IMPORT.md`](USB_IMPORT.md).
