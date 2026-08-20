#!/usr/bin/env bash
# Fail if unresolved bot review threads remain (CodeRabbit, etc.).
#
# Agents MUST run this before merging (and again after addressing feedback).
# CI runs the same gate as a PR check (see .github/workflows/pr-bot-reviews.yml).
# Lesson: https://github.com/mward-sudo/spec_chum/pull/83
#
# Usage:
#   ./scripts/check_pr_reviews.sh [PR_NUMBER]
#   ./scripts/check_pr_reviews.sh [PR_NUMBER] --waive "reason"
#   SPEC_CHUM_REVIEW_WAIVER="reason" ./scripts/check_pr_reviews.sh [PR_NUMBER]
#
# Waiver (local/CI): only when the user explicitly asked, OR PR has label
# `waive-bot-reviews`. Document the reason on the PR.
set -euo pipefail
cd "$(dirname "$0")/.."

WAIVER="${SPEC_CHUM_REVIEW_WAIVER:-}"
PR_ARG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --waive)
      WAIVER="${2:-}"
      shift 2
      ;;
    --waive=*)
      WAIVER="${1#--waive=}"
      shift
      ;;
    -h|--help)
      awk '/^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0" | tail -n +2
      exit 0
      ;;
    -*)
      echo "unknown option: $1" >&2
      echo "usage: $0 [PR_NUMBER] [--waive \"reason\"]" >&2
      exit 2
      ;;
    *)
      PR_ARG="$1"
      shift
      ;;
  esac
done

if [[ -n "$PR_ARG" ]]; then
  PR="$PR_ARG"
else
  PR="$(gh pr view --json number -q .number 2>/dev/null || true)"
fi
if [[ -z "${PR:-}" ]]; then
  echo "usage: $0 <pr-number> [--waive \"reason\"]" >&2
  echo "hint: pass the PR number, or run from a branch with an open PR" >&2
  exit 2
fi

OWNER_REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
OWNER="${OWNER_REPO%/*}"
REPO="${OWNER_REPO#*/}"

QUERY='
query($owner:String!, $name:String!, $number:Int!, $cursor:String) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      reviewThreads(first:100, after:$cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          isResolved
          isOutdated
          comments(first:1) {
            nodes {
              author { login }
              body
              path
              url
            }
          }
        }
      }
    }
  }
}'

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
cursor=""
: >"$tmp"

while true; do
  if [[ -n "$cursor" ]]; then
    page="$(gh api graphql -f query="$QUERY" -F owner="$OWNER" -F name="$REPO" -F number="$PR" -f cursor="$cursor")"
  else
    page="$(gh api graphql -f query="$QUERY" -F owner="$OWNER" -F name="$REPO" -F number="$PR")"
  fi
  if echo "$page" | jq -e '.errors? | select(length > 0)' >/dev/null 2>&1; then
    echo "GraphQL error querying review threads for PR #$PR:" >&2
    echo "$page" | jq '.errors' >&2
    exit 2
  fi
  echo "$page" | jq -c '
    .data.repository.pullRequest.reviewThreads.nodes[]
    | select(.isResolved == false)
    | .comments.nodes[0] as $c
    | select(($c.author.login // "") | test("coderabbit|bot"; "i"))
    | {
        path: ($c.path // "(no path)"),
        author: ($c.author.login // "unknown"),
        url: ($c.url // ""),
        preview: ($c.body // "" | split("\n") | map(select(length > 0)) | .[0:2] | join(" "))
      }
  ' >>"$tmp"
  has_next="$(echo "$page" | jq -r '.data.repository.pullRequest.reviewThreads.pageInfo.hasNextPage')"
  cursor="$(echo "$page" | jq -r '.data.repository.pullRequest.reviewThreads.pageInfo.endCursor // empty')"
  [[ "$has_next" == "true" ]] || break
done

count="$(wc -l <"$tmp" | tr -d ' ')"
if [[ "$count" -eq 0 ]]; then
  echo "==> PR #$PR: no unresolved bot review threads"
  exit 0
fi

echo "==> PR #$PR: $count unresolved bot review thread(s):"
echo
i=0
while IFS= read -r line; do
  i=$((i + 1))
  path="$(echo "$line" | jq -r .path)"
  author="$(echo "$line" | jq -r .author)"
  url="$(echo "$line" | jq -r .url)"
  preview="$(echo "$line" | jq -r .preview)"
  echo "  [$i] @$author — $path"
  if [[ -n "$preview" ]]; then
    echo "      $preview"
  fi
  if [[ -n "$url" ]]; then
    echo "      $url"
  fi
  echo
done <"$tmp"

# Label-based waiver (useful in CI when the user explicitly allowed merge).
if [[ -z "$WAIVER" ]]; then
  if gh pr view "$PR" --json labels -q '.labels[].name' 2>/dev/null | grep -Fxq 'waive-bot-reviews'; then
    WAIVER="PR label waive-bot-reviews present"
  fi
fi

if [[ -n "$WAIVER" ]]; then
  echo "==> WAIVED: $WAIVER"
  echo "    (document this waiver on the PR if merging)"
  exit 0
fi

echo "MERGE BLOCKED (lesson: PR #83 — do not ignore CodeRabbit)." >&2
echo "Unresolved actionable bot threads must be fixed or resolved first." >&2
echo >&2
echo "Next steps:" >&2
echo "  1. Open each URL above; fix in code or reply with a short wontfix reason." >&2
echo "  2. Resolve the thread on GitHub when dispositioned." >&2
echo "  3. Re-run: ./scripts/check_pr_reviews.sh $PR" >&2
echo "     (or re-run the \"Bot review threads\" GitHub Actions check)" >&2
echo >&2
echo "Only if the user explicitly waived:" >&2
echo "  ./scripts/check_pr_reviews.sh $PR --waive 'reason'" >&2
echo "  # or add PR label: waive-bot-reviews" >&2
echo >&2
echo "See: .cursor/rules/pr-review-merge.mdc" >&2
exit 1
