#!/usr/bin/env bash
# Fail if CodeRabbit has not completed a review on HEAD (ready PRs only), or
# unresolved bot review threads remain (CodeRabbit, etc.).
#
# Agents MUST run this before merging (and again after addressing feedback).
# CI runs the same gate as a PR check (see .github/workflows/pr-bot-reviews.yml).
# Ruleset context is the API-published *commit status* "unresolved bot threads"
# (check-run with the same name is supplemental for Checks UI; #323). The
# Actions job is named bot-review-gate so cancelled runs cannot block merge
# after a newer soft-pass (#318). CI re-invokes this script on CodeRabbit
# *commit status* changes (`on: status` in pr-bot-reviews.yml) so pending →
# completed/rate-limited clears the published required status without a manual
# re-run (#312 lacked that trigger until #314; #318 keeps publish authoritative).
# Lesson: https://github.com/mward-sudo/spec_chum/pull/83
#
# Draft vs ready (aligns with on-demand CodeRabbit usage):
#   - Draft PRs: skip CodeRabbit HEAD completeness (CR not required yet).
#     Still fail on unresolved bot threads if any exist.
#   - Ready / non-draft: prefer CodeRabbit "Review completed" on HEAD, then
#     check unresolved bot threads.
#
# Soft-pass (gate 1 only, CI / GitHub status): "Review rate limited" (or similar
# quota unavailability *after* a review was requested) is NOT "Review completed".
# CI parses reset minutes from the status description when reliable
# ("Next included review available in N minutes", etc.):
#   - parsed N ≤ 10  → HOLD (fail) so humans/agents wait/retry
#   - parsed N > 10  → soft-pass CI (loud warning)
#   - unparsable     → soft-pass CI (loud warning; do not invent brittle false reds)
# CI cannot observe local CodeRabbit. Merge is only OK when agents confirm ONE of:
#   (a) Prefer-both / either-completed: local CR completed cleanly (or GitHub
#       "Review completed" — this path is not a rate-limit soft-pass), OR
#   (b) Dual rate-limit: BOTH local CLI and GitHub are rate-limited AND reported
#       resets are >10 minutes (or long/unknown on both) — then soft-pass + revisit
#       issue (no rmw). If EITHER side reports reset ≤10 minutes (or can review
#       now), WAIT/retry that side — do not soft-pass-merge yet.
# Gate 2 (unresolved bot threads) still hard-fails.
#
# Hard-fail (gate 1): pending / missing / error / unexpected / non-completed
# success that is not rate-limited; rate-limited with parsed reset ≤10m; also
# on-demand / label skips ("Review skipped: excluded by label configuration",
# "Review skipped: on demand", etc.) — those mean the review was never requested
# (same as missing).
#
# Usage:
#   ./scripts/check_pr_reviews.sh [PR_NUMBER]
#   ./scripts/check_pr_reviews.sh [PR_NUMBER] --waive "reason"
#   ./scripts/check_pr_reviews.sh --self-test
#   SPEC_CHUM_REVIEW_WAIVER="reason" ./scripts/check_pr_reviews.sh [PR_NUMBER]
#
# Waiver (local/CI): only when the user explicitly asked, OR PR has label
# `waive-bot-reviews`. Document the reason on the PR.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

# shellcheck source=lib/pr_review_cr_classify.sh
source "$SCRIPT_DIR/lib/pr_review_cr_classify.sh"

WAIVER="${SPEC_CHUM_REVIEW_WAIVER:-}"
PR_ARG=""
SELF_TEST=0
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
    --self-test)
      SELF_TEST=1
      shift
      ;;
    -h|--help)
      awk '/^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0" | tail -n +2
      exit 0
      ;;
    -*)
      echo "unknown option: $1" >&2
      echo "usage: $0 [PR_NUMBER] [--waive \"reason\"] | --self-test" >&2
      exit 2
      ;;
    *)
      PR_ARG="$1"
      shift
      ;;
  esac
done

if [[ "$SELF_TEST" -eq 1 ]]; then
  fail=0
  expect() {
    local found="$1" state="$2" desc="$3" want_out="$4" label="$5"
    classify_coderabbit_head_status "$found" "$state" "$desc"
    if [[ "$CR_CLASSIFY_OUTCOME" != "$want_out" ]]; then
      echo "FAIL: $label — got $CR_CLASSIFY_OUTCOME want $want_out (state=$state desc=$desc)" >&2
      fail=1
    else
      echo "ok: $label → $CR_CLASSIFY_OUTCOME"
    fi
  }
  expect true success "Review completed" pass "completed"
  expect true success "REVIEW COMPLETED" pass "completed case-insensitive"
  expect true success "Review rate limited" soft_pass "rate-limited unparsable success"
  expect true failure "Review rate limited" soft_pass "rate-limited unparsable failure"
  expect true success "Rate limit: too many requests" soft_pass "too-many-requests"
  expect true failure "quota exceeded" soft_pass "quota failure"
  expect true success "Review rate limited. Next included review available in 45 minutes" soft_pass "rate-limited >10m"
  expect true failure "Review rate limited. Next included review available in 11 minutes" soft_pass "rate-limited 11m"
  expect true success "Review rate limited. Next included review available in 10 minutes" hold "rate-limited =10m hold"
  expect true success "Review rate limited. Next included review available in 5 minutes" hold "rate-limited <10m hold"
  expect true failure "quota exceeded; available in 1 hour" soft_pass "rate-limited 1h"
  expect true success "Review skipped: on demand" hold "on-demand skip"
  expect true success "Review skipped: excluded by label configuration" hold "label-config skip"
  expect true success "Review skipped: something else" hold "generic skip"
  expect true success "Review skipped: rate limited" hold "skip beats rate-limit"
  expect true pending "Review queued" hold "queued pending"
  expect true pending "Review in progress" hold "in-progress pending"
  expect true pending "Queued" hold "pending"
  expect false "" "" hold "missing"
  expect true error "boom" hold "error"
  expect true failure "Review failed" hold "failure non-rate-limit"
  expect true success "In progress" hold "non-completed success"
  expect true success "" hold "empty success description"
  # Status→PR mapping: only open PRs whose head SHA matches (workflow uses this).
  map_n="$(printf '%s' '[{"state":"open","head":{"sha":"aaa"},"number":1,"updated_at":"2020-01-01"},{"state":"open","head":{"sha":"bbb"},"number":2,"updated_at":"2021-01-01"},{"state":"closed","head":{"sha":"bbb"},"number":3,"updated_at":"2022-01-01"}]' \
    | jq -r --arg sha bbb '[.[] | select(.state == "open" and .head.sha == $sha)] | sort_by(.updated_at) | reverse | .[0].number // empty')"
  if [[ "$map_n" != "2" ]]; then
    echo "FAIL: status→PR head map — got ${map_n:-empty} want 2" >&2
    fail=1
  else
    echo "ok: status→PR head SHA map → $map_n"
  fi
  map_empty="$(printf '%s' '[{"state":"open","head":{"sha":"aaa"},"number":1,"updated_at":"2020-01-01"}]' \
    | jq -r --arg sha zzz '[.[] | select(.state == "open" and .head.sha == $sha)] | sort_by(.updated_at) | reverse | .[0].number // empty')"
  if [[ -n "$map_empty" ]]; then
    echo "FAIL: status→PR head map should be empty for non-head — got $map_empty" >&2
    fail=1
  else
    echo "ok: status→PR head SHA map empty for non-matching"
  fi
  # Workflow must publish a classic commit status (ruleset) with statuses: write (#323).
  # Scope greps to permissions + bot-review-gate publish step (avoid comment/doc matches).
  wf=".github/workflows/pr-bot-reviews.yml"
  if [[ ! -f "$wf" ]]; then
    echo "FAIL: missing $wf for publish-path self-test" >&2
    fail=1
  else
    if ! grep -qE '^[[:space:]]*statuses:[[:space:]]*write[[:space:]]*$' "$wf"; then
      echo "FAIL: $wf must grant statuses: write for commit-status publish" >&2
      fail=1
    else
      echo "ok: workflow statuses: write"
    fi
    if ! grep -Fq 'name: bot-review-gate' "$wf"; then
      echo "FAIL: $wf job name must remain bot-review-gate (≠ ruleset context)" >&2
      fail=1
    else
      echo "ok: job name bot-review-gate"
    fi
    # Status→PR: gh api --jq accepts one expression only — do not pass --arg (#331).
    resolve_step="$(awk '
      $0 ~ /^[[:space:]]*name: bot-review-gate[[:space:]]*$/ { in_job=1 }
      in_job && $0 ~ /^[[:space:]]*- name: Resolve PR number[[:space:]]*$/ { in_res=1 }
      in_res { print }
      in_res && $0 ~ /^[[:space:]]*- name: Resolve PR head SHA[[:space:]]*$/ { exit }
    ' "$wf")"
    if [[ -z "$resolve_step" ]]; then
      echo "FAIL: $wf missing bot-review-gate Resolve PR number step" >&2
      fail=1
    elif printf '%s\n' "$resolve_step" | grep -qE -- '--jq[[:space:]]+--arg'; then
      echo "FAIL: Resolve PR number must not pass --arg through gh api --jq (#331)" >&2
      fail=1
    elif ! printf '%s\n' "$resolve_step" | grep -qE -- '\|[[:space:]]*jq[[:space:]].*--arg[[:space:]]+sha'; then
      echo "FAIL: Resolve PR number must pipe gh api JSON to jq --arg sha (#331)" >&2
      fail=1
    else
      echo "ok: status→PR uses jq --arg, not gh api --jq --arg"
    fi
    # Extract Publish status step body from the gate job only.
    publish_step="$(awk '
      $0 ~ /^[[:space:]]*name: bot-review-gate[[:space:]]*$/ { in_job=1 }
      in_job && $0 ~ /^[[:space:]]*- name: Publish status on PR head[[:space:]]*$/ { in_pub=1 }
      in_pub { print }
      in_pub && $0 ~ /^[[:space:]]*- name: Fail job when gate failed[[:space:]]*$/ { exit }
    ' "$wf")"
    if [[ -z "$publish_step" ]]; then
      echo "FAIL: $wf missing bot-review-gate Publish status on PR head step" >&2
      fail=1
    else
      if ! printf '%s\n' "$publish_step" | grep -Fq 'repos/${GITHUB_REPOSITORY}/statuses/${HEAD_SHA}'; then
        echo "FAIL: publish step must POST statuses/\${HEAD_SHA}" >&2
        fail=1
      elif ! printf '%s\n' "$publish_step" | grep -Fq "context='unresolved bot threads'"; then
        echo "FAIL: publish step must set status context exactly unresolved bot threads" >&2
        fail=1
      else
        echo "ok: publish step POSTs statuses/HEAD_SHA with context unresolved bot threads"
      fi
    fi
  fi
  if [[ "$fail" -ne 0 ]]; then
    echo "self-test failed" >&2
    exit 1
  fi
  echo "==> check_pr_reviews self-test passed"
  exit 0
fi

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
  echo "     request @coderabbitai full review (or label), then after fixes @coderabbitai review)"
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

  classify_coderabbit_head_status "$CR_FOUND" "$CR_STATE" "$CR_DESC"

  case "$CR_CLASSIFY_OUTCOME" in
    soft_pass)
      # Loud warning: GitHub rate-limit alone is NOT enough to merge.
      # CI cannot see local CR; agents must confirm either-completed or dual >10m.
      echo "==> PR #$PR: WARNING: CodeRabbit on HEAD ${HEAD_SHA:0:12} is ${CR_CLASSIFY_REASON}" >&2
      echo "    context=CodeRabbit state=${CR_STATE:-missing} description=${CR_DESC:-"(none)"}" >&2
      echo "    GitHub CodeRabbit rate-limited — gate 1 soft-passes CI only (NOT Review completed)." >&2
      echo "    Soft-pass when reset is >10m (parsed) or unparsable; HOLD when parsed ≤10m." >&2
      echo "    CI cannot verify local CodeRabbit. Before merge, agents MUST confirm ONE of:" >&2
      echo "      (a) Either-completed: local CR finished cleanly (Cursor plugin / coderabbit" >&2
      echo "          review --agent), OR GitHub later reaches Review completed; OR" >&2
      echo "      (b) Dual rate-limit: BOTH local and GitHub are rate-limited AND reported" >&2
      echo "          resets are >10 minutes (or long/unknown on both) — then soft-pass +" >&2
      echo "          revisit issue (no rmw). If either side resets in ≤10 minutes (or can" >&2
      echo "          review now), WAIT/retry that side — do not soft-pass-merge yet." >&2
      echo "    Unresolved bot threads still hard-fail (gate 2). Soft-pass ≠ on-demand skip." >&2
      ;;
    hold)
      cr_hold_reason="CodeRabbit hold on HEAD ${HEAD_SHA:0:12}: ${CR_CLASSIFY_REASON}"
      echo "==> PR #$PR: CodeRabbit hold on HEAD ${HEAD_SHA:0:12}"
      echo "    context=CodeRabbit state=${CR_STATE:-missing} description=${CR_DESC:-"(none)"}"
      apply_waiver_or_fail "$cr_hold_reason" \
        "Next steps:" \
        "  1. Hold the PR — do not merge while CodeRabbit is pending, in progress, missing, or errored." \
        "  2. If reviews are on-demand: first pass '@coderabbitai full review' (or label coderabbit-review); after fixes '@coderabbitai review'." \
        "  3. Wait for a completed CodeRabbit review on the current HEAD (description like \"Review completed\")." \
        "  4. On-demand / label skips (\"excluded by label configuration\", \"on demand\") hard-fail — request a review; do not merge without one." \
        "  5. If GitHub is rate-limited after a request: CI soft-passes when reset is >10m or unparsable; HOLD when parsed reset ≤10m. Merge needs either-completed (local clean or Review completed) OR dual rate-limit (both sides >10m / long-unknown) + revisit issue; if either side ≤10m, wait/retry. Unresolved threads remain a hard fail." \
        "  6. Re-run: ./scripts/check_pr_reviews.sh $PR" \
        "     (or re-run the \"Bot review threads\" GitHub Actions check)"
      ;;
    pass)
      echo "==> PR #$PR: CodeRabbit completed on HEAD ${HEAD_SHA:0:12} (${CR_DESC:-success})"
      ;;
    *)
      echo "internal error: unknown CR_CLASSIFY_OUTCOME=$CR_CLASSIFY_OUTCOME" >&2
      exit 2
      ;;
  esac
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
