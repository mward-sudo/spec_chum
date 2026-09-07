# Build the Spec Chum Inno Setup installer from a staged release tree.
# Usage:
#   build-windows-installer.ps1 <version> <stage-dir> <output-setup.exe>
#
# Requires Inno Setup 6 (ISCC.exe). Release CI installs it via Chocolatey.
# Signing the resulting setup.exe is separate (sign-windows.ps1).
$ErrorActionPreference = "Stop"

if ($args.Count -ne 3) {
    throw "usage: build-windows-installer.ps1 <version> <stage-dir> <output-setup.exe>"
}

$Version = [string]$args[0]
$StageDir = (Resolve-Path -LiteralPath ([string]$args[1])).Path
$OutputExe = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath([string]$args[2])

if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]*$') {
    throw "refusing unsafe version string for Inno defines: $Version"
}

$exe = Join-Path $StageDir "spec_chum.exe"
$license = Join-Path $StageDir "LICENSE"
$readme = Join-Path $StageDir "README.txt"
foreach ($required in @($exe, $license, $readme)) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "staged release tree missing required file: $required"
    }
}

$isccCandidates = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)
$iscc = $isccCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $iscc) {
    throw "ISCC.exe not found (install Inno Setup 6)"
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
$iss = Join-Path $repoRoot "packaging\windows\spec-chum.iss"
if (-not (Test-Path -LiteralPath $iss)) {
    throw "Inno script not found: $iss"
}

$outDir = Split-Path -Parent $OutputExe
$outBase = [IO.Path]::GetFileNameWithoutExtension($OutputExe)
if ([string]::IsNullOrWhiteSpace($outDir)) {
    throw "output path must include a directory: $OutputExe"
}
if ([IO.Path]::GetExtension($OutputExe) -ne ".exe") {
    throw "output path must end with .exe: $OutputExe"
}
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outDirFull = (Resolve-Path -LiteralPath $outDir).Path

# Inno writes OutputDir\OutputBaseFilename.exe — remove a stale file first.
$expected = Join-Path $outDirFull ($outBase + ".exe")
if (Test-Path -LiteralPath $expected) {
    Remove-Item -LiteralPath $expected -Force
}

Write-Host "ISCC version=$Version stage=$StageDir out=$expected"
& $iscc `
    "/DMyAppVersion=$Version" `
    "/DStageDir=$StageDir" `
    "/DOutputDir=$outDirFull" `
    "/DOutputBase=$outBase" `
    $iss
if ($LASTEXITCODE -ne 0) {
    throw "ISCC failed with exit $LASTEXITCODE"
}

if (-not (Test-Path -LiteralPath $expected)) {
    throw "ISCC did not produce expected installer: $expected"
}

Write-Host "created $expected"
