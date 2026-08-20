#!/usr/bin/env bash
# Fail if CodeRabbit has not completed a review on HEAD (ready PRs only), or
# unresolved bot review threads remain (CodeRabbit, etc.).
#
# Agents MUST run this before merging (and again after addressing feedback).
# CI runs the same gate as a PR check (see .github/workflows/pr-bot-reviews.yml).
# Lesson: https://github.com/mward-sudo/spec_chum/pull/83
#
# Draft vs ready (aligns with on-demand CodeRabbit usage):
#   - Draft PRs: skip CodeRabbit HEAD completeness (CR not required yet).
#     Still fail on unresolved bot threads if any exist.
#   - Ready / non-draft: hold until CodeRabbit completed on HEAD, then threads.
#
# Hold until clean (ready PRs): green "Review rate limited" or "Review skipped"
# commit statuses are NOT a pass — the gate can otherwise look clean while CR
# never actually reviewed HEAD.
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

# Label-based waiver (useful in CI when the user explicitly allowed merge).
if [[ -z "$WAIVER" ]]; then
  if gh pr view "$PR" --json labels -q '.labels[].name' 2>/dev/null | grep -Fxq 'waive-bot-reviews'; then
    WAIVER="PR label waive-bot-reviews present"
  fi
fi

apply_waiver_or_fail() {
  local reason="$1"
  shift
  if [[ -n "$WAIVER" ]]; then
    echo "==> WAIVED ($reason): $WAIVER"
    echo "    (document this waiver on the PR if merging)"
    return 0
  fi
  echo "MERGE BLOCKED (lesson: PR #83 — hold until CodeRabbit is clean)." >&2
  echo "$reason" >&2
  if [[ $# -gt 0 ]]; then
    echo >&2
    for line in "$@"; do
      echo "$line" >&2
    done
  fi
  echo >&2
  echo "Only if the user explicitly waived:" >&2
  echo "  ./scripts/check_pr_reviews.sh $PR --waive 'reason'" >&2
  echo "  # or add PR label: waive-bot-reviews" >&2
  echo >&2
  echo "See: .cursor/rules/pr-review-merge.mdc" >&2
  exit 1
}

# --- Gate 1: CodeRabbit completed on HEAD (ready / non-draft PRs only) -------
PR_META="$(gh pr view "$PR" --json headRefOid,isDraft -q '{sha:.headRefOid,draft:.isDraft}')"
HEAD_SHA="$(echo "$PR_META" | jq -r .sha)"
IS_DRAFT="$(echo "$PR_META" | jq -r .draft)"
if [[ ! "$HEAD_SHA" =~ ^[0-9a-f]{7,40}$ ]]; then
  echo "Could not resolve head SHA for PR #$PR" >&2
  exit 2
fi

if [[ "$IS_DRAFT" == "true" ]]; then
  echo "==> PR #$PR: draft — CodeRabbit HEAD completeness not required yet"
  echo "    (still checking unresolved bot threads; when merge-candidate: mark ready,"
  echo "     request @coderabbitai full review or label coderabbit-review, then re-run)"
else
  # Combined status endpoint returns newest-first statuses for each context.
  CR_JSON="$(gh api "repos/${OWNER}/${REPO}/commits/${HEAD_SHA}/status" --jq '
    [.statuses[] | select((.context // "") | test("^CodeRabbit$"; "i"))]
    | if length == 0 then
        {found:false}
      else
        .[0] | {found:true, state:(.state // ""), description:(.description // "")}
      end
  ')"

  CR_FOUND="$(echo "$CR_JSON" | jq -r .found)"
  CR_STATE="$(echo "$CR_JSON" | jq -r '.state // empty')"
  CR_DESC="$(echo "$CR_JSON" | jq -r '.description // empty')"
  CR_DESC_LC="$(printf '%s' "$CR_DESC" | tr '[:upper:]' '[:lower:]')"

  cr_hold_reason=""
  if [[ "$CR_FOUND" != "true" ]]; then
    cr_hold_reason="CodeRabbit has not reported a commit status on HEAD ${HEAD_SHA:0:12} yet (pending / not started / on-demand not requested)."
  elif [[ "$CR_STATE" == "pending" ]]; then
    cr_hold_reason="CodeRabbit is still pending on HEAD ${HEAD_SHA:0:12}: ${CR_DESC:-pending}"
  elif [[ "$CR_DESC_LC" == *rate*limit* ]]; then
    # CodeRabbit often marks rate-limited as success — do not treat as a completed review.
    cr_hold_reason="CodeRabbit is rate-limited on HEAD ${HEAD_SHA:0:12} (status=\"${CR_DESC}\"; state=${CR_STATE}). Full re-review did not run."
  elif [[ "$CR_DESC_LC" == *skip* ]]; then
    # On-demand / label / draft skips can be green — still not a completed review on HEAD.
    cr_hold_reason="CodeRabbit skipped review on HEAD ${HEAD_SHA:0:12} (status=\"${CR_DESC}\"; state=${CR_STATE}). Request @coderabbitai full review (or label coderabbit-review)."
  elif [[ "$CR_STATE" == "failure" || "$CR_STATE" == "error" ]]; then
    cr_hold_reason="CodeRabbit status on HEAD ${HEAD_SHA:0:12} is ${CR_STATE}: ${CR_DESC:-no description}"
  elif [[ "$CR_STATE" != "success" ]]; then
    cr_hold_reason="CodeRabbit status on HEAD ${HEAD_SHA:0:12} is unexpected (${CR_STATE}: ${CR_DESC:-no description})."
  elif [[ "$CR_DESC_LC" != "review completed" ]]; then
    # Only the known final description counts — empty / "in progress" success must not pass.
    cr_hold_reason="CodeRabbit on HEAD ${HEAD_SHA:0:12} is not a completed review (status=\"${CR_DESC:-none}\"; state=${CR_STATE}). Waiting for \"Review completed\"."
  fi

  if [[ -n "$cr_hold_reason" ]]; then
    echo "==> PR #$PR: CodeRabbit hold on HEAD ${HEAD_SHA:0:12}"
    echo "    context=CodeRabbit state=${CR_STATE:-missing} description=${CR_DESC:-"(none)"}"
    apply_waiver_or_fail "$cr_hold_reason" \
      "Next steps:" \
      "  1. Hold the PR — do not merge while CodeRabbit is pending, in progress, missing, skipped, or rate-limited." \
      "  2. If reviews are on-demand: comment '@coderabbitai full review' (or add label coderabbit-review)." \
      "  3. Wait for a completed CodeRabbit review on the current HEAD (description like \"Review completed\")." \
      "  4. Open a follow-up issue if rate-limited (e.g. \"Revisit CodeRabbit on PR #${PR}\")." \
      "  5. Re-run: ./scripts/check_pr_reviews.sh $PR" \
      "     (or re-run the \"Bot review threads\" GitHub Actions check)"
  fi

  echo "==> PR #$PR: CodeRabbit completed on HEAD ${HEAD_SHA:0:12} (${CR_DESC:-success})"
fi

# --- Gate 2: unresolved bot review threads -----------------------------------
# Inspect every comment in each unresolved thread (not only nodes[0]): a human
# may open the thread and a bot reply later; first-comment-only would miss it.
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
          comments(first:100) {
            pageInfo { hasNextPage endCursor }
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

COMMENTS_PAGE_QUERY='
query($id:ID!, $cursor:String!) {
  node(id:$id) {
    ... on PullRequestReviewThread {
      comments(first:100, after:$cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          author { login }
          body
          path
          url
        }
      }
    }
  }
}'

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
cursor=""
: >"$tmp"

append_bot_thread_from_comments() {
  local comments_json="$1"
  echo "$comments_json" | jq -c '
    [.[] | select((.author.login // "") | test("coderabbit|bot"; "i"))]
    | .[0] // empty
    | select(.)
    | {
        path: (.path // "(no path)"),
        author: (.author.login // "unknown"),
        url: (.url // ""),
        preview: (.body // "" | split("\n") | map(select(length > 0)) | .[0:2] | join(" "))
      }
  ' >>"$tmp"
}

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

  while IFS= read -r thread_json; do
    [[ -z "$thread_json" ]] && continue
    thread_id="$(echo "$thread_json" | jq -r .id)"
    comments_json="$(echo "$thread_json" | jq -c '.comments.nodes')"
    c_has_next="$(echo "$thread_json" | jq -r '.comments.pageInfo.hasNextPage')"
    c_cursor="$(echo "$thread_json" | jq -r '.comments.pageInfo.endCursor // empty')"
    while [[ "$c_has_next" == "true" && -n "$c_cursor" ]]; do
      cpage="$(gh api graphql -f query="$COMMENTS_PAGE_QUERY" -f id="$thread_id" -f cursor="$c_cursor")"
      if echo "$cpage" | jq -e '.errors? | select(length > 0)' >/dev/null 2>&1; then
        echo "GraphQL error paginating review-thread comments for PR #$PR:" >&2
        echo "$cpage" | jq '.errors' >&2
        exit 2
      fi
      comments_json="$(jq -c -n --argjson a "$comments_json" --argjson b "$(echo "$cpage" | jq -c '.data.node.comments.nodes')" '$a + $b')"
      c_has_next="$(echo "$cpage" | jq -r '.data.node.comments.pageInfo.hasNextPage')"
      c_cursor="$(echo "$cpage" | jq -r '.data.node.comments.pageInfo.endCursor // empty')"
    done
    append_bot_thread_from_comments "$comments_json"
  done < <(echo "$page" | jq -c '.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved == false)')

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

apply_waiver_or_fail \
  "Unresolved actionable bot threads must be fixed or resolved first." \
  "Next steps:" \
  "  1. Open each URL above; fix in code or reply with a short wontfix reason." \
  "  2. Resolve the thread on GitHub when dispositioned." \
  "  3. Re-run: ./scripts/check_pr_reviews.sh $PR" \
  "     (or re-run the \"Bot review threads\" GitHub Actions check)"
