#!/usr/bin/env bash
# Scoped quality gate: debug-build/clippy/test only the crates needed for the
# current task. Prefer this while iterating; use ./scripts/check.sh before merge.
#
# Usage:
#   ./scripts/check_crates.sh                  # infer crates from git diff vs origin/main
#   ./scripts/check_crates.sh z80 machine      # explicit crates
#   ./scripts/check_crates.sh --base HEAD~3    # diff base for inference
#
# living_room is never debug-built here (Bevy debug is multi‑GB). If living_room
# is in scope, this script runs ./scripts/check_living_room.sh (release by default).
set -euo pipefail
cd "$(dirname "$0")/.."

BASE="${SPEC_CHUM_CHECK_BASE:-origin/main}"
CRATES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      BASE="${2:?--base requires a ref}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      CRATES+=("$1")
      shift
      ;;
  esac
done

# Print unique crate names inferred from changed paths (one per line).
infer_crates() {
  {
    git diff --name-only "${BASE}...HEAD" 2>/dev/null || true
    git diff --name-only 2>/dev/null || true
    git diff --name-only --cached 2>/dev/null || true
    git ls-files --others --exclude-standard 2>/dev/null || true
  } | while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    case "$f" in
      Cargo.toml|Cargo.lock|scripts/check.sh|scripts/check_crates.sh)
        echo "WORKSPACE_ROOT"
        ;;
      scripts/check_living_room.sh)
        echo "living_room"
        ;;
      crates/*/Cargo.toml|crates/*/src/*|crates/*/tests/*|crates/*/examples/*|crates/*/benches/*|crates/*/include/*)
        pkg="${f#crates/}"
        echo "${pkg%%/*}"
        ;;
      apps/macos/*)
        # Native shell links living_room release staticlib — gate those crates.
        echo "living_room"
        echo "host_api"
        ;;
    esac
  done | sort -u
}

if [[ ${#CRATES[@]} -eq 0 ]]; then
  inferred="$(infer_crates)"
  if printf '%s\n' "$inferred" | grep -qx 'WORKSPACE_ROOT'; then
    echo "Workspace root touched — run ./scripts/check.sh instead of scoped check." >&2
    exit 2
  fi
  # shellcheck disable=SC2207
  CRATES=($(printf '%s\n' "$inferred" | grep -v '^$' || true))
  if [[ ${#CRATES[@]} -eq 0 ]]; then
    echo "No crate changes inferred vs ${BASE}; pass crate names explicitly." >&2
    echo "Example: ./scripts/check_crates.sh control_plane agent_server" >&2
    exit 1
  fi
  echo "==> inferred crates: ${CRATES[*]} (base ${BASE})"
else
  echo "==> crates: ${CRATES[*]}"
fi

NEED_LIVING_ROOM=0
PKG_ARGS=()
for pkg in "${CRATES[@]}"; do
  if [[ "$pkg" == "living_room" ]]; then
    NEED_LIVING_ROOM=1
    continue
  fi
  PKG_ARGS+=(-p "$pkg")
done

export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"

if [[ ${#PKG_ARGS[@]} -gt 0 ]]; then
  echo "==> cargo fmt (scoped)"
  cargo fmt "${PKG_ARGS[@]}" -- --check

  echo "==> cargo clippy (debug, scoped)"
  cargo clippy "${PKG_ARGS[@]}" --all-targets -- -D warnings

  echo "==> cargo test (debug, scoped)"
  cargo test "${PKG_ARGS[@]}"
fi

if [[ "$NEED_LIVING_ROOM" -eq 1 ]]; then
  echo "==> living_room in scope — release gate (set SPEC_CHUM_ROOM_DEBUG=1 for debug Bevy)"
  ./scripts/check_living_room.sh
fi

echo "==> OK (scoped)"
