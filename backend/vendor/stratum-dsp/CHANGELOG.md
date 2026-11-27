# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> Detailed phase-by-phase implementation history and tuning logs live in `docs/progress-reports/`.

## [1.0.0] - 2025-12-18

### Production Ready
- **BPM Detection**: 87.7% accuracy (±2 BPM) on 155 verified Beatport/ZipDJ tracks
- **Key Detection**: 72.1% accuracy (exact match vs ground truth, matches Mixed In Key performance)
- **Beat Grid**: HMM-based tracking with stability scoring
- **Performance**: ~200ms per 3-minute track, 21 tracks/sec batch throughput (7.7× speedup)

### Core Features
- **Tempogram BPM** (Grosche et al. 2012): FFT + autocorrelation tempogram with multi-resolution escalation
- **Krumhansl-Kessler key detection**: Chroma-based analysis with HPSS preprocessing and circle-of-fifths weighting
- **Confidence scoring**: Comprehensive confidence metrics for all analysis components
- **Parallel batch processing**: CPU-1 workers default for high-throughput library scans

### Validation
- 155 real-world DJ tracks (Beatport, ZipDJ) with verified ground truth
- Full validation results documented in `docs/progress-reports/PHASE_1F_VALIDATION.md`
- Reference baseline: Mixed In Key achieves 98.1% ±2 BPM and 72.1% key accuracy on same dataset

### API
```rust
use stratum_dsp::{analyze_audio, AnalysisConfig};

let result = analyze_audio(&samples, 44100, AnalysisConfig::default())?;
println!("BPM: {:.1} | Key: {} ({})", 
    result.bpm, 
    result.key.name(), 
    result.key.numerical()
);
```

### Documentation
- Complete pipeline documentation (`PIPELINE.md`)
- Validation reports and benchmarks
- Literature reviews for all algorithms
- Development and contribution guides

---

## [Unreleased]

### Planned
- Phase 2 ML refinement (feature-gated `ml`)
- Key detection improvements (harmonic-only chroma, better aggregation)

### Added
- `examples/analyze_batch.rs`: parallel batch processing (CPU-1 workers default)
- `docs/progress-reports/PHASE_1F_BENCHMARKS.md`: batch throughput + outlier analysis
- `CONTRIBUTING.md`: contributor guidelines and development workflow
- Validation tooling cleanup:
  - `validation/tools/` (run scripts) and `validation/analysis/` (post-run analysis)
  - `validation/_id3.py`, `validation/_keys.py`: shared ID3/key parsing utilities
  - `validation/tools/build_hllmr_metadata.py`: GT snapshot tool for real-world DJ tracks
- `archive/`: archived "construction debris" not compiled as part of the crate

### Changed
- **README.md**: Major update with validation results table, performance benchmarks, known limitations
- Documentation: top-level docs focus on the current pipeline and canonical workflows
- Defaults: HPSS percussive tempogram fallback is opt-in (avoids multi-second outliers)
- Key detection: Fixed Krumhansl-Kessler template alignment (canonical profiles + L2 normalization)
  - Minor keys now correctly detected (was previously biased toward major)
  - Key accuracy improved from 1.5% to 72.1% vs GT

### Removed
- Unused dependencies: `ndarray`, `ndarray-linalg`
- Unimplemented public IO stubs moved out of the crate (archived under `archive/`)
