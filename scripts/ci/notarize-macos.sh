#!/usr/bin/env bash
# Notarise and staple macOS release payloads when Apple notary secrets are present.
#
# Prefers App Store Connect API key auth (CI-friendly). Falls back to Apple ID +
# app-specific password + team ID. No-ops (exit 0) when neither credential set
# is complete — same soft-skip pattern as sign-macos.sh.
#
# Usage:
#   notarize-macos.sh <signed.dmg> [Spec-Chum.app...]
#
# Submit the outer .dmg (Gatekeeper-facing primary). After Accepted, staple the
# DMG and any optional .app paths (so the secondary release .zip stays offline-
# friendly). Requires prior Developer ID codesign (#354; umbrella #231).
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <signed.dmg> [Spec-Chum.app...]" >&2
  exit 2
fi

DMG="$1"
shift

have_api_key=0
if [[ -n "${APPLE_API_KEY_P8_BASE64:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
  have_api_key=1
fi

have_apple_id=0
if [[ -n "${APPLE_ID:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
  have_apple_id=1
fi

if [[ "$have_api_key" -eq 0 && "$have_apple_id" -eq 0 ]]; then
  echo "skip: Apple notary credentials not set; leaving $DMG unnotarised"
  echo "hint: set APPLE_API_KEY_P8_BASE64 + APPLE_API_KEY_ID + APPLE_API_ISSUER_ID"
  echo "      (preferred), or APPLE_ID + APPLE_APP_SPECIFIC_PASSWORD + APPLE_TEAM_ID"
  exit 0
fi

if [[ ! -f "$DMG" ]]; then
  echo "error: DMG not found: $DMG" >&2
  exit 1
fi

TMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/spec-chum-notary-$$"
mkdir -p "$TMP"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

AUTH_ARGS=()
if [[ "$have_api_key" -eq 1 ]]; then
  # Prefer API key even if Apple ID secrets are also present.
  P8="$TMP/AuthKey_${APPLE_API_KEY_ID}.p8"
  # macOS BSD base64 uses -D (GNU --decode is not available on runners).
  echo "$APPLE_API_KEY_P8_BASE64" | base64 -D >"$P8"
  chmod 600 "$P8"
  AUTH_ARGS=(--key "$P8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID")
  echo "notary auth: App Store Connect API key (id=$APPLE_API_KEY_ID)"
else
  AUTH_ARGS=(
    --apple-id "$APPLE_ID"
    --password "$APPLE_APP_SPECIFIC_PASSWORD"
    --team-id "$APPLE_TEAM_ID"
  )
  echo "notary auth: Apple ID ($APPLE_ID, team=$APPLE_TEAM_ID)"
fi

echo "notarytool submit $DMG --wait"
# --wait blocks until Accepted/Invalid; non-zero exit on failure.
xcrun notarytool submit "$DMG" "${AUTH_ARGS[@]}" --wait

echo "stapler staple $DMG"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

for path in "$@"; do
  if [[ ! -d "$path" || "$path" != *.app ]]; then
    echo "error: optional staple target must be a .app bundle: $path" >&2
    exit 1
  fi
  echo "stapler staple $path"
  # Ticket was published for the nested Mach-O when the DMG was accepted;
  # stapling the staged .app keeps the secondary .zip Gatekeeper-friendly offline.
  xcrun stapler staple "$path"
  xcrun stapler validate "$path"
done

echo "notarised and stapled: $DMG${*:+ $*}"
