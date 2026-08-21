#!/usr/bin/env bash
# Optional, slow, third-party machine tests (ULA / ROM boot / 128K / +3).
# Not part of ./scripts/check.sh or default CI.
# TAP fixtures are cached under .rom-cache/system-tests/ and are not in git.
set -euo pipefail
cd "$(dirname "$0")/.."

./scripts/fetch_roms.sh
./scripts/fetch_system_tests.sh

echo "==> system-tests (release)"
cargo test -p machine --features system-tests --release system_tests -- --nocapture

if [[ "${SYSTEM_TESTS_Z80FULL:-0}" == 1 ]]; then
  echo "==> z80full (CPU suite under slow-tests)"
  ./scripts/fetch_z80test.sh
  cargo test -p machine --features slow-tests --release z80full_all_tests_passed -- --nocapture
fi
