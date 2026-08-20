#!/usr/bin/env bash
# Codesign macOS CLI binaries when Apple Developer ID secrets are present.
# No-ops (exit 0) when APPLE_CERTIFICATE_P12_BASE64 is unset so unsigned
# releases still publish.
set -euo pipefail

if [[ -z "${APPLE_CERTIFICATE_P12_BASE64:-}" ]]; then
  echo "skip: APPLE_CERTIFICATE_P12_BASE64 not set; leaving binaries unsigned"
  exit 0
fi

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <file> [file...]" >&2
  exit 2
fi

: "${APPLE_CERTIFICATE_PASSWORD:?APPLE_CERTIFICATE_PASSWORD is required when a certificate is provided}"
: "${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY is required when a certificate is provided}"

TMP="${RUNNER_TEMP:-/tmp}/spec-chum-codesign"
mkdir -p "$TMP"
P12="$TMP/developer-id.p12"
KEYCHAIN="$TMP/signing.keychain-db"
KEYCHAIN_PASSWORD="$(openssl rand -base64 24)"

# macOS BSD base64 uses -D (GNU --decode is not available on runners).
echo "$APPLE_CERTIFICATE_P12_BASE64" | base64 -D >"$P12"

# macOS GitHub runners use bash 3.2 — avoid mapfile/associative arrays.
LOGIN_KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"
if [[ ! -e "$LOGIN_KEYCHAIN" ]]; then
  LOGIN_KEYCHAIN="${HOME}/Library/Keychains/login.keychain"
fi

cleanup() {
  security delete-keychain "$KEYCHAIN" 2>/dev/null || true
  security list-keychains -d user -s "$LOGIN_KEYCHAIN" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
# Import identity + private key; allow codesign without UI prompts.
security import "$P12" -P "$APPLE_CERTIFICATE_PASSWORD" -A -f pkcs12 \
  -k "$KEYCHAIN" -T /usr/bin/codesign
security list-keychains -d user -s "$KEYCHAIN" "$LOGIN_KEYCHAIN"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" \
  "$KEYCHAIN"

for f in "$@"; do
  echo "codesign $f"
  codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$f"
  codesign --verify --verbose "$f"
done

echo "signed $*"
