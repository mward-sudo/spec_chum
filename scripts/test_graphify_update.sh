#!/usr/bin/env bash
# No-network smoke test for graphify_update.sh provider routing and secret handling.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR/bin"

cat >"$TMP_DIR/bin/graphify" <<'FAKE_GRAPHIFY'
#!/usr/bin/env bash
set -euo pipefail
{
  printf 'ARGS=%s\n' "$*"
  printf 'BASE_URL=%s\n' "${OPENAI_BASE_URL:-}"
  printf 'API_KEY=%s\n' "${OPENAI_API_KEY:-}"
  printf 'OPENROUTER_API_KEY=%s\n' "${OPENROUTER_API_KEY:-}"
  printf 'GRAPHIFY_MODEL=%s\n' "${GRAPHIFY_OPENAI_MODEL:-}"
  printf 'OPENAI_MODEL=%s\n' "${OPENAI_MODEL:-}"
} >"$GRAPHIFY_TEST_CAPTURE"
FAKE_GRAPHIFY
chmod +x "$TMP_DIR/bin/graphify"
CLEAN_PATH="$TMP_DIR/bin:/usr/bin:/bin:/usr/sbin:/sbin"

env -i PATH="$CLEAN_PATH" GRAPHIFY_TEST_CAPTURE="$TMP_DIR/default" \
  "$ROOT/scripts/graphify_update.sh" >"$TMP_DIR/default-output"
grep -Fqx 'ARGS=update .' "$TMP_DIR/default"
grep -Fqx 'BASE_URL=' "$TMP_DIR/default"
grep -Fqx 'API_KEY=' "$TMP_DIR/default"
env -i PATH="$CLEAN_PATH" GRAPHIFY_TEST_CAPTURE="$TMP_DIR/openrouter-update" \
  GRAPHIFY_PROVIDER=openrouter \
  "$ROOT/scripts/graphify_update.sh" >"$TMP_DIR/openrouter-update-output"
grep -Fqx 'ARGS=update .' "$TMP_DIR/openrouter-update"
grep -Fqx 'BASE_URL=' "$TMP_DIR/openrouter-update"
grep -Fqx 'API_KEY=' "$TMP_DIR/openrouter-update"
grep -Fqx 'OPENROUTER_API_KEY=' "$TMP_DIR/openrouter-update"
env -i PATH="$CLEAN_PATH" GRAPHIFY_TEST_CAPTURE="$TMP_DIR/default-full" \
  "$ROOT/scripts/graphify_update.sh" --full >"$TMP_DIR/default-full-output"
grep -Fqx 'ARGS=.' "$TMP_DIR/default-full"

SECRET_SENTINEL='openrouter-test-secret-do-not-log'
env -i PATH="$CLEAN_PATH" \
  GRAPHIFY_TEST_CAPTURE="$TMP_DIR/openrouter" \
  GRAPHIFY_PROVIDER=openrouter \
  GRAPHIFY_OPENAI_MODEL=existing/model \
  OPENAI_MODEL=fallback/model \
  GRAPHIFY_OPENROUTER_MODEL=openrouter/model \
  bash -c 'IFS= read -r OPENROUTER_API_KEY; export OPENROUTER_API_KEY; exec "$1" --full' \
  _ "$ROOT/scripts/graphify_update.sh" <<<"$SECRET_SENTINEL" >"$TMP_DIR/openrouter-output"

grep -Fqx 'ARGS=extract . --backend openai' "$TMP_DIR/openrouter"
grep -Fqx 'BASE_URL=https://openrouter.ai/api/v1' "$TMP_DIR/openrouter"
grep -Fqx "API_KEY=$SECRET_SENTINEL" "$TMP_DIR/openrouter"
grep -Fqx 'OPENROUTER_API_KEY=' "$TMP_DIR/openrouter"
grep -Fqx 'GRAPHIFY_MODEL=openrouter/model' "$TMP_DIR/openrouter"
if grep -Fq "$SECRET_SENTINEL" "$TMP_DIR/openrouter-output" || \
  grep -Fq "$SECRET_SENTINEL" <(sed -n 's/^ARGS=//p' "$TMP_DIR/openrouter"); then
  echo 'error: API key appeared in Graphify output or arguments' >&2
  exit 1
fi

env -i PATH="$CLEAN_PATH" \
  GRAPHIFY_TEST_CAPTURE="$TMP_DIR/openrouter-key-update" \
  GRAPHIFY_PROVIDER=openrouter \
  bash -c 'IFS= read -r OPENROUTER_API_KEY; export OPENROUTER_API_KEY; exec "$1"' \
  _ "$ROOT/scripts/graphify_update.sh" <<<"$SECRET_SENTINEL" >"$TMP_DIR/openrouter-key-update-output"
grep -Fqx 'ARGS=update .' "$TMP_DIR/openrouter-key-update"
grep -Fqx 'API_KEY=' "$TMP_DIR/openrouter-key-update"
grep -Fqx 'OPENROUTER_API_KEY=' "$TMP_DIR/openrouter-key-update"
if grep -Fq "$SECRET_SENTINEL" "$TMP_DIR/openrouter-key-update-output"; then
  echo 'error: API key appeared in AST-only Graphify output' >&2
  exit 1
fi
if grep -R -Fq -- "$SECRET_SENTINEL" "$ROOT/graphify-out"; then
  echo 'error: API key appeared in graph artifacts' >&2
  exit 1
fi

if env -i PATH="$CLEAN_PATH" GRAPHIFY_PROVIDER=openrouter \
  GRAPHIFY_TEST_CAPTURE="$TMP_DIR/missing-key" \
  "$ROOT/scripts/graphify_update.sh" --full >"$TMP_DIR/missing-output" 2>&1; then
  echo 'error: OpenRouter mode succeeded without an API key' >&2
  exit 1
fi
grep -q 'requires OPENROUTER_API_KEY for --full' "$TMP_DIR/missing-output"
[[ ! -e "$TMP_DIR/missing-key" ]]

echo 'Graphify OpenRouter routing smoke test passed (no network request made).'
