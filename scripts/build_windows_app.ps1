# Build the native Windows Spec Chum shell (Win32 + host_api).
# Release primary on Windows (#351); staged as spec_chum.exe in GitHub Releases.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

Write-Host "==> cargo build -p windows_shell --release"
cargo build -p windows_shell --release
Write-Host "==> OK: target/release/spec_chum_windows.exe (release packages rename to spec_chum.exe)"
