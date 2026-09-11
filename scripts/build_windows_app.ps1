# Build the native Windows Spec Chum shell (Win32 + host_api). Refs #351.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

Write-Host "==> cargo build -p windows_shell --release"
cargo build -p windows_shell --release
Write-Host "==> OK: target/release/spec_chum_windows.exe"
