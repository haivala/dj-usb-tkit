# Stratum DSP - Processing Pipeline (Authoritative)

This document describes the **exact runtime processing flow** and **decision points** for `stratum-dsp` as implemented today. It is intended to be the persistent, authoritative reference for:

- What happens **when** in `analyze_audio()`
- Why particular branches/fallbacks exist
- Where **legacy (Phase 1B) BPM estimation** fits in the chain
- Which modules/files implement each stage

> Source of truth: `src/lib.rs` (function `analyze_audio()`).

---

## High-level flow (current implementation)

```text
analyze_audio(samples, sample_rate, config)
  ├─ Input validation
  ├─ Preprocessing
  │   ├─ Normalization (Peak/RMS/LUFS)
  │   └─ Silence detection + trim
  ├─ Onset detection (energy flux + optional consensus from STFT-based detectors)
  ├─ BPM estimation (Phase 1F tempogram; Phase 1B legacy fallback)
  │   ├─ STFT magnitudes (shared with chroma extractor)
  │   ├─ Novelty curve (spectral flux + energy flux + HFC + conditioning)
  │   ├─ FFT tempogram + autocorr tempogram
  │   ├─ Candidate folding + scoring + “preferred range” gating
  │   └─ Fallback: legacy onset-list BPM if tempogram fails
  ├─ Beat tracking (HMM Viterbi beat grid) using BPM + energy onsets
  ├─ Key detection (chroma extraction + templates)
  ├─ Result assembly + warnings/flags
  └─ Confidence scoring (computed and logged)
```

---

## Detailed step-by-step with decision points

## 0) Entry point + validation

**Function**: `analyze_audio(samples, sample_rate, config)`  
**File**: `src/lib.rs`

### Decision points

- **Empty samples** → return `AnalysisError::InvalidInput("Empty audio samples")`
- **sample_rate == 0** → return `AnalysisError::InvalidInput("Invalid sample rate")`

---

## 1) Preprocessing (Phase 1A)

### 1.1 Normalization

**File**: `src/preprocessing/normalization.rs`

- Normalization is applied to a mutable working copy of samples.
- Config-driven: `AnalysisConfig.normalization`
- Default loudness target in the pipeline: **-14 LUFS**

### 1.2 Silence detection + trimming

**File**: `src/preprocessing/silence.rs`

**Key parameters**:
- `threshold_db = config.min_amplitude_db`
- `min_duration_ms = 500`
- `frame_size = config.frame_size`

### Decision points

- If trimming removes everything: return `AnalysisError::ProcessingError("Audio is entirely silent after trimming")`

---

## 2) Onset detection (Phase 1A)

### Current runtime behavior

The pipeline computes an onset list in two layers:

1. **Energy flux on raw samples** (always computed; fast baseline)
2. **Optional multi-detector consensus** (default enabled): combines
   - energy flux (samples)
   - spectral flux (STFT-based)
   - HFC (STFT-based)
   - HPSS (STFT-based, optional; more expensive)

**Function**: `detect_energy_flux_onsets(...)`  
**File**: `src/features/onset/energy_flux.rs`

**Consensus voting**: `vote_onsets(...)`  
**Files**:
- `src/features/onset/consensus.rs`
- `src/features/onset/spectral_flux.rs`
- `src/features/onset/hfc.rs`
- `src/features/onset/hpss.rs`

**Output**:
- `onsets_for_beat_tracking: Vec<usize>` (onset sample indices)
- `onsets_for_legacy: Vec<usize>` (onset sample indices)

### Notes (important for understanding “where things happen”)

- The consensus stage clusters onsets within a tolerance window and assigns a confidence based on which detectors agreed.
- Default selection policy:
  - Prefer onsets confirmed by **≥2 methods**
  - If none are confirmed by ≥2, fall back to the full clustered set (≥1 method)
  - If consensus fails or yields nothing, fall back to energy-flux onsets
- Tempogram BPM estimation does **not** consume this onset list; it operates on a novelty curve derived from the STFT magnitude spectrogram (see Phase 1F below).

---

## 3) BPM estimation (Phase 1F primary + Phase 1B legacy fallback)

This is the most complex decision layer in the current system.

### 3.1 Compute STFT magnitude spectrogram

**Function**: `compute_stft(samples, frame_size, hop_size)`  
**File**: `src/features/chroma/extractor.rs`

**Why here?**
- The STFT implementation is shared with chroma extraction; Phase 1F reuses it to avoid duplicating FFT code.

**Output**
- `magnitude_spec_frames: Vec<Vec<f32>>` (frames × bins)

### Decision point

- If STFT returns empty frames → log a warning and fall back to **legacy** BPM estimation (Phase 1B), if enough onsets exist.

### 3.2 Phase 1F tempogram BPM estimation (primary path)

**Entry point**: `estimate_bpm_tempogram(magnitude_spec_frames, sample_rate, hop_size, min_bpm, max_bpm, bpm_resolution)`  
**File**: `src/features/period/tempogram.rs`

#### 3.2.1 Novelty curve extraction (from STFT magnitudes)

**File**: `src/features/period/novelty.rs`

Computed components:
- Spectral flux novelty
- Energy flux novelty (spectrogram energy changes)
- HFC novelty
- Weighted combination

**Novelty conditioning (tuning additions)**:
- local mean subtraction (high-pass in time) + half-wave rectification
- light smoothing
- normalization

#### 3.2.2 Two periodicity representations

**FFT tempogram**  
**File**: `src/features/period/tempogram_fft.rs`

- Preconditioning: mean removal (DC) + Hann window
- FFT → frequency bins → BPM mapping

**Autocorrelation tempogram**  
**File**: `src/features/period/tempogram_autocorr.rs`

- For each BPM hypothesis, score autocorrelation at the corresponding lag

#### 3.2.3 Candidate generation + metrical-level folding

**File**: `src/features/period/tempogram.rs`

Key idea: the system explicitly generates and scores tempo-related candidates to resolve metrical ambiguity:

- Candidate family includes: \(T\), \(T/2\), \(2T\), \(T/3\), \(3T\), \(2T/3\), \(3T/2\)
- Candidate seeds are drawn from top-N peaks of both tempograms

#### 3.2.4 “Preferred tempo range” prior + “outside-range requires stronger evidence”

**File**: `src/features/period/tempogram.rs`

Current policy (DJ-centric):

- Preferred range: **60–180 BPM** (soft prior, not a hard clamp)
- Outside-range penalty: scores are reduced if bpm < 60 or bpm > 180
- **Tempo-octave folding gate**:
  - If best bpm > 180, compare it to best bpm/2
  - Fold to bpm/2 unless the >180 candidate is convincingly stronger in **both** FFT and autocorr evidence

This implements the spirit of:
> “If you get a number outside the common range, you better be DAMN sure it’s really outside.”

#### 3.2.5 Output

Returns a `BpmEstimate`:
- `bpm`
- `confidence` (derived from separation between best and runner-up scored candidates)
- `method_agreement` (how often the primary peak aligns with the chosen BPM)

#### 3.2.6 True multi-resolution tempogram (gated escalation)

**Files**:
- `src/features/period/multi_resolution.rs` (true multi-res implementation)
- `src/lib.rs` (gating + acceptance logic)

**Why**: The dominant empirical failure mode is metrical-level ambiguity (e.g., \(T\) vs \(2T\) vs \(T/2\)).
True multi-resolution (recompute STFT at multiple hop sizes) is a classic discriminator for these octave-family errors.

**What happens** (high level):
- Always compute a **base** tempogram estimate at the configured hop size (typically 512).
- If the base estimate looks ambiguous (low confidence / low agreement, or falls in known “trap” tempo ranges),
  we **escalate** to true multi-resolution:
  - Recompute STFT magnitudes at **hop sizes {256, 512, 1024}**
  - Run the same tempogram pipeline at each hop
  - Fuse candidates using a cross-resolution scoring rule to decide whether to keep \(T\) or fold to a tempo-family member

**Important safety rule** (prevents regressions observed during tuning):
- We do **not** allow “upward promotion” from an in-range tempo to an extreme high tempo
  (e.g., \(120 \rightarrow 240\)). Multi-res is used to correct likely octave-folding errors, not create them.

**Primary knobs** (via `AnalysisConfig` / CLI):
- `enable_tempogram_multi_resolution` (CLI: `--no-tempogram-multi-res` to disable)
- `tempogram_multi_res_top_k` (CLI: `--multi-res-top-k N`)
- `tempogram_multi_res_w512`, `tempogram_multi_res_w256`, `tempogram_multi_res_w1024`
- `tempogram_multi_res_structural_discount`
- `tempogram_multi_res_double_time_512_factor`
- `tempogram_multi_res_margin_threshold`
- `tempogram_multi_res_use_human_prior` (CLI: `--multi-res-human-prior`)

### 3.3 Phase 1B legacy BPM estimation (fallback path)

**Entry point**: `estimate_bpm(onsets, sample_rate, hop_size, min_bpm, max_bpm, bpm_resolution)`  
**File**: `src/features/period/mod.rs`

Legacy path components:
- Autocorrelation on onset-list-derived binary signal: `src/features/period/autocorrelation.rs`
- Comb filterbank scoring: `src/features/period/comb_filter.rs`
- Candidate filtering/merging: `src/features/period/candidate_filter.rs`

### Decision points for BPM estimation

**Tempogram path used when**:
- STFT magnitude frames exist, and `estimate_bpm_tempogram(...)` succeeds

**Legacy fallback used when**:
- STFT magnitude frames are empty **or**
- Tempogram estimation errors out

**If both fail**:
- `bpm = 0.0`, `bpm_confidence = 0.0` and warnings are recorded in metadata

### Optional modes (A/B testing and tuning)

The following modes exist to support validation and controlled experiments:

- **Disable preprocessing**: `enable_normalization=false`, `enable_silence_trimming=false` (CLI: `--no-preprocess`)
- **Disable onset consensus**: `enable_onset_consensus=false` (CLI: `--no-onset-consensus`)
- **Force legacy BPM**: `force_legacy_bpm=true` (CLI: `--force-legacy-bpm`)
- **BPM fusion (validator)**: `enable_bpm_fusion=true` (CLI: `--bpm-fusion`)
  - Current behavior: **does not override** the tempogram BPM; it only adjusts the BPM confidence and emits diagnostics based on legacy agreement.
- **Disable true multi-resolution tempogram**: `enable_tempogram_multi_resolution=false` (CLI: `--no-tempogram-multi-res`)
- **Legacy guardrails**: `enable_legacy_bpm_guardrails=true`
  - Applies configurable confidence multipliers to legacy candidates to discourage extreme tempos from winning.

---

## 4) Beat tracking (Phase 1C)

**Entry point**: `generate_beat_grid(bpm, bpm_confidence, onsets_seconds, sample_rate)`  
**Files**:
- `src/features/beat_tracking/mod.rs` (public entry)
- supporting modules under `src/features/beat_tracking/`

**Inputs**:
- BPM + confidence (from Phase 1F tempogram primary, or Phase 1B fallback)
- Onsets in seconds (derived from energy flux onset list)

### Decision points

- If BPM == 0 or insufficient onsets → skip beat tracking and return empty grid, stability 0.0
- If beat tracking errors → return empty grid, stability 0.0 (non-fatal)

---

## 5) Key detection (Phase 1D + 1F improvements)

**Pipeline** (as wired today):

1. Chroma extraction (key-only STFT override, HPSS harmonic mask):
   - `extract_chroma_with_options(...)` in `src/features/chroma/extractor.rs`
   - Optional higher-resolution STFT (default: 8192 FFT) for key detection only
   - Optional median-filter HPSS harmonic mask to suppress percussive transients
2. Optional chroma sharpening:
   - `src/features/chroma/normalization.rs`
3. Optional smoothing:
   - `src/features/chroma/smoothing.rs`
4. Key detection + clarity:
   - `src/features/key/detector.rs`: Template matching with score normalization and circle-of-fifths weighting
   - Optional mode heuristic (3rd scale degree discrimination) to reduce minor→major mistakes
   - **Accuracy**: 72.1% vs GT (n=68) on real-world DJ tracks, matching MIK performance

### Decision points

- If insufficient samples for chroma → skip key detection and return default key with 0 confidences
- If chroma extraction or key detection fails → default key with 0 confidences (non-fatal)

---

## 6) Result assembly (Phase 1E) + confidence scoring

**Result struct**: `AnalysisResult`  
**Files**:
- `src/analysis/result.rs`
- `src/analysis/metadata.rs`
- `src/analysis/confidence.rs`

### Decision points and warnings/flags

`analyze_audio()` records warnings into metadata for:
- BPM detection failed (bpm == 0)
- low grid stability
- low key confidence / low key clarity (adds flags like weak tonality)

**Confidence computation**:
- `compute_confidence(&result)` is called after assembling the result, and logged.

---

## “Where do the old methods fall?”

Legacy BPM estimation is a **fallback branch** inside Phase 1F BPM detection:

```text
Tempogram path (primary)
  ├─ STFT magnitudes
  ├─ novelty -> tempograms -> fold/score/select
  └─ returns bpm

Legacy path (fallback)
  ├─ energy_flux onsets list
  ├─ autocorrelation + comb filterbank + merge
  └─ returns bpm
```

So, old methods are still “in the chain,” but only as:
- a **fallback** if tempogram fails, and
- a **transition vehicle** for A/B comparisons until Phase 1F is validated.

---

## Validation + A/B testing (how to observe this pipeline)

### CLI example (used by validation harness)

**Binary**: `examples/analyze_file.rs`  
**Build**: `cargo build --release --example analyze_file`  
**Run**: `target/release/examples/analyze_file --json <audio_file>`

### FMA validation harness

**Docs**: `validation/README.md`  
**Scripts**:
- `python -m validation.tools.prepare_test_batch`
- `python -m validation.tools.run_validation`
- `python -m validation.analysis.analyze_results`

Phase 1F tuning/validation status is tracked in:
- `docs/progress-reports/PHASE_1F_VALIDATION.md`

---

## Appendix: Module map (files by phase)

| Phase | Stage | Key entry points | Key files |
|------:|-------|------------------|----------|
| 1A | Preprocessing | `normalize`, `detect_and_trim` | `src/preprocessing/*` |
| 1A | Onsets | `detect_energy_flux_onsets` | `src/features/onset/energy_flux.rs` |
| 1F | BPM (primary) | `estimate_bpm_tempogram` | `src/features/period/{novelty,tempogram*,multi_resolution}.rs` |
| 1B | BPM (legacy) | `estimate_bpm` | `src/features/period/{autocorrelation,comb_filter,candidate_filter}.rs` |
| 1C | Beat grid | `generate_beat_grid` | `src/features/beat_tracking/*` |
| 1D | Key | `extract_chroma_with_options`, `detect_key` | `src/features/chroma/*`, `src/features/key/*` |
| 1E | Confidence | `compute_confidence` | `src/analysis/confidence.rs` |

---

## Appendix: Decision Log / Debugging Guide

This section documents the highest-signal log output for understanding **what path the pipeline took** and **why**.

### Enable debug logging

#### Option A: Using the example binary (recommended)

Build:

```text
cargo build --release --example analyze_file
```

Run with debug enabled:

```text
target/release/examples/analyze_file <audio_file> --debug --json
```

#### Option B: Using environment variables

Set the Rust log level to debug before running your binary:

```text
RUST_LOG=debug
```

### Key decision logs (by stage)

#### 0) Preprocessing

- **Empty/invalid input**
  - Indicates early exit before any DSP work.
- **“Audio is entirely silent after trimming”**
  - Indicates silence trimming removed all content (hard error).

#### 2) Onset detection (energy flux)

- **“Detected {N} onsets using energy flux”**
  - If `N` is very small (0–1), beat tracking will likely be skipped and legacy BPM fallback (if needed) may fail.

#### 3) BPM estimation (Phase 1F tempogram primary)

- **“Computing STFT…” / “Could not compute STFT… falling back to legacy method”**
  - If STFT is empty, the pipeline cannot run tempogram BPM and will fall back to Phase 1B legacy BPM (if onsets exist).

- **“Tempogram BPM estimate: …”**
  - Confirms tempogram succeeded and produced a BPM estimate.

- **Selection diagnostics**
  - `Tempogram metrical selection: chosen …`
    - Shows the chosen tempo and normalized evidence from FFT/autocorr.
  - `Tempo-octave fold applied: … -> …`
    - Shows that the system treated a >180 BPM candidate as likely double-time and folded it unless evidence was overwhelming.

#### 3) BPM estimation (Phase 1B legacy fallback)

- **“Tempogram BPM detection failed: … falling back to legacy method”**
  - Confirms tempogram errored and Phase 1B legacy BPM is being used instead.

#### 4) Beat tracking

- **“Skipping beat tracking: BPM=…, onsets=…”**
  - Happens when BPM is 0 or too few onsets exist; output grid will be empty.

- **“Beat grid generated: … stability=…”**
  - Stability near 1.0 suggests clean tempo/onsets; very low stability suggests mismatch, tempo drift, or poor onset signal.

#### 5) Key detection

- **“Skipping key detection: insufficient samples …”**
  - Short clips may skip key detection.

- **“Detected key: … confidence … clarity …”**
  - Low clarity often correlates with ambiguous tonality.

### Common “what happened?” interpretations

- **BPM=~200 when GT is ~100**
  - Classic metrical-level ambiguity (double-time). Check for `Tempo-octave fold applied`.

- **Many predictions pinned near the minimum BPM**
  - Suggests weak novelty periodicity + strong priors; inspect novelty extraction/conditioning logs and STFT size/hop.

- **Tempogram frequently falling back to legacy**
  - STFT frames may be empty (short clips) or tempogram is erroring; check STFT logs and tempogram errors.



