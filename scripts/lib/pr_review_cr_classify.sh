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
#   soft_pass — rate-limited or skipped (even if state=success); NOT a completed review
#   hold      — pending / missing / error / unexpected / non-completed success
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
  # Soft-pass before treating green success as completed — rate-limit/skip are often success.
  if [[ "$desc_lc" == *rate*limit* ]]; then
    CR_CLASSIFY_OUTCOME="soft_pass"
    CR_CLASSIFY_REASON="rate-limited"
    return 0
  fi
  if [[ "$desc_lc" == *skip* ]]; then
    CR_CLASSIFY_OUTCOME="soft_pass"
    CR_CLASSIFY_REASON="skipped"
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
