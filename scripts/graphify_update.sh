#!/usr/bin/env bash
# Incrementally refresh graphify-out/ after Rust or doc changes.
#
# Usage:
#   ./scripts/graphify_update.sh          # AST-only update (no LLM cost for code)
#   ./scripts/graphify_update.sh --full   # first-time or full rebuild (LLM for docs)
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
  echo "error: graphify not found on PATH (install: pip install graphify or see graphify docs)" >&2
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

if [[ "$MODE" == "full" ]]; then
  echo "==> graphify full rebuild (may use LLM for docs)"
  graphify .
else
  echo "==> graphify update (AST-only for code changes)"
  graphify update .
fi

echo "==> graphify update complete (see graphify-out/GRAPH_REPORT.md)"
