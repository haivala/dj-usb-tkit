<p align="center">
  <img src="vanilla-ui/text-icon.svg" alt="DJ USB Tkit" width="320" />
</p>
# DJ USB Tkit

Local-first DJ library manager and USB exporter built with Rust + Tauri.

## App Requirements

- Local-first desktop workflows for library indexing, playlist management, USB import/export, diagnostics/repairs, and native playback.
- Fast prep flow: index library quickly, build playlists, analyze only missing tracks for the target playlist, then export.
- Analysis engine support: `stratum` (default) and optional `essentia`.
- Export sync modes: `mirror` (exact playlist sync) and `additive` (append missing tracks without removing existing members).
- Strict parity reporting is separate from operational passability so users can distinguish "works" vs "fully parity-clean" outcomes.

Detailed behavior and requirements are documented under `docs/`, starting with `docs/README.md`.

> **Warning:** This software writes to DJ USB drives and library databases. Use it at your own risk: the author and maintainers are not responsible for broken USB exports, corrupted databases, data loss, or other damage to your USB drive. Always keep your own backups.
>
> The app creates timestamped backups of existing PDB/eDB database files before export and repair writes, so you may be able to restore an earlier database state from `PIONEER/rekordbox/backups/`. The repair tools have also recovered broken USB database states in real use, but recovery is not guaranteed. This software is provided without warranty; see [LICENSE](LICENSE).

Project code is licensed under `MIT`; contributions are accepted under the same terms (inbound = outbound). See `CONTRIBUTING.md`.

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 24 LTS (required for frontend build/tests and optional Essentia runtime path)
- System libraries for Tauri v2 (see below)
- Tauri CLI v2 for release bundles:

```
cargo install tauri-cli --version '^2'
```

Ensure `~/.cargo/bin` is available in `PATH`.

### System Libraries

**Arch Linux:**

```
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg base-devel openssl pkg-config
```

**Ubuntu / Debian:**

```
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libappindicator3-dev librsvg2-dev build-essential libssl-dev pkg-config
```

**Fedora:**

```
sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel gcc gcc-c++ openssl-devel pkg-config
```

For other distros see the [Tauri prerequisites docs](https://v2.tauri.app/start/prerequisites/#linux).

## Build & Run

```
cd desktop/src-tauri
cargo build
cargo run
```

`cargo run` does not require Tauri CLI. Tauri CLI is required for release bundle builds (`tauri build`).

## Release Bundles

### macOS

Install prerequisites and build with:

```bash
chmod +x scripts/macos-build-setup.sh
./scripts/macos-build-setup.sh
```

### Windows

Open PowerShell as Administrator and run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows-build-setup.ps1
```

### Linux

Build installers/packages with:

```bash
./scripts/release.sh
```

The script stages a clean frontend bundle into `vanilla-ui/dist` (excluding development folders such as `node_modules`) before packaging.
On Linux, the default bundles are `deb,rpm,appimage`. AppImage packaging requires a working `linuxdeploy` binary on `PATH`.
When AppImage is included, the script also sets `NO_STRIP=1` by default to avoid linuxdeploy strip failures on RELR-enabled system libraries (common on Arch and newer distros). Set `LINUXDEPLOY_NO_STRIP=0` to disable.
Release builds do not bundle Essentia assets or a Node runtime.
The app works without Node when using prebuilt binaries and the default `stratum` analysis engine.
Node is only needed when building from source (frontend/tooling) or when enabling the optional Essentia analysis engine.
Essentia can be downloaded in-app from **Settings** when the user enables the Essentia engine.

Linux packaging notes:

- The AppImage bundles most GUI/runtime dependencies needed by the Tauri/WebKit stack (for example `webkit2gtk`, `gtk-3`, `libsoup`, `javascriptcoregtk`, `glib`, `gio`, `pango`, and `cairo`).
- Core system libraries remain host-provided, following normal AppImage practice. This includes libraries such as `libc`, `libm`, `libpthread`, `libstdc++`, `libX11`, and `libasound`.
- The Tauri UI does not require Node at runtime.

Examples:

```bash
./scripts/release.sh
BUNDLES=deb,rpm ./scripts/release.sh
```

Artifacts are written under:

```bash
target/release/bundle
```

For release build workflow and publication checks, see `docs/RELEASE_BUILD_INSTALL.md`.

## Project Structure

```
backend/       – Rust library: data model, SQLite storage, commands
desktop/       – Tauri host app
vanilla-ui/    – Frontend (vanilla HTML/JS/CSS)
```

## Current Capabilities

- Library scanning, playlist management, and native local playback.
- USB import/export with `mirror` and `additive` playlist sync modes.
- Local BPM, key, waveform, and artwork analysis for missing track metadata.
- USB diagnostics, strict parity reporting, and preview-first repair actions.
- Repair tools can help recover some broken USB database states; backups are
  created before repair writes, but recovery is not guaranteed.
- USB initialization for drives that are writable but missing the expected
  export database structure.
- Automated backend and frontend coverage for core workflows, export behavior,
  diagnostics, and UI interactions.

Backend behavior and app requirements are documented in the functional docs under `docs/`, with command reference details in `docs/COMMANDS.md`.
A redistribution-focused dependency/license audit is tracked in `docs/THIRD_PARTY_LICENSES.md`.
Contribution guidelines are documented in `CONTRIBUTING.md`.

## Donate

If you find this app useful, consider supporting the developer.

[Support DJ USB Tkit on chiph.art](https://chiph.art/en/dj-usb-tkit/support?utm_source=djtkit&utm_medium=readme&utm_campaign=support)
