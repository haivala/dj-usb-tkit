# Contributing

Thanks for your interest in contributing to this project.

This repository is a local-first DJ library manager and USB export tool built with Rust, Tauri, and a vanilla frontend. Contributions are welcome in the form of bug reports, fixes, tests, documentation improvements, and focused feature work that fits the current project direction.

## Project License

This repository is licensed under `MIT`. See `LICENSE`.

## Contribution License Policy

Unless explicitly stated otherwise, all contributions submitted to this repository are accepted under `MIT` (inbound = outbound).

In practical terms:

- you keep your copyright in your contribution
- by submitting a contribution, you agree that your contribution may be distributed under `MIT`
- inbound contributions are treated as outbound under `MIT`

This is the project's default contribution policy. Do not submit code, assets, or other material unless you have the right to contribute it under these terms.

## What To Contribute

Useful contributions include:

- bug fixes
- tests and regression coverage
- documentation improvements
- UI behavior fixes
- backend correctness and safety improvements
- export/import diagnostics improvements
- narrowly scoped features that align with the documented project goals

Before starting larger work, it is best to open an issue or discussion first so the implementation direction can be aligned early.

## Before You Open A PR

Please try to:

1. read the relevant docs in `docs/`
2. keep changes focused and minimal
3. add or update tests when behavior changes
4. update documentation when user-visible or architectural behavior changes
5. avoid unrelated refactors in the same change

Relevant project docs include:

- `docs/LIBRARY_ANALYSIS.md`
- `docs/PLAYLISTS_PLAYBACK.md`
- `docs/USB_IMPORT.md`
- `docs/USB_EXPORT.md`
- `docs/DIAGNOSTICS_REPAIRS.md`
- `docs/COMMANDS.md`
- `docs/APP_DATA_MODEL.md`
- `docs/PDB.md`
- `docs/eDB.md`

## Development Notes

Project structure:

- `backend/` — Rust backend library and tests
- `desktop/` — Tauri desktop host
- `vanilla-ui/` — frontend shell and browser-side tests
- `docs/` — product, backend, command, and operational documentation

## Suggested Workflow

1. create a branch for your change
2. make the smallest change that solves the problem cleanly
3. add or update tests where appropriate
4. run the relevant test suite
5. update docs if needed
6. open a pull request with a clear summary

## Testing

Run the backend tests from `backend/`:

```text
cargo test
```

Run the frontend tests from `vanilla-ui/`:

```text
npm test
```

If your change affects only one area, it is fine to mention exactly which tests you ran.

## Commit And PR Guidance

A good pull request usually includes:

- a short description of the problem
- a short description of the approach
- notes about tests run
- notes about any follow-up work or known limitations

Try to keep pull requests reviewable. Smaller, focused PRs are much easier to merge safely than broad mixed changes.

## Code And Content Requirements

By contributing, you confirm that:

- you wrote the contribution yourself, or otherwise have the legal right to submit it
- you are not knowingly submitting material that violates another party's license, copyright, trademark, or other rights
- any dependency additions are appropriate for the repository's licensing and distribution model
- any copied or adapted third-party material is clearly attributed and legally compatible

## Third-Party Dependencies

Be careful when introducing new dependencies, especially if they:

- have strong copyleft terms
- have unusual redistribution conditions
- bundle non-code assets or runtimes
- affect release packaging obligations

If you add or change dependencies in a meaningful way, also update relevant documentation, including:

- `docs/THIRD_PARTY_LICENSES.md`
- any affected build or release documentation

## Security And Sensitive Content

Please do not include:

- secrets
- API keys
- private credentials
- proprietary datasets you do not have rights to share
- personally sensitive information

If you discover a security issue, prefer responsible disclosure over posting exploit details publicly in an issue.

## Scope And Direction

This project currently prioritizes:

- backend command behavior
- USB import/export correctness
- diagnostics and parity tooling
- local-first analysis and playback behavior
- stable, testable frontend flows

Contributions that fit this direction are more likely to be accepted quickly.

## Questions

If you are unsure whether a change is a good fit, open an issue or draft pull request and describe:

- the problem
- the proposed solution
- any tradeoffs or alternatives considered

That usually makes review much faster and reduces rework.

## Final Note

By submitting a pull request or other contribution to this repository, you agree to the contribution policy described above.
