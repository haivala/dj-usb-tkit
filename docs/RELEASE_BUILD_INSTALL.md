# Release + Build/Install

## Building from source

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 24 LTS (24.15 or newer) or 26 (required for frontend build/tests; the optional Essentia engine uses whichever `node` is installed)
- System libraries for Tauri v2 (see below)
- [CMake](https://cmake.org/) (builds the bundled Opus decoder)
- Tauri CLI v2 for release bundles:

```
cargo install tauri-cli --version '^2'
```

Ensure `~/.cargo/bin` is available in `PATH`.

### System libraries

**Arch Linux:**

```
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg base-devel openssl pkg-config cmake
```

**Ubuntu / Debian:**

```
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libappindicator3-dev librsvg2-dev build-essential libssl-dev pkg-config cmake
```

**Fedora:**

```
sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel gcc gcc-c++ openssl-devel pkg-config cmake
```

For other distros see the [Tauri prerequisites docs](https://v2.tauri.app/start/prerequisites/#linux).

### Build & run

```
cd desktop/src-tauri
cargo build
cargo run
```

`cargo run` does not require Tauri CLI. Tauri CLI is required for release bundle builds (`tauri build`).

### Release bundles

Published builds are available on the [Releases](https://github.com/haivala/dj-usb-tkit/releases) page. To build your own:

**macOS:** install prerequisites and build with:

```bash
chmod +x scripts/macos-build-setup.sh
./scripts/macos-build-setup.sh
```

**Windows:** open PowerShell as Administrator and run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows-build-setup.ps1
```

**Linux:** build installers/packages with:

```bash
./scripts/release.sh
BUNDLES=deb,rpm ./scripts/release.sh
```

Artifacts are written under `target/release/bundle`.

The script stages a clean frontend bundle into `vanilla-ui/dist` (excluding development folders such as `node_modules`) before packaging.
On Linux, the default bundles are `deb,rpm,appimage`. AppImage packaging requires a working `linuxdeploy` binary on `PATH`.
When AppImage is included, the script also sets `NO_STRIP=1` by default to avoid linuxdeploy strip failures on RELR-enabled system libraries (common on Arch and newer distros). Set `LINUXDEPLOY_NO_STRIP=0` to disable.
Release builds do not bundle Essentia assets or a Node runtime: the app works without Node with the default `stratum` analysis engine, and Essentia can be downloaded in-app from **Settings** when the user enables it.

Linux packaging notes:

- The AppImage bundles most GUI/runtime dependencies needed by the Tauri/WebKit stack (for example `webkit2gtk`, `gtk-3`, `libsoup`, `javascriptcoregtk`, `glib`, `gio`, `pango`, and `cairo`).
- Core system libraries remain host-provided, following normal AppImage practice. This includes libraries such as `libc`, `libm`, `libpthread`, `libstdc++`, `libX11`, and `libasound`.
- Published release builds are made in an Ubuntu 22.04 container (`scripts/linux-release-docker.sh`), so they need glibc 2.35 or newer. `./scripts/release.sh` builds against your own system's glibc, so its AppImage only runs on systems at least as new as yours. Use `scripts/linux-release-docker.sh` for a portable build.
- The AppImage, deb, and rpm include AppStream metadata (`usr/share/metainfo/art.chiph.djusbtkit.metainfo.xml`) for software centers and AppImage managers.

### Project structure

```
backend/       – Rust library: data model, SQLite storage, commands
desktop/       – Tauri host app
vanilla-ui/    – Frontend (vanilla HTML/JS/CSS)
```

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

Pushes to `main` that change what the caches hold (`Cargo.lock`, a
`Cargo.toml`, a `package-lock.json`, `.cargo/`, the Linux build image or the
workflow itself) also run the platform builds to populate caches in the
default branch's scope. Ordinary code pushes do not. A version bump changes
`Cargo.lock`, so it triggers one such build: let it finish before pushing the
tag. These runs do not upload bundle artifacts or publish releases. GitHub lets tag builds restore default-branch caches, but
not caches from earlier tags. The first `main` build starts cold; later tag
builds can reuse it when their toolchain and cache keys match. Tag builds
only restore caches and never save them: a cache saved from a tag would be
usable by that tag alone and would take room from the `main` caches in the
repository's cache quota. A tag pushed together with its commit restores the
cache of the previous `main` build, because the two runs start at the same
time.

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
"Update & restart" button (`install_update` in `backend/src/tauri_commands/updates.rs`,
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
- Docs screenshots and animations (`docs/assets/`): `npm run docs:media --prefix vanilla-ui` (needs cargo and ffmpeg). It synthesizes a 15-track demo library, analyzes it with the real backend and records every screenshot and animation (animated WebP) the docs use from the real frontend build; `-- <name>` records only those (e.g. `-- backup-view`). See `vanilla-ui/scripts/doc-media/record.mjs`.
- Release script (Linux): `./scripts/release.sh`
- Build setup (macOS): `./scripts/macos-build-setup.sh`
- Build setup (Windows): `powershell -ExecutionPolicy Bypass -File scripts\windows-build-setup.ps1`

Release pipeline behavior (`scripts/release.sh`):

- syncs `desktop` and `vanilla-ui` package versions from the root Cargo workspace
- runs backend and frontend tests when `RUN_TESTS=1`
- installs the Playwright Chromium browser before frontend tests
- builds desktop bundles from `desktop/src-tauri/`
- uses `scripts/tauri.release.conf.json` for release configuration

macOS note: `scripts/macos-build-setup.sh` installs all prerequisites (Xcode Command Line Tools, CMake, Rust, Node.js) and builds the app. Safe to re-run.

Windows note: `scripts/windows-build-setup.ps1` installs all prerequisites (Visual Studio Build Tools, CMake, Rust, Node.js portable, OpenSSL, WebView2 runtime) and builds the app. Safe to re-run. Must be run from PowerShell as Administrator.

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
