#!/usr/bin/env bash
# Classify CodeRabbit commit-status for gate 1 of check_pr_reviews.sh.
#
# Usage (sourced):
#   classify_coderabbit_head_status <found> <state> <description>
#   Sets: CR_CLASSIFY_OUTCOME = pass | soft_pass | hold
#         CR_CLASSIFY_REASON  = human-readable detail (empty for pass)
#
# Outcomes:
#   pass      — description is "Review completed" (case-insensitive) and state=success
#   soft_pass — GitHub rate-limited / quota unavailable *after* a review was requested
#               (NOT a completed review). CI cannot see local CR; agents MUST still
#               confirm either (a) the other side completed cleanly, or (b) BOTH local
#               and GitHub are rate-limited with reported resets >10m (or long/unknown
#               on both) before merge. Soft-passes for success and failure states —
#               CR may report rate-limit either way. Soft-pass ≠ on-demand skip.
#   hold      — pending / missing / error / unexpected / non-completed success /
#               on-demand or label skips (never requested — same as missing)
classify_coderabbit_head_status() {
  local found="${1:-}"
  local state="${2:-}"
  local desc="${3:-}"
  local desc_lc
  desc_lc="$(printf '%s' "$desc" | tr '[:upper:]' '[:lower:]')"

  CR_CLASSIFY_OUTCOME="hold"
  CR_CLASSIFY_REASON=""

  if [[ "$found" != "true" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit has not reported a commit status on HEAD yet (pending / not started / on-demand not requested)."
    return 0
  fi
  if [[ "$state" == "pending" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit is still pending: ${desc:-pending}"
    return 0
  fi
  # On-demand / label skip: hard-fail (hold) BEFORE rate-limit soft-pass so a
  # description that somehow mentions both cannot soft-pass. Examples:
  #   "Review skipped: excluded by label configuration"
  #   "Review skipped: on demand"
  #   "Review skipped: …"
  # Lesson: PRs #278/#279 soft-passed these and merged without requesting CR.
  if [[ "$desc_lc" == *skip* || "$desc_lc" == *"excluded by label"* || "$desc_lc" == *"on demand"* ]]; then
    CR_CLASSIFY_REASON="CodeRabbit on-demand skip on HEAD (review never requested): ${desc:-no description}. Request '@coderabbitai full review' or label coderabbit-review."
    return 0
  fi
  # Soft-pass only for real quota unavailability after a request was made.
  # Intentionally soft-passes on failure as well as success — CodeRabbit has
  # reported "Review rate limited" under both states.
  if [[ "$desc_lc" == *rate*limit* || "$desc_lc" == *quota* || "$desc_lc" == *"too many request"* ]]; then
    CR_CLASSIFY_OUTCOME="soft_pass"
    CR_CLASSIFY_REASON="rate-limited"
    return 0
  fi
  if [[ "$state" == "failure" || "$state" == "error" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit status is ${state}: ${desc:-no description}"
    return 0
  fi
  if [[ "$state" != "success" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit status is unexpected (${state}: ${desc:-no description})."
    return 0
  fi
  if [[ "$desc_lc" != "review completed" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit is not a completed review (status=\"${desc:-none}\"; state=${state}). Waiting for \"Review completed\"."
    return 0
  fi
  CR_CLASSIFY_OUTCOME="pass"
  CR_CLASSIFY_REASON=""
  return 0
}
