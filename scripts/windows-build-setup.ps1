# DJ USB Tkit — Windows Build Setup
# Installs prerequisites and builds the app.
# Run from anywhere — the script finds the project root relative to itself.
# Safe to re-run: each step checks if the tool is already installed.
#
# Installs (if missing):
#   - Visual Studio Build Tools (system-wide, required by Rust)
#   - Rust via rustup (~/.cargo, modifies user PATH)
#   - Node.js portable (%LOCALAPPDATA%\nodejs, session PATH only)
#   - OpenSSL (%LOCALAPPDATA%\openssl, needed by SQLCipher)
#   - WebView2 runtime (usually pre-installed on Windows 10/11)
#
# Usage: Open PowerShell as Administrator, then run:
#   powershell -ExecutionPolicy Bypass -File Z:\path\to\windows-build-setup.ps1

$ErrorActionPreference = "Stop"

trap {
    Write-Host "`nERROR: $_" -ForegroundColor Red
    Write-Host "`nPress any key to exit..." -ForegroundColor DarkGray
    $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    exit 1
}

Write-Host "`n=== DJ USB Tkit Windows Build Setup ===" -ForegroundColor Cyan

# ─── Helper: reload PATH ────────────────────────────────────────────────────
function Reload-Path {
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
                [System.Environment]::GetEnvironmentVariable("Path", "User")
}

# ─── 1. Visual Studio Build Tools ───────────────────────────────────────────
Write-Host "`n[1/6] Checking Visual Studio Build Tools..." -ForegroundColor Yellow
$vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vsWhere) {
    $installed = & $vsWhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if ($installed) {
        Write-Host "  Already installed at: $installed" -ForegroundColor Green
    }
} else {
    $installed = $null
}

if (-not $installed) {
    Write-Host "  Downloading VS Build Tools..."
    $vsInstaller = "$env:TEMP\vs_BuildTools.exe"
    Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vs_BuildTools.exe" -OutFile $vsInstaller
    Write-Host "  Installing (this may take several minutes)..."
    Start-Process $vsInstaller -ArgumentList `
        '--quiet', '--wait', '--norestart',
        '--add', 'Microsoft.VisualStudio.Workload.VCTools',
        '--includeRecommended' -Wait
    Write-Host "  Done." -ForegroundColor Green
}

# ─── 2. Rust ────────────────────────────────────────────────────────────────
Write-Host "`n[2/6] Checking Rust..." -ForegroundColor Yellow
$cargoPath = "$env:USERPROFILE\.cargo\bin"
if (Test-Path $cargoPath) {
    $env:Path = "$cargoPath;$env:Path"
}
Reload-Path

# Check if rustc works (not just exists — toolchain must be installed)
$rustVer = $null
if (Get-Command rustc -ErrorAction SilentlyContinue) {
    try {
        $ErrorActionPreference = "Continue"
        $out = (& rustc --version 2>&1) | Out-String
        $ErrorActionPreference = "Stop"
        if ($out -match "^rustc \d") {
            $rustVer = $out.Trim()
        }
    } catch {
        $ErrorActionPreference = "Stop"
        $rustVer = $null
    }
}

if ($rustVer) {
    Write-Host "  Already installed: $rustVer" -ForegroundColor Green
} else {
    Write-Host "  Downloading rustup..."
    $rustupExe = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupExe
    Write-Host "  Installing Rust (this downloads ~300MB)..."
    # Run directly (not Start-Process) so it blocks until the toolchain is fully installed
    & $rustupExe -y --default-toolchain stable
    if (Test-Path $cargoPath) {
        $env:Path = "$cargoPath;$env:Path"
    }
    Reload-Path
    # Ensure toolchain is set even if rustup-init skipped it
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        Write-Host "  Ensuring stable toolchain..."
        & rustup toolchain install stable
        & rustup default stable
    }
    $rustVer = (rustc --version 2>$null)
    if ($rustVer) {
        Write-Host "  Installed: $rustVer" -ForegroundColor Green
    } else {
        Write-Host "  WARNING: rustc not working after install. You may need to restart PowerShell and re-run this script." -ForegroundColor Red
        exit 1
    }
}

# ─── 3. Node.js ─────────────────────────────────────────────────────────────
Write-Host "`n[3/6] Checking Node.js..." -ForegroundColor Yellow
Reload-Path
if (Get-Command node -ErrorAction SilentlyContinue) {
    $nodeVer = node --version
    Write-Host "  Already installed: $nodeVer" -ForegroundColor Green
} else {
    Write-Host "  Downloading Node.js LTS (portable zip)..."
    $nodeZip = "$env:TEMP\node-lts.zip"
    $nodePath = "$env:LOCALAPPDATA\nodejs"
    Invoke-WebRequest -Uri "https://nodejs.org/dist/v24.14.1/node-v24.14.1-win-x64.zip" -OutFile $nodeZip
    Write-Host "  Extracting to $nodePath..."
    if (Test-Path $nodePath) { Remove-Item $nodePath -Recurse -Force }
    Expand-Archive $nodeZip -DestinationPath "$env:TEMP\node-extract" -Force
    Move-Item "$env:TEMP\node-extract\node-v24.14.1-win-x64" $nodePath
    Remove-Item "$env:TEMP\node-extract" -Recurse -Force -ErrorAction SilentlyContinue
    # Add to PATH for this session only
    $env:Path = "$nodePath;$env:Path"
    Write-Host "  Installed: $(node --version)" -ForegroundColor Green
    Write-Host "  NOTE: To use node outside this script, add to your PATH: $nodePath" -ForegroundColor DarkGray
}

# ─── 4. OpenSSL (required by SQLCipher) ──────────────────────────────────────
Write-Host "`n[4/6] Checking OpenSSL..." -ForegroundColor Yellow
$opensslDir = "$env:ProgramFiles\OpenSSL-Win64"
if (Test-Path "$opensslDir\include\openssl\ssl.h") {
    Write-Host "  Already installed at: $opensslDir" -ForegroundColor Green
} else {
    Write-Host "  Downloading OpenSSL installer (slproweb.com)..."
    $opensslExe = "$env:TEMP\Win64OpenSSL.exe"
    Invoke-WebRequest -Uri "https://slproweb.com/download/Win64OpenSSL-3_6_1.exe" -OutFile $opensslExe
    Write-Host "  Installing to $opensslDir..."
    Start-Process $opensslExe -ArgumentList '/SILENT', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$opensslDir`"" -Wait
    Write-Host "  Done." -ForegroundColor Green
}
$env:OPENSSL_DIR = $opensslDir
# slproweb layout: lib\VC\x64\{MD,MDd,MT,MTd} — Rust uses MD (dynamic release)
$opensslLibDir = "$opensslDir\lib\VC\x64\MD"
if (-not (Test-Path $opensslLibDir)) { $opensslLibDir = "$opensslDir\lib\VC\x64" }
if (-not (Test-Path $opensslLibDir)) { $opensslLibDir = "$opensslDir\lib\VC" }
if (-not (Test-Path $opensslLibDir)) { $opensslLibDir = "$opensslDir\lib" }
$env:OPENSSL_LIB_DIR = $opensslLibDir
$env:OPENSSL_INCLUDE_DIR = "$opensslDir\include"
Write-Host "  OPENSSL_DIR=$env:OPENSSL_DIR" -ForegroundColor DarkGray
Write-Host "  OPENSSL_LIB_DIR=$env:OPENSSL_LIB_DIR" -ForegroundColor DarkGray
Write-Host "  OPENSSL_INCLUDE_DIR=$env:OPENSSL_INCLUDE_DIR" -ForegroundColor DarkGray

# ─── 5. WebView2 Runtime ────────────────────────────────────────────────────
Write-Host "`n[5/6] Checking WebView2 Runtime..." -ForegroundColor Yellow
$wv2Key = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
if (Test-Path $wv2Key) {
    $wv2Ver = (Get-ItemProperty $wv2Key -ErrorAction SilentlyContinue).pv
    Write-Host "  Already installed: $wv2Ver" -ForegroundColor Green
} else {
    Write-Host "  Downloading WebView2 bootstrapper..."
    $wv2Exe = "$env:TEMP\MicrosoftEdgeWebview2Setup.exe"
    Invoke-WebRequest -Uri "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile $wv2Exe
    Write-Host "  Installing..."
    Start-Process $wv2Exe -ArgumentList '/silent', '/install' -Wait
    Write-Host "  Done." -ForegroundColor Green
}

# ─── 6. Build ───────────────────────────────────────────────────────────────
Write-Host "`n[6/6] Building DJ USB Tkit..." -ForegroundColor Yellow
Reload-Path

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ProjectRoot = (Resolve-Path "$ScriptDir\..").Path

# Install frontend dependencies
Write-Host "  Installing frontend dependencies..."
Set-Location "$ProjectRoot\vanilla-ui"
npm ci
Set-Location "$ProjectRoot\desktop"
npm ci

# Build frontend dist
Write-Host "  Building frontend dist..."
Set-Location "$ProjectRoot\vanilla-ui"
npm run build

# Do not bundle Node runtime in release artifacts.
Write-Host "  Ensuring no bundled Node runtime is staged..."
$runtimeBinDir = "$ProjectRoot\desktop\runtime\bin"
$runtimeModulesDir = "$ProjectRoot\desktop\runtime\node_modules"
if (Test-Path $runtimeBinDir) { Remove-Item $runtimeBinDir -Recurse -Force }
if (Test-Path $runtimeModulesDir) { Remove-Item $runtimeModulesDir -Recurse -Force }

# Install Tauri CLI
Write-Host "  Installing Tauri CLI..."
cargo install tauri-cli --version '^2' --locked

# Clean cached build scripts so they pick up OpenSSL env vars
$buildDir = "$ProjectRoot\target\release\build"
if (Test-Path $buildDir) {
    Get-ChildItem "$buildDir" -Directory -Filter "libsqlite3-sys-*" -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
}

# Build with release config
Set-Location "$ProjectRoot\desktop\src-tauri"
$releaseConf = "$ProjectRoot\scripts\tauri.release.conf.json"
Write-Host "  Building release..."
cargo tauri build --config "$releaseConf" --bundles nsis,msi

# ─── Done ────────────────────────────────────────────────────────────────────
$bundleDir = "$ProjectRoot\target\release\bundle"
Write-Host "`n=== Build complete! ===" -ForegroundColor Green
Write-Host "Installers are in: $bundleDir" -ForegroundColor Cyan
Write-Host "  - MSI:  $bundleDir\msi"
Write-Host "  - NSIS: $bundleDir\nsis"
explorer.exe $bundleDir

Write-Host "`nPress any key to exit..." -ForegroundColor DarkGray
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
