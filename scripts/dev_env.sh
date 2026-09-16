#!/usr/bin/env bash
# Spec Chum developer environment — offload Cargo/Swift caches to External SSD
# when mounted (frees internal disk). Safe to `source` repeatedly (idempotent).
#
# When `/Volumes/External SSD` is present and writable:
#   export CARGO_TARGET_DIR, SPEC_CHUM_HOST_LIB_DIR, SPEC_CHUM_SWIFT_SCRATCH
#   mkdir cache dirs; purge in-repo target/ and apps/macos/.build (if local)
#
# When absent: unset those vars so builds use in-repo defaults.
#
# Usage:
#   source /path/to/spec_chum/scripts/dev_env.sh
#   # or from repo:  source scripts/dev_env.sh
#
# Optional: RUSTC_WRAPPER=sccache if sccache is on PATH (cache dir created either way).
#
# Do not edit plan files; do not delete ~/.cargo, ~/.rustup, graphify-out, .rom-cache.

# Resolve this script's directory (bash + zsh, sourced or executed).
_spec_chum_dev_env_self=""
if [[ -n "${BASH_SOURCE[0]:-}" ]]; then
  _spec_chum_dev_env_self="${BASH_SOURCE[0]}"
elif [[ -n "${ZSH_VERSION:-}" ]]; then
  # zsh: %x is the file being sourced/executed
  # shellcheck disable=SC2296
  _spec_chum_dev_env_self="${(%):-%x}"
else
  _spec_chum_dev_env_self="$0"
fi
_SPEC_CHUM_REPO="$(cd "$(dirname "$_spec_chum_dev_env_self")/.." && pwd)"
unset _spec_chum_dev_env_self

_SPEC_CHUM_SSD_VOLUME="/Volumes/External SSD"
_SPEC_CHUM_SSD_CACHE="${_SPEC_CHUM_SSD_VOLUME}/DeveloperCaches/spec_chum"

_spec_chum_path_on_ssd() {
  local path="$1"
  local resolved=""
  [[ -e "$path" ]] || return 1
  if resolved="$(cd "$path" 2>/dev/null && pwd -P)"; then
    :
  else
    resolved="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$path" 2>/dev/null || true)"
  fi
  [[ -n "$resolved" && "$resolved" == "$_SPEC_CHUM_SSD_VOLUME"/* ]]
}

_spec_chum_ssd_usable() {
  [[ -d "$_SPEC_CHUM_SSD_VOLUME" && -w "$_SPEC_CHUM_SSD_VOLUME" ]]
}

if _spec_chum_ssd_usable; then
  mkdir -p \
    "$_SPEC_CHUM_SSD_CACHE/cargo-target" \
    "$_SPEC_CHUM_SSD_CACHE/swift-build" \
    "$_SPEC_CHUM_SSD_CACHE/sccache"

  export CARGO_TARGET_DIR="$_SPEC_CHUM_SSD_CACHE/cargo-target"
  # Default host lib dir for SpecChumMac linker (native release layout).
  # Cross-target / SPEC_CHUM_MAC_TARGET builds may override SPEC_CHUM_HOST_LIB_DIR.
  export SPEC_CHUM_HOST_LIB_DIR="${CARGO_TARGET_DIR}/release"
  export SPEC_CHUM_SWIFT_SCRATCH="$_SPEC_CHUM_SSD_CACHE/swift-build"

  # Optional sccache: only set wrapper if binary exists and user did not already set one.
  if command -v sccache >/dev/null 2>&1; then
    export SCCACHE_DIR="${SCCACHE_DIR:-$_SPEC_CHUM_SSD_CACHE/sccache}"
    if [[ -z "${RUSTC_WRAPPER:-}" ]]; then
      export RUSTC_WRAPPER="$(command -v sccache)"
    fi
  fi

  # Drop local rebuildables so they do not reclaim internal SSD space.
  if [[ -f "$_SPEC_CHUM_REPO/scripts/purge_local_build_artifacts.sh" ]]; then
    # shellcheck source=/dev/null
    bash "$_SPEC_CHUM_REPO/scripts/purge_local_build_artifacts.sh" || true
  else
    for _p in "$_SPEC_CHUM_REPO/target" "$_SPEC_CHUM_REPO/apps/macos/.build"; do
      if [[ -e "$_p" ]] && ! _spec_chum_path_on_ssd "$_p"; then
        case "$_p" in
          "$_SPEC_CHUM_SSD_VOLUME"|"$_SPEC_CHUM_SSD_VOLUME"/*) ;;
          *) rm -rf "$_p" ;;
        esac
      fi
    done
    unset _p
  fi

  echo "dev_env: using SSD caches under $_SPEC_CHUM_SSD_CACHE" >&2
else
  unset CARGO_TARGET_DIR
  unset SPEC_CHUM_HOST_LIB_DIR
  unset SPEC_CHUM_SWIFT_SCRATCH
  # Leave SCCACHE_DIR / RUSTC_WRAPPER alone if the user set them elsewhere;
  # only clear wrapper when it pointed at our previous SSD layout.
  if [[ "${SCCACHE_DIR:-}" == "$_SPEC_CHUM_SSD_CACHE/sccache" ]]; then
    unset SCCACHE_DIR
  fi
  if [[ "${RUSTC_WRAPPER:-}" == *sccache* && ! -d "$_SPEC_CHUM_SSD_VOLUME" ]]; then
    # Do not unset a user-global sccache; only note local fallback.
    :
  fi
  echo "dev_env: SSD not mounted — local caches (\$REPO/target, apps/macos/.build)" >&2
fi

unset _SPEC_CHUM_REPO _SPEC_CHUM_SSD_VOLUME _SPEC_CHUM_SSD_CACHE
unset -f _spec_chum_path_on_ssd _spec_chum_ssd_usable 2>/dev/null || true
