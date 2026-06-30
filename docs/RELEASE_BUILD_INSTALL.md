# Release + Build/Install

## How it works

Development and release packaging run locally from this repository. Backend and frontend tests are part of the standard release flow, and desktop bundles are produced by the release script.

The standard release flow is:

1. Validate dependencies and platform prerequisites.
2. Run tests.
3. Build desktop bundles through Tauri.
4. Publish artifacts from the release bundle output directory.

## Deep technical details

Primary commands:

- Backend tests: `cargo test -q --manifest-path backend/Cargo.toml`
- Frontend tests: `npm test --prefix vanilla-ui`
- Frontend build: `npm run build --prefix vanilla-ui`
- Release script: `./scripts/release.sh`

Release pipeline behavior (`scripts/release.sh`):

- runs backend and frontend tests when `RUN_TESTS=1`
- builds desktop bundles from `desktop/src-tauri/`
- uses `scripts/tauri.release.conf.json` for release configuration

Linux note: AppImage builds require `linuxdeploy` available on `PATH`.

Runtime notes:

- default analysis engine is Stratum
- Essentia is optional and downloaded in-app when enabled
- default release artifacts do not bundle Node runtime
- app runtime does not require Node when using default Stratum analysis
- Node is required for source build/test workflows and for optional Essentia analysis runtime

The release script stages a clean frontend bundle and then runs Tauri packaging from the desktop host project. Build output is produced under `desktop/src-tauri/target/release/bundle`.

Release quality gates are controlled by environment flags:

- `RUN_TESTS=1` keeps backend/frontend tests in the release path
- `RUN_TESTS=0` skips test execution for packaging-only runs
- `BUNDLES=...` limits target bundle formats for focused builds

On Linux, AppImage packaging depends on host tooling (`linuxdeploy`) and system library compatibility. The project release flow also handles known strip-related issues in this environment through release-script defaults.

Operationally, this gives maintainers a repeatable local release path with explicit knobs for speed versus confidence, while keeping runtime policy clear: Stratum is default, Essentia remains optional, and Node runtime is not shipped in default artifacts.
