# Wgenty Code — Windows installer (PowerShell)
# Usage: irm https://raw.githubusercontent.com/zhentingWu-wzt/wgenty-code/master/install.ps1 | iex

param(
    [string]$InstallDir = "$env:USERPROFILE\.wgenty-code\bin"
)

$ErrorActionPreference = "Stop"
$RepoUrl = "https://github.com/zhentingWu-wzt/wgenty-code.git"
$TempDir = "$env:TEMP\wgenty-code-install-$(Get-Random)"

Write-Host "==> Cloning wgenty-code..." -ForegroundColor Cyan
git clone --depth 1 $RepoUrl $TempDir
Set-Location $TempDir

Write-Host "==> Building wgenty-code (release)..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Build failed" -ForegroundColor Red
    exit 1
}

Write-Host "==> Installing to $InstallDir..." -ForegroundColor Cyan
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item ".\target\release\wgenty-code.exe" "$InstallDir\wgenty-code.exe"

# Add to PATH
$currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if (-not $currentPath.Contains($InstallDir)) {
    [Environment]::SetEnvironmentVariable("PATH", "$currentPath;$InstallDir", "User")
    Write-Host "==> Added $InstallDir to User PATH" -ForegroundColor Green
}

# Cleanup
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "==> Installation complete!" -ForegroundColor Green
Write-Host "    Restart your terminal, then run:"
Write-Host "    wgenty-code --version"
Write-Host ""
Write-Host "    Set your API key: `$env:ANTHROPIC_API_KEY = ""sk-ant-...""`" -ForegroundColor Yellow
