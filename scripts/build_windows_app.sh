#!/usr/bin/env bash
# Build the native Windows Spec Chum shell (Win32 + host_api).
# Must run on Windows (MSVC). On other hosts, prefer cargo test -p windows_shell.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != MINGW* && "$(uname -s)" != MSYS* && "$(uname -s)" != CYGWIN* && "$(uname -s)" != Windows_NT ]]; then
  if [[ "${OS:-}" != "Windows_NT" ]]; then
    echo "error: build_windows_app.sh is intended for Windows hosts." >&2
    echo "On this machine: cargo test -p windows_shell  (keymap + stub)" >&2
    echo "See docs/WINDOWS_NATIVE.md (#351)." >&2
    exit 1
  fi
fi

echo "==> cargo build -p windows_shell --release"
cargo build -p windows_shell --release
echo "==> OK: target/release/spec_chum_windows.exe"
