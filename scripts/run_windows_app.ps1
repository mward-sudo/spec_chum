# Run the native Windows Spec Chum shell. Refs #351.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

if (-not (Test-Path "roms")) {
    Write-Host "==> fetching ROMs"
    bash ./scripts/fetch_roms.sh
}

& ./scripts/build_windows_app.ps1
Write-Host "==> launching spec_chum_windows"
& ./target/release/spec_chum_windows.exe @args
