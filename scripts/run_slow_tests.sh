#!/usr/bin/env bash
# Full slow accuracy suite — required before cutting a vX.Y.Z release.
# Not part of ./scripts/check.sh or default PR CI.
#
# Covers:
#   1. z80doc     (--features slow-tests; fixture in git)
#   2. system-tests (--features system-tests; fetched TAPs)
#   3. z80full    (--features slow-tests, #[ignore]; fetched via fetch_z80test.sh)
#
# See docs/RELEASE.md.
set -euo pipefail
cd "$(dirname "$0")/.."

./scripts/fetch_roms.sh

echo "==> z80doc (slow-tests, release)"
cargo test -p machine --features slow-tests --release z80doc_all_tests_passed -- --nocapture

echo "==> system-tests + z80full"
SYSTEM_TESTS_Z80FULL=1 ./scripts/run_system_tests.sh

echo "==> full slow suite OK"
