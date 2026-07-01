# Release + Build/Install

## How it works

Development and release packaging run locally from this repository. Backend and frontend tests are part of the standard release flow, and desktop bundles are produced by the release script.

The standard release flow is:

1. Validate dependencies and platform prerequisites.
2. Run tests.
3. Build desktop bundles through Tauri.
4. Publish artifacts from the release bundle output directory.

## GitHub Actions release

The `Release` workflow publishes GitHub Releases from existing `v*` tags.

To publish from CI:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds Linux (`deb`, `rpm`, `AppImage`), macOS (`dmg`), and
Windows (`nsis`, `msi`) bundles, uploads them as workflow artifacts, then
creates or updates the GitHub Release assets for the tag.

The workflow can also be run manually from GitHub Actions with an existing tag
name. Manual runs can be marked as draft releases or prereleases.

## Deep technical details

Primary commands:

- Backend tests: `cargo test -q --manifest-path backend/Cargo.toml`
- Frontend tests: `npm test --prefix vanilla-ui`
- Frontend build: `npm run build --prefix vanilla-ui`
- Release script (Linux): `./scripts/release.sh`
- Build setup (macOS): `./scripts/macos-build-setup.sh`
- Build setup (Windows): `powershell -ExecutionPolicy Bypass -File scripts\windows-build-setup.ps1`

Release pipeline behavior (`scripts/release.sh`):

- runs backend and frontend tests when `RUN_TESTS=1`
- installs the Playwright Chromium browser before frontend tests
- builds desktop bundles from `desktop/src-tauri/`
- uses `scripts/tauri.release.conf.json` for release configuration

macOS note: `scripts/macos-build-setup.sh` installs all prerequisites (Xcode Command Line Tools, Rust, Node.js) and builds the app. Safe to re-run.

Windows note: `scripts/windows-build-setup.ps1` installs all prerequisites (Visual Studio Build Tools, Rust, Node.js portable, OpenSSL, WebView2 runtime) and builds the app. Safe to re-run. Must be run from PowerShell as Administrator.

Linux note: AppImage builds require `linuxdeploy` available on `PATH`.

Runtime notes:

- default analysis engine is Stratum
- Essentia is optional and downloaded in-app when enabled
- default release artifacts do not bundle Node runtime
- app runtime does not require Node when using default Stratum analysis
- Node is required for source build/test workflows and for optional Essentia analysis runtime

The release script stages a clean frontend bundle and then runs Tauri packaging from the desktop host project. Build output is produced under `target/release/bundle`.

Release quality gates are controlled by environment flags:

- `RUN_TESTS=1` keeps backend/frontend tests in the release path
- `RUN_TESTS=0` skips test execution for packaging-only runs
- `BUNDLES=...` limits target bundle formats for focused builds

On Linux, AppImage packaging depends on host tooling (`linuxdeploy`) and system library compatibility. The project release flow also handles known strip-related issues in this environment through release-script defaults.

Operationally, this gives maintainers a repeatable local release path with explicit knobs for speed versus confidence, while keeping runtime policy clear: Stratum is default, Essentia remains optional, and Node runtime is not shipped in default artifacts.
