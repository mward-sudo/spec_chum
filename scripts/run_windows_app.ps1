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

& ./scripts/build_windows_app.ps1
Write-Host "==> launching spec_chum_windows"
& ./target/release/spec_chum_windows.exe @args
