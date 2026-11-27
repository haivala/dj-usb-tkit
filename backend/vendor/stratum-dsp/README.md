# Stratum DSP

[![Crates.io](https://img.shields.io/crates/v/stratum-dsp.svg)](https://crates.io/crates/stratum-dsp)
[![Docs.rs](https://docs.rs/stratum-dsp/badge.svg)](https://docs.rs/stratum-dsp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

**Professional-grade audio analysis engine for DJ applications** — pure Rust, zero FFI dependencies, production-ready BPM detection, key detection, and beat tracking.

## Features

- **Tempo (BPM)** detection with confidence scoring
  - Dual tempogram approach (FFT + autocorrelation) based on Grosche et al. (2012)
  - Multi-resolution escalation for metrical-level disambiguation
  - **87.7% accuracy within ±2 BPM** on real-world DJ tracks (155 Beatport/ZipDJ tracks)
- **Key detection** with musical notation + numerical DJ notation (`1A/1B`)
  - Chroma-based analysis with Krumhansl-Kessler template matching
  - Supports both major/minor keys
- **Beat grid** generation with stability scoring
  - HMM-based beat tracking with tempo drift correction
- **Preprocessing**: Peak/RMS/LUFS normalization (ITU-R BS.1770-4) + silence trimming
- **Batch processing**: Parallel analysis with CPU-1 workers (7.7× throughput improvement)

## Installation

### From crates.io

Add to your `Cargo.toml`:

```toml
[dependencies]
stratum-dsp = "1.0"
```

### From Git (Development)

Add to your `Cargo.toml`:

```toml
[dependencies]
stratum-dsp = { git = "https://github.com/HLLMR/stratum-dsp" }
```

**Note**: The API is stable for core features (BPM/key/beat-grid). Advanced tuning parameters may change in future versions.

## Quick Start

### Library Usage

```rust
use stratum_dsp::{analyze_audio, compute_confidence, AnalysisConfig};

// Load your audio samples (mono, f32 in [-1, 1])
let samples: Vec<f32> = vec![]; // Your audio data
let sample_rate = 44_100;

// Analyze
let result = analyze_audio(&samples, sample_rate, AnalysisConfig::default())?;
let conf = compute_confidence(&result);

println!("BPM: {:.2} (confidence: {:.2})", result.bpm, conf.bpm_confidence);
println!("Key: {} ({})", result.key.name(), result.key.numerical()); // e.g., "C major (1A)", "Am (1B)"
println!("Beat grid stability: {:.2}", result.grid_stability);
# Ok::<(), stratum_dsp::AnalysisError>(())
```

### CLI Examples

**Single file analysis:**

```bash
cargo build --release --example analyze_file
target/release/examples/analyze_file --json <audio_file>
```

**Batch processing (parallel):**

```bash
cargo build --release --example analyze_batch
target/release/examples/analyze_batch --jobs 7 <file1> <file2> ...
```

## Validation Results

Stratum DSP has been validated on real-world DJ tracks from Beatport and ZipDJ (155 tracks with verified ground truth):

| Metric | Result |
|--------|--------|
| **BPM accuracy (±2 BPM)** | **87.7%** (136/155 tracks) |
| **BPM accuracy (±5 BPM)** | 88.4% (137/155 tracks) |
| **BPM accuracy (±10 BPM)** | 89.0% (138/155 tracks) |
| **BPM MAE** | 6.08 BPM |
| **Key accuracy** | **72.1%** exact match vs GT (68 tracks) — *matches MIK performance* |

**Dataset**: 155 verified DJ tracks (Beatport/ZipDJ) with ground truth BPM/key from vendor tags (pre-MIK snapshot).  
**Reference baseline**: Mixed-in-Key (TAG) achieves 98.1% ±2 BPM and 72.1% key accuracy on the same dataset.

**Note**: FMA Small dataset results (used for algorithm development) show lower accuracy (56.7% ±2 BPM) due to dataset diversity. Real-world DJ tracks achieve production-grade accuracy as shown above.

For detailed validation reports, see:
- `docs/progress-reports/PHASE_1F_VALIDATION.md` (FMA Small development results + real-world DJ results)
- `docs/literature/stratum_2025_key_detection_real_world.md` (key detection improvements)
- `validation/README.md` (validation workflow)

## Performance

### Single-Track Processing

- **~200–210 ms** for a 3-minute track (synthetic benchmark, Criterion.rs)
- **~150–350 ms** typical range for real-world tracks (varies by track length and complexity)

### Batch Throughput

- **Single worker**: ~2.8 tracks/sec
- **CPU-1 workers** (parallel): ~21.3 tracks/sec (**7.7× speedup**)

**Example**: Processing 1000 tracks takes ~6 minutes with CPU-1 workers (vs ~45 minutes single-threaded).

For detailed benchmark reports, see `docs/progress-reports/PHASE_1F_BENCHMARKS.md`.

## Known Limitations

### BPM Detection

- **Metrical-level errors**: ~12% of tracks show octave/harmonic confusions (1/2×, 2×, 2/3× ratios)
  - Common on double-time feels, triplet-based tracks, or tracks with weak kick-snare patterns
  - Multi-resolution escalation helps but doesn't eliminate all cases
- **Very low BPM** (<60 BPM): May be confused with half-tempo if the track has strong sub-harmonic content

### Key Detection

- **Current accuracy**: 72.1% exact match vs ground truth (matches Mixed In Key performance)
- **Known issue**: Some tracks with weak tonality or heavy percussion may have lower confidence
- **Workaround**: Key confidence scores indicate reliability; low confidence (<0.3) suggests the result may be unreliable

### General

- **CPU-only**: No GPU acceleration (Phase 2 will add optional ML refinement)
- **Fixed sample rate**: Optimized for 44.1 kHz (other rates work but may have slight accuracy differences)

## Algorithm References

- **BPM Detection**: Grosche, P., Müller, M., & Kurth, F. (2012). *Cyclic Tempogram—A Mid-Level Tempo Representation for Music Signals*. ICASSP.
- **Key Detection**: Krumhansl, C. L., & Kessler, E. J. (1982). *Tracing the Dynamic Changes in Perceived Tonal Organization in a Spatial Representation of Musical Keys*. Psychological Review.
- **Beat Tracking**: Ellis, D. P. W. (2007). *Beat Tracking by Dynamic Programming*. Journal of New Music Research.
- **Preprocessing**: ITU-R BS.1770-4 (Loudness normalization)

Full literature review: `docs/literature/`

## Documentation

- **Pipeline (authoritative)**: `PIPELINE.md` — exact runtime flow and decision points
- **Development workflow**: `DEVELOPMENT.md` — build, test, benchmark, validate
- **Roadmap**: `ROADMAP.md` — high-level milestones
- **Progress reports**: `docs/progress-reports/` — detailed phase histories and tuning logs
- **Validation**: `validation/README.md` — how to reproduce validation results

## Contributing

Contributions are welcome! Please see the documentation in `docs/` for algorithm details and `DEVELOPMENT.md` for build/test workflows.

**Note**: This project follows a phased development approach. Phase 1 (DSP-first) is focused on core tempo/key/beat-grid accuracy. Phase 2 will add optional ML refinement.

## License

Dual-licensed under **MIT OR Apache-2.0** at your option.
