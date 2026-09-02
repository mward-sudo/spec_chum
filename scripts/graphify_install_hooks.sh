#!/usr/bin/env bash
# Install graphify git hooks (post-commit AST refresh, merge driver for graph.json).
#
# Run once per clone / worktree checkout. Hooks live in .git/hooks/ (not committed);
# this script is the repo-documented installer.
#
# Usage:
#   ./scripts/graphify_install_hooks.sh          # install
#   ./scripts/graphify_install_hooks.sh status   # check
#   ./scripts/graphify_install_hooks.sh uninstall
#
# After install, every git commit that touches code files triggers `graphify update`
# (AST-only, no LLM). Doc-only changes are skipped by the hook — run
# ./scripts/graphify_update.sh --full manually when needed.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v graphify >/dev/null 2>&1; then
  echo "error: graphify not found on PATH" >&2
  exit 1
fi

ACTION="${1:-install}"
case "$ACTION" in
  install)
    echo "==> graphify hook install"
    graphify hook install
    ;;
  status)
    graphify hook status
    ;;
  uninstall)
    echo "==> graphify hook uninstall"
    graphify hook uninstall
    ;;
  *)
    echo "usage: $0 [install|status|uninstall]" >&2
    exit 1
    ;;
esac
