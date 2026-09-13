#!/usr/bin/env bash
# Build and run the optional GTK4 Linux shell (#351).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "run_linux_shell.sh: GTK4 shell requires Linux (see docs/LINUX_NATIVE.md)." >&2
  echo "On this host: cargo run -p app --release" >&2
  exit 1
fi

cargo run -p linux_shell --release "$@"
