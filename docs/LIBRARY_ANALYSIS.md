# Library + Analysis

## How it works

Library management starts with source folders selected in the desktop UI. Scanning is intentionally split into two stages so the library becomes usable quickly: indexing first, then analysis. Indexing only walks the folders and adds the audio files to the library, so it is quick; analysis is slow and runs as a separate job the user can pause or cancel. They stay separate commands (`scan_library`, `analyze_new_tracks`), and the frontend runs one after the other. Adding a source folder, or relocating a missing one, indexes it in the same command (`add_source_folders`, `relocate_source_root`), so its tracks can go into playlists right away; analysis still waits for the user.

The primary workflow is fast playlist preparation, not full-library pre-analysis. Users can add tracks to playlists immediately after indexing, then run analysis only for tracks needed for the current export target.

After indexing, analysis runs in batches and progressively fills waveform, BPM, key, and artwork-related fields. The UI updates rows as each track is processed instead of waiting for one end-of-job refresh.

If analysis encounters unsupported files or decode failures, the app reports command/job errors and continues processing the rest of the queue.

Scanning also flags a WAV-specific hardware compatibility issue: some Pioneer CDJs reject WAV files whose `fmt ` chunk uses the `WAVE_FORMAT_EXTENSIBLE` tag, even when the wrapped audio (sample rate/bit depth) is otherwise within spec. `scan_library` detects this per-file (cheap header parse, no decode) and the UI shows it as a format-badge tooltip on the track row. When the extensible header wraps plain PCM/IEEE-float data, export automatically rewrites it to a standard header — see `docs/USB_EXPORT.md`. When it wraps some other subformat, it's shown as a hard warning instead, since there's nothing safe to rewrite.

Analysis engine selection is user-controlled from Settings. The default engine is `stratum`. The optional `essentia` engine can be enabled after runtime assets are installed, and analysis requests then use the selected engine for BPM/key detection.

### The analysis pipeline

Every analysis in the app, of a library track or of a file on a USB that a
repair draws, goes through the same steps. Nothing is guessed or copied from
elsewhere: each value comes from the track's own audio, unless it's one the
analysis keeps (step 1).

1. **What's kept.** Before anything is decoded, `kept_analysis_values`
   decides what the run leaves alone. A first analysis (the track has no
   waveform yet) keeps a BPM or key you set, or one imported from rekordbox
   or Mixxx (sources `user`, `rekordbox`, `mixxx`), and an edited or imported
   first beat (see "Analyze and Reanalyze" below). A reanalysis keeps
   nothing. With a BPM kept and no key wanted, nothing is detected: the audio
   is only decoded and the first beat placed.
2. **Decode, once.** `decode_audio_with_waveform` reads the file one time,
   mixed down to mono (rodio first, Symphonia when rodio can't read it). A
   file with no decodable audio fails here; it's never drawn from its raw
   bytes. The track's length comes from its metadata.
3. **Waveform, from all of the audio.** While decoding, `WaveformAccumulator`
   splits every sample into three bands with 2nd-order Butterworth filters
   (low under 350 Hz, high over 1000 Hz, mid what's left) and fills one bin
   per stretch of the track: its peak, the RMS of each band and how often the
   signal crosses zero. The bin count follows the length,
   `max(2400, ceil(seconds × 150) + 4)`, so each bin covers the same time on
   any track, and the whole track is drawn however long it is. A file with no
   known length gets no waveform rather than one stretched to a made-up
   length. These bins become rekordbox's waveform chunks (`PWAV`, `PWV2`–
   `PWV7`) and the track table's small waveform; see `docs/WAVEFORMS.md`.
4. **The first pieces reach the screen.** As soon as the decode is done the
   row gets its cover (the file's embedded art, else a cover image in its
   folder or the one above, saved as a thumbnail), its waveform and its length, before the slow part.
5. **BPM and key.** Only the first 24M samples (about 9 minutes at 44.1 kHz,
   `BPM_KEY_MAX_SAMPLES`) are kept for detection, to bound memory. The
   selected engine detects from them (`detect_bpm_key`): `stratum` in Rust,
   or `essentia` in its own runner, which fails the track rather than falling
   back to stratum. stratum's raw BPM is then refined and its first beat
   placed again, as "Stratum BPM and first beat" below describes, within the
   BPM range set in Settings.
6. **First beat.** A kept first beat stays. With a kept BPM, the first beat is
   placed from the audio at that BPM (`first_beat_secs`); otherwise the
   engine's first beat is used. When the audio gives none (silence), the old
   one stays.
7. **Analysis files.** `write_track_bundle`, the one writer of a new analysis
   bundle, writes the `.DAT`, `.EXT` and `.2EX` with the waveform, the beat
   grid built from the BPM and first beat, and the cues, plus an MP3's seek
   index (`PVBR`) when it can be reproduced exactly. A library track's bundle
   goes to the app's `analysis/waveforms` folder, with no cues: the next save
   or export puts them in. Export copies these files to the USB; see
   `docs/USB_EXPORT.md`.
8. **Saved, with where each value came from.** The coordinator saves the
   result in one transaction per track: BPM with `bpm_analyzer` (the engine
   and its version, `stratum-v2` or `essentia`), key with `key_source`, first
   beat with `first_beat_ms_source = 'estimated'` when this run placed it,
   length, cover and the analysis files' path. A kept value keeps its source,
   so the app always knows whether a value is its own detection, yours or an
   import's. The row then gets its BPM and key.

A failure in one track (a file that won't decode, a detection that fails) is
reported for that track and the job goes on with the rest. A USB repair runs
steps 2–7 through the same coordinator (`analyze_audio_files`), writing the
bundle on the USB with the stick's own cues and track path, and for a FLAC its
seek index (`PVB2`); a track whose BPM can't be found there still gets its
waveform, with no beat grid. A transcode gets
only steps 2, 3 and 7, its own waveform on its own timeline. How the
coordinator schedules tracks, and the memory and worker limits, are under
"Batch concurrency model" below.

### Analysis engine matrix

| Engine | Runtime | Default | Typical use |
| --- | --- | --- | --- |
| `stratum` | Rust-native backend path | Yes | fast local default with no extra runtime setup |
| `essentia` | JS/WASM runner path | No (opt-in) | alternate BPM/key behavior when users explicitly install and select it |

### Stratum BPM and first beat

stratum searches whole-BPM steps and is typically off by up to ±2 BPM, so its
raw estimate isn't stored as is. `refine_bpm` (`service/bpm_key.rs`) scores
tempos by how consistently the onset envelope stays in phase with their beat
over the whole track; a tempo even 0.1 BPM off drifts a beat over ten
minutes, so the right one stands out. Each tempo is scored on the beat and
on its 2× and 4× subdivisions, which keeps the score sharp when a backbeat
falls half a beat off (drum & bass analyzed at 87 instead of 174).

1. **Tempo family.** stratum's estimate is compared with its 3/2, 3/4, 2/3,
   4/3, 5/4 and 4/5 relatives inside the analysis BPM range (5/4: 156 for a
   125 BPM track); a relative replaces it only when it scores clearly higher
   (`TEMPO_FAMILY`: 8× for 3/2 and 3/4, whose subdivisions also line up with
   stratum's own beats, 2.5× for the others). Halving and doubling are never
   done: at those, every onset still lands on the grid and the score can't
   tell which is right.
2. **Precise tempo.** Every tempo within ±2 BPM is tried in 0.01 steps. The
   best one must score at least 0.06 and 1.5× anything 0.5 BPM or more away;
   the nearest whole or half BPM is used instead when it scores at least 0.9
   of the best (`GRID_SNAP_COHERENCE`), so tempos between whole and half
   values (174.79) are found and whole or half ones stay exact. When no
   tempo stands out, the whole and half BPMs alone are compared the same
   way, and failing that the estimate is rounded to a whole BPM.
3. **First beat.** stratum's beats follow its own estimate and are often well
   off a beat, so the first beat is placed from the onsets of the track's
   first minute at the final BPM (`first_beat_secs`): the phase on the beat
   picks which beat it is, and the phases on the 2× and 4× subdivisions
   place it more precisely. stratum's first beat is used only when that
   isn't possible (silence).

A BPM detected this way is stored with `bpm_analyzer = "stratum-v2"`; a bare
`"stratum"` marks a track analyzed before these steps existed. Bump the
version in `AnalysisEngine::bpm_analyzer` whenever a change here would give
already-analyzed tracks a different BPM.

The opt-in `bpm_reference_accuracy` test (`service/bpm_reference_eval.rs`)
compares the BPM and first beat with rekordbox's analysis of a USB playlist
or a list of tracks, for checking changes to these steps.

### How to use engines

1. Open Settings and choose analysis engine (`stratum` or `essentia`).
2. If selecting `essentia`, install/download runtime assets from Settings first.
3. Run library analysis (`analyze_new_tracks` / track-level analysis actions).
4. Verify resulting BPM/key fields in track rows.

### Quick export-prep flow

1. Scan/index sources to materialize tracks quickly.
2. Build or update the target playlist.
3. Use "Analyze Missing Tracks" for only that playlist's unanalyzed local tracks.
   On a playlist, a track whose local analysis files (`DAT`/`EXT`) are gone or damaged
   counts as unanalyzed too, as the export gate sees it; analyzing it writes
   new files as a first analysis, so edited and imported values are kept.
4. Export after missing-analysis count reaches zero.

### Analyze and Reanalyze

What an analysis run does to a track depends on whether the track is already
analyzed (has its waveform); the buttons' tooltips say the same:

- **Analyze** (track row) and **Analyze Missing Tracks** (playlist page): a
  first analysis. They add the missing waveform, BPM, key and first beat; a BPM,
  key or first beat you edited, or one imported from rekordbox or Mixxx, is
  kept (an imported first beat only together with the BPM imported with it).
- **Reanalyze** (track row, once analyzed): resets the waveform, BPM, key and
  first beat to a fresh analysis, replacing imported or edited values. Cues
  are kept.
- **Reanalyze** in the selection bar (Library and app playlists): every
  selected track gets the fresh analysis, even one never analyzed before, so
  imported or edited values are replaced (`reanalyze` on
  `analyze_new_tracks`). A selected track in a source folder that isn't in the
  library yet is added first, as its own Analyze button does. Filling in only
  what's missing stays with the header's Analyze Library and the playlist's
  Analyze Missing Tracks.

The rule lives in one place, `kept_analysis_values` (`service/analysis.rs`).

"Local" here is decided by the backend-computed `Track.isUsbPath` field (see
`docs/APP_DATA_MODEL.md`), not a frontend guess — a track whose `filePath`
falls under any known USB device root is excluded from "Analyze Missing"
regardless of which drive is currently selected. If a track still turns out
to be unavailable at export time, export skips it and reports a warning via
the Event Log rather than failing (`export.missing-source-file`).

### Missing or moved source folders

Source folders are recoverable state. If a configured source folder is renamed,
moved, unmounted, or otherwise missing, the app must not interpret that as "all
tracks under this folder were deleted".

The backend stores the source folders; `get_library_sources` reports each
one's `missing` state on startup and after every change. Missing folders are
rendered as warning-colored source chips. The normal filter
checkbox is shown unchecked/disabled for a missing source, and clicking the chip
opens relocation instead of toggling the filter. The chip remove action remains
the explicit destructive path and uses the existing confirmation before
`remove_source_folder` removes the folder and prunes its tracks.

Scanning also treats missing folders as non-destructive. `scan_library` logs
missing roots as a warning, scans only usable roots, and only prunes stale
tracks for roots that currently exist. If every enabled source folder is
missing, scan returns zero indexed/updated/removed rows with a warning.

Relocation uses `relocate_source_root`: the backend rewrites `tracks.file_path`
from `oldRoot/relative/path` to `newRoot/relative/path` only when the target
file exists and no indexed-path conflict exists. Track IDs are preserved, so
local playlist membership remains intact. If some files cannot be resolved under
the new root, those tracks remain under the old missing root and the backend
keeps the old root configured for follow-up. After rewriting the paths, the same command indexes the
new root (indexing only, as adding a folder does), so files only the new folder
has join the library; the status line counts them.

## Deep technical details

The backend separates indexing and analysis at the command level. `scan_library` is responsible for file discovery, metadata indexing, and local database updates, while `analyze_new_tracks` performs CPU-heavy enrichment on tracks already indexed in local storage. This separation is what allows the UI to show track rows quickly and then fill in analysis fields progressively.

`scan_library` intentionally keeps scanner-side file processing conservative instead of adding broad worker fan-out. In typical music-library workloads, scan speed is dominated by storage I/O and metadata reads; aggressive scanner parallelism can increase random I/O pressure and reduce stability/determinism. Parallelism is applied where it has stronger payoff: analysis workloads (`analyze_new_tracks`) that perform heavier CPU work after decode.

The runtime flow is:

1. UI triggers `scan_library`.
2. Backend returns indexed tracks, including tracks with incomplete analysis fields.
3. UI schedules analysis commands for tracks missing core analysis data.
4. Backend emits progress payloads while analysis is running.
5. UI applies row-level updates as each track result arrives.

Progress payloads are strongly typed in the backend (`AnalyzeTrackProgress`) and passed through the Tauri event bridge so the frontend can patch specific fields (`bpm`, `key`, waveform/artwork references) without reloading the full table.

Analysis output ownership is local and deterministic:

- waveform and ANLZ-related outputs are generated from decoded local audio
- BPM/key values are generated by local analysis engines
- generated artifact paths are stored in local DB fields and reused by export

The beat-grid anchor `first_beat_ms` is estimated at analysis time
(see "Stratum BPM and first beat" above; `estimate_first_beat_ms` from the
waveform when analysis gives none) but is **user-editable** from the track-detail
("Cues") modal alongside cue points. Once edited, `first_beat_ms_source` flips
to `'user'`; an import sets `'rekordbox'` or `'mixxx'`. Like the BPM and key,
a first analysis keeps an edited first beat, and an imported one when the BPM
imported with it is kept; a reanalysis places a new one. Every first beat an
analysis places is stored as `'estimated'`, even when it lands on the old value.
A key the analysis detects is stored with the engine as its source
(`key_source`), so a later rekordbox or Mixxx import asks before replacing it
instead of taking it for its own; a kept key keeps its source.
See `docs/CUE_EDITOR.md`, `docs/APP_DATA_MODEL.md` (TrackCue) and `docs/USB_EXPORT.md`.

Waveform output has two resolutions. The frontend receives a downsampled preview (`2400` bins), but local `DAT/EXT/2EX` cache files are generated from detail-resolution waveform data: `max(2400, ceil(duration_seconds * 150) + 4)`. This keeps `PWV3`, `PWV5`, and `PWV7` dense enough for CDJ detailed waveform views while avoiding large UI payloads. See `docs/WAVEFORMS.md`.

This means export depends on prior successful analysis state instead of trying to infer missing analysis from USB metadata at export time. If required fields are missing, that is surfaced as validation or workflow state rather than silently fabricating values.

Error handling is non-fatal by design. Unsupported files, decode failures, or partial analysis failures are reported through response payloads and job events, while remaining tracks continue processing. This keeps long-running library jobs resilient for mixed-quality music collections.

Engine resolution model:

- request-level override can select engine for a specific analysis request
- otherwise backend uses persisted setting key `ui_analysis_engine_v1`
- default is `stratum` when no explicit setting/override is present

Runtime behavior differences:

- `stratum` runs fully in backend Rust and is available by default
- `essentia` requires runtime assets and runner availability
- when `essentia` is selected and runtime setup fails, the request returns an explicit error instead of silently switching to `stratum`

Batch concurrency model:

- there is a single analysis dispatch path: the frontend always calls the `analyze_new_tracks` command with the full list of track IDs that need (re-)analysis, whether triggered by "Analyze Library", "Analyze Missing Tracks", or a manual single-track reanalyze — there is no separate frontend-driven per-track concurrency mechanism
- multi-track analysis uses a shared-work-queue worker model; for a single track it processes sequentially without spinning up a thread pool
- worker count is bounded by host parallelism, batch size, and available system memory, always reserving ~2 cores for the OS/UI thread (`resolve_analysis_parallelism_budget_with_cap`) so a high-core-count/high-RAM host doesn't peg every core and make the app unresponsive during a large batch
- a conservative per-worker memory budget (heavier for the opt-in `essentia` engine than the default `stratum` engine) caps parallelism on RAM-constrained hosts to avoid OOM crashes on high-core-count/lower-RAM machines — because every caller now goes through this one path, this protection applies uniformly, including the common "Analyze Library" flow that used to bypass it entirely
- optional worker caps/diagnostics are available for development and debugging (`DJTKIT_ANALYSIS_MAX_WORKERS`, `DJTKIT_ANALYSIS_AVAILABLE_MEMORY_BYTES`, `DJTKIT_ANALYSIS_DEBUG_WORKERS`); with `DJTKIT_ANALYSIS_DEBUG_WORKERS=1`, the resolved cap breakdown prints to the terminal (stderr) as well as the in-app Event Log
- per-piece progress (artwork, then waveform, then duration, then bpm/key — matching the track row's left-to-right column order) still streams to the UI incrementally as each piece finishes, via `job.progress` events — this is independent of the dispatch path and unaffected by the above
- a job can be paused/resumed or cancelled via `set_analysis_paused`/`cancel_analysis`, messages to the coordinator. A track already in flight always finishes normally either way, and with nothing running both do nothing
- one coordinator thread (`service/analysis/coordinator.rs`) runs all analysis, for as long as there is work: it owns a database connection outside the pool, the queue, the worker budget (one slot per track, read once per engine for the job) and what the user is told. `analyze_new_tracks` adds to its job and returns at once, so requests during a job grow it, first come first served. Workers only analyze: they send track pieces to the user and their result to the coordinator, which saves and reports it. A track already in the job is skipped; a Reanalyze of one still waiting makes it replace its kept values. Pause stops new tracks from starting; cancel clears the queue and lets running tracks finish. A track waits for memory while others run, and starts anyway when nothing is running, so a job never fails a track for lack of memory
- the USB repairs analyze audio files outside the library through the same coordinator (`analyze_audio_files`): the files join the queue after whatever is waiting, take the same worker slots and memory check, and are paused and cancelled with it (a cancelled file comes back as cancelled). A worker runs the analysis pipeline's core (`analyze_audio_core`) on a file and the result goes back to the waiting repair; nothing is saved to the library and the job shows no row for it

Implemented throughput optimizations:

- dynamic shared-queue scheduling for better tail utilization
- per-track transaction commits for batched DB writes (a transaction is opened, the cached statement executed, and committed once per track — not held open for the whole batch — so other writers aren't blocked with "database is locked")
- decode-path allocation reuse

Verification tools:

- waveform/analysis support tools: `backend/src/bin/analyze_playlist_tracks.rs`, `backend/src/bin/analyze_one_track.rs`

Operational commands related to engine runtime:

- `download_essentia`
- `cancel_essentia_download`
- `remove_essentia`

Operational commands for controlling a running batch:

- `set_analysis_paused`
- `cancel_analysis`

Implementation anchors:

- command façade: `backend/src/commands.rs`
- Tauri bridge: `backend/src/tauri_commands/analysis.rs`
- analysis progress type: `backend/src/service/analysis.rs:153`
- analysis command entry: `backend/src/service/analysis.rs:490`
- batched analysis path: `backend/src/service/analysis.rs:497`
- worker cap resolution: `backend/src/service/analysis.rs` (`resolve_worker_cap_for_engine`)
- pause/cancel flags and checks: `backend/src/service/analysis.rs` (`set_analysis_paused`, `cancel_analysis`)
- WAV `fmt ` chunk parsing/classification: `backend/src/wav_format.rs`
- scan-time WAV_EXTENSIBLE detection call site: `backend/src/scanner.rs` (`read_wav_extensible_kind`)

## Verification links

- Backend analysis coverage: `backend/tests/anlz_pipeline_functional.rs`
- Backend command contract coverage: `backend/tests/core_command_contract_functional.rs`
- Engine behavior coverage: `backend/tests/lib_integration.rs`
- Frontend analysis and hydration coverage: `vanilla-ui/tests/e2e/scan_analysis_batches.spec.mjs`, `vanilla-ui/tests/e2e/waveform_startup_hydration.spec.mjs`
- Pause/resume/cancel coverage: `backend/tests/lib_integration.rs` (`analyze_new_tracks_pauses_and_resumes_between_tracks`, `analyze_new_tracks_cancel_stops_early_without_hanging`), `vanilla-ui/tests/e2e/scan_analysis_batches.spec.mjs`
