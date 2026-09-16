#!/usr/bin/env bash
# Remove in-repo rebuildable build dirs (Cargo `target/`, SwiftPM `.build`).
#
# Safe to run when External SSD caches are active — refuses to delete anything
# that resolves onto `/Volumes/External SSD`. Does **not** touch `~/.cargo`,
# `~/.rustup`, source, assets, `graphify-out/`, or `.rom-cache/`.
#
# Usage (from repo root or any cwd):
#   ./scripts/purge_local_build_artifacts.sh
#   source scripts/dev_env.sh   # may call this when SSD is mounted
set -euo pipefail

ROOT="${SPEC_CHUM_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)}"
SSD_VOLUME="/Volumes/External SSD"

# Return 0 if $1 exists and its canonical path is under the External SSD volume.
_on_external_ssd() {
  local path="$1"
  local resolved
  if [[ ! -e "$path" ]]; then
    return 1
  fi
  resolved="$(cd "$path" 2>/dev/null && pwd -P)" || resolved="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$path" 2>/dev/null || true)"
  [[ -n "$resolved" && "$resolved" == "$SSD_VOLUME"/* ]]
}

_purge_one() {
  local path="$1"
  local label="$2"
  if [[ ! -e "$path" ]]; then
    return 0
  fi
  if _on_external_ssd "$path"; then
    echo "purge: skip $label (resolves onto External SSD)" >&2
    return 0
  fi
  # Extra guard: never rm if path string itself points at the volume.
  case "$path" in
    "$SSD_VOLUME"|"$SSD_VOLUME"/*)
      echo "purge: refuse $label (path on External SSD)" >&2
      return 0
      ;;
  esac
  echo "purge: removing $label ($path)" >&2
  rm -rf "$path"
}

_purge_one "$ROOT/target" "\$REPO/target"
_purge_one "$ROOT/apps/macos/.build" "\$REPO/apps/macos/.build"
