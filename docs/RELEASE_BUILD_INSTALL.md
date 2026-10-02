# Release + Build/Install

## How it works

Development and release packaging run locally from this repository. Backend and frontend tests are part of the standard release flow, and desktop bundles are produced by the release script.

The standard release flow is:

1. Validate dependencies and platform prerequisites.
2. Set the workspace version and sync package metadata.
3. Run tests.
4. Build desktop bundles through Tauri.
5. Publish artifacts from the release bundle output directory.

## GitHub Actions release

The `Release` workflow publishes GitHub Releases from existing `v*` tags.

To publish from CI:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds Linux (`deb`, `rpm`, `AppImage`), macOS (`dmg`, plus the
`.app.tar.gz` the updater installs), and Windows (`nsis`) bundles, uploads them
as workflow artifacts, then creates or updates the GitHub Release assets for
the tag. MSI is no longer built: it needs admin rights to install and update,
while the NSIS setup installs per-user.

### In-app updater

AppImage, NSIS and macOS installs update themselves from the banner's
"Update & restart" button (`install_update` in `backend/src/tauri_commands.rs`,
`tauri-plugin-updater`). deb/rpm installs belong to the package manager and only
get a direct download link to their asset.

Updates are verified with a minisign key (free, not a code-signing
certificate). One-time setup:

1. `npx --prefix desktop tauri signer generate -w ~/.tauri/djtkit.key`
2. Put the public key (`~/.tauri/djtkit.key.pub`, one line) in
   `desktop/src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
3. Add repository secrets `TAURI_SIGNING_PRIVATE_KEY` (contents of
   `~/.tauri/djtkit.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
4. Back up the private key. Without it, installed apps can never update
   themselves again: a new key means a new pubkey, which old builds don't trust.

With the secrets set, `scripts/release.sh` and `scripts/linux-release-docker.sh`
add `scripts/tauri.updater.conf.json` (`createUpdaterArtifacts`), so every
updater artifact gets a `.sig`. The publish job runs
`scripts/make_updater_manifest.mjs`, which:

- renames assets with spaces (GitHub would turn them into dots);
- checks that each signature is bound to the release version (the app sets
  `requireSignedVersion`);
- writes `latest.json` with only installer-specific keys
  (`linux-x86_64-appimage`, `windows-x86_64-nsis`, `darwin-aarch64-app`).

The app fetches the manifest from
`releases/latest/download/latest.json`, so drafts and prereleases are never
offered. Builds without the key (local builds, or CI before the secrets
exist) skip all of this and publish as before.

The workflow can also be run manually from GitHub Actions with an existing tag
name. Manual runs can be marked as draft releases or prereleases.

## Deep technical details

Primary commands:

- Version sync after editing root `Cargo.toml`: `node scripts/sync_versions.mjs`
- Backend tests: `cargo test -q --manifest-path backend/Cargo.toml`
- Frontend tests: `npm test --prefix vanilla-ui`
- Frontend build: `npm run build --prefix vanilla-ui`
- Docs screenshots and GIFs (`docs/assets/`): `npm run docs:media --prefix vanilla-ui` (needs cargo and ffmpeg). It synthesizes a 15-track demo library, analyzes it with the real backend and records every screenshot and GIF the docs use from the real frontend build; `-- <name>` records only those (e.g. `-- backup-view`). See `vanilla-ui/scripts/doc-media/record.mjs`.
- Release script (Linux): `./scripts/release.sh`
- Build setup (macOS): `./scripts/macos-build-setup.sh`
- Build setup (Windows): `powershell -ExecutionPolicy Bypass -File scripts\windows-build-setup.ps1`

Release pipeline behavior (`scripts/release.sh`):

- syncs `desktop` and `vanilla-ui` package versions from the root Cargo workspace
- runs backend and frontend tests when `RUN_TESTS=1`
- installs the Playwright Chromium browser before frontend tests
- builds desktop bundles from `desktop/src-tauri/`
- uses `scripts/tauri.release.conf.json` for release configuration

macOS note: `scripts/macos-build-setup.sh` installs all prerequisites (Xcode Command Line Tools, Rust, Node.js) and builds the app. Safe to re-run.

Windows note: `scripts/windows-build-setup.ps1` installs all prerequisites (Visual Studio Build Tools, Rust, Node.js portable, OpenSSL, WebView2 runtime) and builds the app. Safe to re-run. Must be run from PowerShell as Administrator.

Linux note: AppImage builds require `linuxdeploy` available on `PATH`.

Linux release builds (`scripts/linux-release-docker.sh`, also used by the release workflow) run in an Ubuntu 22.04 container (`scripts/Dockerfile.linux-build`). The container sets the glibc floor: the AppImage runs on glibc 2.35 or newer. Move the base image up only when 22.04 leaves standard support (April 2027).

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
