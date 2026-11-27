# Vendored stratum-dsp

Upstream: stratum-dsp 1.0.0 from crates.io (https://github.com/HLLMR/stratum-dsp),
`MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). The workspace `Cargo.toml`
points `stratum-dsp` here through `[patch.crates-io]` and excludes this folder
from the workspace, so it builds as a dependency and isn't linted as app code.

Kept from the crate: `src/`, the licenses, `README.md`, `CHANGELOG.md`,
`PIPELINE.md`. Left out: tests, benches, examples, docs, validation data and
archive. `Cargo.toml` is upstream's without the dev-dependencies, those
targets and the profiles (the workspace sets stratum-dsp's release profile).

## Changes from upstream

- `src/lib.rs`: `analyze_bpm` and `analyze_key` run `analyze_audio`'s
  pipeline with only the steps one result needs. `analyze_bpm` skips the key
  step; `analyze_key` skips onsets, tempo estimation and beat tracking. For
  the same input each gives exactly `analyze_audio`'s values for its half
  (test: `stratum_split_entry_points_match_full_analysis` in
  `backend/src/service/bpm_key.rs`). `analyze_audio` runs both, unchanged.

To update: copy the new crate's `src/` and licenses over, then reapply the
changes above.
