# Wgenty Code — Installer (Windows)
# Usage:
#   irm https://raw.githubusercontent.com/zhentingWu-wzt/wgenty-code/main/scripts/install.ps1 | iex
#   .\scripts\install.ps1                    # local
#   .\scripts\install.ps1 -Version v0.1.0    # specific version
#   .\scripts\install.ps1 -BuildFromSource   # force source build

param(
    [string]$Version = $env:WGENTY_VERSION ?? "latest",
    [switch]$BuildFromSource,
    [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"

$Repo      = "zhentingWu-wzt/wgenty-code"
$Binary    = "wgenty-code"
$BinaryExe = "$Binary.exe"

# ── helpers ──────────────────────────────────────────────────────────────────

function Header  { Write-Host ""; Write-Host "══ $args ══" -ForegroundColor Blue }
function Ok      { Write-Host "✓ $args"           -ForegroundColor Green }
function Warn    { Write-Host "⚠ $args"           -ForegroundColor Yellow }
function Err     { Write-Host "✗ $args"           -ForegroundColor Red }
function Info    { Write-Host "› $args"           -ForegroundColor Cyan }

# ── detect arch ──────────────────────────────────────────────────────────────

function Detect-Arch {
    $arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture) {
        "X64"  { "x86_64" }
        "Arm64" { "aarch64" }
        default { $null }
    }
    if (-not $arch) {
        Err "Unsupported architecture"
        exit 1
    }
    Ok "Platform: windows-${arch}"
    return $arch
}

# ── pick install dir ────────────────────────────────────────────────────────

function Pick-InstallDir {
    if ($InstallDir) {
        Ok "Install dir (user-specified): ${InstallDir}"
        return
    }
    $dir = Join-Path $env:LOCALAPPDATA "wgenty-code\bin"
    if (-not (Test-Path $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
    $script:InstallDir = $dir
    Ok "Install dir: ${InstallDir}"
}

# ── download release binary ─────────────────────────────────────────────────

function Download-Release {
    param([string]$Arch)

    $tag = $Version
    if ($tag -eq "latest") {
        Info "Resolving latest release..."
        try {
            $release = Invoke-RestMethod -Uri "https://api.github.com/repos/${Repo}/releases/latest"
            $tag = $release.tag_name
        } catch {
            Warn "No GitHub release found"
            return $false
        }
    }

    $asset  = "${Binary}-windows-${Arch}.exe"
    $url    = "https://github.com/${Repo}/releases/download/${tag}/${asset}"
    $dest   = Join-Path $InstallDir $BinaryExe

    Info "Downloading ${url} ..."
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
        Ok "Downloaded ${tag}"
        return $true
    } catch {
        Warn "Download failed: ${url}"
        return $false
    }
}

# ── build from source ───────────────────────────────────────────────────────

function Build-FromSource {
    $repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
    if (-not (Test-Path (Join-Path $repoRoot "Cargo.toml"))) {
        Err "Not a wgenty-code source tree: ${repoRoot}"
        exit 1
    }

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Err "Rust is not installed. Visit https://rustup.rs/"
        exit 1
    }

    Info "Building from source (this may take a few minutes)..."
    Push-Location $repoRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        Pop-Location
    }

    $src  = Join-Path $repoRoot "target\release" $BinaryExe
    $dest = Join-Path $InstallDir $BinaryExe
    Copy-Item $src $dest -Force
    Ok "Built & installed from source"
}

# ── ensure PATH ─────────────────────────────────────────────────────────────

function Ensure-Path {
    $current = [Environment]::GetEnvironmentVariable("Path", "User") ?? ""
    if ($current -notlike "*${InstallDir}*") {
        Warn "${InstallDir} is not in PATH"
        [Environment]::SetEnvironmentVariable("Path", "${InstallDir};${current}", "User")
        $env:Path = "${InstallDir};$env:Path"
        Ok "Added to user PATH (restart your terminal to take effect)"
    } else {
        Ok "PATH includes ${InstallDir}"
    }
}

# ── verify ───────────────────────────────────────────────────────────────────

function Verify-Install {
    Header "Verify"
    $exe = Join-Path $InstallDir $BinaryExe
    try {
        $ver = & $exe --version 2>&1
        Ok $ver
    } catch {
        Err "Verification failed — binary does not run"
        exit 1
    }
    Write-Host ""
    Info "Try: ${Binary} repl"
    Info "     ${Binary} --help"
}

# ── main ─────────────────────────────────────────────────────────────────────

Header "Wgenty Code Installer"

$arch = Detect-Arch
Pick-InstallDir

if ($BuildFromSource) {
    Build-FromSource
} elseif (-not (Download-Release -Arch $arch)) {
    Warn "Falling back to source build..."
    Build-FromSource
}

Ensure-Path
Verify-Install

Write-Host ""
Ok "Done — wgenty-code is ready!"
