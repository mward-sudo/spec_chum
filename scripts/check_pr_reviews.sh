#!/usr/bin/env bash
# Before merge: fail if unresolved bot review threads remain (CodeRabbit, etc.).
# Not a required GitHub status check — agents/humans run this locally / in PR notes.
#
# Usage:
#   ./scripts/check_pr_reviews.sh [PR_NUMBER]
#   SPEC_CHUM_REVIEW_WAIVER="reason" ./scripts/check_pr_reviews.sh [PR_NUMBER]
#   ./scripts/check_pr_reviews.sh [PR_NUMBER] --waive "reason"
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
    -*)
      echo "unknown option: $1" >&2
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
  echo "$page" | jq -c '
    .data.repository.pullRequest.reviewThreads.nodes[]
    | select(.isResolved == false)
    | .comments.nodes[0] as $c
    | select(($c.author.login // "") | test("coderabbit|bot"; "i"))
    | {
        path: $c.path,
        author: $c.author.login,
        url: $c.url,
        preview: ($c.body | split("\n") | map(select(length > 0)) | .[0:3] | join(" | "))
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
sed 's/^/  /' "$tmp"

if [[ -n "$WAIVER" ]]; then
  echo "==> WAIVED: $WAIVER"
  echo "    (document this waiver on the PR if merging)"
  exit 0
fi

echo >&2
echo "Refuse merge until threads are fixed/resolved, or re-run with:" >&2
echo "  SPEC_CHUM_REVIEW_WAIVER='reason' $0 $PR" >&2
echo "  $0 $PR --waive 'reason'" >&2
exit 1
