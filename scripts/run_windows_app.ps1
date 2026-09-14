# Run the native Windows Spec Chum shell. Refs #351.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

$rom48 = Join-Path "roms" "spec48.rom"
if (-not (Test-Path $rom48)) {
    Write-Host "==> fetching ROMs"
    $bash = Get-Command bash -ErrorAction SilentlyContinue
    if (-not $bash) {
        Write-Error @"
roms/spec48.rom is missing and 'bash' was not found on PATH.
Install Git for Windows (includes bash), then re-run this script, or fetch ROMs manually:
  ./scripts/fetch_roms.sh
"@
        exit 1
    }
    & bash ./scripts/fetch_roms.sh
    if ($LASTEXITCODE -ne 0) {
        Write-Error "fetch_roms.sh failed (exit $LASTEXITCODE)"
        exit $LASTEXITCODE
    }
    if (-not (Test-Path $rom48)) {
        Write-Error "fetch_roms.sh finished but roms/spec48.rom is still missing"
        exit 1
    }
}

# Repo root is the parent of scripts/; ROM search joins roms/ under this.
$env:SPEC_CHUM_ROOT = (Get-Location).Path

& ./scripts/build_windows_app.ps1
Write-Host "==> launching spec_chum_windows"
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path (Get-Location) "target" }
$exe = Join-Path $targetDir "release/spec_chum_windows.exe"
if (-not (Test-Path $exe)) {
    # Workspace with per-crate target subdir (e.g. CARGO_TARGET_DIR=C:\cargo-target\spec_chum).
    $alt = Join-Path $targetDir "spec_chum/release/spec_chum_windows.exe"
    if (Test-Path $alt) { $exe = $alt }
}
if (-not (Test-Path $exe)) {
    Write-Error "spec_chum_windows.exe not found under $targetDir (set CARGO_TARGET_DIR if using a custom target)"
    exit 1
}
& $exe @args
