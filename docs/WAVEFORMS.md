# Waveforms and ANLZ

## How it works

Waveform and ANLZ data are generated from local audio. USB metadata is not a waveform truth source.

The app stores a small waveform preview for UI use, but player ANLZ files need higher-resolution detail chunks for the CDJ detailed waveform view.

## Resolution Rules

- UI preview payloads are downsampled to `WAVEFORM_PREVIEW_BINS` (`2400`) before being returned to the frontend.
- Local ANLZ cache generation uses detail-resolution waveform data: `max(2400, ceil(duration_seconds * 150) + 4)`.
- USB export does not generate ANLZ; it copies previously generated `DAT/EXT/2EX` bundles.
- Normal USB export requires waveform path, BPM, duration, and existing `DAT/EXT/2EX` files before media copy starts.
- ANLZ detail chunk entry counts are `ceil(duration_seconds * 150) + 4`, with a minimum of `400`.
- Preview chunks stay fixed-size:
  - `PWAV` = 400 entries in `.DAT`
  - `PWV2` = 100 entries in `.DAT`
  - `PWV4` = 1200 entries in `.EXT`
  - `PWV6` = 1200 entries in `.2EX`
- Detail chunks use duration-derived entry counts:
  - `PWV3` = mono detail in `.EXT`
  - `PWV5` = color detail in `.EXT`
  - `PWV7` = 3-band detail in `.2EX`

For example, a 180 second track needs `27004` detail entries (`ceil(180 * 150) + 4`).

## Export Cache Policy

USB export does not generate or regenerate ANLZ from source audio. Export is a copy/linking step:

- if a track has a local or USB-side `DAT/EXT/2EX` bundle, export uses that bundle even if the
  contents are older or low-detail,
- local analysis cache bundles are generated without a `PPTH` path chunk,
- export injects or replaces the `PPTH` path chunk when the file is structurally parseable,
- if the bundle files are missing, export is blocked before media copy starts.

Run track analysis before export to create or refresh the local bundle. This keeps CPU-heavy audio
decoding out of the USB export path.

## Decode Bounds

Waveform generation decodes up to `24_000_000` mono samples during analysis. Duration for ANLZ
entry counts is resolved from track duration metadata first, then falls back to decoded sample count.

## Implementation Anchors

- Local analysis path: `backend/src/service/analysis.rs`
- USB export ANLZ path: `backend/src/service/export_helpers/export_paths.rs`
- ANLZ chunk writer: `backend/src/service/anlz.rs`

## Verification

- `cargo test -q --manifest-path backend/Cargo.toml waveform_detail`
- `cargo test -q --manifest-path backend/Cargo.toml persisted_waveform_preview`
- `cargo test -q --manifest-path backend/Cargo.toml export_analysis`
- `cargo test -q --manifest-path backend/Cargo.toml anlz_pipeline_with_generated_kick_pattern`
