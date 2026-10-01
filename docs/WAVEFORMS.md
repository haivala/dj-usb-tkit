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

### Why `+ 4`

The `+ 4` is not a rounding fudge factor — it is a verified fixed tail pad that only
becomes a flat constant once the right "duration" is fed into the formula.

The duration used here is `detect_track_duration_ms` (`backend/src/service/analysis.rs`),
which comes from Symphonia's `codec_params.n_frames`: the MP3 header's declared total
frame count (`n_frames * samples_per_frame / sample_rate`). This is the **raw, encoder
delay/padding-unstripped** duration — it is *longer* than the gapless-trimmed playable
length that `ffmpeg`/`mutagen` report, because it does not subtract the LAME/Xing encoder
priming and padding samples.

This was verified against a real rekordbox-generated export covering many tracks of varied
duration and encoder settings: comparing rekordbox's actual on-disk `PWV3` entry counts to
`ceil(duration_seconds * 150)` computed from the stored integer `length` (seconds) field
gives deltas scattered across a wide range per track — clearly not a flat `+4`. But computed
from the raw MP3 frame count (total encoded frames × `1152` samples/frame ÷ sample rate,
i.e. what Symphonia's `n_frames` reflects), the delta is **exactly `+4` on every track** in
the sample.

## Export Cache Policy

USB export does not generate or regenerate ANLZ from source audio. Export is a copy/linking step:

- if a track has a local or USB-side `DAT/EXT/2EX` bundle, export uses that bundle even if the
  contents are older or low-detail,
- local analysis cache bundles are generated without a `PPTH` path chunk,
- export injects or replaces the `PPTH` path chunk when the file is structurally parseable,
- if the bundle files are missing, export is blocked before media copy starts.

Run track analysis before export to create or refresh the local bundle. This keeps CPU-heavy audio
decoding out of the USB export path.

## Cue and beat-grid chunks

`anlz.rs` writes **real** cue chunks when a track has edited cues (see
`docs/APP_DATA_MODEL.md` → TrackCue):

- `.DAT`: `PCOB`/`PCPT` (hot + memory)
- `.EXT`: `PCOB`/`PCPT` + `PCO2`/`PCP2` (hot + memory) — `PCP2` carries the
  hot-cue colour and comment.

A track with no cues still emits the historical **empty** `PCOB`/`PCO2`
placeholders byte-for-byte, so unedited exports are unchanged.

`save_track_analysis_edits` and USB export both apply
`apply_analysis_edits_to_anlz`, an in-place transform that rebuilds only
`PQTZ`/`PQT2` (from `first_beat_ms`) and the cue chunks, copying every other
chunk — including any `PSSI` phrase data rekordbox wrote — verbatim. This
app never writes `PSSI` itself: it doesn't analyze phrases. `read_cues_from_anlz` /
`read_first_beat_from_anlz` decode them back on USB re-import.

The beat grid also rebuilds from `bpm` **alone**, with no explicit
`first_beat_ms` and no cues: when a track's on-USB bundle already exists
(the export "retain" path, `ensure_analysis_bundle_ppth`), a positive
`track.bpm` is enough to trigger `apply_analysis_edits_to_anlz`, which reuses
whatever anchor is already embedded in the bundle (`read_first_beat_from_anlz`)
rather than requiring a fresh one. This closes a real failure mode: a track
re-analyzed to the correct BPM but with no confident first-beat detection
(`stratum-dsp` can return a tempo with an empty `beat_grid.beats`) used to
leave its *existing* beat grid untouched on every future export — including
one baked at a stale tempo by an earlier analysis. The ANLZ writer never
invents a tempo or a length: with no known BPM it writes no beat grid
(`PQTZ`/`PQT2`), and it takes the track length as a required input (the
detail waveform's entry count and the grid's beat count both come from it),
so a track with no known length gets no bundle. `fix_empty_analysis_files`
carries the track's real PDB `tempo_x100`/duration forward, measuring the
source file when the PDB has no length. PDB
and eDB tempo metadata could be completely correct while CDJ hardware still
read the stale beat-grid tempo for its live/master display — a normal
re-export now corrects it.

The track-detail modal renders the **PWV5 colour-detail** waveform from the
`.EXT`, read raw by `read_pwv5_from_anlz` (`usb_utils.rs`), **base64-encoded**
onto `get_track_detail`'s `detailWaveform` (the raw payload is tens of KB — a
JSON number array would be ~3× that on the wire). The modal decodes it and
renders only the visible `[startMs, endMs]` slice, so **scroll-to-zoom /
drag-to-pan** shows the full ~150 entries/sec detail; it opens zoomed to the
first 60 bars at the track's BPM. The magnifier button is disabled for tracks with no analysis
(no `.EXT` ⇒ no PWV5). See `vanilla-ui/components/track-detail/waveform_detail.mjs`.

## Seek-index chunks (`PVBR`, `PVB2`)

rekordbox writes two chunks that help a player find the byte position of a given time in
files whose bitrate varies. This app writes `PVBR` as all zeros and never writes `PVB2`.
Tracks play correctly this way, including FLACs on a CDJ-2000NXS2.

Our working guess (not verified) is that newer players use the audio file's own seek
data: the `SEEKTABLE` in a FLAC, or the `Xing` table of contents in a VBR MP3. Not every
file has one, though; in the libraries checked, about one FLAC in seven had no
`SEEKTABLE`. The test that would settle it: on the player, try cue jumps, needle search
and loops in files with no seek data anywhere (a FLAC without a `SEEKTABLE`, a VBR MP3
without a `Xing` header), exported by this app.
- If those land correctly, the player needs no index at all.
- If they are off or slow while files with their own seek data are fine, the guess holds.

Either way, only older players would be left as candidates for needing rekordbox's chunks.

We only write data that is identical to what rekordbox writes, so both stay empty until a
hardware test shows that some player needs them. A seek table can be computed from the
audio file, so the formats and rules below were worked out from rekordbox exports so the
decision can be revisited.

### `PVBR` (`.DAT`, MP3 seek index)

Layout: `len_header` 16, `len_tag` 1620. The body is 400 big-endian `u32` entries followed
by a `u32` total sample count.

What rekordbox writes:

| File | Entries | Total |
|---|---|---|
| CBR MP3 | all 0 | counted frames × 1152 |
| VBR MP3 | byte offsets (below) | counted frames × 1152 |
| FLAC / WAV / AIFF / ALAC | all 0 | 0 |
| some AAC `.m4a` | all 0 | the stream's sample count, for some files only |

- **VBR entries:** `entry[i]` is the byte offset of counted frame
  `max(floor((i + 1) · N / 400) − 8, 0)`. `N` is the number of counted frames, and the
  offset is measured from the first counted frame, not from the start of the file.
- **Counted frames** are the MPEG frames between any ID3v2 tag and an ID3v1 tag. Whether
  the first frame (a `Xing`/`Info` header) counts depends on the encoder string in that
  header:
  - `LAME…` or `iTunes…` → counted
  - `Lavc…` (ffmpeg) or mixed-case `Lame…` → **not** counted; offsets start at the second
    frame
  - no header → every frame counts
- **CBR vs VBR** is decided by whether the audio frames' bitrates vary.

Checked byte for byte against three rekordbox exports: a fresh export of a purpose-built
test set (encoders, bitrate modes, header types, tags, short clips, other formats) and two
real libraries built up over years. Every MP3 these rules cover matched, except one CBR
file that got a filled index for no reason we could find in the file.

rekordbox's behaviour is not predictable in these cases, so this app would keep `PVBR`
at zeros for them:

- **APE or Lyrics3 tags:** rekordbox appears to count those bytes as about one more frame
  in CBR files.
- **VBRI headers:** rekordbox skipped the header frame in some files and counted it in
  others.
- **Anything that doesn't parse cleanly:** lost sync, a truncated last frame, a
  `Xing` frame count that disagrees with the file, or an unknown encoder string.
- **MPEG-2 / 2.5:** one example used frames × 576.

### `PVB2` (`.EXT`, FLAC seek index)

Layout: `len_header` 32, `len_tag` 8032. The header after the common 12 bytes is:
- 4 zero bytes
- `u64` total samples, equal to `STREAMINFO`
- `u32` entry count
- `u32` 20 (entry size)

Each 20-byte entry is `u64` first sample of the frame, `u64` byte offset of the frame, and
`u32` samples in the frame (the block size, for example 1152, 4096 or 4608). The byte
offset is measured from the first audio frame, not the start of the file. rekordbox
writes it for every FLAC, with or without a `SEEKTABLE`. It isn't copied from the file's
own `SEEKTABLE`.

- **Tracks of 400 frames or fewer** get one entry per frame except the last.
- **Longer tracks** get 400 entries. Each entry is the last frame starting at or before
  position `i / 400` of a length slightly shorter than the real total: 0.3–8 ms shorter
  at 44.1 kHz.
  - That shortfall depends on the audio. The same audio in six different FLAC encodings
    gave the same shortfall, and different tracks gave different ones. A 48 kHz file
    matched the plain `i · total / 400` rule exactly.
  - This suggests rekordbox measures the length after its own resampling, which can't be
    reproduced exactly. The plain rule matches about 98% of entries; the others are one
    frame earlier in rekordbox.

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
