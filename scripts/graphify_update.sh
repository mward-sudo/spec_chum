#!/usr/bin/env bash
# Incrementally refresh graphify-out/ after Rust or doc changes.
#
# Usage:
#   ./scripts/graphify_update.sh          # AST-only update (no LLM cost for code)
#   ./scripts/graphify_update.sh --full   # first-time or full rebuild (LLM for docs)
#   GRAPHIFY_PROVIDER=openrouter ./scripts/graphify_update.sh --full  # requires exported OPENROUTER_API_KEY
#
# When to use which:
#   - update (default): after editing .rs / other code — re-extracts changed files only.
#   - full rebuild: no graphify-out/graph.json yet, or you changed docs/papers/images and
#     need semantic re-extraction (set GEMINI_API_KEY or GOOGLE_API_KEY).
#
# Outputs committed under graphify-out/: graph.json, graph.html, GRAPH_REPORT.md,
# manifest.json, cost.json, cache/ (AST), .graphify_* metadata. See AGENTS.md.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v graphify >/dev/null 2>&1; then
  echo "error: graphify not found on PATH" >&2
  echo "install the current CLI with: uv tool install graphifyy" >&2
  echo "then ensure the uv tool bin directory is on PATH and verify with: command -v graphify && graphify --version" >&2
  exit 1
fi

if [[ ! -f graphify-out/graph.json ]]; then
  echo "error: graphify-out/graph.json missing — run a full build first:" >&2
  echo "  graphify ." >&2
  exit 1
fi

MODE=update
if [[ "${1:-}" == "--full" ]]; then
  MODE=full
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--full]" >&2
  exit 1
fi

# Keep the existing Graphify provider selection as the default. OpenRouter is
# opt-in and uses Graphify's OpenAI-compatible backend. Assign the key with a
# shell builtin so it is never included in a child process's argument list.
PROVIDER="${GRAPHIFY_PROVIDER:-default}"
case "$PROVIDER" in
  default)
    ;;
  openrouter)
    if [[ "$MODE" == "full" ]]; then
      if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
        echo "error: GRAPHIFY_PROVIDER=openrouter requires OPENROUTER_API_KEY for --full" >&2
        exit 1
      fi
      export OPENAI_BASE_URL="https://openrouter.ai/api/v1"
      export OPENAI_API_KEY="$OPENROUTER_API_KEY"
      unset OPENROUTER_API_KEY
      # Graphify prefers GRAPHIFY_OPENAI_MODEL over OPENAI_MODEL. Map the
      # provider-specific override there so it wins over inherited settings.
      if [[ -n "${GRAPHIFY_OPENROUTER_MODEL:-}" ]]; then
        export GRAPHIFY_OPENAI_MODEL="$GRAPHIFY_OPENROUTER_MODEL"
      fi
    else
      # Incremental AST refreshes do not use an LLM or need provider secrets.
      unset OPENROUTER_API_KEY
    fi
    ;;
  *)
    echo "error: unsupported GRAPHIFY_PROVIDER '$PROVIDER' (expected default or openrouter)" >&2
    exit 1
    ;;
esac

run_graphify() {
  if [[ "$MODE" == "full" ]]; then
    echo "==> graphify full rebuild (may use LLM for docs)"
    if [[ "$PROVIDER" == "openrouter" ]]; then
      graphify extract . --backend openai
    else
      graphify .
    fi
  else
    echo "==> graphify update (AST-only for code changes)"
    graphify update .
  fi
}

run_graphify

echo "==> graphify update complete (see graphify-out/GRAPH_REPORT.md)"
